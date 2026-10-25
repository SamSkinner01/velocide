//! Enemy locomotion on top of the AI decisions (`ai.rs`):
//!
//! * **ground**: path following on the nav grid ([`crate::sim::nav`]): direct when a straight
//!   walk is possible, else A* (budgeted per tick, cached, re-planned when the goal moves or the
//!   agent gets stuck) with string-pulled waypoints; direct steering as the fallback when the
//!   agent is off the grid or there is no path;
//! * the **edge guard** ([`guard`]): never step towards lava / kill volumes or off a ledge
//!   deeper than allowed (paths may drop down, strafing never does);
//! * **flying** ([`fly_velocity`]): circle the target at the preferred range, hover 4–8 m above
//!   the ground (or the target's level), avoid walls with feeler sweeps;
//! * [`AiWorld`]: the shared AI state on `SimWorld` (nav grid, A* buffers, budget, stats).

use std::sync::Arc;

use engine::physics::{CollisionWorld, MASK_MOVEMENT};
use glam::Vec3;

use super::Enemy;
use super::ai::PlayerView;
use crate::level::LevelCollision;
use crate::sim::nav::{MAX_DROP, NavGrid, NavScratch, PathResult, STEP_UP};

/// A* searches allowed per tick (all enemies together); the rest keep their old path one more
/// tick. Straight-line checks are not budgeted (cheap).
pub const PATHS_PER_TICK: u32 = 4;
/// Node expansions per A* search before it gives up with a partial path.
pub const MAX_EXPAND: usize = 3000;
/// A waypoint counts as reached within this horizontal distance.
const WAYPOINT_RADIUS: f32 = 0.7;
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

/// Path following state of one enemy.
#[derive(Clone, Debug, Default)]
pub struct NavAgent {
    pub mode: PathMode,
    /// String-pulled waypoints (floor points).
    pub waypoints: Vec<Vec3>,
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
}

impl NavAgent {
    /// Forget the plan (target changed, teleported...).
    pub fn reset(&mut self) {
        *self = NavAgent::default();
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
}

/// Shared AI state on `SimWorld` (`SimWorld::ai`).
#[derive(Default)]
pub struct AiWorld {
    /// The level's nav grid. Built lazily on the first enemy tick, or up front by
    /// [`crate::sim::SimWorld::build_nav`] (the server does that at level load).
    pub nav: Option<Arc<NavGrid>>,
    scratch: NavScratch,
    pub stats: AiStats,
}

/// Per-tick navigation context (taken out of [`AiWorld`] while enemies step).
pub(crate) struct NavCtx {
    pub nav: Option<Arc<NavGrid>>,
    /// Door slot → open (`NavGrid::door_ids` order).
    pub doors: Vec<bool>,
    pub scratch: NavScratch,
    pub paths_left: u32,
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
    (secs.max(0.0) * rate as f32).round().clamp(1.0, u16::MAX as f32) as u16
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
    let Some(nav) = cx.nav.clone() else {
        e.nav.mode = PathMode::Fallback;
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
                let near_goal = |p: Vec3| {
                    horizontal(goal - p).length() < 1.6 && (goal.y - p.y).abs() < 1.2
                };
                if nav.walkable_line(s, g, class, &cx.doors) {
                    cx.stats.direct += 1;
                    a.mode = PathMode::Direct;
                    a.waypoints.clear();
                } else if cx.paths_left > 0 {
                    cx.paths_left -= 1;
                    cx.stats.paths += 1;
                    let result =
                        nav.find_path(&mut cx.scratch, s, g, class, &cx.doors, MAX_EXPAND);
                    cx.stats.expanded += cx.scratch.last_expanded as u64;
                    match result {
                        PathResult::Found(p) => {
                            a.waypoints = nav.smooth(&p, class, &cx.doors);
                            if near_goal(nav.nodes[g as usize].pos) {
                                a.waypoints.push(goal);
                            }
                            a.mode = PathMode::Path;
                        }
                        PathResult::Partial(p) => {
                            cx.stats.partial += 1;
                            a.waypoints = nav.smooth(&p, class, &cx.doors);
                            a.mode = PathMode::Partial;
                        }
                        PathResult::NotFound => {
                            cx.stats.failed += 1;
                            a.waypoints.clear();
                            a.mode = PathMode::Fallback;
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
            _ => {
                a.mode = PathMode::Fallback;
                a.waypoints.clear();
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
        return ((side * 0.8 + to * 0.2).normalize_or_zero() * speed, STEP_UP + 0.1);
    }

    let target = match a.mode {
        PathMode::None | PathMode::Direct | PathMode::Fallback => goal,
        PathMode::Path | PathMode::Partial => {
            while a.next < a.waypoints.len()
                && horizontal(a.waypoints[a.next] - pos).length() < WAYPOINT_RADIUS
            {
                a.next += 1;
            }
            // Look-ahead shortcut (string pulling from where we actually are).
            if a.next + 1 < a.waypoints.len() && (cx.tick + id).is_multiple_of(8) {
                let here = nav.node_at(pos, 1.0, class, &cx.doors);
                let there = nav.node_at(a.waypoints[a.next + 1], 0.5, class, &cx.doors);
                if let (Some(h), Some(t)) = (here, there)
                    && nav.walkable_line(h, t, class, &cx.doors)
                {
                    a.next += 1;
                }
            }
            match a.waypoints.get(a.next) {
                Some(&w) => w,
                // Reached the end of a partial path: wait there.
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

/// Flight steering for a flying enemy: circle `tp` inside the preferred range band, hover
/// between `min_height` and `max_height` above the ground (or the target's level when that is
/// higher), dodge walls and ceilings with feeler sweeps. `scale` slows the horizontal part
/// (wind-ups).
pub(crate) fn fly_velocity(
    world: &CollisionWorld,
    e: &mut Enemy,
    tp: &PlayerView,
    now: u32,
    scale: f32,
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
    let hidden = e.unseen_ticks > 45;
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
    let mut vy = ((want - pos.y) * 1.5).clamp(-speed * 0.5, speed * 0.5);
    if world
        .cast_ray(pos, Vec3::Y, r + 1.5, MASK_MOVEMENT)
        .is_some()
    {
        vy = vy.min(-2.0);
    }
    v.y = vy;

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
        if into < 0.0 {
            v -= n * into;
        }
        v += n * speed * (1.0 - hit.fraction) * 0.8;
        if hit.fraction < 0.4 && e.strafe_flip_in == 0 {
            e.strafe_sign = -e.strafe_sign;
            e.strafe_flip_in = 40;
        }
    }
    e.strafe_flip_in = e.strafe_flip_in.saturating_sub(1);
    v
}

/// Can a ground enemy strafe along `dir` (unit, horizontal) from where it is: nothing solid
/// right there and safe ground (no ledge, no lava)?
pub(crate) fn can_step(world: &CollisionWorld, nav: Option<&NavGrid>, e: &Enemy, dir: Vec3) -> bool {
    let r = e.def.radius;
    let feet = e.feet();
    let probe = feet + Vec3::Y * (e.def.height * 0.5);
    if world
        .cast_ray(probe, dir, r + 0.6, MASK_MOVEMENT)
        .is_some()
    {
        return false;
    }
    nav.is_none_or(|n| n.is_empty() || n.ground_ok(feet + dir * (r + 0.4), feet.y, STEP_UP + 0.1))
}
