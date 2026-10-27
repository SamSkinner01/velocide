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
//! * **large** ([`node_flags::LARGE`], wide agents: the Gaoler) when it and all 8 neighbours
//!   are walkable at about the same height and the neighbours have ≥ [`WIDE_SHOULDER_HEADROOM`].
//!
//! Every floor also records its **headroom** (free height above it). An agent class
//! ([`NavClass`]) is a footprint (narrow: the node itself; wide: the `LARGE` 3×3 block) plus
//! the headroom it needs, so tall-but-thin agents (Censer 2.5 m, Heron 3.4 m) only refuse low
//! ceilings while the 3.7 m Gaoler needs both room around it and above it.
//!
//! Neighbouring walkable nodes are **linked** (8 directions) when the step up is ≤ [`STEP_UP`]
//! (or a continuous ramp up to ~50°), the drop is ≤ [`MAX_DROP`] (one-way: enemies can jump
//! down but not back up), and nothing blocks the way at knee/chest height. Diagonals need both
//! orthogonal routes (no corner cutting). Door brushes are ignored while building; nodes under
//! a door remember it and are impassable while that door is closed.
//!
//! On top of the walking links the grid has **jump links** ([`NavJump`], one-way):
//!
//! * **leaps** from the edge of a floor (a node missing a walking link) across gaps — over
//!   lava and deep water too — and up / down ledges beyond the step and drop limits, up to
//!   [`LEAP_MAX_DISTANCE`] across, [`LEAP_MAX_UP`] up and [`LEAP_MAX_DOWN`] down. A leap
//!   follows a fixed arc ([`leap_point`]: its apex [`leap_apex`] above the higher end) and is
//!   only linked where clearance capsules swept along that arc are free of the level, the feet
//!   never pass through a kill volume, and the landing is safe (not deadly, no drop or kill
//!   volume just past it). Each leap records which agent sizes clear it ([`jump_clear`]);
//!   agent classes carry their own limits ([`NavClass::with_leaps`]);
//! * **pad rides**: from the floor under a jump pad to where its launch lands (the pad sets the
//!   velocity like `movement::step` does; the flight is simulated with each clearance capsule).
//!
//! Queries: [`NavGrid::node_at`], [`NavGrid::find_path`] (A* over walking and jump links with a
//! node budget, partial results), [`NavGrid::walkable_line`] (straight walk test through the
//! walking links) used for direct chasing and string-pulling ([`NavGrid::smooth_route`]: jumps
//! stay waypoints of their own), the reverse links ([`NavGrid::for_each_predecessor`]: who can
//! reach a node) and [`NavGrid::ground_ok`] (the edge guard that keeps enemies out of lava and
//! off ledges). See `docs/enemies.md` for measurements.

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
/// Clearance capsule every walkable floor fits (the narrow footprint). Agents up to
/// [`NARROW_RADIUS`] use it: collide-and-slide copes with the extra 0.1–0.2 m on a 1 m grid.
pub const SMALL_RADIUS: f32 = 0.5;
pub const SMALL_HEIGHT: f32 = 2.0;
/// Widest agent that uses the narrow (single node) footprint (Gnawer, Censer: 0.6).
pub const NARROW_RADIUS: f32 = 0.7;
/// Headroom the 8 neighbours of a `LARGE` node need: a wide agent's shoulders (Gaoler: its
/// r 1.3 capsule is 3.2 m tall 1 m from its axis).
pub const WIDE_SHOULDER_HEADROOM: f32 = 3.3;
/// Headroom a node's own column needs on top of the agent's height.
pub const HEADROOM_MARGIN: f32 = 0.1;
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

/// Longest leap any class makes across (horizontal, metres), highest up and deepest down
/// ([`NavClass::with_leaps`] sets each class's own, within these).
pub const LEAP_MAX_DISTANCE: f32 = 8.0;
pub const LEAP_MAX_UP: f32 = 4.0;
pub const LEAP_MAX_DOWN: f32 = 8.0;
/// Shortest leap worth a link (horizontal).
const LEAP_MIN_DISTANCE: f32 = 1.0;
/// Extra A* cost of a leap and of a pad ride (metres of walking each is worth): a walk round
/// that is at most this much longer is taken instead.
pub const LEAP_COST: f32 = 5.0;
pub const PAD_COST: f32 = 3.0;
/// Leap rays per edge node (every 22.5°; only those within ~25° of a missing walking link).
const LEAP_RAYS: u32 = 16;
/// Candidate landings swept per ray before giving up on it.
const LEAP_TRIES: u32 = 2;
/// The swept clearance capsules ride this far above the feet's arc (kerbs and lips at either
/// end don't count; collide-and-slide steps over them).
const ARC_LIFT: f32 = 0.25;
/// Radius of the narrow clearance capsule swept along leaps (the narrow footprint's, less a
/// little so a takeoff right by a wall doesn't start in it).
const ARC_RADIUS: f32 = 0.48;
/// Heights of the narrow-short and narrow-tall clearance capsules.
const ARC_HEIGHT: f32 = 2.0;
const ARC_TALL_HEIGHT: f32 = 3.5;
/// The wide clearance capsule (pad rides only: heavies don't leap).
const ARC_WIDE_RADIUS: f32 = 1.3;
const ARC_WIDE_HEIGHT: f32 = 3.8;
/// Longest pad flight simulated (seconds).
const PAD_FLIGHT: f32 = 6.0;

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
    /// Walkable for the narrow footprint (an r 0.5 capsule up to 2 m fits).
    pub const SMALL: u8 = 1 << 0;
    /// Walkable for the wide footprint (the Gaoler): the 3×3 block around it is walkable.
    pub const LARGE: u8 = 1 << 1;
    /// Next to a kill volume (walkable, expensive).
    pub const HAZARD: u8 = 1 << 2;
    /// Inside a kill volume: never stand here.
    pub const DEADLY: u8 = 1 << 3;
}

/// [`NavJump::clear`]: the agent sizes whose capsule clears a jump's arc (r 0.48 swept for the
/// narrow ones; collide-and-slide copes with the last 0.1–0.2 m like on the ground).
pub mod jump_clear {
    /// The narrow footprint up to 2.1 m (Gnawer, Snuffer).
    pub const NARROW: u8 = 1 << 0;
    /// The narrow footprint up to 3.6 m (Censer, Heron, Bellwether).
    pub const TALL: u8 = 1 << 1;
    /// The wide footprint up to 3.9 m (Gaoler, Smelter; pad rides only).
    pub const WIDE: u8 = 1 << 2;
}

/// Agent size class: a footprint plus the headroom it needs, and how it may jump (leap
/// limits, pads).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NavClass {
    /// Needs the wide footprint ([`node_flags::LARGE`]) instead of a single node.
    pub wide: bool,
    /// Free height needed above the floor (decimetres); 0 = what the `SMALL` clearance
    /// capsule guarantees (2 m) is enough.
    pub headroom_dm: u8,
    /// Leap limits (decimetres): across (horizontal; 0 = never leaps), up and down.
    pub leap_dm: u8,
    pub leap_up_dm: u8,
    pub leap_down_dm: u8,
    /// Rides jump pads.
    pub pads: bool,
}

impl NavClass {
    /// Up to r 0.7 × 2.1 m (the Gnawer's size), walking only.
    pub const SMALL: NavClass = NavClass {
        wide: false,
        headroom_dm: 0,
        leap_dm: 0,
        leap_up_dm: 0,
        leap_down_dm: 0,
        pads: false,
    };
    /// The Gaoler (r 1.3 × 3.7 m), walking only.
    pub const LARGE: NavClass = NavClass {
        wide: true,
        headroom_dm: 38,
        leap_dm: 0,
        leap_up_dm: 0,
        leap_down_dm: 0,
        pads: false,
    };

    /// The class an enemy capsule of this size needs (walking only: add
    /// [`NavClass::with_leaps`] / [`NavClass::with_pads`]).
    pub fn for_size(radius: f32, height: f32) -> NavClass {
        let wide = radius > NARROW_RADIUS;
        let headroom_dm = if !wide && height <= SMALL_HEIGHT + HEADROOM_MARGIN {
            0
        } else {
            ((height + HEADROOM_MARGIN) * 10.0).ceil().clamp(0.0, 60.0) as u8
        };
        NavClass {
            headroom_dm,
            wide,
            ..NavClass::SMALL
        }
    }

