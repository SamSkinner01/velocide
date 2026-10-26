//! Collision queries against static (and toggleable) level geometry.
//!
//! This is *not* a physics simulation: there are no rigid bodies. The game's kinematic character
//! controller, projectiles and hitscan weapons ask this world questions ("if I sweep this capsule
//! by this motion, what do I hit first?") and decide what to do themselves, which keeps movement
//! deterministic and fully under our control.
//!
//! Colliders are bucketed into a uniform XZ grid for broadphase. Each collider carries a `groups`
//! bitmask; queries pass a `mask` and only colliders with `groups & mask != 0` are considered.
//! Colliders can be disabled at runtime (e.g. an arena door that opens).
//!
//! **Robustness** (levels and tuning can come from a remote server): colliders covering more
//! than [`MAX_CELLS_PER_COLLIDER`] grid cells are kept in an "oversize" list that every query
//! checks directly instead of being inserted into every cell; colliders with a non-finite
//! bounding box are never hit. Queries with non-finite inputs report nothing, a query box
//! spanning more cells than there are colliders scans the collider list instead of the cells,
//! and rays are limited to [`MAX_RAY_DISTANCE`] and stop once they leave the grid.

use std::cell::Cell;
use std::collections::HashMap;

use glam::{Mat3, Quat, Vec3};
use parry3d::math::Pose;
use parry3d::query::{self, Ray, ShapeCastOptions};
use parry3d::shape::{Cuboid, Shape, SharedShape};

/// Solid level geometry: blocks players, enemies, projectiles and bullets.
pub const GROUP_WORLD: u32 = 1 << 0;
/// Blocks players and enemies but not bullets/projectiles (e.g. invisible clip walls).
pub const GROUP_PLAYER_CLIP: u32 = 1 << 1;
/// Doors of arena encounters (toggled on/off).
pub const GROUP_DOOR: u32 = 1 << 2;

/// Mask used by character movement.
pub const MASK_MOVEMENT: u32 = GROUP_WORLD | GROUP_PLAYER_CLIP | GROUP_DOOR;
/// Mask used by bullets, projectiles and line-of-sight checks.
pub const MASK_SHOTS: u32 = GROUP_WORLD | GROUP_DOOR;

const CELL_SIZE: f32 = 8.0;

/// Colliders whose bounding box covers more grid cells than this are not inserted into the
/// grid; every query tests them directly (a few huge floors are cheap to test, a brush with
/// 100 km extents in every 8 m cell would exhaust memory).
pub const MAX_CELLS_PER_COLLIDER: i64 = 1024;
/// Rays are clamped to this length (metres).
pub const MAX_RAY_DISTANCE: f32 = 1.0e5;

/// Margin around the query region kept when clipping large boxes (see [`clip_box`]).
const CLIP_MARGIN: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColliderId(pub u32);

pub struct Collider {
    pub shape: SharedShape,
    pub pose: Pose,
    pub aabb_min: Vec3,
    pub aabb_max: Vec3,
    pub groups: u32,
    pub enabled: bool,
    /// Free-form user data (e.g. surface material index or door id).
    pub tag: u32,
}

/// Result of sweeping a shape through the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepHit {
    /// Fraction of the requested motion that can be travelled before touching, in `[0, 1]`.
    pub fraction: f32,
    /// Surface normal of the obstacle, pointing back towards the moving shape.
    pub normal: Vec3,
    /// Contact point on the obstacle (world space).
    pub point: Vec3,
    pub collider: ColliderId,
    pub tag: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub collider: ColliderId,
    pub tag: u32,
}

/// A penetration (or near contact) between a query shape and a collider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactInfo {
    /// Points out of the obstacle, towards the query shape.
    pub normal: Vec3,
    /// Signed distance; negative means penetrating by that much.
    pub distance: f32,
    pub collider: ColliderId,
}

#[derive(Default)]
pub struct CollisionWorld {
    colliders: Vec<Collider>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// Colliders too large for the grid (see [`MAX_CELLS_PER_COLLIDER`]); tested by every query.
    oversize: Vec<u32>,
    /// Smallest / largest grid cell holding a collider (`None` while the grid is empty).
    grid_bounds: Option<((i32, i32), (i32, i32))>,
    /// Per-collider stamp used to avoid testing a collider twice in one query.
    stamps: Vec<Cell<u32>>,
    stamp: Cell<u32>,
}

