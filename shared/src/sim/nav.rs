//! Navigation grid for ground enemies (server only).
//!
//! Built once per level from the collision world ([`NavGrid::build`]): the level's XZ bounds are
//! cut into [`CELL`] (1 m) columns; each column gets one **node per walkable floor** found by
//! downward ray casts started just above the top and just below the bottom of every collider
//! overlapping the column (so stacked floors, galleries and bridges over lava all get their
//! own node). A floor is
//!
//! * walkable when its slope is ≤ ~50° and an enemy-sized capsule (r [`SMALL_RADIUS`], from
//!   [`STEP_UP`] to [`SMALL_HEIGHT`] above it) fits there ([`node_flags::SMALL`]);
//! * **deadly** when it lies inside a kill volume (lava, pits): never walked on, and the edge
//!   guard never steps onto it;
//! * a **hazard** when a kill volume is within ~1.25 m (sideways, or a short drop away): walkable
//!   but expensive, so paths keep away from lava edges;
//! * **large** ([`node_flags::LARGE`], Bastion-sized agents) when it and all 8 neighbours are
//!   walkable at about the same height and the headroom is ≥ [`LARGE_HEADROOM`].
//!
//! Neighbouring walkable nodes are **linked** (8 directions) when the step up is ≤ [`STEP_UP`]
//! (or a continuous ramp up to ~50°), the drop is ≤ [`MAX_DROP`] (one-way: enemies can jump
//! down but not back up), and nothing blocks the way at knee/chest height. Diagonals need both
//! orthogonal routes (no corner cutting). Jump pads and gaps have no links (enemies can't use
//! pads). Door brushes are ignored while building; nodes under a door remember it and are
//! impassable while that door is closed.
//!
//! Queries: [`NavGrid::node_at`], [`NavGrid::find_path`] (A* with a node budget, partial
//! results), [`NavGrid::walkable_line`] (straight walk test through the links) used for direct
//! chasing and string-pulling ([`NavGrid::smooth`]), and [`NavGrid::ground_ok`] (the edge guard
//! that keeps enemies out of lava and off ledges). See `docs/enemies.md` for measurements.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::Instant;

use engine::physics::{CollisionWorld, GROUP_PLAYER_CLIP, GROUP_WORLD};
use glam::{Vec2, Vec3};

use crate::level::{LevelDef, Volume};

/// Cell size (metres).
pub const CELL: f32 = 1.0;
/// What blocks enemies (doors are handled per node).
pub const NAV_MASK: u32 = GROUP_WORLD | GROUP_PLAYER_CLIP;
/// Highest step an enemy walks up.
pub const STEP_UP: f32 = 0.45;
/// Deepest drop an enemy walks off (one-way link).
pub const MAX_DROP: f32 = 5.5;
/// Steepest walkable floor: normal.y ≥ cos(50°).
const MIN_FLOOR_NORMAL_Y: f32 = 0.64;
/// Clearance capsule of "small" agents (Husk, Spitter).
pub const SMALL_RADIUS: f32 = 0.5;
pub const SMALL_HEIGHT: f32 = 2.0;
/// Headroom "large" agents (Bastion) need.
pub const LARGE_HEADROOM: f32 = 3.4;
/// Kill volumes closer than this (sideways) make a node a hazard.
const HAZARD_DISTANCE: f32 = 1.25;
/// Extra path cost of a hazard node (metres).
const HAZARD_COST: f32 = 4.0;
/// Extra cost per metre of height change (prefer flat routes).
const CLIMB_COST: f32 = 0.5;
/// No node / no link.
pub const NONE: u32 = u32::MAX;
/// Refuse to build absurdly large grids (columns).
const MAX_COLUMNS: usize = 4_000_000;
/// Broadphase bucket size for the build (metres).
const BUCKET: f32 = 4.0;

/// Neighbour directions `(dx, dz)`: 4 orthogonal, then 4 diagonal.
pub const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

fn dir_index(dx: i32, dz: i32) -> Option<usize> {
    DIRS.iter().position(|d| *d == (dx, dz))
}

/// [`NavNode::flags`].
pub mod node_flags {
    /// Walkable for small agents (r ≤ 0.6, h ≤ 2.1).
    pub const SMALL: u8 = 1 << 0;
    /// Walkable for large agents (Bastion).
    pub const LARGE: u8 = 1 << 1;
    /// Next to a kill volume (walkable, expensive).
    pub const HAZARD: u8 = 1 << 2;
    /// Inside a kill volume: never stand here.
    pub const DEADLY: u8 = 1 << 3;
}

/// Agent size class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NavClass {
    Small,
    Large,
}