    /// The same class leaping up to `distance` across, `up` and `down` (metres, within the
    /// build's [`LEAP_MAX_DISTANCE`] / [`LEAP_MAX_UP`] / [`LEAP_MAX_DOWN`]).
    pub fn with_leaps(self, distance: f32, up: f32, down: f32) -> NavClass {
        let dm = |m: f32, max: f32| (m.clamp(0.0, max) * 10.0).floor() as u8;
        NavClass {
            leap_dm: dm(distance, LEAP_MAX_DISTANCE),
            leap_up_dm: dm(up, LEAP_MAX_UP),
            leap_down_dm: dm(down, LEAP_MAX_DOWN),
            ..self
        }
    }

    /// The same class riding jump pads or not.
    pub fn with_pads(self, pads: bool) -> NavClass {
        NavClass { pads, ..self }
    }

    /// The clearance capsule ([`jump_clear`]) this class needs along a jump (0: none fits).
    pub fn clear_bit(self) -> u8 {
        if self.wide {
            jump_clear::WIDE
        } else if self.headroom_dm == 0 {
            jump_clear::NARROW
        } else if self.headroom_dm as f32 * 0.1 <= ARC_TALL_HEIGHT + HEADROOM_MARGIN + 0.01 {
            jump_clear::TALL
        } else {
            0
        }
    }

    /// May an agent of this class take jump `j` (its size clears the arc, within its leap
    /// limits / it rides pads)? Doors and the landing's passability are checked separately.
    pub fn can_jump(self, j: &NavJump) -> bool {
        if self.clear_bit() & j.clear == 0 {
            return false;
        }
        match j.kind {
            JumpKind::Leap => {
                self.leap_dm > 0
                    && j.dist <= self.leap_dm as f32 * 0.1 + 1e-3
                    && j.rise <= self.leap_up_dm as f32 * 0.1 + 1e-3
                    && -j.rise <= self.leap_down_dm as f32 * 0.1 + 1e-3
            }
            JumpKind::Pad(_) => self.pads,
        }
    }

    pub fn flag(self) -> u8 {
        if self.wide {
            node_flags::LARGE
        } else {
            node_flags::SMALL
        }
    }

    /// Headroom needed above a node (metres).
    pub fn headroom(self) -> f32 {
        self.headroom_dm as f32 * 0.1
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

/// What a jump link is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpKind {
    /// A leap on the arc of [`leap_point`] (a crouch, then airborne: `sim::enemy`).
    Leap,
    /// Riding jump pad `n` (`LevelDef::jump_pads` index) from the floor under it.
    Pad(u16),
}

/// A one-way off-mesh link: a leap across a gap / up or down a ledge, or a pad ride.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavJump {
    /// Takeoff node (the floor under the pad for a ride).
    pub from: u32,
    /// Landing node.
    pub to: u32,
    pub kind: JumpKind,
    /// Horizontal distance and height change (landing − takeoff), metres.
    pub dist: f32,
    pub rise: f32,
    /// [`jump_clear`] bits: the agent sizes whose capsule clears the arc.
    pub clear: u8,
    /// Door slot + 1 (0 = none) whose brush the arc passes: impassable while it is closed.
    pub door: u8,
}

/// One waypoint of a route ([`NavGrid::smooth_route`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    /// Floor point (feet).
    pub pos: Vec3,
    pub node: u32,
    /// Reached from the previous waypoint (its takeoff) by this jump (index for
    /// [`NavGrid::jump`]); `None` = walked to.
    pub jump: Option<u32>,
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
    /// Jump links: leaps (and how many reach over a deadly floor: lava, deep water) and pad
    /// rides.
    pub leaps: usize,
    pub leaps_over_deadly: usize,
    pub pad_links: usize,
    /// Arc sweeps (capsule casts along candidate arcs) and the time the jump links took.
    pub arc_sweeps: usize,
    pub jumps_ms: f32,
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
    /// The last A* stopped at its node budget (a `Partial` result that may still be
    /// reachable) rather than running out of nodes (unreachable).
    pub last_budget_hit: bool,
}

impl NavScratch {
    /// Starts a search over `n` nodes: a fresh generation (buffers resized when the grid
    /// changed).
    fn begin(&mut self, n: usize) -> u32 {
        if self.g.len() != n {
            self.g = vec![0.0; n];
            self.parent = vec![NONE; n];
            self.seen = vec![0; n];
            self.closed = vec![0; n];
            self.generation = 0;
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.seen.fill(0);
            self.closed.fill(0);
            self.generation = 1;
        }
        self.heap.clear();
        self.last_expanded = 0;
        self.last_budget_hit = false;
        self.generation
    }
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
    /// Jump links sorted by takeoff: `jumps[jump_start[n]..jump_start[n + 1]]` leave node `n`
    /// (empty when the grid has none).
    jumps: Vec<NavJump>,
    jump_start: Vec<u32>,
    /// Reverse index: `jumps_in[jump_in_start[n]..jump_in_start[n + 1]]` are the jumps (indices)
    /// landing on `n`.
    jumps_in: Vec<u32>,
    jump_in_start: Vec<u32>,
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

fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(b.x - a.x, b.z - a.z).length()
}

/// The penalty A* adds to a jump's distance.
fn jump_cost(j: &NavJump) -> f32 {
    match j.kind {
        JumpKind::Leap => LEAP_COST,
        JumpKind::Pad(_) => PAD_COST,
    }
}

/// Apex height (feet) of the leap arc from `from` to `to`: above the higher end by 0.5 m +
/// 15 % of the horizontal distance (a readable hop that clears the lip it lands over).
pub fn leap_apex(from: Vec3, to: Vec3) -> f32 {
    from.y.max(to.y) + 0.5 + 0.15 * flat_distance(from, to)
}

/// Feet on the leap arc from `from` to `to` after fraction `s` (0..1) of its flight time: a
/// parabola through both ends with its apex at [`leap_apex`]. Its shape doesn't depend on
/// gravity (only its timing does), so the nav build sweeps exactly the path an agent flies.
pub fn leap_point(from: Vec3, to: Vec3, s: f32) -> Vec3 {
    let apex = leap_apex(from, to);
    let a = (apex - from.y).max(1e-4).sqrt();
    let b = (apex - to.y).max(1e-4).sqrt();
    let peak = a / (a + b);
    let flat = from.lerp(to, s);
    Vec3::new(
        flat.x,
        apex - (a + b) * (a + b) * (s - peak) * (s - peak),
        flat.z,
    )
}

/// Flies a capsule (`r`, `h`, feet at `feet`) launched at `velocity` under gravity the way an
/// airborne enemy moves (60 Hz, collide-and-slide against [`NAV_MASK`]) until it lands on a
/// walkable surface: its feet there, or `None` (never landed, or the feet entered a kill
/// volume).
fn simulate_flight(
    level: &LevelDef,
    world: &CollisionWorld,
    feet: Vec3,
    velocity: Vec3,
    r: f32,
    h: f32,
    gravity: f32,
) -> Option<Vec3> {
    let dt = 1.0 / 60.0;
    let half = (h * 0.5 - r).max(0.0);
    let lift = Vec3::Y * (h * 0.5);
    let mut c = feet + lift + Vec3::Y * 0.05;
    let mut v = velocity;
    for _ in 0..(PAD_FLIGHT * 60.0) as u32 {
        v.y -= gravity * dt;
        let mut motion = v * dt;
        for _ in 0..4 {
            if motion.length_squared() < 1e-10 {
                break;
            }
            let Some(hit) = world.cast_capsule(c, half, r, motion, 0.01, NAV_MASK) else {
                c += motion;
                break;
            };
            c += motion * hit.fraction;
            let n = hit.normal;
            if n.y > 0.7 && v.y <= 0.5 {
                return Some(c - lift);
            }
            let rest = motion * (1.0 - hit.fraction);
            motion = rest - n * rest.dot(n);
            let into = v.dot(n);
            if into < 0.0 {
                v -= n * into;
            }
            c += n * 0.002;
        }
        if !c.is_finite() || level.kill_volumes.iter().any(|k| k.contains(c - lift)) {
            return None;
        }
    }
    None
}