impl CollisionWorld {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.colliders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colliders.is_empty()
    }

    pub fn collider(&self, id: ColliderId) -> &Collider {
        &self.colliders[id.0 as usize]
    }

    pub fn colliders(&self) -> impl Iterator<Item = (ColliderId, &Collider)> {
        self.colliders
            .iter()
            .enumerate()
            .map(|(i, c)| (ColliderId(i as u32), c))
    }

    /// Adds an oriented box.
    pub fn add_box(
        &mut self,
        center: Vec3,
        half_extents: Vec3,
        rotation: Quat,
        groups: u32,
        tag: u32,
    ) -> ColliderId {
        let shape = SharedShape::cuboid(half_extents.x, half_extents.y, half_extents.z);
        self.add_shape(shape, Pose::from_parts(center, rotation), groups, tag)
    }

    /// Adds a convex hull of the given world-space points (wedges, ramps, pillars...).
    pub fn add_convex_hull(
        &mut self,
        points: &[Vec3],
        groups: u32,
        tag: u32,
    ) -> Option<ColliderId> {
        let shape = SharedShape::convex_hull(points)?;
        Some(self.add_shape(shape, Pose::IDENTITY, groups, tag))
    }

    /// Adds a static triangle mesh in world space (e.g. from a glTF level).
    pub fn add_trimesh(
        &mut self,
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
        groups: u32,
        tag: u32,
    ) -> Option<ColliderId> {
        let shape = SharedShape::trimesh(vertices, indices).ok()?;
        Some(self.add_shape(shape, Pose::IDENTITY, groups, tag))
    }

    pub fn add_shape(
        &mut self,
        shape: SharedShape,
        pose: Pose,
        groups: u32,
        tag: u32,
    ) -> ColliderId {
        let aabb = shape.compute_aabb(&pose);
        let id = self.colliders.len() as u32;
        let (aabb_min, aabb_max) = (aabb.mins, aabb.maxs);
        if !aabb_min.is_finite() || !aabb_max.is_finite() {
            // Kept (ids stay dense) but never queried.
            log::warn!("collider {id} has a non-finite bounding box; it will never be hit");
        } else if cell_count(aabb_min, aabb_max) > MAX_CELLS_PER_COLLIDER {
            self.oversize.push(id);
        } else {
            let (lo, hi) = (cell_of(aabb_min), cell_of(aabb_max));
            for cell in cells_overlapping(aabb_min, aabb_max) {
                self.grid.entry(cell).or_default().push(id);
            }
            self.grid_bounds = Some(match self.grid_bounds {
                None => (lo, hi),
                Some((a, b)) => (
                    (a.0.min(lo.0), a.1.min(lo.1)),
                    (b.0.max(hi.0), b.1.max(hi.1)),
                ),
            });
        }
        self.colliders.push(Collider {
            shape,
            pose,
            aabb_min,
            aabb_max,
            groups,
            enabled: true,
            tag,
        });
        self.stamps.push(Cell::new(0));
        ColliderId(id)
    }

    pub fn set_enabled(&mut self, id: ColliderId, enabled: bool) {
        self.colliders[id.0 as usize].enabled = enabled;
    }

    pub fn is_enabled(&self, id: ColliderId) -> bool {
        self.colliders[id.0 as usize].enabled
    }

    /// Sweeps `shape` (axis-aligned, centred at `start`) along `motion`.
    ///
    /// `skin` keeps the shape that far away from surfaces: the returned fraction stops the shape
    /// once it is within `skin` of the obstacle. Initially-penetrating contacts whose motion
    /// separates are ignored so shapes can always move out of geometry.
    pub fn cast_shape(
        &self,
        shape: &dyn Shape,
        start: Vec3,
        motion: Vec3,
        skin: f32,
        mask: u32,
    ) -> Option<SweepHit> {
        if !start.is_finite() || !motion.is_finite() || !skin.is_finite() {
            return None;
        }
        let start_pose = Pose::from_translation(start);
        let local_aabb = shape.compute_local_aabb();
        let end = start + motion;
        let lo = start.min(end) + local_aabb.mins - Vec3::splat(skin);
        let hi = start.max(end) + local_aabb.maxs + Vec3::splat(skin);
        let options = ShapeCastOptions {
            max_time_of_impact: 1.0,
            target_distance: skin,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };

        let mut best: Option<SweepHit> = None;
        self.for_each_candidate(lo, hi, mask, |id, collider| {
            let clipped = clip_box(collider, lo, hi);
            let (shape2, pose2) = query_shape(collider, &clipped);
            let Ok(Some(hit)) = query::cast_shapes(
                &start_pose,
                motion,
                shape,
                pose2,
                Vec3::ZERO,
                shape2,
                options,
            ) else {
                return;
            };
            if best.is_some_and(|b| b.fraction <= hit.time_of_impact) {
                return;
            }
            // `start_pose` has no rotation, so the query frame equals world orientation and the
            // witness point is relative to `start`.
            let normal = (-hit.normal1).normalize_or(Vec3::Y);
            best = Some(SweepHit {
                fraction: hit.time_of_impact.clamp(0.0, 1.0),
                normal,
                point: start + hit.witness1 + motion * hit.time_of_impact,
                collider: ColliderId(id),
                tag: collider.tag,
            });
        });
        best
    }

    /// Sweeps a vertical capsule. `center` is the capsule centre, `half_height` is half the
    /// length of the inner segment (total height = 2 * (half_height + radius)).
    pub fn cast_capsule(
        &self,
        center: Vec3,
        half_height: f32,
        radius: f32,
        motion: Vec3,
        skin: f32,
        mask: u32,
    ) -> Option<SweepHit> {
        let capsule = parry3d::shape::Capsule::new_y(half_height, radius);
        self.cast_shape(&capsule, center, motion, skin, mask)
    }

    pub fn cast_sphere(
        &self,
        center: Vec3,
        radius: f32,
        motion: Vec3,
        mask: u32,
    ) -> Option<SweepHit> {
        let ball = parry3d::shape::Ball::new(radius);
        self.cast_shape(&ball, center, motion, 0.0, mask)
    }

    /// Casts a ray. `dir` does not need to be normalized; `max_distance` is in world units
    /// (clamped to [`MAX_RAY_DISTANCE`]). Non-finite inputs never hit.
    pub fn cast_ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_distance: f32,
        mask: u32,
    ) -> Option<RayHit> {
        if !origin.is_finite() || !dir.is_finite() || max_distance.is_nan() {
            return None;
        }
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO || max_distance <= 0.0 {
            return None;
        }
        let ray = Ray::new(origin, dir);
        let mut best: Option<RayHit> = None;
        let mut limit = max_distance.min(MAX_RAY_DISTANCE);
        let stamp = self.next_stamp();
        let mut test = |id: u32, limit: &mut f32| {
            let collider = &self.colliders[id as usize];
            if !collider.enabled || collider.groups & mask == 0 {
                return;
            }
            let s = &self.stamps[id as usize];
            if s.get() == stamp {
                return;
            }
            s.set(stamp);
            if let Some(hit) =
                collider
                    .shape
                    .cast_ray_and_get_normal(&collider.pose, &ray, *limit, true)
            {
                *limit = hit.time_of_impact;
                best = Some(RayHit {
                    distance: hit.time_of_impact,
                    point: origin + dir * hit.time_of_impact,
                    normal: hit.normal.normalize_or(Vec3::Y),
                    collider: ColliderId(id),
                    tag: collider.tag,
                });
            }
        };
        for &id in &self.oversize {
            test(id, &mut limit);
        }
        let Some((grid_lo, grid_hi)) = self.grid_bounds else {
            return best;
        };

        // Walk the grid cells the ray passes through (2D DDA on XZ) so long rays stay cheap.
        let mut cell = cell_of(origin);
        let step_x = if dir.x > 0.0 { 1 } else { -1 };
        let step_z = if dir.z > 0.0 { 1 } else { -1 };
        let next_boundary = |c: i32, step: i32| -> f32 {
            (if step > 0 { c as i64 + 1 } else { c as i64 }) as f32 * CELL_SIZE
        };
        let mut t_max_x = if dir.x.abs() > 1e-8 {
            (next_boundary(cell.0, step_x) - origin.x) / dir.x
        } else {
            f32::INFINITY
        };
        let mut t_max_z = if dir.z.abs() > 1e-8 {
            (next_boundary(cell.1, step_z) - origin.z) / dir.z
        } else {
            f32::INFINITY
        };
        let t_delta_x = if dir.x.abs() > 1e-8 {
            CELL_SIZE / dir.x.abs()
        } else {
            f32::INFINITY
        };
        let t_delta_z = if dir.z.abs() > 1e-8 {
            CELL_SIZE / dir.z.abs()
        } else {
            f32::INFINITY
        };
        // A cell outside the grid's range on an axis the ray moves away from (or doesn't move
        // along) can never lead back into the grid.
        let gone = |c: i32, step: i32, moves: bool, lo: i32, hi: i32| {
            (c < lo && (step < 0 || !moves)) || (c > hi && (step > 0 || !moves))
        };
        let mut t_cell_start = 0.0f32;
        // A ray of length `limit` crosses at most this many cells (each step moves one cell
        // along one axis): a hard bound on the loop whatever the floats do.
        let max_steps = (2.0 * limit / CELL_SIZE) as usize + 4;

        for _ in 0..max_steps {
            if gone(cell.0, step_x, t_delta_x.is_finite(), grid_lo.0, grid_hi.0)
                || gone(cell.1, step_z, t_delta_z.is_finite(), grid_lo.1, grid_hi.1)
            {
                break;
            }
            if let Some(ids) = self.grid.get(&cell) {
                for &id in ids {
                    test(id, &mut limit);
                }
            }
            let t_cell_end = t_max_x.min(t_max_z);
            if t_cell_start > limit || t_cell_end > limit || !t_cell_end.is_finite() {
                break;
            }
            t_cell_start = t_cell_end;
            // Far from the origin an f32 sum can stop changing: no progress, stop.
            if t_max_x < t_max_z {
                cell.0 = cell.0.saturating_add(step_x);
                let next = t_max_x + t_delta_x;
                if next <= t_max_x {
                    break;
                }
                t_max_x = next;
            } else {
                cell.1 = cell.1.saturating_add(step_z);
                let next = t_max_z + t_delta_z;
                if next <= t_max_z {
                    break;
                }
                t_max_z = next;
            }
        }
        best
    }

    /// True if there is any geometry between `a` and `b`.
    pub fn line_blocked(&self, a: Vec3, b: Vec3, mask: u32) -> bool {
        let d = b - a;
        let len = d.length();
        len > 1e-4 && self.cast_ray(a, d / len, len, mask).is_some()
    }

    /// True if `shape` placed at `center` overlaps any collider.
    pub fn intersects(&self, shape: &dyn Shape, center: Vec3, mask: u32) -> bool {
        if !center.is_finite() {
            return false;
        }
        let pose = Pose::from_translation(center);
        let aabb = shape.compute_aabb(&pose);
        let mut hit = false;
        self.for_each_candidate(aabb.mins, aabb.maxs, mask, |_, collider| {
            if !hit {
                let clipped = clip_box(collider, aabb.mins, aabb.maxs);
                let (shape2, pose2) = query_shape(collider, &clipped);
                hit = query::intersection_test(&pose, shape, pose2, shape2)
                    .is_ok_and(|r| r.intersecting);
            }
        });
        hit
    }

    pub fn intersects_capsule(
        &self,
        center: Vec3,
        half_height: f32,
        radius: f32,
        mask: u32,
    ) -> bool {
        let capsule = parry3d::shape::Capsule::new_y(half_height, radius);
        self.intersects(&capsule, center, mask)
    }

    /// All contacts within `prediction` distance of `shape` at `center` (used for depenetration).
    pub fn contacts(
        &self,
        shape: &dyn Shape,
        center: Vec3,
        prediction: f32,
        mask: u32,
    ) -> Vec<ContactInfo> {
        if !center.is_finite() || !prediction.is_finite() {
            return Vec::new();
        }
        let pose = Pose::from_translation(center);
        let aabb = shape.compute_aabb(&pose);
        let lo = aabb.mins - Vec3::splat(prediction);
        let hi = aabb.maxs + Vec3::splat(prediction);
        let mut out = Vec::new();
        self.for_each_candidate(lo, hi, mask, |id, collider| {
            let clipped = clip_box(collider, lo, hi);
            let (shape2, pose2) = query_shape(collider, &clipped);
            if let Ok(Some(c)) = query::contact(&pose, shape, pose2, shape2, prediction) {
                out.push(ContactInfo {
                    // normal1 points from the query shape towards the obstacle.
                    normal: (-c.normal1).normalize_or(Vec3::Y),
                    distance: c.dist,
                    collider: ColliderId(id),
                });
            }
        });
        out
    }

    pub fn capsule_contacts(
        &self,
        center: Vec3,
        half_height: f32,
        radius: f32,
        prediction: f32,
        mask: u32,
    ) -> Vec<ContactInfo> {
        let capsule = parry3d::shape::Capsule::new_y(half_height, radius);
        self.contacts(&capsule, center, prediction, mask)
    }

    fn next_stamp(&self) -> u32 {
        let s = self.stamp.get().wrapping_add(1).max(1);
        self.stamp.set(s);
        s
    }

    /// Calls `f` once for every enabled collider matching `mask` whose bounding box overlaps
    /// `lo..=hi` (nothing for a non-finite box).
    fn for_each_candidate(&self, lo: Vec3, hi: Vec3, mask: u32, mut f: impl FnMut(u32, &Collider)) {
        if !lo.is_finite() || !hi.is_finite() {
            return;
        }
        let stamp = self.next_stamp();
        let mut visit = |id: u32| {
            let s = &self.stamps[id as usize];
            if s.get() == stamp {
                return;
            }
            s.set(stamp);
            let c = &self.colliders[id as usize];
            if !c.enabled || c.groups & mask == 0 {
                return;
            }
            if !(c.aabb_max.cmpge(lo).all() && c.aabb_min.cmple(hi).all()) {
                return;
            }
            // Broken colliders are never hit (they are in no cell, but a full scan sees them).
            if !c.aabb_min.is_finite() || !c.aabb_max.is_finite() {
                return;
            }
            f(id, c);
        };
        for &id in &self.oversize {
            visit(id);
        }
        if cell_count(lo, hi) > self.colliders.len() as i64 {
            // A huge query box: testing every collider beats visiting billions of cells.
            for id in 0..self.colliders.len() as u32 {
                visit(id);
            }
            return;
        }
        for cell in cells_overlapping(lo, hi) {
            let Some(ids) = self.grid.get(&cell) else {
                continue;
            };
            for &id in ids {
                visit(id);
            }
        }
    }
}

