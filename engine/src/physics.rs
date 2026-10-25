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

use std::cell::Cell;
use std::collections::HashMap;

use glam::{Quat, Vec3};
use parry3d::math::Pose;
use parry3d::query::{self, Ray, ShapeCastOptions};
use parry3d::shape::{Shape, SharedShape};

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
    pub fn add_convex_hull(&mut self, points: &[Vec3], groups: u32, tag: u32) -> Option<ColliderId> {
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

    pub fn add_shape(&mut self, shape: SharedShape, pose: Pose, groups: u32, tag: u32) -> ColliderId {
        let aabb = shape.compute_aabb(&pose);
        let id = self.colliders.len() as u32;
        let (aabb_min, aabb_max) = (aabb.mins, aabb.maxs);
        for cell in cells_overlapping(aabb_min, aabb_max) {
            self.grid.entry(cell).or_default().push(id);
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
            let Ok(Some(hit)) = query::cast_shapes(
                &start_pose,
                motion,
                shape,
                &collider.pose,
                Vec3::ZERO,
                collider.shape.as_ref(),
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

    /// Casts a ray. `dir` does not need to be normalized; `max_distance` is in world units.
    pub fn cast_ray(&self, origin: Vec3, dir: Vec3, max_distance: f32, mask: u32) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO || max_distance <= 0.0 {
            return None;
        }
        let ray = Ray::new(origin, dir);
        let mut best: Option<RayHit> = None;
        let mut limit = max_distance;

        // Walk the grid cells the ray passes through (2D DDA on XZ) so long rays stay cheap.
        let stamp = self.next_stamp();
        let mut cell = cell_of(origin);
        let step_x = if dir.x > 0.0 { 1 } else { -1 };
        let step_z = if dir.z > 0.0 { 1 } else { -1 };
        let next_boundary = |c: i32, step: i32| -> f32 {
            (if step > 0 { c + 1 } else { c }) as f32 * CELL_SIZE
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
        let t_delta_x = if dir.x.abs() > 1e-8 { CELL_SIZE / dir.x.abs() } else { f32::INFINITY };
        let t_delta_z = if dir.z.abs() > 1e-8 { CELL_SIZE / dir.z.abs() } else { f32::INFINITY };
        let mut t_cell_start = 0.0f32;

        loop {
            if let Some(ids) = self.grid.get(&cell) {
                for &id in ids {
                    let collider = &self.colliders[id as usize];
                    if !collider.enabled || collider.groups & mask == 0 {
                        continue;
                    }
                    let s = &self.stamps[id as usize];
                    if s.get() == stamp {
                        continue;
                    }
                    s.set(stamp);
                    if let Some(hit) =
                        collider
                            .shape
                            .cast_ray_and_get_normal(&collider.pose, &ray, limit, true)
                    {
                        limit = hit.time_of_impact;
                        best = Some(RayHit {
                            distance: hit.time_of_impact,
                            point: origin + dir * hit.time_of_impact,
                            normal: hit.normal.normalize_or(Vec3::Y),
                            collider: ColliderId(id),
                            tag: collider.tag,
                        });
                    }
                }
            }
            let t_cell_end = t_max_x.min(t_max_z);
            if t_cell_start > limit || t_cell_end > limit || !t_cell_end.is_finite() {
                break;
            }
            t_cell_start = t_cell_end;
            if t_max_x < t_max_z {
                cell.0 += step_x;
                t_max_x += t_delta_x;
            } else {
                cell.1 += step_z;
                t_max_z += t_delta_z;
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
        let pose = Pose::from_translation(center);
        let aabb = shape.compute_aabb(&pose);
        let mut hit = false;
        self.for_each_candidate(aabb.mins, aabb.maxs, mask, |_, collider| {
            if !hit {
                hit = query::intersection_test(&pose, shape, &collider.pose, collider.shape.as_ref())
                    .is_ok_and(|r| r.intersecting);
            }
        });
        hit
    }

    pub fn intersects_capsule(&self, center: Vec3, half_height: f32, radius: f32, mask: u32) -> bool {
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
        let pose = Pose::from_translation(center);
        let aabb = shape.compute_aabb(&pose);
        let lo = aabb.mins - Vec3::splat(prediction);
        let hi = aabb.maxs + Vec3::splat(prediction);
        let mut out = Vec::new();
        self.for_each_candidate(lo, hi, mask, |id, collider| {
            if let Ok(Some(c)) =
                query::contact(&pose, shape, &collider.pose, collider.shape.as_ref(), prediction)
            {
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

    fn for_each_candidate(
        &self,
        lo: Vec3,
        hi: Vec3,
        mask: u32,
        mut f: impl FnMut(u32, &Collider),
    ) {
        let stamp = self.next_stamp();
        for cell in cells_overlapping(lo, hi) {
            let Some(ids) = self.grid.get(&cell) else { continue };
            for &id in ids {
                let s = &self.stamps[id as usize];
                if s.get() == stamp {
                    continue;
                }
                s.set(stamp);
                let c = &self.colliders[id as usize];
                if !c.enabled || c.groups & mask == 0 {
                    continue;
                }
                if c.aabb_max.cmplt(lo).any() || c.aabb_min.cmpgt(hi).any() {
                    continue;
                }
                f(id, c);
            }
        }
    }
}

fn cell_of(p: Vec3) -> (i32, i32) {
    (
        (p.x / CELL_SIZE).floor() as i32,
        (p.z / CELL_SIZE).floor() as i32,
    )
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
            .cast_capsule(Vec3::new(0.0, 1.0, 0.0), 0.5, 0.4, Vec3::new(10.0, 0.0, 0.0), 0.0, MASK_MOVEMENT)
            .expect("hit");
        // Capsule surface touches the wall at x = 5 when its centre is at x = 4.6.
        assert!((hit.fraction * 10.0 - 4.6).abs() < 1e-2, "fraction {}", hit.fraction);
        assert!(hit.normal.dot(-Vec3::X) > 0.99);
    }

    #[test]
    fn capsule_resting_on_floor_can_move_up() {
        let w = floor_world();
        let center = Vec3::new(0.0, 0.9, -3.0);
        assert!(w.cast_capsule(center, 0.5, 0.4, Vec3::Y, 0.0, MASK_MOVEMENT).is_none());
    }

    #[test]
    fn disabled_colliders_are_ignored() {
        let mut w = floor_world();
        w.set_enabled(ColliderId(1), false);
        let hit = w
            .cast_ray(Vec3::new(-19.0, 1.0, 0.0), Vec3::X, 100.0, MASK_SHOTS);
        assert!(hit.is_none());
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