impl NavClass {
    /// The class an enemy capsule of this size needs.
    pub fn for_size(radius: f32, height: f32) -> NavClass {
        if radius <= SMALL_RADIUS + 0.1 && height <= SMALL_HEIGHT + 0.1 {
            NavClass::Small
        } else {
            NavClass::Large
        }
    }

    pub fn flag(self) -> u8 {
        match self {
            NavClass::Small => node_flags::SMALL,
            NavClass::Large => node_flags::LARGE,
        }
    }
}

/// One walkable (or deadly) floor in a column.
#[derive(Clone, Debug)]
pub struct NavNode {
    /// Floor point at the cell centre.
    pub pos: Vec3,
    pub flags: u8,
    /// Door slot + 1 (0 = none): impassable while that door is closed.
    pub door: u8,
    /// Free height above the floor (capped at 6 m).
    pub headroom: f32,
    /// Linked neighbour per [`DIRS`] entry ([`NONE`] = no link).
    pub links: [u32; 8],
}

impl NavNode {
    pub fn is(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// Build statistics.
#[derive(Clone, Debug, Default)]
pub struct NavStats {
    pub build_ms: f32,
    pub columns: usize,
    /// Floors found (including deadly / too cramped ones).
    pub floors: usize,
    pub small: usize,
    pub large: usize,
    pub hazard: usize,
    pub deadly: usize,
    pub links: usize,
    pub rays: usize,
    pub capsule_tests: usize,
}

/// Result of [`NavGrid::find_path`].
#[derive(Clone, Debug, PartialEq)]
pub enum PathResult {
    /// Node path from start to goal (both included).
    Found(Vec<u32>),
    /// The goal is unreachable (or the budget ran out): path to the reachable node closest to
    /// it.
    Partial(Vec<u32>),
    NotFound,
}

/// Reusable A* buffers (no allocation per search once warmed up).
#[derive(Default)]
pub struct NavScratch {
    g: Vec<f32>,
    parent: Vec<u32>,
    /// Search generation that last touched each node (`g`/`parent` valid when equal).
    seen: Vec<u32>,
    closed: Vec<u32>,
    generation: u32,
    heap: BinaryHeap<(Reverse<u32>, u32)>,
    /// Nodes expanded by the last search.
    pub last_expanded: usize,
}

/// The navigation grid of a level.
#[derive(Default)]
pub struct NavGrid {
    /// Min XZ corner of column (0, 0).
    pub origin: Vec2,
    pub nx: u32,
    pub nz: u32,
    /// `columns[c]..columns[c + 1]` = the nodes of column `c` (ascending height).
    columns: Vec<u32>,
    pub nodes: Vec<NavNode>,
    /// Door id per door slot (`NavNode::door - 1`).
    pub door_ids: Vec<u32>,
    pub stats: NavStats,
}

/// Collider AABBs bucketed on a coarse XZ grid (build-time broadphase).
struct Buckets {
    aabbs: Vec<(Vec3, Vec3)>,
    origin: Vec2,
    bx: i32,
    bz: i32,
    cells: Vec<Vec<u32>>,
}

impl Buckets {
    fn new(aabbs: Vec<(Vec3, Vec3)>, origin: Vec2, size: Vec2) -> Self {
        let bx = ((size.x / BUCKET).ceil() as i32).max(1);
        let bz = ((size.y / BUCKET).ceil() as i32).max(1);
        let mut cells = vec![Vec::new(); (bx * bz) as usize];
        for (i, (lo, hi)) in aabbs.iter().enumerate() {
            let (x0, z0) = (
                ((lo.x - origin.x) / BUCKET).floor() as i32,
                ((lo.z - origin.y) / BUCKET).floor() as i32,
            );
            let (x1, z1) = (
                ((hi.x - origin.x) / BUCKET).floor() as i32,
                ((hi.z - origin.y) / BUCKET).floor() as i32,
            );
            for x in x0.max(0)..=x1.min(bx - 1) {
                for z in z0.max(0)..=z1.min(bz - 1) {
                    cells[(x * bz + z) as usize].push(i as u32);
                }
            }
        }
        Self {
            aabbs,
            origin,
            bx,
            bz,
            cells,
        }
    }

    /// Calls `f` with every collider AABB overlapping the box (may repeat a collider when the
    /// box spans several buckets; callers only ask "any?" or collect heights).
    fn any_overlap(&self, lo: Vec3, hi: Vec3) -> bool {
        let x0 = ((lo.x - self.origin.x) / BUCKET).floor() as i32;
        let x1 = ((hi.x - self.origin.x) / BUCKET).floor() as i32;
        let z0 = ((lo.z - self.origin.y) / BUCKET).floor() as i32;
        let z1 = ((hi.z - self.origin.y) / BUCKET).floor() as i32;
        for x in x0.max(0)..=x1.min(self.bx - 1) {
            for z in z0.max(0)..=z1.min(self.bz - 1) {
                for &i in &self.cells[(x * self.bz + z) as usize] {
                    let (a, b) = self.aabbs[i as usize];
                    if a.cmple(hi).all() && b.cmpge(lo).all() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Colliders whose AABB contains the vertical line at (x, z).
    fn column(&self, x: f32, z: f32, out: &mut Vec<u32>) {
        out.clear();
        let bx = ((x - self.origin.x) / BUCKET).floor() as i32;
        let bz = ((z - self.origin.y) / BUCKET).floor() as i32;
        if bx < 0 || bz < 0 || bx >= self.bx || bz >= self.bz {
            return;
        }
        for &i in &self.cells[(bx * self.bz + bz) as usize] {
            let (a, b) = self.aabbs[i as usize];
            if x >= a.x - 1e-3 && x <= b.x + 1e-3 && z >= a.z - 1e-3 && z <= b.z + 1e-3 {
                out.push(i);
            }
        }
    }
}

fn horizontal_distance(v: &Volume, x: f32, z: f32) -> f32 {
    let dx = (v.min.x - x).max(0.0).max(x - v.max.x);
    let dz = (v.min.z - z).max(0.0).max(z - v.max.z);
    (dx * dx + dz * dz).sqrt()
}

/// AABB of a (possibly rotated) brush.
fn brush_aabb(b: &crate::level::Brush) -> (Vec3, Vec3) {
    let rot = glam::Mat3::from_quat(b.rotation_quat());
    let he = b.half_extents();
    let ext = Vec3::new(
        rot.row(0).abs().dot(he),
        rot.row(1).abs().dot(he),
        rot.row(2).abs().dot(he),
    );
    (b.center() - ext, b.center() + ext)
}

impl NavGrid {
    /// Builds the grid for `level` from its collision world (doors are ignored; see the module
    /// docs). Deterministic.
    pub fn build(level: &LevelDef, world: &CollisionWorld) -> NavGrid {
        let started = Instant::now();
        let mut stats = NavStats::default();
        let aabbs: Vec<(Vec3, Vec3)> = world
            .colliders()
            .filter(|(_, c)| c.groups & NAV_MASK != 0)
            .map(|(_, c)| (c.aabb_min, c.aabb_max))
            .collect();
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for (a, b) in &aabbs {
            lo = lo.min(*a);
            hi = hi.max(*b);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return NavGrid::default();
        }
        let origin = Vec2::new(lo.x.floor(), lo.z.floor());
        let nx = ((hi.x - origin.x) / CELL).ceil().max(1.0) as u32;
        let nz = ((hi.z - origin.y) / CELL).ceil().max(1.0) as u32;
        if (nx as usize) * (nz as usize) > MAX_COLUMNS {
            log::warn!("nav grid: level too large ({nx} x {nz} columns), no navigation");
            return NavGrid::default();
        }
        let buckets = Buckets::new(
            aabbs,
            origin,
            Vec2::new(nx as f32 * CELL, nz as f32 * CELL),
        );
        let ymin = lo.y - 0.5;
        let top = hi.y + 1.0;

        let mut grid = NavGrid {
            origin,
            nx,
            nz,
            columns: Vec::with_capacity((nx * nz + 1) as usize),
            nodes: Vec::new(),
            door_ids: Vec::new(),
            stats: NavStats::default(),
        };
        stats.columns = (nx * nz) as usize;

        // 1. Floors per column.
        let mut cands = Vec::new();
        let mut starts: Vec<f32> = Vec::new();
        let mut floors: Vec<(f32, f32)> = Vec::new();
        let r = SMALL_RADIUS;
        let cap_half = ((SMALL_HEIGHT - STEP_UP) * 0.5 - r).max(0.0);
        for i in 0..nx {
            for j in 0..nz {
                grid.columns.push(grid.nodes.len() as u32);
                let (x, z) = grid.column_center(i, j);
                buckets.column(x, z, &mut cands);
                if cands.is_empty() {
                    continue;
                }
                starts.clear();
                starts.push(top);
                for &c in &cands {
                    let (a, b) = buckets.aabbs[c as usize];
                    starts.push(b.y + 0.05);
                    if a.y - 0.05 > ymin {
                        starts.push(a.y - 0.05);
                    }
                }
                starts.sort_by(|a, b| b.total_cmp(a));
                starts.dedup_by(|a, b| (*a - *b).abs() < 0.02);
                floors.clear();
                for &s in &starts {
                    stats.rays += 1;
                    let Some(hit) =
                        world.cast_ray(Vec3::new(x, s, z), Vec3::NEG_Y, s - ymin, NAV_MASK)
                    else {
                        continue;
                    };
                    if hit.distance > 1e-4 && hit.normal.y >= MIN_FLOOR_NORMAL_Y {
                        floors.push((hit.point.y, hit.normal.y));
                    }
                }
                floors.sort_by(|a, b| a.0.total_cmp(&b.0));
                floors.dedup_by(|a, b| (a.0 - b.0).abs() < 0.05);
                for &(y, _) in &floors {
                    stats.floors += 1;
                    let feet = Vec3::new(x, y, z);
                    let mut flags = 0;
                    if level
                        .kill_volumes
                        .iter()
                        .any(|v| v.contains(feet + Vec3::Y * 0.1))
                    {
                        flags |= node_flags::DEADLY;
                    } else {
                        // Clearance (only ask the physics when some collider is close).
                        let clo = Vec3::new(x - r, y + STEP_UP, z - r);
                        let chi = Vec3::new(x + r, y + SMALL_HEIGHT, z + r);
                        let blocked = buckets.any_overlap(clo, chi) && {
                            stats.capsule_tests += 1;
                            let center = Vec3::new(x, y + (STEP_UP + SMALL_HEIGHT) * 0.5, z);
                            world.intersects_capsule(center, cap_half, r, NAV_MASK)
                        };
                        if !blocked {
                            flags |= node_flags::SMALL;
                        }
                        let hazard = level.kill_volumes.iter().any(|v| {
                            horizontal_distance(v, x, z) <= HAZARD_DISTANCE
                                && v.max.y >= y - MAX_DROP
                                && v.min.y <= y + 2.0
                        });
                        if hazard {
                            flags |= node_flags::HAZARD;
                        }
                    }
                    // Headroom: only cast when something is overhead.
                    let overhead = cands.iter().any(|&c| buckets.aabbs[c as usize].0.y > y + 0.05);
                    let headroom = if overhead {
                        stats.rays += 1;
                        world
                            .cast_ray(feet + Vec3::Y * 0.05, Vec3::Y, 6.0, NAV_MASK)
                            .map_or(6.0, |h| h.distance + 0.05)
                    } else {
                        6.0
                    };
                    grid.nodes.push(NavNode {
                        pos: feet,
                        flags,
                        door: 0,
                        headroom,
                        links: [NONE; 8],
                    });
                }
            }
        }
        grid.columns.push(grid.nodes.len() as u32);

        // 2. Orthogonal links.
        for n in 0..grid.nodes.len() {
            if !grid.nodes[n].is(node_flags::SMALL) {
                continue;
            }
            let (i, j) = grid.column_of_node(n);
            for (d, &(dx, dz)) in DIRS.iter().enumerate().take(4) {
                let Some(col) = grid.column_index(i as i32 + dx, j as i32 + dz) else {
                    continue;
                };
                if let Some(m) = grid.best_link(world, &buckets, n, col, &mut stats) {
                    grid.nodes[n].links[d] = m;
                    stats.links += 1;
                }
            }
        }
        // 3. Diagonal links: only where both orthogonal routes agree (no corner cutting).
        for n in 0..grid.nodes.len() {
            if !grid.nodes[n].is(node_flags::SMALL) {
                continue;
            }
            for (d, &(dx, dz)) in DIRS.iter().enumerate().skip(4) {
                let (Some(dxi), Some(dzi)) = (dir_index(dx, 0), dir_index(0, dz)) else {
                    continue;
                };
                let a = grid.nodes[n].links[dxi];
                let b = grid.nodes[n].links[dzi];
                if a == NONE || b == NONE {
                    continue;
                }
                let via_a = grid.nodes[a as usize].links[dzi];
                let via_b = grid.nodes[b as usize].links[dxi];
                if via_a != NONE && via_a == via_b {
                    grid.nodes[n].links[d] = via_a;
                    stats.links += 1;
                }
            }
        }

        // 4. Doors: nodes a closed door would block.
        for door in level.doors.iter().take(254) {
            let slot = grid.door_ids.len() as u8 + 1;
            grid.door_ids.push(door.id);
            let (a, b) = brush_aabb(&door.brush);
            let a = a - Vec3::new(r, 0.0, r);
            let b = b + Vec3::new(r, 0.0, r);
            let (i0, j0) = grid.column_coords(a.x, a.z);
            let (i1, j1) = grid.column_coords(b.x, b.z);
            for i in i0.max(0)..=i1.min(nx as i32 - 1) {
                for j in j0.max(0)..=j1.min(nz as i32 - 1) {
                    let Some(col) = grid.column_index(i, j) else {
                        continue;
                    };
                    for n in grid.column_nodes(col) {
                        let y = grid.nodes[n as usize].pos.y;
                        if y >= a.y - 1.0 && y <= b.y {
                            grid.nodes[n as usize].door = slot;
                        }
                    }
                }
            }
        }

        // 5. Large agents: walkable all around at about the same height, with headroom.
        let mut large = Vec::new();
        for (n, node) in grid.nodes.iter().enumerate() {
            if !node.is(node_flags::SMALL) || node.headroom < LARGE_HEADROOM {
                continue;
            }
            let ok = node.links.iter().all(|&m| {
                m != NONE && {
                    let o = &grid.nodes[m as usize];
                    (o.pos.y - node.pos.y).abs() <= 0.6 && o.headroom >= LARGE_HEADROOM - 0.4
                }
            });
            if ok {
                large.push(n);
            }
        }
        for n in large {
            grid.nodes[n].flags |= node_flags::LARGE;
        }

        for node in &grid.nodes {
            stats.small += node.is(node_flags::SMALL) as usize;
            stats.large += node.is(node_flags::LARGE) as usize;
            stats.hazard += node.is(node_flags::HAZARD) as usize;
            stats.deadly += node.is(node_flags::DEADLY) as usize;
        }
        stats.build_ms = started.elapsed().as_secs_f32() * 1000.0;
        grid.stats = stats;
        grid
    }

    /// The best node of column `col` that node `n` links to (closest height that obeys the
    /// step / ramp / drop rules with nothing in the way).
    fn best_link(
        &self,
        world: &CollisionWorld,
        buckets: &Buckets,
        n: usize,
        col: usize,
        stats: &mut NavStats,
    ) -> Option<u32> {
        let from = self.nodes[n].pos;
        let mut best: Option<(f32, u32)> = None;
        for m in self.column_nodes(col) {
            let node = &self.nodes[m as usize];
            if !node.is(node_flags::SMALL) {
                continue;
            }
            let to = node.pos;
            let dy = to.y - from.y;
            if dy < -MAX_DROP || dy > 1.2 * CELL {
                continue;
            }
            if best.is_some_and(|(b, _)| b <= dy.abs()) {
                continue;
            }
            if dy > STEP_UP {
                // Only a continuous ramp climbs more than a step.
                let mid = (from + to) * 0.5;
                stats.rays += 1;
                let ramp = world
                    .cast_ray(
                        Vec3::new(mid.x, to.y + 0.3, mid.z),
                        Vec3::NEG_Y,
                        dy + 0.8,
                        NAV_MASK,
                    )
                    .is_some_and(|h| {
                        h.distance > 1e-4
                            && h.normal.y >= MIN_FLOOR_NORMAL_Y
                            && (h.point.y - mid.y).abs() <= 0.2
                    });
                if !ramp {
                    continue;
                }
            }
            // Nothing in the way at knee and chest height above the higher floor.
            let base = from.y.max(to.y);
            let lo = Vec3::new(from.x.min(to.x), base + 0.6, from.z.min(to.z));
            let hi = Vec3::new(from.x.max(to.x), base + 1.6, from.z.max(to.z));
            if buckets.any_overlap(lo, hi) {
                stats.rays += 2;
                let blocked = [0.6, 1.6].iter().any(|&h| {
                    world.line_blocked(
                        Vec3::new(from.x, base + h, from.z),
                        Vec3::new(to.x, base + h, to.z),
                        NAV_MASK,
                    )
                });
                if blocked {
                    continue;
                }
            }
            best = Some((dy.abs(), m));
        }
        best.map(|(_, m)| m)
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn column_center(&self, i: u32, j: u32) -> (f32, f32) {
        (
            self.origin.x + (i as f32 + 0.5) * CELL,
            self.origin.y + (j as f32 + 0.5) * CELL,
        )
    }

    fn column_coords(&self, x: f32, z: f32) -> (i32, i32) {
        (
            ((x - self.origin.x) / CELL).floor() as i32,
            ((z - self.origin.y) / CELL).floor() as i32,
        )
    }

    fn column_index(&self, i: i32, j: i32) -> Option<usize> {
        (i >= 0 && j >= 0 && (i as u32) < self.nx && (j as u32) < self.nz)
            .then(|| (i as u32 * self.nz + j as u32) as usize)
    }

    fn column_nodes(&self, col: usize) -> std::ops::Range<u32> {
        match (self.columns.get(col), self.columns.get(col + 1)) {
            (Some(&a), Some(&b)) => a..b,
            _ => 0..0,
        }
    }

    fn column_of_node(&self, n: usize) -> (u32, u32) {
        let p = self.nodes[n].pos;
        let (i, j) = self.column_coords(p.x, p.z);
        (i.max(0) as u32, j.max(0) as u32)
    }

    /// Column index of a world position.
    fn column_at(&self, p: Vec3) -> Option<usize> {
        let (i, j) = self.column_coords(p.x, p.z);
        self.column_index(i, j)
    }

    /// Can `class` stand on node `n` with the doors as given?
    pub fn passable(&self, n: u32, class: NavClass, doors_open: &[bool]) -> bool {
        let node = &self.nodes[n as usize];
        node.is(class.flag())
            && (node.door == 0 || doors_open.get(node.door as usize - 1).copied().unwrap_or(true))
    }

    /// The node an agent at `pos` (feet) stands on: the highest passable floor in its column
    /// at most 0.6 m above and `max_below` below the feet; else the closest such node in the
    /// 8 neighbouring columns (agents brushing a wall).
    pub fn node_at(
        &self,
        pos: Vec3,
        max_below: f32,
        class: NavClass,
        doors_open: &[bool],
    ) -> Option<u32> {
        let (i, j) = self.column_coords(pos.x, pos.z);
        let pick = |col: usize| {
            self.column_nodes(col)
                .filter(|&n| {
                    let y = self.nodes[n as usize].pos.y;
                    y <= pos.y + 0.6 && y >= pos.y - max_below && self.passable(n, class, doors_open)
                })
                .max_by(|&a, &b| {
                    self.nodes[a as usize]
                        .pos
                        .y
                        .total_cmp(&self.nodes[b as usize].pos.y)
                })
        };
        if let Some(n) = self.column_index(i, j).and_then(pick) {
            return Some(n);
        }
        let mut best: Option<(f32, u32)> = None;
        for &(dx, dz) in &DIRS {
            if let Some(n) = self.column_index(i + dx, j + dz).and_then(pick) {
                let d = self.nodes[n as usize].pos.distance_squared(pos);
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, n));
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// Closest passable node within `max_cells` columns (spiral search), preferring floors at
    /// most 3 m from `pos` vertically. For targets standing somewhere agents can't (a narrow
    /// beam, mid-air) and for spawn placement.
    pub fn nearest_node(
        &self,
        pos: Vec3,
        max_cells: i32,
        class: NavClass,
        doors_open: &[bool],
    ) -> Option<u32> {
        let (ci, cj) = self.column_coords(pos.x, pos.z);
        let mut best: Option<(f32, u32)> = None;
        for ring in 0..=max_cells {
            for i in (ci - ring)..=(ci + ring) {
                for j in (cj - ring)..=(cj + ring) {
                    if (i - ci).abs() != ring && (j - cj).abs() != ring {
                        continue;
                    }
                    let Some(col) = self.column_index(i, j) else {
                        continue;
                    };
                    for n in self.column_nodes(col) {
                        if !self.passable(n, class, doors_open) {
                            continue;
                        }
                        let p = self.nodes[n as usize].pos;
                        let dy = (p.y - pos.y).abs();
                        let d = Vec2::new(p.x - pos.x, p.z - pos.z).length() + dy.max(0.0) * 2.0;
                        if dy <= 3.0 + MAX_DROP && best.is_none_or(|(b, _)| d < b) {
                            best = Some((d, n));
                        }
                    }
                }
            }
            // A ring further out can't be closer than (ring - 1) cells.
            if best.is_some_and(|(b, _)| b <= ring as f32 * CELL) {
                break;
            }
        }
        best.map(|(_, n)| n)
    }

    /// Is there safe ground at `p`'s column for an agent whose feet are at `from_y`: a non-deadly
    /// floor between `max_drop` below and a step / ramp above? (Edge guard: keeps agents out of
    /// lava and off ledges.) Outside the grid: false.
    pub fn ground_ok(&self, p: Vec3, from_y: f32, max_drop: f32) -> bool {
        let Some(col) = self.column_at(p) else {
            return false;
        };
        // The floor an agent would end up on: the highest one not above a step / ramp.
        let landing = self
            .column_nodes(col)
            .map(|n| &self.nodes[n as usize])
            .filter(|n| n.pos.y <= from_y + 1.0)
            .max_by(|a, b| a.pos.y.total_cmp(&b.pos.y));
        landing.is_some_and(|n| !n.is(node_flags::DEADLY) && n.pos.y >= from_y - max_drop)
    }

    /// Is the node a hazard (next to a kill volume)?
    pub fn is_hazard(&self, n: u32) -> bool {
        self.nodes[n as usize].is(node_flags::HAZARD)
    }

    /// Can an agent walk straight from node `from` to node `to` through linked passable nodes
    /// (the line never leaves the grid, never cuts a corner, never steps on lava)?
    pub fn walkable_line(&self, from: u32, to: u32, class: NavClass, doors_open: &[bool]) -> bool {
        if from == to {
            return true;
        }
        let a = self.nodes[from as usize].pos;
        let b = self.nodes[to as usize].pos;
        let d = Vec2::new(b.x - a.x, b.z - a.z);
        let len = d.length();
        let steps = (len / (CELL * 0.25)).ceil().max(1.0) as u32;
        let mut cur = from;
        let (mut ci, mut cj) = self.column_coords(a.x, a.z);
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let (i, j) = self.column_coords(a.x + d.x * t, a.z + d.y * t);
            if (i, j) == (ci, cj) {
                continue;
            }
            let Some(dir) = dir_index(i - ci, j - cj) else {
                return false;
            };
            let next = self.nodes[cur as usize].links[dir];
            if next == NONE || !self.passable(next, class, doors_open) {
                return false;
            }
            cur = next;
            (ci, cj) = (i, j);
        }
        cur == to
    }

    /// A* from `from` to `to` for `class`, expanding at most `max_expand` nodes. The start node
    /// doesn't need to be passable (agents pushed onto cramped floors still find their way).
    pub fn find_path(
        &self,
        s: &mut NavScratch,
        from: u32,
        to: u32,
        class: NavClass,
        doors_open: &[bool],
        max_expand: usize,
    ) -> PathResult {
        let n = self.nodes.len();
        if from as usize >= n || to as usize >= n {
            return PathResult::NotFound;
        }
        if s.g.len() != n {
            s.g = vec![0.0; n];
            s.parent = vec![NONE; n];
            s.seen = vec![0; n];
            s.closed = vec![0; n];
            s.generation = 0;
        }
        s.generation = s.generation.wrapping_add(1);
        if s.generation == 0 {
            s.seen.fill(0);
            s.closed.fill(0);
            s.generation = 1;
        }
        let generation = s.generation;
        s.heap.clear();
        s.last_expanded = 0;
        let goal = self.nodes[to as usize].pos;
        let h = |p: Vec3| Vec2::new(p.x - goal.x, p.z - goal.z).length() * 1.1;
        let key = |f: f32| Reverse((f.max(0.0) * 100.0) as u32);
        s.g[from as usize] = 0.0;
        s.parent[from as usize] = NONE;
        s.seen[from as usize] = generation;
        let start_h = h(self.nodes[from as usize].pos);
        s.heap.push((key(start_h), from));
        let mut best = (start_h, from);
        let mut reached = false;
        while let Some((_, cur)) = s.heap.pop() {
            let ci = cur as usize;
            if s.closed[ci] == generation {
                continue;
            }
            s.closed[ci] = generation;
            s.last_expanded += 1;
            if cur == to {
                reached = true;
                break;
            }
            let node = &self.nodes[ci];
            let hc = h(node.pos);
            if hc < best.0 {
                best = (hc, cur);
            }
            if s.last_expanded >= max_expand {
                break;
            }
            for (d, &m) in node.links.iter().enumerate() {
                if m == NONE || !self.passable(m, class, doors_open) {
                    continue;
                }
                let mi = m as usize;
                if s.closed[mi] == generation {
                    continue;
                }
                let other = &self.nodes[mi];
                let step = if d < 4 { CELL } else { CELL * std::f32::consts::SQRT_2 };
                let mut cost = step + (other.pos.y - node.pos.y).abs() * CLIMB_COST;
                if other.is(node_flags::HAZARD) {
                    cost += HAZARD_COST;
                }
                let g = s.g[ci] + cost;
                if s.seen[mi] != generation || g < s.g[mi] {
                    s.seen[mi] = generation;
                    s.g[mi] = g;
                    s.parent[mi] = cur;
                    s.heap.push((key(g + h(other.pos)), m));
                }
            }
        }
        let end = if reached { to } else { best.1 };
        if !reached && end == from {
            return PathResult::NotFound;
        }
        let mut path = vec![end];
        let mut cur = end;
        while s.parent[cur as usize] != NONE && path.len() <= n {
            cur = s.parent[cur as usize];
            path.push(cur);
        }
        path.reverse();
        if reached {
            PathResult::Found(path)
        } else {
            PathResult::Partial(path)
        }
    }

    /// String-pulls a node path into waypoints: from each kept node, skip ahead to the
    /// farthest node still reachable by [`NavGrid::walkable_line`]. The first node (where the
    /// agent stands) is dropped.
    pub fn smooth(&self, path: &[u32], class: NavClass, doors_open: &[bool]) -> Vec<Vec3> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < path.len() {
            let mut k = i + 1;
            while k + 1 < path.len() && self.walkable_line(path[i], path[k + 1], class, doors_open)
            {
                k += 1;
            }
            out.push(self.nodes[path[k] as usize].pos);
            i = k;
        }
        out
    }

    /// Debug: every link as a line segment (for overlays / tests).
    pub fn link_segments(&self) -> impl Iterator<Item = (Vec3, Vec3)> + '_ {
        self.nodes.iter().flat_map(move |n| {
            n.links
                .iter()
                .filter(|&&m| m != NONE)
                .map(move |&m| (n.pos, self.nodes[m as usize].pos))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{Brush, LevelDef};

    fn brush(min: [f32; 3], max: [f32; 3]) -> Brush {
        Brush {
            min: Vec3::from(min),
            max: Vec3::from(max),
            rotation: Vec3::ZERO,
            material: "concrete".into(),
            tint: Vec3::ONE,
            clip_only: false,
            no_collide: false,
        }
    }

    fn grid(level: &LevelDef) -> NavGrid {
        let col = level.build_collision();
        NavGrid::build(level, &col.world)
    }

    #[test]
    fn flat_floor_with_a_wall() {
        let level = LevelDef {
            brushes: vec![
                brush([-10.0, -1.0, -10.0], [10.0, 0.0, 10.0]),
                // A wall across the middle with a 3 m gap at x > 6.
                brush([-10.0, 0.0, -0.5], [6.0, 3.0, 0.5]),
            ],
            ..Default::default()
        };
        let g = grid(&level);
        let open = [];
        let a = g.node_at(Vec3::new(0.0, 0.0, -5.0), 1.0, NavClass::Small, &open).unwrap();
        let b = g.node_at(Vec3::new(0.0, 0.0, 5.0), 1.0, NavClass::Small, &open).unwrap();
        assert!(!g.walkable_line(a, b, NavClass::Small, &open), "the wall is in the way");
        let mut s = NavScratch::default();
        let PathResult::Found(path) = g.find_path(&mut s, a, b, NavClass::Small, &open, 10_000)
        else {
            panic!("no path around the wall");
        };
        assert!(
            path.iter().any(|&n| g.nodes[n as usize].pos.x > 6.0),
            "goes through the gap"
        );
        let smooth = g.smooth(&path, NavClass::Small, &open);
        assert!(smooth.len() < path.len() / 2, "{} waypoints", smooth.len());
        // Nothing walkable on top of the 3 m wall is linked to the floor.
        let top = g.node_at(Vec3::new(0.0, 3.0, 0.0), 0.5, NavClass::Small, &open);
        if let Some(top) = top {
            assert!(g.nodes[top as usize].pos.y > 2.9);
            assert!(!matches!(
                g.find_path(&mut s, a, top, NavClass::Small, &open, 10_000),
                PathResult::Found(_)
            ));
        }
    }

    #[test]
    fn stacked_floors_and_drops() {
        // Ground floor, and a 3 m high platform reached by a ramp whose surface runs from
        // (z 6, y 0) up to the platform edge (z -4, y 3), 2 m thick, ends buried.
        let angle = (3.0f32 / 10.0).atan();
        let normal = Vec3::new(0.0, angle.cos(), angle.sin());
        let center = Vec3::new(0.0, 1.5, 1.0) - normal;
        let half = Vec3::new(2.0, 1.0, 7.2);
        let mut ramp = brush((center - half).into(), (center + half).into());
        ramp.rotation = Vec3::new(angle.to_degrees(), 0.0, 0.0);
        let level = LevelDef {
            brushes: vec![
                brush([-20.0, -1.0, -20.0], [20.0, 0.0, 20.0]),
                brush([-6.0, 0.0, -14.0], [6.0, 3.0, -4.0]),
                ramp,
            ],
            ..Default::default()
        };
        let g = grid(&level);
        let open = [];
        let low = g.node_at(Vec3::new(8.0, 0.0, 10.0), 1.0, NavClass::Small, &open).unwrap();
        let high = g.node_at(Vec3::new(0.0, 3.0, -10.0), 1.0, NavClass::Small, &open).unwrap();
        assert!((g.nodes[high as usize].pos.y - 3.0).abs() < 0.01);
        let mut s = NavScratch::default();
        let PathResult::Found(up) = g.find_path(&mut s, low, high, NavClass::Small, &open, 20_000)
        else {
            panic!("platform reachable by the ramp");
        };
        // Going up uses the ramp (|x| < 2 where the height changes).
        for w in up.windows(2) {
            let (p, q) = (g.nodes[w[0] as usize].pos, g.nodes[w[1] as usize].pos);
            if (q.y - p.y) > STEP_UP {
                panic!("climbed a ledge from {p} to {q}");
            }
        }
        // Coming down may drop off the 3 m edge (one-way link).
        let PathResult::Found(down) =
            g.find_path(&mut s, high, low, NavClass::Small, &open, 20_000)
        else {
            panic!("can come down");
        };
        assert!(down.len() <= up.len());
    }
}
