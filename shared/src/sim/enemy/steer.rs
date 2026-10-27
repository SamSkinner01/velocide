//! Enemy locomotion on top of the AI decisions (`ai.rs`):
//!
//! * **ground**: path following on the nav grid ([`crate::sim::nav`]): direct when a straight
//!   walk is possible, else A* (budgeted per tick, cached, re-planned when the goal moves or the
//!   agent gets stuck) with string-pulled waypoints; a jump on the route (a leap or a pad ride)
//!   is walked up to precisely and handed to the AI (`NavAgent::jump_ready`); direct steering
//!   as the fallback when the agent is off the grid or there is no path;
//! * the **edge guard** ([`guard`]): never step towards lava / kill volumes or off a ledge
//!   deeper than allowed (paths may drop down, strafing never does);
//! * **flying** ([`fly_velocity`]): circle the target at the preferred range, hover 4–8 m above
//!   the ground (or the target's level), avoid walls with feeler sweeps; with the target hidden
//!   it flies to a detour point that sees it (over or round the obstacle) instead of dithering;
//! * the searches for a **re-form site** ([`reform_site`]: a stranded Mould melts and rises
//!   near its target) and a **vantage point** ([`vantage_site`]: a ranged Mould with no sight
//!   of its target walks to a spot that has it);
//! * [`AiWorld`]: the shared AI state on `SimWorld` (nav grid, A* buffers, budget, stats).

use std::sync::Arc;

use engine::physics::{CollisionWorld, MASK_MOVEMENT, MASK_SHOTS};
use glam::Vec3;

use super::Enemy;
use super::ai::PlayerView;
use crate::level::{LevelCollision, LevelDef, Volume};
use crate::sim::nav::{
    MAX_DROP, NavClass, NavGrid, NavScratch, PathResult, STEP_UP, Waypoint, node_flags,
};

/// A* searches allowed per tick (all enemies together); the rest keep their old path one more
/// tick. Straight-line checks are not budgeted (cheap).
pub const PATHS_PER_TICK: u32 = 4;
/// Re-form and vantage searches allowed per tick (all enemies together; a flyer's detour
/// search is throttled per flyer instead).
pub const SEARCHES_PER_TICK: u32 = 1;
/// Node expansions per A* search before it gives up with a partial path.
pub const MAX_EXPAND: usize = 3000;
/// A waypoint counts as reached within this horizontal distance.
const WAYPOINT_RADIUS: f32 = 0.7;
/// A jump starts once the agent stands this close to its takeoff node (horizontal); within
/// [`APPROACH`] of it the agent slows down and the edge guard trusts the node.
const TAKEOFF_RADIUS: f32 = 0.3;
const APPROACH: f32 = 1.5;
/// Re-plan when the goal moved more than this (+ 20 % of the distance to it).
const GOAL_MOVED: f32 = 1.5;
/// Stuck check period (ticks) and minimum progress (m) expected in it.
const PROGRESS_TICKS: u16 = 30;
const PROGRESS_MIN: f32 = 0.5;

/// How an agent is getting to its goal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathMode {
    /// Not planned yet.
    #[default]
    None,
    /// Straight walk (checked on the nav grid).
    Direct,
    /// Following A* waypoints to the goal.
    Path,
    /// The goal is unreachable: going to the closest reachable point.
    Partial,
    /// Off the grid / no path at all: direct steering (edge-guarded).
    Fallback,
}

/// Can the last plan reach its goal?
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    /// Not planned yet.
    #[default]
    Unknown,
    /// A full path (or a straight walk) to the goal.
    Full,
    /// The search hit its node budget first: far, maybe reachable (progress decides).
    Far,
    /// Unreachable from here: the partial path ends as close as it gets.
    Unreachable,
    /// The agent isn't on the nav grid.
    OffGrid,
}

/// Path following state of one enemy.
#[derive(Clone, Debug, Default)]
pub struct NavAgent {
    pub mode: PathMode,
    /// String-pulled waypoints (floor points; a jump's landing carries the jump).
    pub waypoints: Vec<Waypoint>,
    pub next: usize,
    /// Goal the current plan was made for.
    pub goal: Vec3,
    /// Ticks until the next re-plan.
    pub repath_in: u16,
    progress_pos: Vec3,
    progress_in: u16,
    /// Made no progress: re-plan and side-step.
    pub stuck: bool,
    /// Ticks of side-stepping left (unsticking).
    pub sidestep: u16,
    /// Whether the last plan reaches the goal.
    pub reach: Reach,
    /// Steering onto a jump's takeoff from close by this tick (the edge guard trusts it).
    pub approaching: bool,
    /// Standing on the takeoff of a jump: (jump index, index of its landing waypoint). The AI
    /// takes it and starts the leap / pad ride.
    pub jump_ready: Option<(u32, usize)>,
}