/// Launch velocity and flight time of the leap arc from `from` to `to` under gravity `g`
/// (the arc of [`leap_point`]).
pub fn leap_velocity(from: Vec3, to: Vec3, g: f32) -> (Vec3, f32) {
    let g = g.max(1.0);
    let apex = leap_apex(from, to);
    let up = (2.0 * (apex - from.y).max(1e-4) / g).sqrt();
    let down = (2.0 * (apex - to.y).max(1e-4) / g).sqrt();
    let time = up + down;
    let flat = Vec3::new(to.x - from.x, 0.0, to.z - from.z) / time;
    (flat + Vec3::Y * (g * up), time)
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
        let buckets = Buckets::new(aabbs, origin, Vec2::new(nx as f32 * CELL, nz as f32 * CELL));
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
            jumps: Vec::new(),
            jump_start: Vec::new(),
            jumps_in: Vec::new(),
            jump_in_start: Vec::new(),
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
                    let overhead = cands
                        .iter()
                        .any(|&c| buckets.aabbs[c as usize].0.y > y + 0.05);
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

        // 5. Wide footprint: walkable all around at about the same height, with shoulder room
        //    (the class's own headroom is checked on the node itself, see `passable`).
        let mut large = Vec::new();
        for (n, node) in grid.nodes.iter().enumerate() {
            if !node.is(node_flags::SMALL) || node.headroom < WIDE_SHOULDER_HEADROOM {
                continue;
            }
            let ok = node.links.iter().all(|&m| {
                m != NONE && {
                    let o = &grid.nodes[m as usize];
                    (o.pos.y - node.pos.y).abs() <= 0.6 && o.headroom >= WIDE_SHOULDER_HEADROOM
                }
            });
            if ok {
                large.push(n);
            }
        }
        for n in large {
            grid.nodes[n].flags |= node_flags::LARGE;
        }

        // 6. Jump links: leaps off every floor's edges, pad rides.
        grid.build_jumps(level, world, &mut stats);

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
            if !(-MAX_DROP..=1.2 * CELL).contains(&dy) {
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

    /// Builds the jump links (step 6 of the build): leaps off every floor's edges, then a ride
    /// per pad and clearance capsule; sorted by takeoff with a reverse index.
    fn build_jumps(&mut self, level: &LevelDef, world: &CollisionWorld, stats: &mut NavStats) {
        let started = Instant::now();
        let doors: Vec<(Vec3, Vec3)> = level
            .doors
            .iter()
            .take(254)
            .map(|d| brush_aabb(&d.brush))
            .collect();
        let mut jumps = Vec::new();
        // Leaps: rays fanned towards the missing walking links of every edge node.
        let mut missing = Vec::with_capacity(8);
        for n in 0..self.nodes.len() as u32 {
            let node = &self.nodes[n as usize];
            if !node.is(node_flags::SMALL) || node.is(node_flags::DEADLY) {
                continue;
            }
            missing.clear();
            missing.extend(
                DIRS.iter()
                    .zip(node.links)
                    .filter(|(_, l)| *l == NONE)
                    .map(|(&(dx, dz), _)| Vec2::new(dx as f32, dz as f32).normalize()),
            );
            if missing.is_empty() {
                continue;
            }
            for k in 0..LEAP_RAYS {
                let angle = k as f32 * std::f32::consts::TAU / LEAP_RAYS as f32;
                let dir = Vec2::new(angle.cos(), angle.sin());
                if missing.iter().any(|m| m.dot(dir) > 0.9) {
                    self.leap_ray(level, world, &doors, n, dir, &mut jumps, stats);
                }
            }
        }
        // Pad rides, per clearance capsule.
        let gravity = crate::tuning::MovementTuning::default().gravity;
        for (i, pad) in level.jump_pads.iter().enumerate().take(u16::MAX as usize) {
            self.pad_rides(level, world, i as u16, pad, gravity, &mut jumps);
        }
        // One link per (takeoff, landing): merge the clearance of duplicates.
        jumps.sort_by(|a: &NavJump, b: &NavJump| {
            (a.from, a.to, a.kind == JumpKind::Leap).cmp(&(b.from, b.to, b.kind == JumpKind::Leap))
        });
        jumps.dedup_by(|later, kept| {
            let same = later.from == kept.from && later.to == kept.to && later.kind == kept.kind;
            if same {
                kept.clear |= later.clear;
            }
            same
        });
        let n = self.nodes.len();
        self.jump_start = vec![0; n + 1];
        self.jump_in_start = vec![0; n + 1];
        for j in &jumps {
            self.jump_start[j.from as usize + 1] += 1;
            self.jump_in_start[j.to as usize + 1] += 1;
            match j.kind {
                JumpKind::Leap => {
                    stats.leaps += 1;
                    let a = self.nodes[j.from as usize].pos;
                    let b = self.nodes[j.to as usize].pos;
                    if self.crosses_deadly(a, b) {
                        stats.leaps_over_deadly += 1;
                    }
                }
                JumpKind::Pad(_) => stats.pad_links += 1,
            }
        }
        for i in 0..n {
            self.jump_start[i + 1] += self.jump_start[i];
            self.jump_in_start[i + 1] += self.jump_in_start[i];
        }
        let mut fill = self.jump_in_start.clone();
        self.jumps_in = vec![0; jumps.len()];
        for (i, j) in jumps.iter().enumerate() {
            let slot = &mut fill[j.to as usize];
            self.jumps_in[*slot as usize] = i as u32;
            *slot += 1;
        }
        self.jumps = jumps;
        stats.jumps_ms = started.elapsed().as_secs_f32() * 1000.0;
    }

    /// Marches one leap ray from edge node `n` along `dir` (0.5 m steps up to
    /// [`LEAP_MAX_DISTANCE`]): the first floor past a gap (or above / below the step and drop
    /// limits) whose arc is clear becomes a leap.
    #[allow(clippy::too_many_arguments)]
    fn leap_ray(
        &self,
        level: &LevelDef,
        world: &CollisionWorld,
        doors: &[(Vec3, Vec3)],
        n: u32,
        dir: Vec2,
        out: &mut Vec<NavJump>,
        stats: &mut NavStats,
    ) {
        let from = self.nodes[n as usize].pos;
        // Passed over a column with no floor at our level (a gap, a pit, a wall).
        let mut gap = false;
        let mut tries = 0;
        let mut last = None;
        let steps = (LEAP_MAX_DISTANCE / 0.5) as i32;
        for step in (LEAP_MIN_DISTANCE / 0.5) as i32..=steps {
            let s = step as f32 * 0.5;
            let p = Vec3::new(from.x + dir.x * s, from.y, from.z + dir.y * s);
            let Some(col) = self.column_at(p) else {
                gap = true;
                continue;
            };
            if last == Some(col) {
                continue;
            }
            last = Some(col);
            let mut level_floor = false;
            let mut cand: Option<(f32, u32)> = None;
            for m in self.column_nodes(col) {
                let o = &self.nodes[m as usize];
                let rise = o.pos.y - from.y;
                let walkable = o.is(node_flags::SMALL) && !o.is(node_flags::DEADLY);
                if walkable && rise.abs() <= STEP_UP + 0.15 {
                    level_floor = true;
                }
                if !walkable || !(-LEAP_MAX_DOWN..=LEAP_MAX_UP).contains(&rise) {
                    continue;
                }
                // More of our own floor isn't worth a leap: a ledge or the far side of a gap is.
                if !gap && rise.abs() <= STEP_UP + 0.15 {
                    continue;
                }
                if cand.is_none_or(|(r, _)| rise.abs() < r) {
                    cand = Some((rise.abs(), m));
                }
            }
            if let Some((_, m)) = cand
                && !self.nodes[n as usize].links.contains(&m)
            {
                tries += 1;
                if self.try_leap(level, world, doors, n, m, dir, out, stats) || tries >= LEAP_TRIES
                {
                    return;
                }
            }
            if !level_floor {
                gap = true;
            }
        }
    }

    /// A leap from `n` (or, climbing, from up to two cells further back along the ray, so the
    /// arc clears the lip) onto `m`, if its landing is safe and its arc is clear.
    #[allow(clippy::too_many_arguments)]
    fn try_leap(
        &self,
        level: &LevelDef,
        world: &CollisionWorld,
        doors: &[(Vec3, Vec3)],
        n: u32,
        m: u32,
        dir: Vec2,
        out: &mut Vec<NavJump>,
        stats: &mut NavStats,
    ) -> bool {
        let to = self.nodes[m as usize].pos;
        if !self.overshoot_safe(to, dir) {
            return false;
        }
        let base = self.nodes[n as usize].pos.y;
        let mut takeoff = n;
        for back in 0..3 {
            if back > 0 {
                let Some(b) = self.link_towards(takeoff, -dir) else {
                    break;
                };
                if (self.nodes[b as usize].pos.y - base).abs() > 0.3 {
                    break;
                }
                takeoff = b;
            }
            let from = self.nodes[takeoff as usize].pos;
            let dist = flat_distance(from, to);
            if dist > LEAP_MAX_DISTANCE {
                break;
            }
            if dist >= LEAP_MIN_DISTANCE
                && let Some(j) = self.leap_link(level, world, doors, takeoff, m, stats)
            {
                out.push(j);
                return true;
            }
            // Only climbs gain from a run-up.
            if to.y - from.y <= 1.0 {
                break;
            }
        }
        false
    }

    /// The leap `a` → `b` if a capsule clears its arc (narrow, and tall if that clears too).
    fn leap_link(
        &self,
        level: &LevelDef,
        world: &CollisionWorld,
        doors: &[(Vec3, Vec3)],
        a: u32,
        b: u32,
        stats: &mut NavStats,
    ) -> Option<NavJump> {
        let (from, to) = (self.nodes[a as usize].pos, self.nodes[b as usize].pos);
        if !self.arc_columns_free(from, to, ARC_HEIGHT) {
            return None;
        }
        // Most arcs are in the open: the tall capsule first (it clears for both), the narrow
        // one only when it doesn't.
        let tall = self
            .arc_columns_free(from, to, ARC_TALL_HEIGHT)
            .then(|| {
                self.sweep_arc(
                    level,
                    world,
                    doors,
                    from,
                    to,
                    ARC_RADIUS,
                    ARC_TALL_HEIGHT,
                    stats,
                )
            })
            .flatten();
        let (clear, door) = match tall {
            Some(door) => (jump_clear::NARROW | jump_clear::TALL, door),
            None => (
                jump_clear::NARROW,
                self.sweep_arc(level, world, doors, from, to, ARC_RADIUS, ARC_HEIGHT, stats)?,
            ),
        };
        Some(NavJump {
            from: a,
            to: b,
            kind: JumpKind::Leap,
            dist: flat_distance(from, to),
            rise: to.y - from.y,
            clear,
            door,
        })
    }

    /// Cheap pre-filter on the grid's own data: along the arc, every column's free space (the
    /// floor below the capsule and its headroom) holds a capsule of height `h`. Columns with a
    /// floor inside the capsule's span, or with floors only above it (inside a solid), fail.
    fn arc_columns_free(&self, from: Vec3, to: Vec3, h: f32) -> bool {
        let d = flat_distance(from, to);
        let n = ((d / 0.5).ceil() as u32).max(4);
        for i in 1..n {
            let p = leap_point(from, to, i as f32 / n as f32);
            // The takeoff and landing columns are the end floors themselves.
            if flat_distance(p, from) < 0.6 || flat_distance(p, to) < 0.6 {
                continue;
            }
            let Some(col) = self.column_at(p) else {
                continue;
            };
            let (lo, hi) = (p.y + ARC_LIFT - 0.05, p.y + ARC_LIFT + h);
            let mut below: Option<&NavNode> = None;
            let mut above = false;
            for m in self.column_nodes(col) {
                let f = &self.nodes[m as usize];
                if f.pos.y <= lo {
                    below = Some(f);
                } else if f.pos.y < hi {
                    return false;
                } else {
                    above = true;
                }
            }
            match below {
                // A measured headroom (< 6 m) that ends inside the capsule: a ceiling, a beam.
                Some(f) if f.headroom < 5.9 && f.pos.y + f.headroom < hi => return false,
                None if above => return false,
                _ => {}
            }
        }
        true
    }

    /// Sweeps a capsule (`r`, `h`) along the arc `from` → `to` in chords ([`ARC_LIFT`] above the
    /// feet) and checks the feet never pass through a kill volume. `Some(door slot + 1)` of a
    /// door brush the arc passes (0: none) when clear, `None` when blocked.
    #[allow(clippy::too_many_arguments)]
    fn sweep_arc(
        &self,
        level: &LevelDef,
        world: &CollisionWorld,
        doors: &[(Vec3, Vec3)],
        from: Vec3,
        to: Vec3,
        r: f32,
        h: f32,
        stats: &mut NavStats,
    ) -> Option<u8> {
        // Chords of ~3 m: below the parabola by at most ~10 cm (the capsule rides 0.25 m up).
        let apex = leap_apex(from, to);
        let length = flat_distance(from, to) + (apex - from.y) + (apex - to.y);
        let n = ((length / 3.0).ceil() as u32).clamp(3, 10);
        let half = (h * 0.5 - r).max(0.0);
        let lift = Vec3::Y * (ARC_LIFT + h * 0.5);
        let mut door = 0u8;
        let mut prev = from;
        for i in 1..=n {
            let p = leap_point(from, to, i as f32 / n as f32);
            if i < n && level.kill_volumes.iter().any(|v| v.contains(p)) {
                return None;
            }
            stats.arc_sweeps += 1;
            if world
                .cast_capsule(prev + lift, half, r, p - prev, 0.0, NAV_MASK)
                .is_some()
            {
                return None;
            }
            if door == 0 {
                let lo = prev.min(p) + Vec3::new(-r, ARC_LIFT, -r);
                let hi = prev.max(p) + Vec3::new(r, ARC_LIFT + h, r);
                if let Some(slot) = doors
                    .iter()
                    .position(|(a, b)| a.cmple(hi).all() && b.cmpge(lo).all())
                {
                    door = slot as u8 + 1;
                }
            }
            prev = p;
        }
        Some(door)
    }

    /// Is it safe to land on `to` coming along `dir`: 0.75 m further on there is floor that
    /// isn't deadly and no more than a metre lower, or a wall to bump into (not a drop, not
    /// lava or water)?
    fn overshoot_safe(&self, to: Vec3, dir: Vec2) -> bool {
        let p = Vec3::new(to.x + dir.x * 0.75, to.y, to.z + dir.y * 0.75);
        let Some(col) = self.column_at(p) else {
            return false;
        };
        let mut floor = None;
        let mut above = false;
        for m in self.column_nodes(col) {
            let f = &self.nodes[m as usize];
            if f.pos.y <= to.y + 0.6 {
                floor = Some(f);
            } else {
                above = true;
            }
        }
        match floor {
            Some(f) => !f.is(node_flags::DEADLY) && f.pos.y >= to.y - 1.0,
            None => above,
        }
    }

    /// The walking link of `n` closest to direction `dir` (within ~25°).
    fn link_towards(&self, n: u32, dir: Vec2) -> Option<u32> {
        DIRS.iter()
            .zip(self.nodes[n as usize].links)
            .filter(|(_, l)| *l != NONE)
            .map(|(&(dx, dz), l)| (Vec2::new(dx as f32, dz as f32).normalize().dot(dir), l))
            .filter(|(d, _)| *d > 0.9)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, l)| l)
    }

    /// Does the straight line between two floor points pass over a deadly floor (statistics)?
    fn crosses_deadly(&self, a: Vec3, b: Vec3) -> bool {
        let steps = (flat_distance(a, b) / 0.5).ceil().max(1.0) as u32;
        (1..steps).any(|i| {
            let p = a.lerp(b, i as f32 / steps as f32);
            self.column_at(p).is_some_and(|col| {
                let top = self
                    .column_nodes(col)
                    .map(|m| &self.nodes[m as usize])
                    .filter(|f| f.pos.y <= a.y.max(b.y) + 0.5)
                    .max_by(|x, y| x.pos.y.total_cmp(&y.pos.y));
                top.is_some_and(|f| f.is(node_flags::DEADLY))
            })
        })
    }

    /// The rides of pad `index`: from the walkable floor under its trigger, the launch (the
    /// pad sets the velocity, as `movement::step` does) flown with each clearance capsule
    /// until it lands on a floor that capsule's class can stand on.
    fn pad_rides(
        &self,
        level: &LevelDef,
        world: &CollisionWorld,
        index: u16,
        pad: &crate::level::JumpPad,
        gravity: f32,
        out: &mut Vec<NavJump>,
    ) {
        let c = pad.trigger.center();
        let Some(col) = self.column_at(c) else {
            return;
        };
        let Some(start) = self
            .column_nodes(col)
            .filter(|&m| {
                let f = &self.nodes[m as usize];
                f.is(node_flags::SMALL)
                    && !f.is(node_flags::DEADLY)
                    && f.pos.y >= pad.trigger.min.y - 1.0
                    && f.pos.y <= pad.trigger.max.y
            })
            .max_by(|&a, &b| {
                self.nodes[a as usize]
                    .pos
                    .y
                    .total_cmp(&self.nodes[b as usize].pos.y)
            })
        else {
            return;
        };
        let from = self.nodes[start as usize].pos;
        for (bit, r, h, flag) in [
            (jump_clear::NARROW, 0.6, 2.0, node_flags::SMALL),
            (jump_clear::TALL, 0.6, ARC_TALL_HEIGHT, node_flags::SMALL),
            (
                jump_clear::WIDE,
                ARC_WIDE_RADIUS,
                ARC_WIDE_HEIGHT,
                node_flags::LARGE,
            ),
        ] {
            if !self.nodes[start as usize].is(flag) {
                continue;
            }
            let Some(feet) = simulate_flight(level, world, from, pad.velocity, r, h, gravity)
            else {
                continue;
            };
            let Some(m) = self.column_at(feet).and_then(|col| {
                self.column_nodes(col)
                    .filter(|&m| {
                        let f = &self.nodes[m as usize];
                        f.is(flag) && !f.is(node_flags::DEADLY) && (f.pos.y - feet.y).abs() < 0.7
                    })
                    .min_by(|&a, &b| {
                        let da = (self.nodes[a as usize].pos.y - feet.y).abs();
                        da.total_cmp(&(self.nodes[b as usize].pos.y - feet.y).abs())
                    })
            }) else {
                continue;
            };
            if m == start {
                continue;
            }
            let to = self.nodes[m as usize].pos;
            out.push(NavJump {
                from: start,
                to: m,
                kind: JumpKind::Pad(index),
                dist: flat_distance(from, to),
                rise: to.y - from.y,
                clear: bit,
                door: 0,
            });
        }
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

    /// Can `class` stand on node `n` (footprint and headroom) with the doors as given?
    pub fn passable(&self, n: u32, class: NavClass, doors_open: &[bool]) -> bool {
        let node = &self.nodes[n as usize];
        node.is(class.flag())
            && (class.headroom_dm == 0 || node.headroom >= class.headroom())
            && (node.door == 0
                || doors_open
                    .get(node.door as usize - 1)
                    .copied()
                    .unwrap_or(true))
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
                    y <= pos.y + 0.6
                        && y >= pos.y - max_below
                        && self.passable(n, class, doors_open)
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

    /// The jump links leaving node `n`.
    pub fn jumps_from(&self, n: u32) -> &[NavJump] {
        match (
            self.jump_start.get(n as usize),
            self.jump_start.get(n as usize + 1),
        ) {
            (Some(&a), Some(&b)) => &self.jumps[a as usize..b as usize],
            _ => &[],
        }
    }

    /// Jump link `i` (a [`Waypoint::jump`]).
    pub fn jump(&self, i: u32) -> Option<&NavJump> {
        self.jumps.get(i as usize)
    }

    /// Every jump link.
    pub fn jumps(&self) -> &[NavJump] {
        &self.jumps
    }

    /// Index of the jump `from` → `to` that `class` may take (doors as given), if any.
    pub fn jump_between(
        &self,
        from: u32,
        to: u32,
        class: NavClass,
        doors_open: &[bool],
    ) -> Option<u32> {
        let start = *self.jump_start.get(from as usize)?;
        self.jumps_from(from)
            .iter()
            .position(|j| j.to == to && class.can_jump(j) && self.jump_open(j, doors_open))
            .map(|i| start + i as u32)
    }

    /// Is no door that the jump's arc passes closed?
    pub fn jump_open(&self, j: &NavJump, doors_open: &[bool]) -> bool {
        j.door == 0 || doors_open.get(j.door as usize - 1).copied().unwrap_or(true)
    }

    /// Calls `f(m, cost)` for every node `m` an agent of `class` can step, drop, leap or ride
    /// onto `n` from (the reverse of the links; `cost` as A* counts it). With it a search from
    /// a target finds everywhere that can reach the target.
    pub fn for_each_predecessor(
        &self,
        n: u32,
        class: NavClass,
        doors_open: &[bool],
        mut f: impl FnMut(u32, f32),
    ) {
        let node = &self.nodes[n as usize];
        let (i, j) = self.column_coords(node.pos.x, node.pos.z);
        let hazard = if node.is(node_flags::HAZARD) {
            HAZARD_COST
        } else {
            0.0
        };
        for (d, &(dx, dz)) in DIRS.iter().enumerate() {
            let Some(col) = self.column_index(i - dx, j - dz) else {
                continue;
            };
            for m in self.column_nodes(col) {
                let o = &self.nodes[m as usize];
                if o.links[d] == n && self.passable(m, class, doors_open) {
                    let step = if d < 4 {
                        CELL
                    } else {
                        CELL * std::f32::consts::SQRT_2
                    };
                    f(m, step + (node.pos.y - o.pos.y).abs() * CLIMB_COST + hazard);
                }
            }
        }
        let (Some(&a), Some(&b)) = (
            self.jump_in_start.get(n as usize),
            self.jump_in_start.get(n as usize + 1),
        ) else {
            return;
        };
        for &ji in &self.jumps_in[a as usize..b as usize] {
            let jl = &self.jumps[ji as usize];
            if class.can_jump(jl)
                && self.jump_open(jl, doors_open)
                && self.passable(jl.from, class, doors_open)
            {
                f(
                    jl.from,
                    jl.dist + jl.rise.abs() * CLIMB_COST + jump_cost(jl) + hazard,
                );
            }
        }
    }

    /// Calls `f(m)` for every node an agent of `class` can step, drop, leap or ride to from `n`.
    pub fn for_each_successor(
        &self,
        n: u32,
        class: NavClass,
        doors_open: &[bool],
        mut f: impl FnMut(u32),
    ) {
        for &m in &self.nodes[n as usize].links {
            if m != NONE && self.passable(m, class, doors_open) {
                f(m);
            }
        }
        for j in self.jumps_from(n) {
            if class.can_jump(j)
                && self.jump_open(j, doors_open)
                && self.passable(j.to, class, doors_open)
            {
                f(j.to);
            }
        }
    }

    /// A* from `from` to `to` for `class` over the walking links and the jumps the class may
    /// take, expanding at most `max_expand` nodes. The start node doesn't need to be passable
    /// (agents pushed onto cramped floors still find their way).
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
        let generation = s.begin(n);
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
                s.last_budget_hit = true;
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
                let step = if d < 4 {
                    CELL
                } else {
                    CELL * std::f32::consts::SQRT_2
                };
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
            // Leaps and pad rides the class may take.
            for j in self.jumps_from(cur) {
                if !class.can_jump(j)
                    || !self.jump_open(j, doors_open)
                    || !self.passable(j.to, class, doors_open)
                {
                    continue;
                }
                let mi = j.to as usize;
                if s.closed[mi] == generation {
                    continue;
                }
                let other = &self.nodes[mi];
                let mut cost = j.dist + j.rise.abs() * CLIMB_COST + jump_cost(j);
                if other.is(node_flags::HAZARD) {
                    cost += HAZARD_COST;
                }
                let g = s.g[ci] + cost;
                if s.seen[mi] != generation || g < s.g[mi] {
                    s.seen[mi] = generation;
                    s.g[mi] = g;
                    s.parent[mi] = cur;
                    s.heap.push((key(g + h(other.pos)), j.to));
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

    /// Dijkstra from `start` over the links `class` may take (walking and jumps, doors as
    /// given), or over their reverse (`reverse`: every node settled can reach `start`). Calls
    /// `visit(node, cost)` for each node in cost order (`start` first, cost 0) until the cost
    /// passes `max_cost` or `max_nodes` were settled; `visit` returning false stops the search.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        s: &mut NavScratch,
        start: u32,
        class: NavClass,
        doors_open: &[bool],
        reverse: bool,
        max_cost: f32,
        max_nodes: usize,
        mut visit: impl FnMut(u32, f32) -> bool,
    ) {
        let n = self.nodes.len();
        if start as usize >= n {
            return;
        }
        let generation = s.begin(n);
        let key = |f: f32| Reverse((f.max(0.0) * 100.0) as u32);
        s.g[start as usize] = 0.0;
        s.seen[start as usize] = generation;
        s.heap.push((key(0.0), start));
        let mut next = Vec::with_capacity(16);
        while let Some((_, cur)) = s.heap.pop() {
            let ci = cur as usize;
            if s.closed[ci] == generation {
                continue;
            }
            s.closed[ci] = generation;
            s.last_expanded += 1;
            let g = s.g[ci];
            if g > max_cost || !visit(cur, g) || s.last_expanded >= max_nodes {
                break;
            }
            next.clear();
            if reverse {
                self.for_each_predecessor(cur, class, doors_open, |m, c| next.push((m, c)));
            } else {
                let node = &self.nodes[ci];
                for (d, &m) in node.links.iter().enumerate() {
                    if m != NONE && self.passable(m, class, doors_open) {
                        let o = &self.nodes[m as usize];
                        let step = if d < 4 {
                            CELL
                        } else {
                            CELL * std::f32::consts::SQRT_2
                        };
                        let hazard = if o.is(node_flags::HAZARD) {
                            HAZARD_COST
                        } else {
                            0.0
                        };
                        next.push((m, step + (o.pos.y - node.pos.y).abs() * CLIMB_COST + hazard));
                    }
                }
                for j in self.jumps_from(cur) {
                    if class.can_jump(j)
                        && self.jump_open(j, doors_open)
                        && self.passable(j.to, class, doors_open)
                    {
                        let hazard = if self.nodes[j.to as usize].is(node_flags::HAZARD) {
                            HAZARD_COST
                        } else {
                            0.0
                        };
                        next.push((
                            j.to,
                            j.dist + j.rise.abs() * CLIMB_COST + jump_cost(j) + hazard,
                        ));
                    }
                }
            }
            for &(m, c) in &next {
                let mi = m as usize;
                if s.closed[mi] == generation {
                    continue;
                }
                let gm = g + c;
                if s.seen[mi] != generation || gm < s.g[mi] {
                    s.seen[mi] = generation;
                    s.g[mi] = gm;
                    s.heap.push((key(gm), m));
                }
            }
        }
    }

    /// String-pulls a node path into waypoint positions ([`NavGrid::smooth_route`] without the
    /// jumps).
    pub fn smooth(&self, path: &[u32], class: NavClass, doors_open: &[bool]) -> Vec<Vec3> {
        self.smooth_route(path, class, doors_open)
            .into_iter()
            .map(|w| w.pos)
            .collect()
    }

    /// String-pulls a node path into waypoints: from each kept node, skip ahead to the
    /// farthest node still reachable by [`NavGrid::walkable_line`], never past a jump: a jump's
    /// takeoff and landing are waypoints of their own (the landing carries the jump). The
    /// first node (where the agent stands) is dropped.
    pub fn smooth_route(
        &self,
        path: &[u32],
        class: NavClass,
        doors_open: &[bool],
    ) -> Vec<Waypoint> {
        let walked = |a: u32, b: u32| self.nodes[a as usize].links.contains(&b);
        let jump = |a: u32, b: u32| {
            if walked(a, b) {
                None
            } else {
                self.jump_between(a, b, class, doors_open)
            }
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < path.len() {
            if let Some(j) = jump(path[i], path[i + 1]) {
                out.push(Waypoint {
                    pos: self.nodes[path[i + 1] as usize].pos,
                    node: path[i + 1],
                    jump: Some(j),
                });
                i += 1;
                continue;
            }
            let mut k = i + 1;
            while k + 1 < path.len()
                && jump(path[k], path[k + 1]).is_none()
                && self.walkable_line(path[i], path[k + 1], class, doors_open)
            {
                k += 1;
            }
            out.push(Waypoint {
                pos: self.nodes[path[k] as usize].pos,
                node: path[k],
                jump: None,
            });
            i = k;
        }
        out
    }

    /// Debug: every walking link as a line segment (for overlays / tests).
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
    use crate::level::{Brush, JumpPad, LevelDef, Volume};

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
        let a = g
            .node_at(Vec3::new(0.0, 0.0, -5.0), 1.0, NavClass::SMALL, &open)
            .unwrap();
        let b = g
            .node_at(Vec3::new(0.0, 0.0, 5.0), 1.0, NavClass::SMALL, &open)
            .unwrap();
        assert!(
            !g.walkable_line(a, b, NavClass::SMALL, &open),
            "the wall is in the way"
        );
        let mut s = NavScratch::default();
        let PathResult::Found(path) = g.find_path(&mut s, a, b, NavClass::SMALL, &open, 10_000)
        else {
            panic!("no path around the wall");
        };
        assert!(
            path.iter().any(|&n| g.nodes[n as usize].pos.x > 6.0),
            "goes through the gap"
        );
        let smooth = g.smooth(&path, NavClass::SMALL, &open);
        assert!(smooth.len() < path.len() / 2, "{} waypoints", smooth.len());
        // Nothing walkable on top of the 3 m wall is linked to the floor.
        let top = g.node_at(Vec3::new(0.0, 3.0, 0.0), 0.5, NavClass::SMALL, &open);
        if let Some(top) = top {
            assert!(g.nodes[top as usize].pos.y > 2.9);
            assert!(!matches!(
                g.find_path(&mut s, a, top, NavClass::SMALL, &open, 10_000),
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
        let low = g
            .node_at(Vec3::new(8.0, 0.0, 10.0), 1.0, NavClass::SMALL, &open)
            .unwrap();
        let high = g
            .node_at(Vec3::new(0.0, 3.0, -10.0), 1.0, NavClass::SMALL, &open)
            .unwrap();
        assert!((g.nodes[high as usize].pos.y - 3.0).abs() < 0.01);
        let mut s = NavScratch::default();
        let PathResult::Found(up) = g.find_path(&mut s, low, high, NavClass::SMALL, &open, 20_000)
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
            g.find_path(&mut s, high, low, NavClass::SMALL, &open, 20_000)
        else {
            panic!("can come down");
        };
        assert!(down.len() <= up.len());
    }

    #[test]
    fn classes_need_their_headroom_and_footprint() {
        // A 3 m high tunnel (2.95 m of headroom) through a wall, and a 6 m wide open gap.
        let level = LevelDef {
            brushes: vec![
                brush([-20.0, -1.0, -20.0], [20.0, 0.0, 20.0]),
                // Wall across z ~0.3 from x -20..-2 and 4..20 (gap 6 m wide at x -2..4)...
                brush([-20.0, 0.0, -0.2], [-2.0, 6.0, 0.8]),
                brush([4.0, 0.0, -0.2], [20.0, 6.0, 0.8]),
                // ...with a 3 m high lintel over the gap's left half (x -2..1).
                brush([-2.0, 2.95, -0.2], [1.0, 6.0, 0.8]),
            ],
            ..Default::default()
        };
        let g = grid(&level);
        let open = [];
        let gnawer = NavClass::for_size(0.6, 2.0);
        let censer = NavClass::for_size(0.6, 2.5);
        let heron = NavClass::for_size(0.5, 3.4);
        let gaoler = NavClass::for_size(1.3, 3.7);
        assert_eq!(gnawer, NavClass::SMALL);
        assert_eq!(gaoler, NavClass::LARGE);
        assert!(!censer.wide && censer.headroom_dm == 26);
        assert!(!heron.wide && heron.headroom_dm == 35);
        let under = g
            .node_at(Vec3::new(-0.5, 0.0, 0.5), 0.5, NavClass::SMALL, &open)
            .unwrap();
        assert!(g.passable(under, gnawer, &open));
        assert!(g.passable(under, censer, &open), "2.5 m fits under 2.95 m");
        assert!(!g.passable(under, heron, &open), "3.4 m does not");
        let open_gap = g
            .node_at(Vec3::new(2.5, 0.0, 0.5), 0.5, NavClass::SMALL, &open)
            .unwrap();
        assert!(g.passable(open_gap, heron, &open));
        // The Gaoler needs room around it too: the middle of a 6 m field is fine, a node next
        // to the wall is not.
        let field = g
            .node_at(Vec3::new(0.5, 0.0, 10.5), 0.5, NavClass::SMALL, &open)
            .unwrap();
        assert!(g.passable(field, gaoler, &open));
        let by_wall = g
            .node_at(Vec3::new(10.5, 0.0, 1.5), 0.5, NavClass::SMALL, &open)
            .unwrap();
        assert!(g.passable(by_wall, gnawer, &open) && !g.passable(by_wall, gaoler, &open));
        // Paths: a Heron goes through the open half of the gap, never under the lintel.
        let mut s = NavScratch::default();
        let a = g
            .node_at(Vec3::new(-0.5, 0.0, -8.0), 0.5, NavClass::SMALL, &open)
            .unwrap();
        let b = g
            .node_at(Vec3::new(-0.5, 0.0, 8.0), 0.5, NavClass::SMALL, &open)
            .unwrap();
        let PathResult::Found(p) = g.find_path(&mut s, a, b, heron, &open, 20_000) else {
            panic!("heron path");
        };
        for n in &p {
            let node = &g.nodes[*n as usize];
            assert!(
                node.headroom >= 3.5,
                "heron under the lintel at {}",
                node.pos
            );
        }
    }

    fn vol(min: [f32; 3], max: [f32; 3]) -> Volume {
        Volume {
            min: Vec3::from(min),
            max: Vec3::from(max),
        }
    }

    /// A Gnawer-like class (leaps 8 m across, 3.5 m up, 8 m down, rides pads).
    fn leaper() -> NavClass {
        NavClass::SMALL.with_leaps(8.0, 3.5, 8.0).with_pads(true)
    }

    /// Floor A (x < 0) and floor B (x > 6) with a 6 m strip of deep water between them (a
    /// bed 4 m down, a kill volume to just over the surface), plus `extra` brushes.
    fn water_gap(extra: Vec<Brush>) -> LevelDef {
        let mut brushes = vec![
            brush([-12.0, -1.0, -6.0], [0.0, 0.0, 6.0]),
            brush([0.0, -5.0, -6.0], [6.0, -4.0, 6.0]),
            brush([6.0, -1.0, -6.0], [18.0, 0.0, 6.0]),
        ];
        brushes.extend(extra);
        LevelDef {
            brushes,
            kill_volumes: vec![
                vol([0.0, -5.0, -6.0], [6.0, -0.5, 6.0]),
                vol([-100.0, -40.0, -100.0], [100.0, -20.0, 100.0]),
            ],
            ..Default::default()
        }
    }

    fn at(g: &NavGrid, p: [f32; 3]) -> u32 {
        g.node_at(Vec3::from(p), 1.0, NavClass::SMALL, &[])
            .unwrap_or_else(|| panic!("no node at {p:?}"))
    }

    fn in_kill_volume(level: &LevelDef, p: Vec3) -> bool {
        level.kill_volumes.iter().any(|v| v.contains(p))
    }

    #[test]
    fn the_leap_arc_passes_both_ends_and_its_apex() {
        let (a, b) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(6.0, 2.0, 1.0));
        assert!(leap_point(a, b, 0.0).distance(a) < 1e-4);
        assert!(leap_point(a, b, 1.0).distance(b) < 1e-4);
        let top = (0..=100)
            .map(|i| leap_point(a, b, i as f32 / 100.0).y)
            .fold(f32::MIN, f32::max);
        assert!((top - leap_apex(a, b)).abs() < 0.01, "{top}");
        // The launch velocity flies exactly that arc (any gravity).
        for g in [20.0, 38.0, 60.0] {
            let (v, time) = leap_velocity(a, b, g);
            for i in 0..=10 {
                let t = time * i as f32 / 10.0;
                let p = a + v * t - Vec3::Y * (0.5 * g * t * t);
                let q = leap_point(a, b, i as f32 / 10.0);
                assert!(p.distance(q) < 1e-3, "g {g}: {p} vs {q}");
            }
        }
    }

    #[test]
    fn leaps_cross_a_water_strip_within_the_class_limits() {
        let level = water_gap(Vec::new());
        let g = grid(&level);
        assert!(g.stats.leaps > 10, "{:?}", g.stats);
        assert!(g.stats.leaps_over_deadly > 10);
        let (a, b) = (at(&g, [-4.5, 0.0, 0.5]), at(&g, [10.5, 0.0, 0.5]));
        let mut s = NavScratch::default();
        // Walking only: no way across.
        assert!(matches!(
            g.find_path(&mut s, a, b, NavClass::SMALL, &[], 20_000),
            PathResult::Partial(_) | PathResult::NotFound
        ));
        // A 4.5 m leaper can't make 7 m either; an 8 m one can.
        let short = NavClass::SMALL.with_leaps(4.5, 2.0, 6.0);
        assert!(!matches!(
            g.find_path(&mut s, a, b, short, &[], 20_000),
            PathResult::Found(_)
        ));
        let PathResult::Found(path) = g.find_path(&mut s, a, b, leaper(), &[], 20_000) else {
            panic!("an 8 m leaper crosses the water");
        };
        let route = g.smooth_route(&path, leaper(), &[]);
        let leaps: Vec<&Waypoint> = route.iter().filter(|w| w.jump.is_some()).collect();
        assert_eq!(leaps.len(), 1, "{route:?}");
        let j = g.jump(leaps[0].jump.unwrap()).unwrap();
        assert_eq!(j.kind, JumpKind::Leap);
        assert!(j.dist > 6.0 && j.dist <= 8.0, "{j:?}");
        // Nothing on the arc or the path is in the water.
        let from = g.nodes[j.from as usize].pos;
        let to = g.nodes[j.to as usize].pos;
        assert!(from.x < 0.0 && to.x > 6.0);
        for i in 0..=40 {
            let p = leap_point(from, to, i as f32 / 40.0);
            assert!(!in_kill_volume(&level, p), "arc in the water at {p}");
        }
        for &n in &path {
            assert!(!g.nodes[n as usize].is(node_flags::DEADLY));
        }
        // No leap ever lands on (or takes off from) a deadly floor.
        for j in g.jumps() {
            assert!(!g.nodes[j.to as usize].is(node_flags::DEADLY));
            assert!(!g.nodes[j.from as usize].is(node_flags::DEADLY));
        }
    }

    #[test]
    fn walking_round_beats_a_leap_that_saves_little() {
        // A 2 m bridge at z 4..6 where the water ends (z 4): its far half isn't a hazard.
        let mut level = water_gap(vec![brush([0.0, -1.0, 4.0], [6.0, 0.0, 6.0])]);
        level.kill_volumes[0].max.z = 4.0;
        let g = grid(&level);
        let (a, b) = (at(&g, [-3.5, 0.0, 2.5]), at(&g, [9.5, 0.0, 2.5]));
        let mut s = NavScratch::default();
        let PathResult::Found(path) = g.find_path(&mut s, a, b, leaper(), &[], 20_000) else {
            panic!("path");
        };
        let route = g.smooth_route(&path, leaper(), &[]);
        assert!(route.iter().all(|w| w.jump.is_none()), "{route:?}");
        // Far from the bridge the leap wins.
        let (a, b) = (at(&g, [-3.5, 0.0, -4.5]), at(&g, [9.5, 0.0, -4.5]));
        let PathResult::Found(path) = g.find_path(&mut s, a, b, leaper(), &[], 20_000) else {
            panic!("path");
        };
        let route = g.smooth_route(&path, leaper(), &[]);
        assert!(route.iter().any(|w| w.jump.is_some()), "{route:?}");
    }

    #[test]
    fn no_leap_where_the_arc_is_blocked() {
        // A block hanging low over the strip (from 1.2 m above the floors, too high to climb
        // onto): no headroom for the arc.
        let level = water_gap(vec![brush([0.0, 1.2, -6.0], [6.0, 12.0, 6.0])]);
        let g = grid(&level);
        let (a, b) = (at(&g, [-4.5, 0.0, 0.5]), at(&g, [10.5, 0.0, 0.5]));
        let mut s = NavScratch::default();
        assert!(!matches!(
            g.find_path(&mut s, a, b, leaper(), &[], 20_000),
            PathResult::Found(_)
        ));
        for j in g.jumps() {
            let (p, q) = (g.nodes[j.from as usize].pos, g.nodes[j.to as usize].pos);
            assert!(
                !(p.x < 0.0 && q.x > 6.0) && !(p.x > 6.0 && q.x < 0.0),
                "leap under the slab {p} -> {q}"
            );
        }
        // A 4 m high overhang: the narrow (2 m) arc fits under it, a tall (3.5 m) one doesn't.
        let level = water_gap(vec![brush([0.0, 4.0, -6.0], [6.0, 4.5, 6.0])]);
        let g = grid(&level);
        let (a, b) = (at(&g, [-4.5, 0.0, 0.5]), at(&g, [10.5, 0.0, 0.5]));
        let tall = NavClass::for_size(0.5, 3.4).with_leaps(8.0, 3.5, 8.0);
        assert_eq!(tall.clear_bit(), jump_clear::TALL);
        assert!(matches!(
            g.find_path(&mut s, a, b, leaper(), &[], 20_000),
            PathResult::Found(_)
        ));
        assert!(!matches!(
            g.find_path(&mut s, a, b, tall, &[], 20_000),
            PathResult::Found(_)
        ));
        // A wide class never leaps (no clearance bit for it on a leap).
        let wide = NavClass::LARGE.with_leaps(8.0, 3.5, 8.0);
        assert!(g.jumps().iter().all(|j| !wide.can_jump(j)));
    }

    #[test]
    fn no_leap_onto_a_floor_with_lava_right_past_it() {
        // Floor A (x < 0), then lava to x 12 with a 1 m island at x 4..5 in it, floor B past.
        let level = LevelDef {
            brushes: vec![
                brush([-12.0, -1.0, -6.0], [0.0, 0.0, 6.0]),
                brush([0.0, -3.0, -6.0], [12.0, -2.0, 6.0]),
                brush([4.0, -2.0, -6.0], [5.0, 0.0, 6.0]),
                brush([12.0, -1.0, -6.0], [24.0, 0.0, 6.0]),
            ],
            kill_volumes: vec![vol([0.0, -3.0, -6.0], [12.0, -1.0, 6.0])],
            ..Default::default()
        };
        let g = grid(&level);
        let island = at(&g, [4.5, 0.0, 0.5]);
        assert!(!g.nodes[island as usize].is(node_flags::DEADLY));
        for j in g.jumps() {
            let (p, q) = (g.nodes[j.from as usize].pos, g.nodes[j.to as usize].pos);
            assert!(
                !(q.x > 4.0 && q.x < 5.0 && p.x < 4.0),
                "a leap from {p} lands on the island at {q}: lava right past it"
            );
            assert!(!g.nodes[j.to as usize].is(node_flags::DEADLY));
        }
    }

    #[test]
    fn leaps_climb_a_ledge_with_a_run_up_and_drop_from_high_ones() {
        // A 3 m block (no ramp) and a 7.5 m one.
        let level = LevelDef {
            brushes: vec![
                brush([-20.0, -1.0, -20.0], [20.0, 0.0, 20.0]),
                brush([2.0, 0.0, -5.0], [10.0, 3.0, 5.0]),
                brush([-12.0, 0.0, -5.0], [-6.0, 7.5, 5.0]),
            ],
            ..Default::default()
        };
        let g = grid(&level);
        let (floor, top) = (at(&g, [-2.5, 0.0, 0.5]), at(&g, [6.5, 3.0, 0.5]));
        let high = g
            .node_at(Vec3::new(-8.5, 7.5, 0.5), 1.0, NavClass::SMALL, &[])
            .unwrap();
        let mut s = NavScratch::default();
        let PathResult::Found(up) = g.find_path(&mut s, floor, top, leaper(), &[], 20_000) else {
            panic!("a 3.5 m climber gets onto the 3 m block");
        };
        let route = g.smooth_route(&up, leaper(), &[]);
        let j = route
            .iter()
            .find_map(|w| w.jump)
            .and_then(|j| g.jump(j))
            .unwrap();
        assert!(j.rise > 2.9, "{j:?}");
        // The takeoff is a run-up away from the wall (the arc clears the lip).
        let from = g.nodes[j.from as usize].pos;
        assert!(from.x < 1.0, "takeoff at {from}");
        // A 2 m climber can't; nobody gets onto the 7.5 m one, but a leaper comes down.
        let low = NavClass::SMALL.with_leaps(8.0, 2.0, 8.0);
        assert!(!matches!(
            g.find_path(&mut s, floor, top, low, &[], 20_000),
            PathResult::Found(_)
        ));
        assert!(!matches!(
            g.find_path(&mut s, floor, high, leaper(), &[], 20_000),
            PathResult::Found(_)
        ));
        assert!(matches!(
            g.find_path(&mut s, high, floor, leaper(), &[], 20_000),
            PathResult::Found(_)
        ));
        assert!(!matches!(
            g.find_path(&mut s, high, floor, NavClass::SMALL, &[], 20_000),
            PathResult::Found(_)
        ));
    }

    #[test]
    fn pad_rides_land_where_the_pad_throws() {
        // A pad at the origin throwing onto a 5 m platform 10..16 m north.
        let level = LevelDef {
            brushes: vec![
                brush([-20.0, -1.0, -30.0], [20.0, 0.0, 20.0]),
                brush([-4.0, 0.0, -16.0], [4.0, 5.0, -10.0]),
            ],
            jump_pads: vec![JumpPad {
                trigger: vol([-1.0, 0.0, -1.0], [1.0, 0.5, 1.0]),
                velocity: Vec3::new(0.0, 22.0, -15.0),
            }],
            ..Default::default()
        };
        let g = grid(&level);
        assert!(g.stats.pad_links >= 1, "{:?}", g.stats);
        let rides: Vec<&NavJump> = g
            .jumps()
            .iter()
            .filter(|j| j.kind == JumpKind::Pad(0))
            .collect();
        for j in &rides {
            let (p, q) = (g.nodes[j.from as usize].pos, g.nodes[j.to as usize].pos);
            assert!(p.length() < 1.0, "the ride starts on the pad: {p}");
            assert!(
                (q.y - 5.0).abs() < 0.1 && q.z < -10.0 && q.z > -16.0,
                "lands on the platform: {q}"
            );
        }
        assert!(rides.iter().any(|j| j.clear & jump_clear::WIDE != 0));
        let (pad, top) = (at(&g, [0.5, 0.0, 0.5]), at(&g, [0.5, 5.0, -12.5]));
        let mut s = NavScratch::default();
        let walker = NavClass::SMALL;
        assert!(!matches!(
            g.find_path(&mut s, pad, top, walker, &[], 20_000),
            PathResult::Found(_)
        ));
        let rider = walker.with_pads(true);
        let PathResult::Found(p) = g.find_path(&mut s, pad, top, rider, &[], 20_000) else {
            panic!("the pad takes it up");
        };
        let route = g.smooth_route(&p, rider, &[]);
        let j = route.iter().find_map(|w| w.jump).and_then(|j| g.jump(j));
        assert_eq!(j.map(|j| j.kind), Some(JumpKind::Pad(0)));
    }

    #[test]
    fn predecessors_are_the_reverse_of_successors() {
        let level = water_gap(vec![brush([10.0, 0.0, -3.0], [14.0, 3.0, 3.0])]);
        let g = grid(&level);
        let class = leaper();
        let mut checked = 0;
        for n in 0..g.nodes.len() as u32 {
            if !g.passable(n, class, &[]) {
                continue;
            }
            g.for_each_successor(n, class, &[], |m| {
                let mut found = false;
                g.for_each_predecessor(m, class, &[], |p, cost| {
                    found |= p == n;
                    assert!(cost > 0.0);
                });
                assert!(found, "{n} -> {m} has no reverse");
                checked += 1;
            });
        }
        assert!(checked > 1000, "{checked}");
    }
}