/// GJK-based queries (shape casts, contacts, intersections) lose precision with large shapes
/// because their tolerances scale with the shapes' extents: against a 400 m floor brush a
/// capsule sweep that starts already penetrating can report no hit at all, and contact normals
/// come back off by several degrees. A query only cares about the part of a box near it, so
/// boxes are clipped (in their own frame) to the query region grown by [`CLIP_MARGIN`]. The
/// clipped-away faces lie outside anything the query can touch, so results are unchanged
/// except for precision. Returns `None` when clipping isn't needed.
fn clip_box(collider: &Collider, lo: Vec3, hi: Vec3) -> Option<(Cuboid, Pose)> {
    let he = collider.shape.as_cuboid()?.half_extents;
    let half = (hi - lo) * 0.5 + Vec3::splat(CLIP_MARGIN);
    let local_center = collider.pose.inverse_transform_point((lo + hi) * 0.5);
    let to_local = Mat3::from_quat(collider.pose.rotation.inverse());
    let local_half = Vec3::new(
        to_local.row(0).abs().dot(half),
        to_local.row(1).abs().dot(half),
        to_local.row(2).abs().dot(half),
    );
    let clo = (local_center - local_half).max(-he);
    let chi = (local_center + local_half).min(he);
    if clo.cmpge(chi).any() || (clo.cmple(-he) & chi.cmpge(he)).all() {
        return None;
    }
    let center = (clo + chi) * 0.5;
    Some((
        Cuboid::new((chi - clo) * 0.5),
        collider.pose * Pose::from_translation(center),
    ))
}