impl NavAgent {
    /// Forget the plan (target changed, teleported...).
    pub fn reset(&mut self) {
        *self = NavAgent::default();
    }

    /// At the end of a partial path: as close to an unreachable goal as it gets.
    pub fn at_partial_end(&self) -> bool {
        self.mode == PathMode::Partial && self.next >= self.waypoints.len()
    }

    /// Landed from the jump onto waypoint `landing`: follow the route on from there.
    pub fn landed(&mut self, landing: usize) {
        self.next = landing + 1;
        self.stuck = false;
        self.sidestep = 0;
        self.repath_in = self.repath_in.min(6);
    }
}

/// AI statistics (tests, debug overlay, `docs/enemies.md` measurements).
#[derive(Clone, Debug, Default)]
pub struct AiStats {
    /// A* searches run.
    pub paths: u64,
    pub partial: u64,
    pub failed: u64,
    /// Plans that turned out to be straight walks (no A*).
    pub direct: u64,
    /// Nodes expanded by all searches.
    pub expanded: u64,
    /// Plans postponed because the per-tick budget was used up.
    pub deferred: u64,
    /// Leaps and pad rides started; melts (re-forms) and stranded Moulds that found no
    /// re-form site; vantage and flyer detour searches.
    pub leaps: u64,
    pub pad_rides: u64,
    pub melts: u64,
    pub melt_no_site: u64,
    pub vantage_searches: u64,
    pub detours: u64,
}

/// Shared AI state on `SimWorld` (`SimWorld::ai`).
#[derive(Default)]
pub struct AiWorld {
    /// The level's nav grid. Built lazily on the first enemy tick, or up front by
    /// [`crate::sim::SimWorld::build_nav`] (the server does that at level load).
    pub nav: Option<Arc<NavGrid>>,
    scratch: NavScratch,
    pub stats: AiStats,
    /// Counter damage of punches that threw a latched enemy off this tick: applied once every
    /// enemy is back in the world (`special.rs`).
    pub(crate) throws: Vec<(crate::protocol::NetId, crate::sim::combat::DamageInfo)>,
}

/// Per-tick navigation context (taken out of [`AiWorld`] while enemies step).
pub(crate) struct NavCtx {
    pub nav: Option<Arc<NavGrid>>,
    /// Door slot → open (`NavGrid::door_ids` order).
    pub doors: Vec<bool>,
    pub scratch: NavScratch,
    pub paths_left: u32,
    /// Re-form / vantage searches left this tick.
    pub searches_left: u32,
    pub stats: AiStats,
    pub tick: u32,
}

impl AiWorld {
    pub(crate) fn begin_tick(&mut self, collision: &LevelCollision, tick: u32) -> NavCtx {
        let doors = self.nav.as_ref().map_or_else(Vec::new, |n| {
            n.door_ids
                .iter()
                .map(|&d| collision.is_door_open(d))
                .collect()
        });
        NavCtx {
            nav: self.nav.clone(),
            doors,
            scratch: std::mem::take(&mut self.scratch),
            paths_left: PATHS_PER_TICK,
            searches_left: SEARCHES_PER_TICK,
            stats: std::mem::take(&mut self.stats),
            tick,
        }
    }

    pub(crate) fn end_tick(&mut self, cx: NavCtx) {
        self.scratch = cx.scratch;
        self.stats = cx.stats;
    }
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

fn ticks(secs: f32, rate: u32) -> u16 {
    (secs.max(0.0) * rate as f32)
        .round()
        .clamp(1.0, u16::MAX as f32) as u16
}

/// Desired horizontal velocity (and the drop the edge guard may allow) for a ground enemy
/// walking to `goal` at `speed`.
pub(crate) fn ground_velocity(
    e: &mut Enemy,
    goal: Vec3,
    speed: f32,
    cx: &mut NavCtx,
    tick_rate: u32,
) -> (Vec3, f32) {
    let pos = e.feet();
    let id = e.id.0;
    e.nav.approaching = false;
    e.nav.jump_ready = None;
    let Some(nav) = cx.nav.clone() else {
        e.nav.mode = PathMode::Fallback;
        e.nav.reach = Reach::Unknown;
        return (horizontal(goal - pos).normalize_or_zero() * speed, MAX_DROP);
    };
    let class = e.nav_class();
    let on_ground = e.on_ground;
    let interval = ticks(e.def.ai.repath_interval, tick_rate);
    let a = &mut e.nav;
    a.repath_in = a.repath_in.saturating_sub(1);

    // Stuck detection (wanted to move, barely did).
    if a.progress_in == 0 {
        if a.mode != PathMode::None && horizontal(pos - a.progress_pos).length() < PROGRESS_MIN {
            a.stuck = true;
            a.sidestep = 18;
        }
        a.progress_pos = pos;
        a.progress_in = PROGRESS_TICKS;
    } else {
        a.progress_in -= 1;
    }

    let dist = horizontal(goal - pos).length();
    let moved = a.goal.distance(goal);
    let need = match a.mode {
        PathMode::None => true,
        _ if a.stuck || a.repath_in == 0 => true,
        // Re-checking a straight walk is cheap; re-planning a path waits a little.
        PathMode::Direct => moved > GOAL_MOVED,
        _ => moved > GOAL_MOVED + 0.2 * dist && a.repath_in < interval / 2,
    };
    if need {
        let below = if on_ground { 1.0 } else { MAX_DROP + 2.0 };
        let start = nav.node_at(pos, below, class, &cx.doors);
        let end = nav
            .node_at(goal, MAX_DROP + 2.0, class, &cx.doors)
            .or_else(|| nav.nearest_node(goal, 4, class, &cx.doors));
        let mut deferred = false;
        match (start, end) {
            (Some(s), Some(g)) => {
                let near_goal =
                    |p: Vec3| horizontal(goal - p).length() < 1.6 && (goal.y - p.y).abs() < 1.2;
                if nav.walkable_line(s, g, class, &cx.doors) {
                    cx.stats.direct += 1;
                    a.mode = PathMode::Direct;
                    a.reach = Reach::Full;
                    a.waypoints.clear();
                } else if cx.paths_left > 0 {
                    cx.paths_left -= 1;
                    cx.stats.paths += 1;
                    let result = nav.find_path(&mut cx.scratch, s, g, class, &cx.doors, MAX_EXPAND);
                    cx.stats.expanded += cx.scratch.last_expanded as u64;
                    match result {
                        PathResult::Found(p) => {
                            a.waypoints = nav.smooth_route(&p, class, &cx.doors);
                            if near_goal(nav.nodes[g as usize].pos) {
                                a.waypoints.push(Waypoint {
                                    pos: goal,
                                    node: g,
                                    jump: None,
                                });
                            }
                            a.mode = PathMode::Path;
                            a.reach = Reach::Full;
                        }
                        PathResult::Partial(p) => {
                            cx.stats.partial += 1;
                            a.waypoints = nav.smooth_route(&p, class, &cx.doors);
                            a.mode = PathMode::Partial;
                            a.reach = if cx.scratch.last_budget_hit {
                                Reach::Far
                            } else {
                                Reach::Unreachable
                            };
                        }
                        PathResult::NotFound => {
                            cx.stats.failed += 1;
                            a.waypoints.clear();
                            a.mode = PathMode::Fallback;
                            a.reach = Reach::Unreachable;
                        }
                    }
                } else {
                    cx.stats.deferred += 1;
                    deferred = true;
                    if a.waypoints.is_empty() && a.mode != PathMode::Direct {
                        a.mode = PathMode::Fallback;
                    }
                }
            }
            (start, _) => {
                a.mode = PathMode::Fallback;
                a.waypoints.clear();
                a.reach = if start.is_none() {
                    Reach::OffGrid
                } else {
                    Reach::Unreachable
                };
            }
        }
        if deferred {
            a.repath_in = 1;
        } else {
            a.next = 0;
            a.goal = goal;
            a.stuck = false;
            a.repath_in = interval + (id % 8) as u16;
        }
    }

    // Unsticking: a short side-step.
    if a.sidestep > 0 {
        a.sidestep -= 1;
        let to = horizontal(goal - pos).normalize_or(Vec3::Z);
        let side = Vec3::new(-to.z, 0.0, to.x) * e.strafe_sign;
        return (
            (side * 0.8 + to * 0.2).normalize_or_zero() * speed,
            STEP_UP + 0.1,
        );
    }

    let mut speed = speed;
    let target = match a.mode {
        PathMode::None | PathMode::Direct | PathMode::Fallback => goal,
        PathMode::Path | PathMode::Partial => {
            // Reached walking waypoints are passed; a jump's landing only by the jump itself.
            while a.next < a.waypoints.len()
                && a.waypoints[a.next].jump.is_none()
                && horizontal(a.waypoints[a.next].pos - pos).length() < WAYPOINT_RADIUS
            {
                a.next += 1;
            }
            // Look-ahead shortcut (string pulling from where we actually are), never across a
            // jump.
            if a.next + 1 < a.waypoints.len()
                && a.waypoints[a.next].jump.is_none()
                && a.waypoints[a.next + 1].jump.is_none()
                && (cx.tick + id).is_multiple_of(8)
            {
                let here = nav.node_at(pos, 1.0, class, &cx.doors);
                let there = a.waypoints[a.next + 1].node;
                if let Some(h) = here
                    && nav.walkable_line(h, there, class, &cx.doors)
                {
                    a.next += 1;
                }
            }
            match a.waypoints.get(a.next) {
                // The next stretch is a jump: walk onto its takeoff precisely, slowing down
                // (the edge guard trusts the node), then hand it to the AI.
                Some(&Waypoint { jump: Some(j), .. }) => {
                    let Some(takeoff) = nav.jump(j).map(|l| nav.nodes[l.from as usize].pos) else {
                        a.mode = PathMode::None;
                        return (Vec3::ZERO, STEP_UP);
                    };
                    let d = horizontal(takeoff - pos).length();
                    if d <= TAKEOFF_RADIUS && on_ground {
                        a.jump_ready = Some((j, a.next));
                        return (Vec3::ZERO, STEP_UP);
                    }
                    if d < APPROACH {
                        a.approaching = true;
                        speed *= (d / APPROACH).clamp(0.25, 1.0);
                    }
                    takeoff
                }
                Some(w) => w.pos,
                // Reached the end of a partial path: the AI prowls there.
                None if a.mode == PathMode::Partial => return (Vec3::ZERO, STEP_UP),
                None => goal,
            }
        }
    };
    let to = horizontal(target - pos);
    if to.length() < 0.05 {
        return (Vec3::ZERO, STEP_UP);
    }
    // Paths and straight walks were checked on the grid and may drop down ledges.
    let drop = match a.mode {
        PathMode::Fallback | PathMode::None => STEP_UP + 0.1,
        _ => MAX_DROP,
    };
    (to.normalize() * speed, drop)
}

/// Edge guard: if walking along `desired` would put a grounded enemy over lava / a kill volume
/// or off a ledge deeper than `max_drop`, try turning up to 90° either way, else stop.
pub(crate) fn guard(nav: Option<&NavGrid>, e: &Enemy, desired: Vec3, max_drop: f32) -> Vec3 {
    let Some(nav) = nav else {
        return desired;
    };
    let flat = horizontal(desired);
    let speed = flat.length();
    if e.is_flying() || !e.on_ground || speed < 1e-3 || nav.is_empty() {
        return desired;
    }
    let pos = e.feet();
    let look = e.def.radius + 0.35;
    let ok = |dir: Vec3| nav.ground_ok(pos + dir * look, pos.y, max_drop);
    let dir = flat / speed;
    if ok(dir) {
        return desired;
    }
    for deg in [45.0f32, -45.0, 90.0, -90.0] {
        let d = glam::Quat::from_rotation_y(deg.to_radians()) * dir;
        if ok(d) {
            return Vec3::new(d.x * speed * 0.8, desired.y, d.z * speed * 0.8);
        }
    }
    Vec3::new(0.0, desired.y, 0.0)
}

/// Is a grounded enemy's current horizontal velocity about to carry it over lava / off a
/// ledge deeper than `max_drop` (within this tick plus its radius)?
pub(crate) fn heading_off_edge(nav: Option<&NavGrid>, e: &Enemy, max_drop: f32, dt: f32) -> bool {
    let Some(nav) = nav.filter(|n| !n.is_empty()) else {
        return false;
    };
    let v = horizontal(e.velocity);
    let speed = v.length();
    if e.is_flying() || !e.on_ground || speed < 0.5 {
        return false;
    }
    let pos = e.feet();
    let look = pos + v / speed * (e.def.radius + 0.3 + speed * dt);
    !nav.ground_ok(look, pos.y, max_drop)
}

/// A point (hover centre) from which a flyer at `pos` (radius `r`) sees `chest`, and that a
/// sphere sweep reaches from where it is: straight up first (over a low wall), no higher than
/// `top` above it, then to either side of the line to the target (round a pillar), its
/// orbit's side first, at its height or a little higher. `None`: nothing nearby sees it.
fn find_detour(
    world: &CollisionWorld,
    pos: Vec3,
    r: f32,
    chest: Vec3,
    top: f32,
    side: f32,
) -> Option<Vec3> {
    let ceiling = world
        .cast_ray(pos, Vec3::Y, 40.0, MASK_MOVEMENT)
        .map_or(40.0, |h| h.distance)
        - r
        - 0.5;
    let top = top.min(ceiling);
    let sees = |p: Vec3| !world.line_blocked(p, chest, MASK_SHOTS);
    let reach = |p: Vec3| {
        world
            .cast_sphere(pos, r + 0.1, p - pos, MASK_MOVEMENT)
            .is_none()
    };
    for up in [3.0, 6.0, 9.0] {
        if up > top {
            break;
        }
        let p = pos + Vec3::Y * up;
        if sees(p) && reach(p) {
            return Some(p);
        }
    }
    let to = horizontal(chest - pos).normalize_or(Vec3::Z);
    for deg in [60.0f32, 100.0, 140.0] {
        for s in [side, -side] {
            for dh in [0.0f32, 3.0] {
                let dir = glam::Quat::from_rotation_y((deg * s).to_radians()) * to;
                let p = pos + dir * 7.0 + Vec3::Y * dh.min(top.max(0.0));
                if sees(p) && reach(p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Flight steering for a flying enemy: circle `tp` inside the preferred range band, hover
/// between `min_height` and `max_height` above the ground (or the target's level when that is
/// higher), dodge walls and ceilings with feeler sweeps. With the target out of sight for a
/// moment (or the orbit turning back and forth at a wall) it flies to a detour point that sees
/// it ([`find_detour`]: over or round the obstacle) and circles on from there. `scale` slows
/// the horizontal part (wind-ups).
pub(crate) fn fly_velocity(
    world: &CollisionWorld,
    e: &mut Enemy,
    tp: &PlayerView,
    now: u32,
    scale: f32,
    stats: &mut AiStats,
) -> Vec3 {
    let def = e.def.clone();
    let (min_h, max_h) = match def.locomotion {
        super::Locomotion::Flying {
            min_height,
            max_height,
        } => (min_height, max_height.max(min_height)),
        super::Locomotion::Ground => (0.0, 0.0),
    };
    let speed = def.speed;
    // Everything below is about the hover centre (the model's origin).
    let pos = e.center();
    let r = def.radius;

    // Ground below (re-measured every few ticks).
    if e.ground_check_in == 0 {
        e.ground_y = world
            .cast_ray(pos, Vec3::NEG_Y, 60.0, MASK_MOVEMENT)
            .map_or(pos.y - max_h, |h| h.point.y);
        e.ground_check_in = 6;
    } else {
        e.ground_check_in -= 1;
    }

    // Detour: hidden for a moment (or dithering at a wall, below): fly to a point that sees
    // the target instead of nosing into what is in the way. Dropped once there, once the
    // target is back in sight, or after 2 s.
    let hidden = e.unseen_ticks > 45;
    if let Some((point, until)) = e.detour
        && (now >= until || pos.distance(point) < 1.5 || e.unseen_ticks == 0)
    {
        e.detour = None;
    }
    if e.detour.is_none() && hidden && now >= e.search_after {
        e.search_after = now + 30;
        let top = e.ground_y + max_h + 1.0 - pos.y;
        e.detour = find_detour(world, pos, r, tp.chest, top, e.strafe_sign).map(|p| (p, now + 120));
        stats.detours += e.detour.is_some() as u64;
    }

    let mut v = if let Some((point, _)) = e.detour {
        (point - pos).normalize_or_zero() * speed * scale.max(0.6)
    } else {
        // Horizontal: orbit inside the band, close in when the target stays out of sight.
        let (rmin, rmax) = def.ai.preferred_range.unwrap_or((5.0, 12.0));
        let mid = (rmin + rmax) * 0.5;
        let away = horizontal(pos - tp.position);
        let dist = away.length();
        let out = if dist > 1e-3 {
            away / dist
        } else {
            Vec3::new(e.yaw.sin(), 0.0, e.yaw.cos())
        };
        let tangent = Vec3::new(-out.z, 0.0, out.x) * e.strafe_sign;
        let radial = if hidden {
            -1.0
        } else {
            ((mid - dist) / ((rmax - rmin) * 0.5).max(1.0)).clamp(-1.0, 1.0)
        };
        let orbit = if def.ai.strafe && !hidden { 0.85 } else { 0.0 };
        let mut v = (tangent * orbit + out * radial).normalize_or_zero() * speed * scale;

        // Altitude: hover band above the ground, or above the target when it stands higher.
        let base = e.ground_y.max(tp.position.y);
        let bob = ((now as f32 * 0.03) + e.id.0 as f32).sin() * 0.8;
        let want = (base + (min_h + max_h) * 0.5 + bob)
            .clamp(base + min_h, base + max_h)
            .max(e.ground_y + min_h);
        v.y = ((want - pos.y) * 1.5).clamp(-speed * 0.5, speed * 0.5);
        v
    };
    if world
        .cast_ray(pos, Vec3::Y, r + 1.5, MASK_MOVEMENT)
        .is_some()
    {
        v.y = v.y.min(-2.0);
    }

    // Feelers: sweep ahead; slide along / push off what is in the way.
    let ahead = v * 0.45;
    let ahead = if ahead.length() < 2.0 {
        ahead.normalize_or_zero() * 2.0
    } else {
        ahead
    };
    if ahead != Vec3::ZERO
        && let Some(hit) = world.cast_sphere(pos, r + 0.2, ahead, MASK_MOVEMENT)
    {
        let n = hit.normal;
        let into = v.dot(n);
        // Flying (nearly) straight into it: sliding along won't get round it.
        let head_on = into < -0.7 * v.length();
        if into < 0.0 {
            v -= n * into;
        }
        v += n * speed * (1.0 - hit.fraction) * 0.8;
        // Walls turn the orbit round (when close, or when it heads straight at one: a pillar
        // on its circle would otherwise hold it in place); a ceiling or floor just deflects it
        // (flipping under a roof made flyers dither in place). A second flip soon after the
        // last one is dithering: it looks for a way over or round instead.
        let wall = n.y.abs() < 0.5;
        if wall && (hit.fraction < 0.4 || head_on) && e.strafe_flip_in == 0 && e.detour.is_none() {
            if now.saturating_sub(e.last_flip) < 100 && now >= e.search_after {
                e.search_after = now + 30;
                let top = e.ground_y + max_h + 1.0 - pos.y;
                e.detour = find_detour(world, pos, r, tp.chest, top, e.strafe_sign)
                    .map(|p| (p, now + 120));
                stats.detours += e.detour.is_some() as u64;
            }
            if e.detour.is_none() {
                e.strafe_sign = -e.strafe_sign;
            }
            e.strafe_flip_in = 40;
            e.last_flip = now;
        }
    }
    e.strafe_flip_in = e.strafe_flip_in.saturating_sub(1);
    v
}

/// What a re-form / vantage search looks for ([`reform_site`], [`vantage_site`]).
pub(crate) struct SiteQuery<'a> {
    pub class: NavClass,
    pub doors: &'a [bool],
    /// The target's feet and eye (re-form) or chest (vantage).
    pub target: Vec3,
    pub target_eye: Vec3,
    /// Preferred distance band from the target (metres; horizontal for a re-form, eye to
    /// chest for a vantage point).
    pub band: (f32, f32),
    /// The Mould's eye height and radius.
    pub eye_height: f32,
    pub radius: f32,
    /// Other enemies (feet, radius) not to rise on top of.
    pub others: &'a [(Vec3, f32)],
    /// Stay inside this volume (an encounter's arena while the fight is on).
    pub arena: Option<Volume>,
}

/// Where a stranded Mould re-forms (docs/enemies.md "Melting and re-forming"): searching back
/// from its target's node over the reverse links (doors as they are now, so never in a room
/// cut off from the target), a node of its class from which the target is reachable — never
/// deadly or at a hazard edge, never in a kill volume, never on another Mould, at least 3 m
/// from the target, preferably in the band, and the first of the best 24 that is out of the
/// target's sight (else the best one).
pub(crate) fn reform_site(
    nav: &NavGrid,
    scratch: &mut NavScratch,
    world: &CollisionWorld,
    level: &LevelDef,
    q: &SiteQuery,
) -> Option<Vec3> {
    let goal = nav
        .node_at(q.target, 3.0, q.class, q.doors)
        .or_else(|| nav.nearest_node(q.target, 6, q.class, q.doors))?;
    let (near, far) = q.band;
    let mut cands: Vec<(f32, u32)> = Vec::new();
    nav.search(
        scratch,
        goal,
        q.class,
        q.doors,
        true,
        far * 2.5 + 20.0,
        6000,
        |n, cost| {
            let node = &nav.nodes[n as usize];
            let p = node.pos;
            let d = horizontal(p - q.target).length();
            let ok = !node.is(node_flags::HAZARD)
                && !node.is(node_flags::DEADLY)
                && d >= 3.0
                && !level
                    .kill_volumes
                    .iter()
                    .any(|v| v.contains(p + Vec3::Y * 0.1))
                && q.arena.is_none_or(|a| a.contains(p + Vec3::Y * 0.1))
                && !q
                    .others
                    .iter()
                    .any(|(o, r)| horizontal(*o - p).length() < r + q.radius + 0.4);
            if ok {
                let miss = (near - d).max(0.0) + (d - far).max(0.0);
                cands.push((miss * 2.0 + cost * 0.05, n));
            }
            true
        },
    );
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let eye = |n: u32| nav.nodes[n as usize].pos + Vec3::Y * q.eye_height;
    cands
        .iter()
        .take(24)
        .find(|(_, n)| world.line_blocked(q.target_eye, eye(*n), MASK_SHOTS))
        .or(cands.first())
        .map(|(_, n)| nav.nodes[*n as usize].pos)
}

/// A spot a ranged Mould standing on `start` can walk to (forward over its links) from which
/// it sees the target's chest (`q.target`) within its band (eye to chest): candidates scored by
/// walking cost plus their distance from the band's middle, the sight lines of the best 24
/// tested (at least 1.5 m apart, so a row of blind spots doesn't use them all up). Not a
/// hazard edge.
pub(crate) fn vantage_site(
    nav: &NavGrid,
    scratch: &mut NavScratch,
    world: &CollisionWorld,
    start: u32,
    q: &SiteQuery,
) -> Option<Vec3> {
    let (min, max) = q.band;
    let mid = (min + max) * 0.5;
    let mut cands: Vec<(f32, u32)> = Vec::new();
    nav.search(
        scratch,
        start,
        q.class,
        q.doors,
        false,
        40.0,
        3000,
        |n, cost| {
            let node = &nav.nodes[n as usize];
            let d = (node.pos + Vec3::Y * q.eye_height).distance(q.target);
            if !node.is(node_flags::HAZARD) && d >= min && d <= max {
                cands.push((cost + (d - mid).abs(), n));
            }
            true
        },
    );
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut tested: Vec<Vec3> = Vec::new();
    for (_, n) in cands {
        let p = nav.nodes[n as usize].pos;
        if tested.iter().any(|t| t.distance(p) < 1.5) {
            continue;
        }
        if !world.line_blocked(p + Vec3::Y * q.eye_height, q.target, MASK_SHOTS) {
            return Some(p);
        }
        tested.push(p);
        if tested.len() >= 24 {
            break;
        }
    }
    None
}

/// Can a ground enemy strafe along `dir` (unit, horizontal) from where it is: nothing solid
/// right there and safe ground (no ledge, no lava)?
pub(crate) fn can_step(
    world: &CollisionWorld,
    nav: Option<&NavGrid>,
    e: &Enemy,
    dir: Vec3,
) -> bool {
    let r = e.def.radius;
    let feet = e.feet();
    let probe = feet + Vec3::Y * (e.def.height * 0.5);
    if world.cast_ray(probe, dir, r + 0.6, MASK_MOVEMENT).is_some() {
        return false;
    }
    nav.is_none_or(|n| n.is_empty() || n.ground_ok(feet + dir * (r + 0.4), feet.y, STEP_UP + 0.1))
}