fn query_shape<'a>(
    collider: &'a Collider,
    clipped: &'a Option<(Cuboid, Pose)>,
) -> (&'a dyn Shape, &'a Pose) {
    match clipped {
        Some((cuboid, pose)) => (cuboid, pose),
        None => (collider.shape.as_ref(), &collider.pose),
    }
}

fn cell_of(p: Vec3) -> (i32, i32) {
    (
        (p.x / CELL_SIZE).floor() as i32,
        (p.z / CELL_SIZE).floor() as i32,
    )
}

/// Number of grid cells the box `lo..=hi` covers (computed without overflow).
fn cell_count(lo: Vec3, hi: Vec3) -> i64 {
    let (x0, z0) = cell_of(lo);
    let (x1, z1) = cell_of(hi);
    let nx = (x1 as i64 - x0 as i64 + 1).max(0);
    let nz = (z1 as i64 - z0 as i64 + 1).max(0);
    nx.saturating_mul(nz)
}

fn cells_overlapping(lo: Vec3, hi: Vec3) -> impl Iterator<Item = (i32, i32)> {
    let (x0, z0) = cell_of(lo);
    let (x1, z1) = cell_of(hi);
    (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| (x, z)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_world() -> CollisionWorld {
        let mut w = CollisionWorld::new();
        // 40x1x40 floor whose top surface is at y = 0.
        w.add_box(
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.5, 20.0),
            Quat::IDENTITY,
            GROUP_WORLD,
            0,
        );
        // A wall at x = 5.
        w.add_box(
            Vec3::new(5.5, 2.0, 0.0),
            Vec3::new(0.5, 2.0, 5.0),
            Quat::IDENTITY,
            GROUP_WORLD,
            1,
        );
        w
    }

    #[test]
    fn ray_hits_floor() {
        let w = floor_world();
        let hit = w
            .cast_ray(Vec3::new(0.0, 10.0, 0.0), -Vec3::Y, 100.0, MASK_SHOTS)
            .expect("hit");
        assert!((hit.distance - 10.0).abs() < 1e-3);
        assert!(hit.normal.dot(Vec3::Y) > 0.99);
    }

    #[test]
    fn long_ray_crosses_cells() {
        let w = floor_world();
        let hit = w
            .cast_ray(Vec3::new(-19.0, 1.0, 0.3), Vec3::X, 100.0, MASK_SHOTS)
            .expect("hit wall");
        assert_eq!(hit.tag, 1);
        assert!((hit.point.x - 5.0).abs() < 1e-3);
    }

    #[test]
    fn capsule_sweep_stops_at_wall() {
        let w = floor_world();
        let hit = w
            .cast_capsule(
                Vec3::new(0.0, 1.0, 0.0),
                0.5,
                0.4,
                Vec3::new(10.0, 0.0, 0.0),
                0.0,
                MASK_MOVEMENT,
            )
            .expect("hit");
        // Capsule surface touches the wall at x = 5 when its centre is at x = 4.6.
        assert!(
            (hit.fraction * 10.0 - 4.6).abs() < 1e-2,
            "fraction {}",
            hit.fraction
        );
        assert!(hit.normal.dot(-Vec3::X) > 0.99);
    }

    #[test]
    fn capsule_resting_on_floor_can_move_up() {
        let w = floor_world();
        let center = Vec3::new(0.0, 0.9, -3.0);
        assert!(
            w.cast_capsule(center, 0.5, 0.4, Vec3::Y, 0.0, MASK_MOVEMENT)
                .is_none()
        );
    }

    #[test]
    fn disabled_colliders_are_ignored() {
        let mut w = floor_world();
        w.set_enabled(ColliderId(1), false);
        let hit = w.cast_ray(Vec3::new(-19.0, 1.0, 0.0), Vec3::X, 100.0, MASK_SHOTS);
        assert!(hit.is_none());
    }

    #[test]
    fn huge_boxes_stay_precise() {
        // A 400 m floor: GJK on the full box misses hits that start inside the skin distance
        // and returns badly tilted normals; clipping the box to the query keeps it exact.
        let mut w = CollisionWorld::new();
        w.add_box(
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(200.0, 0.5, 200.0),
            Quat::IDENTITY,
            GROUP_WORLD,
            0,
        );
        for (x, z) in [(0.0, 0.0), (-0.0035, -0.0035), (3.7, -11.3), (123.4, -77.7)] {
            for gap in [-0.0056, -0.002, 0.002, 0.005, 0.008, 0.0099] {
                let c = Vec3::new(x, 0.9 + gap, z);
                let hit = w
                    .cast_capsule(
                        c,
                        0.5,
                        0.4,
                        Vec3::new(0.0, -0.0106, 0.0),
                        0.01,
                        MASK_MOVEMENT,
                    )
                    .unwrap_or_else(|| panic!("missed the floor at ({x}, {gap}, {z})"));
                assert_eq!(hit.fraction, 0.0);
                if gap > 0.001 {
                    assert!(hit.normal.y > 0.9999, "normal {} at gap {gap}", hit.normal);
                }
            }
            let contacts = w.capsule_contacts(Vec3::new(x, 0.91, z), 0.5, 0.4, 0.02, MASK_MOVEMENT);
            assert_eq!(contacts.len(), 1);
            assert!(contacts[0].normal.y > 0.99999 && (contacts[0].distance - 0.01).abs() < 1e-4);
        }
    }

    /// Runs `f` and fails if it takes longer than `secs` (the defects these guard against
    /// made a query take minutes or loop forever).
    fn quick<T>(secs: f32, f: impl FnOnce() -> T) -> T {
        let start = std::time::Instant::now();
        let out = f();
        let took = start.elapsed().as_secs_f32();
        assert!(took < secs, "took {took:.2} s");
        out
    }

    #[test]
    fn very_long_rays_terminate() {
        let w = floor_world();
        for len in [1.0e8, 1.3e8, 1.0e12, f32::MAX, f32::INFINITY] {
            // Through the wall: still hit.
            let hit = quick(1.0, || {
                w.cast_ray(Vec3::new(-19.0, 1.0, 0.3), Vec3::X, len, MASK_SHOTS)
            })
            .expect("hit wall");
            assert_eq!(hit.tag, 1);
            // Over everything, away from the grid: nothing, quickly.
            for dir in [
                Vec3::new(1.0, 0.2, 0.7),
                Vec3::new(-0.3, 0.01, -1.0),
                Vec3::Y,
            ] {
                let miss = quick(1.0, || {
                    w.cast_ray(Vec3::new(0.0, 30.0, 0.0), dir, len, MASK_SHOTS)
                });
                assert!(miss.is_none());
            }
        }
        // From far outside the grid (and beyond the i32 cell range) towards it.
        for origin in [Vec3::new(5.0e9, 1.0, 0.0), Vec3::new(1.0e30, 1.0, -1.0e30)] {
            let r = quick(1.0, || w.cast_ray(origin, -origin, f32::MAX, MASK_SHOTS));
            assert!(r.is_none(), "{r:?}");
        }
    }

    #[test]
    fn non_finite_queries_report_nothing() {
        let w = floor_world();
        let nan = Vec3::splat(f32::NAN);
        let inf = Vec3::new(f32::INFINITY, 0.0, 0.0);
        for bad in [nan, inf, -inf] {
            assert!(w.cast_ray(bad, Vec3::X, 10.0, MASK_SHOTS).is_none());
            assert!(
                w.cast_ray(Vec3::new(0.0, 1.0, 0.0), bad, 10.0, MASK_SHOTS)
                    .is_none()
            );
            assert!(w.cast_sphere(bad, 0.5, Vec3::X, MASK_SHOTS).is_none());
            assert!(
                w.cast_sphere(Vec3::new(0.0, 1.0, 0.0), 0.5, bad, MASK_SHOTS)
                    .is_none()
            );
            assert!(!w.intersects_capsule(bad, 0.5, 0.4, MASK_MOVEMENT));
            assert!(
                w.capsule_contacts(bad, 0.5, 0.4, 0.01, MASK_MOVEMENT)
                    .is_empty()
            );
        }
        assert!(
            w.cast_ray(Vec3::new(0.0, 1.0, 0.0), -Vec3::Y, f32::NAN, MASK_SHOTS)
                .is_none()
        );
        // A non-finite shape size: no hang, no hit.
        assert!(
            quick(1.0, || w.cast_sphere(
                Vec3::new(0.0, 1.0, 0.0),
                f32::NAN,
                Vec3::X,
                MASK_SHOTS
            ))
            .is_none()
        );
    }

    #[test]
    fn huge_sweeps_stay_fast() {
        // A projectile parried back and forth used to reach speeds whose swept box covered
        // billions of grid cells.
        let w = floor_world();
        for len in [1.0e6f32, 1.0e9, 1.0e15, 3.0e38] {
            let hit = quick(1.0, || {
                w.cast_sphere(Vec3::new(-19.0, 1.0, 0.3), 0.2, Vec3::X * len, MASK_SHOTS)
            });
            assert!(hit.is_some_and(|h| h.fraction < 1e-3), "{len}: {hit:?}");
            quick(1.0, || {
                w.capsule_contacts(Vec3::new(0.0, 0.8, 0.0), 0.5, 0.4, len, MASK_MOVEMENT)
            });
        }
    }

    #[test]
    fn oversize_and_broken_colliders() {
        let mut w = CollisionWorld::new();
        // 200 km floor: one collider, not 600 million cell entries.
        quick(1.0, || {
            w.add_box(
                Vec3::new(0.0, -0.5, 0.0),
                Vec3::new(1.0e5, 0.5, 1.0e5),
                Quat::IDENTITY,
                GROUP_WORLD,
                7,
            )
        });
        assert_eq!(w.oversize.len(), 1);
        assert!(w.grid.is_empty());
        // Broken colliders are kept (ids stay dense) but never hit.
        w.add_box(
            Vec3::splat(f32::NAN),
            Vec3::ONE,
            Quat::IDENTITY,
            GROUP_WORLD,
            8,
        );
        w.add_box(
            Vec3::ZERO,
            Vec3::splat(f32::INFINITY),
            Quat::IDENTITY,
            GROUP_WORLD,
            9,
        );
        w.add_box(
            Vec3::new(30.0, 1.0, 0.0),
            Vec3::ONE,
            Quat::IDENTITY,
            GROUP_WORLD,
            10,
        );
        assert_eq!(w.len(), 4);
        // Queries still see the huge floor (and the ordinary box) everywhere.
        for x in [0.0, 5.0e4, -9.0e4] {
            let hit = w
                .cast_ray(Vec3::new(x, 10.0, 3.0), -Vec3::Y, 100.0, MASK_SHOTS)
                .expect("floor");
            assert_eq!(hit.tag, 7);
            let c = w.capsule_contacts(Vec3::new(x, 0.85, 3.0), 0.5, 0.4, 0.1, MASK_MOVEMENT);
            assert_eq!(c.len(), 1);
            assert!(c[0].normal.y > 0.999);
        }
        let hit = w
            .cast_ray(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 100.0, MASK_SHOTS)
            .expect("box");
        assert_eq!(hit.tag, 10);
        assert!(w.intersects_capsule(Vec3::new(30.0, 1.0, 0.0), 0.5, 0.4, MASK_MOVEMENT));
        // A query so large it scans every collider still skips the broken ones.
        let all = quick(1.0, || {
            w.capsule_contacts(Vec3::new(0.0, 0.85, 3.0), 0.5, 0.4, 1.0e6, MASK_MOVEMENT)
        });
        let mut tags: Vec<u32> = all.iter().map(|c| w.collider(c.collider).tag).collect();
        tags.sort();
        assert_eq!(tags, [7, 10]);
    }

    #[test]
    fn contacts_report_penetration() {
        let w = floor_world();
        let contacts = w.capsule_contacts(Vec3::new(0.0, 0.8, -3.0), 0.5, 0.4, 0.01, MASK_MOVEMENT);
        assert_eq!(contacts.len(), 1);
        assert!(contacts[0].distance < 0.0);
        assert!(contacts[0].normal.dot(Vec3::Y) > 0.99);
    }
}
