//! Enemy AI (server only): **idle → alert → chase → wind-up (telegraph) → strike → recover →
//! chase**, plus stunned and dying.
//!
//! * **Alert**: a targetable player within sight range with line of sight, or getting hurt
//!   (`EnemyAlerted` → alert bark fx). Arena spawns are `aware`: they always know where the
//!   players are.
//! * **Targeting** (every `retarget_interval`): a player who dealt ≥ `aggro_switch_fraction`
//!   of max health in the last 2 s; else keep the current target unless another visible one is
//!   much closer (`target_stickiness`); else the nearest visible (aware: nearest at all).
//!   Targets that die / disconnect / become untargetable ([`is_targetable`]) are dropped at
//!   once; lost from sight for `lose_target_time` → idle (not for aware enemies).
//! * **Chase**: ground enemies follow the nav grid (`steer.rs`); ranged ones hold their range
//!   band, strafe and look for line of sight; flyers circle at hover height.
//! * **Attack**: the first usable attack in definition order (cooldown, range, target speed,
//!   line of sight) starts a **wind-up** (telegraph: `EnemyAttack` windup fx + replicated
//!   progress; parryable attacks open the parry window; laser attacks track their aim and lock
//!   it at the end), then the **strike** (melee arc / lunge or pounce / flyer dive / slam
//!   shockwave / projectile burst, lobbed or along the locked aim), then **recover**. Lunges
//!   and dives end early when the level stops them.
//! * Movement: capsule collide-and-slide against the level ([`move_enemy`]), soft separation
//!   while moving plus a hard push-apart pass, an edge guard against lava and ledges, knockback
//!   with stagger; enemies whose feet enter a kill volume die.
//! * **Getting there** (docs/enemies.md "Leaps, pads, melting"): a jump on the nav path is a
//!   **leap** (a crouch, then the nav arc, guided past a lip that steals its speed) or a **pad
//!   ride**; melee Moulds whose target is unreachable **prowl** the nearest reachable edge,
//!   ranged ones walk to a **vantage point** that sees it; a Mould stranded or stuck for
//!   `MeltDef::after` **melts** into a puddle and **re-forms** near its target.

use engine::physics::{CollisionWorld, MASK_MOVEMENT, MASK_SHOTS};
use glam::Vec3;

use super::steer::{self, NavCtx, Reach, SiteQuery};
use super::{
    AiState, AimLaser, AttackDef, AttackKind, Enemy, EnemyDef, LeapPlan, Locomotion, ParryWindow,
};
use crate::movement::SKIN;
use crate::protocol::NetId;
use crate::sim::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use crate::sim::nav::{JumpKind, MAX_DROP, STEP_UP, leap_velocity};
use crate::sim::parry::ParryKey;
use crate::sim::weapons::{SpreadRng, spread_dir};
use crate::sim::{HitTarget, PlayerId, SimEvent, SimPlayer, SimWorld};
use crate::tuning::Tuning;

/// Knockback (m/s, after `knockback_scale`) above which an enemy staggers.
pub(crate) const STAGGER_KNOCKBACK: f32 = 4.0;
/// Stagger duration (s): no steering, slow braking.
pub(crate) const STAGGER_TIME: f32 = 0.3;
/// Reach (between body surfaces) of a lunging strike while it travels.
const LUNGE_REACH: f32 = 1.4;
/// Reach (between body surfaces, in 3D) of a diving flyer.
const DIVE_REACH: f32 = 0.6;
/// A lunge (along the ground) whose last move covered less than this fraction of its speed
/// hit a wall (or the edge guard stopped it): the strike ends.
const LUNGE_BLOCKED: f32 = 0.35;
/// Push-apart relaxation passes per tick.
const PUSH_PASSES: u32 = 3;
/// Extra clearance kept beyond the sweep skin (see `move_enemy`).
const NUDGE: f32 = SKIN * 0.2;
/// Penetration (m) beyond which a depenetration contact is double-checked (`move_enemy`).
const DEEP_CONTACT: f32 = 0.05;

/// Can enemies target / attack this player? Dead (and, once game modes add it, downed or
/// spectating) players are not targets. **Hook for the game modes**: extend it when adding
/// player states that enemies must ignore.
pub fn is_targetable(p: &SimPlayer) -> bool {
    !p.combat.dead && !p.notarget
}

/// What enemies know about a player this tick.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlayerView {
    pub id: PlayerId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub chest: Vec3,
    pub radius: f32,
    pub height: f32,
    pub targetable: bool,
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

fn move_towards(from: Vec3, to: Vec3, max_delta: f32) -> Vec3 {
    let d = to - from;
    let len = d.length();
    if len <= max_delta || len < 1e-6 {
        to
    } else {
        from + d * (max_delta / len)
    }
}

/// Yaw that looks along `dir` (see `input::look_dir`).
fn yaw_towards(dir: Vec3) -> Option<f32> {
    let h = horizontal(dir);
    (h.length_squared() > 1e-6).then(|| (-h.x).atan2(-h.z))
}

fn turn_towards(yaw: f32, target: f32, max_step: f32) -> f32 {
    let mut d = (target - yaw).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    yaw + d.clamp(-max_step, max_step)
}

/// Rotates unit vector `from` towards `to` by at most `max_angle` radians.
fn rotate_towards(from: Vec3, to: Vec3, max_angle: f32) -> Vec3 {
    let angle = from.angle_between(to);
    if !angle.is_finite() || angle <= max_angle {
        return to;
    }
    let axis = from.cross(to);
    if axis.length_squared() < 1e-12 {
        return to;
    }
    (glam::Quat::from_axis_angle(axis.normalize(), max_angle) * from).normalize_or(to)
}

/// Shortest reach of an enemy's plain melee attacks (no lunges; `INFINITY` without any).
fn melee_reach(def: &EnemyDef) -> f32 {
    def.attacks
        .iter()
        .filter(|a| matches!(a.kind, AttackKind::Melee | AttackKind::Slam) && a.lunge <= 0.0)
        .map(|a| a.range)
        .fold(f32::INFINITY, f32::min)
}

/// Closest point to `p` on the segment `a`–`b`.
fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-9 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

/// Launch velocity for a lob from `from` that lands on `to` under gravity `g`: the arc that
/// needs the least speed (launched at 45° + half the elevation angle), capped at `max_speed`
/// (out of reach: `max_speed` at 45°, falling short). Returns (velocity, flight time).
pub fn lob_velocity(from: Vec3, to: Vec3, g: f32, max_speed: f32) -> (Vec3, f32) {
    let d = to - from;
    let flat = horizontal(d);
    let x = flat.length().max(1e-3);
    let fwd = flat.normalize_or(Vec3::NEG_Z);
    if g <= 1e-3 {
        let v = d.normalize_or(fwd) * max_speed;
        return (v, d.length() / max_speed.max(1e-3));
    }
    let y = d.y;
    let mut theta = std::f32::consts::FRAC_PI_4 + 0.5 * y.atan2(x);
    let mut speed = (g * (y + (x * x + y * y).sqrt())).max(0.0).sqrt();
    if !speed.is_finite() || speed > max_speed {
        theta = std::f32::consts::FRAC_PI_4;
        speed = max_speed;
    }
    let v = fwd * (speed * theta.cos()) + Vec3::Y * (speed * theta.sin());
    (v, x / (speed * theta.cos()).max(1e-3))
}

/// The low (flat) arc from `from` onto `to` under gravity `g` at exactly `speed`: (velocity,
/// flight time), or `None` when `speed` can't reach it.
fn lob_low_arc(from: Vec3, to: Vec3, g: f32, speed: f32) -> Option<(Vec3, f32)> {
    let d = to - from;
    let flat = horizontal(d);
    let x = flat.length();
    if g <= 1e-3 || x < 1e-3 || speed <= 1e-3 {
        return None;
    }
    let v2 = speed * speed;
    let disc = v2 * v2 - g * (g * x * x + 2.0 * d.y * v2);
    if disc < 0.0 {
        return None;
    }
    let theta = ((v2 - disc.sqrt()) / (g * x)).atan();
    let fwd = flat / x;
    let v = fwd * (speed * theta.cos()) + Vec3::Y * (speed * theta.sin());
    Some((v, x / (speed * theta.cos()).max(1e-3)))
}

/// Yaw offset (radians) of shot `i` of `count` fanned evenly across `spread` degrees: the
/// first one straight, then alternating left and right.
fn fan_offset(i: u32, count: u32, spread: f32) -> f32 {
    if count <= 1 {
        return 0.0;
    }
    let step = spread.to_radians() / (count - 1) as f32;
    let k = i.div_ceil(2) as f32;
    if i % 2 == 1 { -k * step } else { k * step }
}

impl SimWorld {
    pub(crate) fn player_views(&self, t: &Tuning) -> Vec<PlayerView> {
        self.players
            .values()
            .map(|p| {
                let (center, _, radius) = p.movement.capsule(&t.movement);
                PlayerView {
                    id: p.id,
                    position: p.movement.position,
                    velocity: p.movement.velocity,
                    chest: center + Vec3::Y * 0.3,
                    radius,
                    height: p.movement.height(&t.movement),
                    targetable: is_targetable(p),
                }
            })
            .collect()
    }

    /// Builds the level's nav grid now (the server calls this at level load so the first enemy
    /// tick doesn't stall; otherwise it is built lazily). Returns the build time (ms).
    pub fn build_nav(&mut self) -> f32 {
        let grid = crate::sim::nav::NavGrid::build(&self.level, &self.collision.world);
        let ms = grid.stats.build_ms;
        let s = &grid.stats;
        log::info!(
            "nav grid for '{}': {}x{} columns, {} floors ({} walkable, {} large, {} hazard, {} deadly), {} links, {} leaps, {} pad rides, {:.1} ms",
            self.level_name,
            grid.nx,
            grid.nz,
            s.floors,
            s.small,
            s.large,
            s.hazard,
            s.deadly,
            s.links,
            s.leaps,
            s.pad_links,
            ms
        );
        self.ai.nav = Some(std::sync::Arc::new(grid));
        ms
    }

    /// Dev (`/melt`): every living ground Mould that can melt does so on its next chasing tick,
    /// stranded or not, and re-forms near its target. Returns how many will.
    pub fn melt_enemies(&mut self) -> usize {
        let mut n = 0;
        for e in self.enemies.values_mut() {
            if e.is_alive() && !e.is_flying() && !e.is_dummy() && e.def.melt.after > 0.0 {
                e.melt_now = true;
                n += 1;
            }
        }
        n
    }

    /// Runs every enemy's AI and movement for one tick.
    pub(crate) fn step_enemies(&mut self, t: &Tuning) {
        // Slag lines burn on (and expire) whether or not their Smelters are still around.
        self.step_enemy_hazards(t);
        if self.enemies.is_empty() {
            return;
        }
        if self.frozen {
            self.step_frozen_enemies(t);
            return;
        }
        if self.ai.nav.is_none() {
            self.build_nav();
        }
        let mut cx = self.ai.begin_tick(&self.collision, self.tick);
        let players = self.player_views(t);
        let bodies: Vec<(NetId, Vec3, f32)> = self
            .enemies
            .values()
            .filter(|e| e.is_alive())
            .map(|e| (e.id, e.feet(), e.radius()))
            .collect();
        let ids: Vec<NetId> = self.enemies.keys().copied().collect();
        for id in ids {
            let Some(mut e) = self.enemies.remove(&id) else {
                continue;
            };
            // Soft separation from other enemies while moving (no stacking).
            let mut separation = Vec3::ZERO;
            if e.is_alive() {
                let feet = e.feet();
                for (other, pos, r) in &bodies {
                    if *other == id || (pos.y - feet.y).abs() > 2.5 {
                        continue;
                    }
                    let d = horizontal(feet - *pos);
                    let min = e.radius() + r + 0.3;
                    let len = d.length();
                    if len < min && len > 1e-4 {
                        separation += d / len * ((min - len) / min) * e.def.speed;
                    }
                }
            }
            let keep = self.step_enemy(t, &mut e, &players, separation, &mut cx);
            if keep {
                self.enemies.insert(id, e);
            } else {
                self.events.push(SimEvent::EnemyRemoved { id });
            }
        }
        // Punches that threw a latched Snuffer off land now that it is back in the world.
        self.apply_pending_throws(t);
        self.push_enemies_apart(cx.nav.as_deref());
        self.enemy_world_rules(t);
        self.ai.end_tick(cx);
        let tick = self.tick;
        for e in self.enemies.values_mut() {
            e.trail.record(tick, e.position);
        }
    }

    /// The match is over (`SimWorld::frozen`): living enemies drop their target and any attack
    /// in progress and just stand (gravity still applies); corpses stay.
    fn step_frozen_enemies(&mut self, t: &Tuning) {
        let dt = self.dt();
        let gravity = t.movement.gravity;
        let world = &self.collision.world;
        for e in self.enemies.values_mut() {
            if e.is_alive() && (e.state != AiState::Idle || e.target.is_some()) {
                e.target = None;
                // A latched Snuffer lets go; an unpoured line just burns out (harmlessly); a
                // leap or a melt is off (it stands where it is).
                e.latched = None;
                e.pour_line = None;
                e.leap = None;
                e.reform_at = None;
                e.nav.reset();
                e.set_state(AiState::Idle, 0);
            }
            if e.is_alive() {
                move_enemy(world, e, Vec3::ZERO, dt, gravity);
            }
        }
        let tick = self.tick;
        for e in self.enemies.values_mut() {
            e.trail.record(tick, e.position);
        }
    }

    /// One enemy, one tick. Returns false when it should be removed.
    fn step_enemy(
        &mut self,
        t: &Tuning,
        e: &mut Enemy,
        players: &[PlayerView],
        separation: Vec3,
        cx: &mut NavCtx,
    ) -> bool {
        let rate = self.tick_rate;
        let now = self.tick;
        let dt = self.dt();
        let gravity = t.movement.gravity;
        let def = e.def.clone();
        e.state_ticks += 1;
        for cd in &mut e.cooldowns {
            *cd = cd.saturating_sub(1);
        }
        let window = 2 * rate;
        e.aggro.retain(|a| now.saturating_sub(a.2) < window);
        e.stagger = e.stagger.saturating_sub(1);

        if e.state == AiState::Dying {
            move_enemy(&self.collision.world, e, Vec3::ZERO, dt, gravity);
            return e.state_ticks < e.state_len;
        }
        // Training dummies have no AI: they flinch, heal and stand there.
        if e.is_dummy() {
            self.step_dummy(e);
            move_enemy(&self.collision.world, e, Vec3::ZERO, dt, gravity);
            return true;
        }
        // Clinging to a victim (no targeting, no movement of its own); once it lets go it
        // falls through to the normal tick in its new state.
        if e.state == AiState::Latched && self.step_latched(t, e) {
            return true;
        }

        // Getting hurt by someone while unaware: that's the target now.
        if let Some(p) = e.alerted_by.take()
            && e.target.is_none()
            && let Some(v) = players.iter().find(|v| v.id == p && v.targetable)
        {
            e.target = Some(p);
            e.last_seen_tick = now;
            e.last_seen_pos = v.position;
            e.retarget_in = secs_to_ticks(def.ai.retarget_interval, rate, 1) as u32;
        }

        // Target selection (periodic, or immediately when the target became invalid).
        let target_valid = e
            .target
            .is_some_and(|id| players.iter().any(|p| p.id == id && p.targetable));
        if e.retarget_in == 0 || !target_valid {
            let (target, visible) = self.select_target(e, players);
            if target != e.target {
                e.nav.reset();
                if target.is_some() {
                    e.last_seen_tick = now;
                }
            }
            e.target = target;
            if visible && let Some(tp) = target.and_then(|id| players.iter().find(|p| p.id == id)) {
                e.last_seen_tick = now;
                e.last_seen_pos = tp.position;
            }
            e.retarget_in = secs_to_ticks(def.ai.retarget_interval, rate, 1) as u32;
        } else {
            e.retarget_in -= 1;
        }
        let target = e
            .target
            .and_then(|id| players.iter().find(|p| p.id == id))
            .copied();
        // A soot cloud hides the target (into, out of or through it, `knacks.rs`): it is lost
        // at once, and a laser's wind-up breaks off.
        let target = match target {
            Some(tp) if self.soot_blocks(e.eye(), tp.chest) => {
                e.target = None;
                e.nav.reset();
                if e.aiming() {
                    e.aim_locked = false;
                    e.set_state(AiState::Chase, 0);
                }
                None
            }
            other => other,
        };
        // Line of sight to the target (attacks, ranged positioning, losing interest).
        e.has_los = target.is_some_and(|tp| {
            tp.chest.distance(e.eye()) <= def.ai.sight_range
                && !self
                    .collision
                    .world
                    .line_blocked(e.eye(), tp.chest, MASK_SHOTS)
        });
        if let Some(tp) = &target
            && e.has_los
        {
            e.last_seen_tick = now;
            e.last_seen_pos = tp.position;
            e.unseen_ticks = 0;
        } else {
            e.unseen_ticks = e.unseen_ticks.saturating_add(1);
        }
        let lose = secs_to_ticks(def.ai.lose_target_time, rate, 1) as u32;
        if e.target.is_some() && !e.aware && now.saturating_sub(e.last_seen_tick) > lose {
            e.target = None;
            e.nav.reset();
        }
        let target = if e.target.is_some() { target } else { None };

        let mut desired = Vec3::ZERO;
        let mut max_drop = STEP_UP + 0.1;
        let mut face: Option<Vec3> = None;
        // A flyer climbing back after a dive steers during its recovery.
        let mut flying_recovery = false;
        match e.state {
            AiState::Idle => {
                if target.is_some() {
                    e.set_state(
                        AiState::Alert,
                        secs_to_ticks(def.ai.reaction_time, rate, 1) as u32,
                    );
                    self.events.push(SimEvent::EnemyAlerted {
                        id: e.id,
                        target: e.target,
                    });
                }
            }
            AiState::Alert => {
                face = target.map(|p| p.position);
                if e.state_ticks >= e.state_len {
                    e.set_state(AiState::Chase, 0);
                }
            }
            AiState::Chase => match target {
                None => e.set_state(AiState::Idle, 0),
                // Dev `/melt`: now, whatever it was about to do.
                Some(tp) if e.melt_now => {
                    face = Some(tp.position);
                    self.begin_melt(e, &tp, cx);
                }
                Some(tp) => {
                    face = Some(tp.position);
                    let mut chosen = self.choose_attack(e, &tp, cx);
                    // A pour lays its line now; no room for one here: try again shortly.
                    if let Some(i) = chosen
                        && let Some(pour) = def.attacks.get(i).and_then(|a| a.pour.clone())
                        && !self.begin_pour(e, i, &pour, tp.position, tp.velocity)
                    {
                        if let Some(cd) = e.cooldowns.get_mut(i) {
                            *cd = secs_to_ticks(0.5, rate, 1) as u32;
                        }
                        chosen = None;
                    }
                    if let Some(i) = chosen {
                        self.begin_windup(e, i, &tp);
                        if e.aiming() || e.pour_line.is_some() {
                            // The body follows the laser from its first frame; a pour faces
                            // its line.
                            face = None;
                        }
                    } else {
                        (desired, max_drop) = self.chase_velocity(e, &tp, cx, 1.0);
                        if let Some((jump, landing)) = e.nav.jump_ready.take()
                            && e.stagger == 0
                        {
                            // On a jump's takeoff: crouch and leap / ride the pad.
                            self.begin_leap(t, e, jump, landing, cx);
                            desired = Vec3::ZERO;
                            face = e.leap.map(|l| l.to);
                        } else if self.stranded(e, &tp) {
                            self.begin_melt(e, &tp, cx);
                        }
                    }
                }
            },
            AiState::Leap => {
                face = self.step_leap(t, e);
            }
            AiState::Melt => {
                // A puddle doesn't move (knockback included); gravity still holds it down.
                e.velocity = Vec3::new(0.0, e.velocity.y.min(0.0), 0.0);
                if e.state_ticks >= e.state_len {
                    self.finish_melt(e, target.as_ref());
                }
            }
            AiState::Reform => {
                e.velocity = Vec3::new(0.0, e.velocity.y.min(0.0), 0.0);
                face = target.map(|p| p.position);
                if e.state_ticks >= e.state_len {
                    e.stranded = 0;
                    let next = if target.is_some() {
                        AiState::Chase
                    } else {
                        AiState::Idle
                    };
                    e.set_state(next, 0);
                }
            }
            AiState::Windup => {
                let Some(a) = def.attacks.get(e.attack) else {
                    e.set_state(AiState::Chase, 0);
                    return true;
                };
                if let Some(laser) = &a.laser {
                    // The aim (and the body with it) follows the laser, not `face`.
                    self.track_aim(e, a, laser, target.as_ref());
                } else if a.track_target {
                    face = target.map(|p| p.position);
                }
                if let Some(tp) = &target
                    && a.windup_move_scale > 0.0
                {
                    (desired, max_drop) = self.chase_velocity(e, tp, cx, a.windup_move_scale);
                }
                if e.state_ticks >= e.state_len {
                    let nav = cx.nav.clone();
                    self.strike(t, e, players, target.as_ref(), nav.as_deref());
                    // The strike set its own facing (a led leap, a dive).
                    face = None;
                }
            }
            AiState::Strike => {
                self.continue_burst(t, e, target.as_ref());
                match def.attacks.get(e.attack) {
                    // Rake whoever the dive / leap reaches (momentum below).
                    Some(a) if a.kind == AttackKind::Dive => self.dive_hits(t, e, players),
                    Some(a) if a.lunge > 0.0 => self.lunge_hits(t, e, players),
                    _ => {}
                }
                if e.state_ticks >= e.state_len {
                    self.end_strike(e);
                }
            }
            AiState::Recover => {
                if e.hovering()
                    && let Some(tp) = &target
                {
                    // Climb back to hover height (after a dive).
                    desired =
                        steer::fly_velocity(&self.collision.world, e, tp, now, 0.5, &mut cx.stats);
                    flying_recovery = true;
                }
                if e.state_ticks >= e.state_len {
                    e.set_state(AiState::Chase, 0);
                }
            }
            AiState::Stunned => {
                if e.state_ticks >= e.state_len {
                    e.set_state(AiState::Chase, 0);
                }
            }
            // (Handled above; a latch that just took hold is placed on its victim.)
            AiState::Dying | AiState::Latched => {}
        }
        if e.pour_line.is_some() {
            self.settle_pour(e);
        }
        if e.state == AiState::Latched {
            return true;
        }
        // Lunges and dives keep their momentum through the strike.
        let mut momentum = None;
        if e.state == AiState::Strike
            && let Some(a) = def.attacks.get(e.attack)
        {
            if a.kind == AttackKind::Dive {
                desired = e.aim * a.lunge;
                max_drop = MAX_DROP;
                momentum = Some((a.lunge, true));
            } else if a.lunge > 0.0 {
                desired = e.forward() * a.lunge;
                max_drop = STEP_UP + 0.1;
                momentum = Some((a.lunge, false));
            }
        }

        if let Some(point) = face
            && let Some(yaw) = yaw_towards(point - e.position)
        {
            e.yaw = turn_towards(e.yaw, yaw, def.turn_rate.to_radians() * dt);
        }
        let control = flying_recovery
            || matches!(e.state, AiState::Chase | AiState::Windup | AiState::Strike);
        let mut desired = if control {
            desired + separation
        } else {
            separation
        };
        if e.hovering() && !control {
            // Hover in place (no gravity while alive).
            desired = horizontal(desired);
        }
        if e.stagger > 0 || matches!(e.state, AiState::Leap | AiState::Melt | AiState::Reform) {
            // (A leap flies its arc; a puddle stays put.)
            desired = Vec3::ZERO;
        }
        // Walking onto a jump's takeoff, the node is the guard: it sits right at the edge.
        let approaching = std::mem::take(&mut e.nav.approaching) && e.state == AiState::Chase;
        let guarded = if approaching {
            desired
        } else {
            steer::guard(cx.nav.as_deref(), e, desired, max_drop)
        };
        // Own momentum (a lunge, braking after a run) stops dead at a ledge / lava edge;
        // knockback (stagger) doesn't: players may launch enemies into the lava.
        if e.stagger == 0
            && !approaching
            && steer::heading_off_edge(cx.nav.as_deref(), e, max_drop, dt)
        {
            e.velocity = Vec3::new(0.0, e.velocity.y, 0.0);
        }
        let before = e.position;
        let touched = move_enemy(&self.collision.world, e, guarded, dt, gravity);
        // The level ends a dive; a wall (head-on) stops a lunge: the strike is over.
        if let Some((speed, dive)) = momentum
            && e.state == AiState::Strike
        {
            let moved = horizontal(e.position - before).length();
            let blocked = if dive {
                touched
            } else {
                moved < LUNGE_BLOCKED * speed * dt
            };
            if blocked {
                self.end_strike(e);
            }
        }
        true
    }

    /// Strike over (its time ran out, or a lunge / dive was stopped): cooldown, recover. A
    /// strike-long parry window closes now; a dive brakes; ranged enemies that reposition
    /// switch their strafe direction.
    fn end_strike(&self, e: &mut Enemy) {
        let rate = self.tick_rate;
        let Some(a) = e.current_attack().cloned() else {
            e.set_state(AiState::Recover, secs_to_ticks(0.5, rate, 1) as u32);
            return;
        };
        // Difficulty, frenzy and elites shorten cooldowns and recoveries (never wind-ups).
        let aggression = self.fury(e).aggression;
        if let Some(cd) = e.cooldowns.get_mut(e.attack) {
            *cd = secs_to_ticks((a.cooldown + a.recover) / aggression, rate, 0) as u32;
        }
        if a.parry_strike
            && let Some(w) = &mut e.parry_window
        {
            w.end = w.end.min(self.tick);
        }
        if a.kind == AttackKind::Dive {
            e.velocity *= 0.25;
        }
        if a.kind == AttackKind::Projectile && e.def.ai.reposition {
            e.strafe_sign = -e.strafe_sign;
            e.strafe_flip_in = 90;
        }
        e.set_state(
            AiState::Recover,
            secs_to_ticks(a.recover / aggression, rate, 1) as u32,
        );
    }

    /// Laser wind-up: the aim follows the target at `track_rate` until the last `lock`
    /// seconds, then stays put (dodge window). The body turns with it.
    fn track_aim(
        &self,
        e: &mut Enemy,
        a: &AttackDef,
        laser: &AimLaser,
        target: Option<&PlayerView>,
    ) {
        let remaining = e.state_len.saturating_sub(e.state_ticks);
        let lock = secs_to_ticks(laser.lock, self.tick_rate, 0) as u32;
        if remaining <= lock {
            e.aim_locked = true;
            return;
        }
        let muzzle = Self::muzzle(e, a);
        let goal = match target {
            Some(tp) if e.has_los => {
                let speed = a.projectile.as_ref().map_or(100.0, |p| p.speed.max(1.0))
                    * self.fury(e).projectile_speed;
                let flight = tp.chest.distance(muzzle) / speed;
                tp.chest + horizontal(tp.velocity) * (flight * laser.lead)
            }
            Some(_) => e.last_seen_pos + Vec3::Y * 1.1,
            None => return,
        };
        let want = (goal - muzzle).normalize_or(e.aim);
        let step = laser.track_rate.to_radians() * self.dt();
        e.aim = rotate_towards(e.aim, want, step);
        if let Some(yaw) = yaw_towards(e.aim) {
            e.yaw = yaw;
        }
    }

    /// World-space muzzle of attack `a` (projectile muzzle, else the eye).
    fn muzzle(e: &Enemy, a: &AttackDef) -> Vec3 {
        a.projectile
            .as_ref()
            .map_or(e.eye(), |p| e.local_to_world(p.muzzle))
    }

    /// (target, target currently visible).
    fn select_target(&self, e: &Enemy, players: &[PlayerView]) -> (Option<PlayerId>, bool) {
        let eye = e.eye();
        let world = &self.collision.world;
        let ai = &e.def.ai;
        let visible: Vec<(PlayerId, f32)> = players
            .iter()
            .filter(|p| p.targetable)
            .map(|p| (p.id, p.chest.distance(eye), p.chest))
            .filter(|(_, d, chest)| {
                *d <= ai.sight_range
                    && !world.line_blocked(eye, *chest, MASK_SHOTS)
                    && !self.soot_blocks(eye, *chest)
            })
            .map(|(id, d, _)| (id, d))
            .collect();
        let is_visible = |id: PlayerId| visible.iter().any(|v| v.0 == id);
        // Someone hurting us badly takes priority.
        let threshold = ai.aggro_switch_fraction * e.max_health;
        // (Nobody hidden in soot, `knacks.rs`.)
        let hidden = |p: &PlayerView| self.soot_blocks(eye, p.chest);
        if let Some(a) = e
            .aggro
            .iter()
            .filter(|a| {
                a.1 >= threshold
                    && players
                        .iter()
                        .any(|p| p.id == a.0 && p.targetable && !hidden(p))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            return (Some(a.0), is_visible(a.0));
        }
        let nearest = visible.iter().min_by(|a, b| a.1.total_cmp(&b.1)).copied();
        let current = e
            .target
            .filter(|id| players.iter().any(|p| p.id == *id && p.targetable));
        match (current, nearest) {
            (Some(cur), Some((best, best_d))) => match visible.iter().find(|v| v.0 == cur) {
                Some(&(_, cur_d)) if best_d >= ai.target_stickiness * cur_d => (Some(cur), true),
                _ => (Some(best), true),
            },
            (Some(cur), None) => (Some(cur), false),
            (None, Some((best, _))) => (Some(best), true),
            (None, None) if e.aware => {
                // Arena spawns hunt the nearest player even without seeing them.
                let nearest = players
                    .iter()
                    .filter(|p| p.targetable && !hidden(p))
                    .min_by(|a, b| {
                        a.position
                            .distance_squared(eye)
                            .total_cmp(&b.position.distance_squared(eye))
                    })
                    .map(|p| p.id);
                (nearest, false)
            }
            (None, None) => (None, false),
        }
    }

    /// First usable attack (definition order = priority) against `tp`. Everything but a toll
    /// needs line of sight.
    fn choose_attack(&self, e: &Enemy, tp: &PlayerView, cx: &NavCtx) -> Option<usize> {
        let world = &self.collision.world;
        let feet = e.feet();
        let speed = horizontal(tp.velocity).length();
        e.def.attacks.iter().enumerate().find_map(|(i, a)| {
            if e.cooldowns.get(i).copied().unwrap_or(0) > 0 || speed < a.min_target_speed {
                return None;
            }
            if !e.has_los && a.kind != AttackKind::Toll {
                return None;
            }
            let ok = match a.kind {
                AttackKind::Melee | AttackKind::Slam => {
                    let to = horizontal(tp.position - feet);
                    let surface = to.length() - e.radius() - tp.radius;
                    let vertical = tp.position.y < feet.y + e.height()
                        && tp.position.y + tp.height > feet.y - 0.3;
                    // A low sweep is pointless against someone standing on a crate.
                    let reachable = a.jumpable.is_none_or(|h| tp.position.y < feet.y + h + 0.6);
                    let far_enough = a.min_range <= 0.0 || surface >= a.min_range;
                    let mut ok = surface <= a.range && far_enough && vertical && reachable;
                    // One latch per victim: the others nip instead.
                    if ok && a.latch.is_some() && self.carries_latch(tp.id) {
                        ok = false;
                    }
                    if ok && a.lunge > 0.0 {
                        // Don't leap into lava or off a ledge.
                        let dir = to.normalize_or_zero();
                        let reach = (surface + e.radius()).min(a.lunge * a.strike);
                        ok = cx.nav.as_deref().is_none_or(|n| {
                            n.is_empty()
                                || [0.5, 1.0].iter().all(|f| {
                                    n.ground_ok(feet + dir * reach * f, feet.y, STEP_UP + 0.1)
                                })
                        });
                    }
                    ok && !world.line_blocked(e.center(), tp.chest, MASK_SHOTS)
                }
                AttackKind::Projectile => {
                    let muzzle = Self::muzzle(e, a);
                    let d = tp.chest.distance(muzzle);
                    d <= a.range
                        && d >= a.min_range
                        && !world.line_blocked(muzzle, tp.chest, MASK_SHOTS)
                }
                AttackKind::Dive => {
                    let center = e.center();
                    let d = tp.chest.distance(center);
                    e.hovering()
                        && d <= a.range
                        && d >= a.min_range
                        && !world.line_blocked(center, tp.chest, MASK_SHOTS)
                }
                AttackKind::Toll => a.toll.as_ref().is_some_and(|toll| {
                    tp.chest.distance(e.center()) <= a.range
                        && self.toll_allies(e, toll.radius) >= toll.allies.max(1) as usize
                }),
                AttackKind::Pour => {
                    // A target on its own level (slag flows along the floor).
                    let d = horizontal(tp.position - feet).length();
                    a.pour.is_some()
                        && d <= a.range
                        && d >= a.min_range
                        && (tp.position.y - feet.y).abs() < 1.5
                }
            };
            ok.then_some(i)
        })
    }

    /// Movement while chasing (`scale` < 1 while winding up): ground enemies walk the nav grid
    /// (melee stop at 60 % of their reach; ranged hold their band, strafe, and walk until they
    /// see the target); flyers circle. Returns (velocity, drop the edge guard may allow).
    fn chase_velocity(
        &self,
        e: &mut Enemy,
        tp: &PlayerView,
        cx: &mut NavCtx,
        scale: f32,
    ) -> (Vec3, f32) {
        let def = e.def.clone();
        let rate = self.tick_rate;
        // Frenzied and elite enemies move faster.
        let scale = scale * self.fury(e).speed;
        if let Locomotion::Flying { .. } = def.locomotion {
            let v = steer::fly_velocity(
                &self.collision.world,
                e,
                tp,
                self.tick,
                scale,
                &mut cx.stats,
            );
            return (v, MAX_DROP);
        }
        let speed = def.speed * scale;
        let feet = e.feet();
        let to = horizontal(tp.position - feet);
        let dist = to.length();
        let dir = to.normalize_or_zero();
        match def.ai.preferred_range {
            Some((min, max)) => {
                let d = tp.chest.distance(e.eye());
                if !e.has_los || d > max {
                    // Walk to a spot that sees the target when walking at it doesn't get it
                    // in sight, else towards it until it is in sight and in range.
                    let goal = self
                        .vantage_goal(e, tp, (min, max), cx)
                        .unwrap_or(tp.position);
                    return steer::ground_velocity(e, goal, speed, cx, rate);
                }
                e.vantage = None;
                e.nav.reset();
                let world = &self.collision.world;
                let nav = cx.nav.as_deref();
                let side = Vec3::new(-dir.z, 0.0, dir.x);
                if e.strafe_flip_in > 0 {
                    e.strafe_flip_in -= 1;
                }
                let mut strafe = Vec3::ZERO;
                if def.ai.strafe {
                    if !steer::can_step(world, nav, e, side * e.strafe_sign)
                        && e.strafe_flip_in == 0
                    {
                        e.strafe_sign = -e.strafe_sign;
                        e.strafe_flip_in = 45;
                    } else if e.strafe_flip_in == 0 && (self.tick + e.id.0).is_multiple_of(150) {
                        // Change direction now and then so it isn't predictable.
                        e.strafe_sign = -e.strafe_sign;
                        e.strafe_flip_in = 45;
                    }
                    strafe = side * e.strafe_sign;
                    // Don't strafe behind cover: keep the line of sight.
                    if e.strafe_flip_in == 0
                        && world.line_blocked(e.eye() + strafe * 1.5, tp.chest, MASK_SHOTS)
                    {
                        e.strafe_sign = -e.strafe_sign;
                        e.strafe_flip_in = 45;
                        strafe = -strafe;
                    }
                    if !steer::can_step(world, nav, e, strafe) {
                        strafe = Vec3::ZERO;
                    }
                }
                let radial = if d < min {
                    // Too close: back off (if there is room), else just strafe.
                    if steer::can_step(world, nav, e, -dir) {
                        -dir
                    } else {
                        Vec3::ZERO
                    }
                } else {
                    Vec3::ZERO
                };
                let v = (radial + strafe * 0.6).normalize_or_zero() * speed;
                (v, STEP_UP + 0.1)
            }
            None => {
                let reach = melee_reach(&def);
                let surface = dist - e.radius() - tp.radius;
                let level = (tp.position.y - feet.y).abs() < 1.0;
                if reach.is_finite() && surface <= reach * 0.6 && level {
                    return (Vec3::ZERO, STEP_UP);
                }
                let v = steer::ground_velocity(e, tp.position, speed, cx, rate);
                if e.nav.at_partial_end() {
                    return self.prowl(e, tp, speed, cx);
                }
                v
            }
        }
    }

    /// Pacing the reachable edge nearest an unreachable target (the end of its partial path):
    /// side to side across the line to it at a prowl, facing it, turning at walls, ledges and
    /// lava, every ~1.5 s, and when 2.5 m from where the path ended.
    fn prowl(&self, e: &mut Enemy, tp: &PlayerView, speed: f32, cx: &NavCtx) -> (Vec3, f32) {
        let world = &self.collision.world;
        let nav = cx.nav.as_deref();
        let feet = e.feet();
        let end = e.nav.waypoints.last().map_or(e.progress_from, |w| w.pos);
        let to = horizontal(tp.position - feet).normalize_or(e.forward());
        let mut dir = Vec3::new(-to.z, 0.0, to.x) * e.strafe_sign;
        if e.strafe_flip_in > 0 {
            e.strafe_flip_in -= 1;
        } else if horizontal(feet - end).dot(dir) > 2.5
            || !steer::can_step(world, nav, e, dir)
            || (self.tick + e.id.0).is_multiple_of(90)
        {
            e.strafe_sign = -e.strafe_sign;
            e.strafe_flip_in = 30;
            dir = -dir;
        }
        if !steer::can_step(world, nav, e, dir) {
            return (Vec3::ZERO, STEP_UP);
        }
        (dir * speed * 0.45, STEP_UP + 0.1)
    }

    /// A ranged Mould with no sight of its target (`band` its preferred range): the vantage
    /// point it is walking to, or a new one ([`steer::vantage_site`]) once walking at the target
    /// hasn't brought it into sight for 1.5 s or the target can't be reached (one search a
    /// second per enemy, within the per-tick search budget). Given up on after 4 s.
    fn vantage_goal(
        &self,
        e: &mut Enemy,
        tp: &PlayerView,
        band: (f32, f32),
        cx: &mut NavCtx,
    ) -> Option<Vec3> {
        let now = self.tick;
        if let Some((p, until)) = e.vantage {
            if now < until && horizontal(p - e.feet()).length() > 0.8 {
                return Some(p);
            }
            e.vantage = None;
        }
        let need =
            e.unseen_ticks > 90 || matches!(e.nav.reach, Reach::Unreachable | Reach::OffGrid);
        if !need || now < e.search_after || cx.searches_left == 0 {
            return None;
        }
        cx.searches_left -= 1;
        cx.stats.vantage_searches += 1;
        e.search_after = now + self.tick_rate;
        let nav = cx.nav.clone()?;
        let class = e.nav_class();
        let start = nav.node_at(e.feet(), 1.0, class, &cx.doors)?;
        let q = SiteQuery {
            class,
            doors: &cx.doors,
            target: tp.chest,
            target_eye: tp.chest,
            band,
            eye_height: e.def.eye_height,
            radius: e.radius(),
            others: &[],
            arena: None,
        };
        let p = steer::vantage_site(&nav, &mut cx.scratch, &self.collision.world, start, &q)?;
        e.vantage = Some((p, now + 4 * self.tick_rate));
        Some(p)
    }

    /// Is a chasing Mould in a fight right now: a ranged one seeing its target within its
    /// band, a melee one within a metre and a half of its reach (at about its level)?
    fn engaged(&self, e: &Enemy, tp: &PlayerView) -> bool {
        match e.def.ai.preferred_range {
            Some((_, max)) => e.has_los && tp.chest.distance(e.eye()) <= max,
            None => {
                let reach = melee_reach(&e.def);
                let surface = horizontal(tp.position - e.feet()).length() - e.radius() - tp.radius;
                let reach = if reach.is_finite() { reach } else { 2.0 };
                surface <= reach + 1.5 && (tp.position.y - e.feet().y).abs() < e.height().max(2.0)
            }
        }
    }

    /// Stranded bookkeeping of a chasing ground Mould (docs/enemies.md "Melting and
    /// re-forming"), once a tick: in a fight it isn't stranded; else its target unreachable
    /// counts, and so does a reachable (or far) target it isn't getting anywhere towards (it
    /// moved less than 1 m in the last 2 s window); off the nav grid and not moving counts
    /// triple. True when it has been stranded for `MeltDef::after`.
    fn stranded(&self, e: &mut Enemy, tp: &PlayerView) -> bool {
        let melt = e.def.melt;
        if melt.after <= 0.0 || e.is_flying() || e.is_dummy() {
            return false;
        }
        if e.progress_in == 0 {
            let moved = e.feet() - e.progress_from;
            e.progressing = horizontal(moved).length() >= 1.0 || moved.y.abs() >= 1.5;
            e.progress_from = e.feet();
            e.progress_in = 2 * self.tick_rate;
        } else {
            e.progress_in -= 1;
        }
        if self.engaged(e, tp) {
            e.stranded = 0;
            return false;
        }
        e.stranded += match e.nav.reach {
            Reach::Unreachable => 1,
            Reach::OffGrid if !e.progressing => 3,
            Reach::OffGrid => 0,
            _ if e.progressing => {
                e.stranded = 0;
                0
            }
            _ => 1,
        };
        e.stranded >= secs_to_ticks(melt.after, self.tick_rate, 1) as u32
    }

    /// Starts a melt: finds where it re-forms ([`steer::reform_site`]; one search a tick for all
    /// enemies) and slumps into a puddle. With nowhere to re-form (an isolated target), or
    /// when its target stands where it can't (a floe too small for a Gaoler) and it is already
    /// near, it keeps prowling and tries again in half the time.
    fn begin_melt(&mut self, e: &mut Enemy, tp: &PlayerView, cx: &mut NavCtx) {
        if cx.searches_left == 0 {
            return;
        }
        cx.searches_left -= 1;
        e.melt_now = false;
        let Some(nav) = cx.nav.clone() else {
            return;
        };
        let melt = e.def.melt;
        let rate = self.tick_rate;
        let class = e.nav_class();
        let near = horizontal(tp.position - e.feet()).length() <= melt.far;
        if near && nav.node_at(tp.position, 3.0, class, &cx.doors).is_none() {
            cx.stats.melt_no_site += 1;
            e.stranded = secs_to_ticks(melt.after * 0.5, rate, 1) as u32;
            return;
        }
        let others: Vec<(Vec3, f32)> = self
            .enemies
            .values()
            .filter(|o| o.is_alive() && o.id != e.id)
            .map(|o| (o.reform_at.unwrap_or(o.feet()), o.radius()))
            .collect();
        // An encounter's Moulds stay in its arena while their target fights there.
        let arena = e
            .encounter
            .and_then(|(i, _)| self.level.encounters.get(i as usize))
            .and_then(|enc| enc.arena)
            .filter(|a| a.contains(tp.position + Vec3::Y * 0.1));
        let q = SiteQuery {
            class,
            doors: &cx.doors,
            target: tp.position,
            target_eye: tp.position + Vec3::Y * (tp.height - 0.2),
            band: (melt.near, melt.far.max(melt.near)),
            eye_height: e.def.eye_height,
            radius: e.radius(),
            others: &others,
            arena,
        };
        let site = steer::reform_site(
            &nav,
            &mut cx.scratch,
            &self.collision.world,
            &self.level,
            &q,
        );
        let Some(site) = site else {
            cx.stats.melt_no_site += 1;
            e.stranded = secs_to_ticks(melt.after * 0.5, rate, 1) as u32;
            return;
        };
        cx.stats.melts += 1;
        e.reform_at = Some(site);
        e.leap = None;
        e.vantage = None;
        e.nav.reset();
        e.aim_locked = false;
        e.velocity = Vec3::new(0.0, e.velocity.y.min(0.0), 0.0);
        if let Some(line) = e.pour_line.take() {
            self.drop_unlit_line(line);
        }
        e.set_state(AiState::Melt, secs_to_ticks(melt.sink, rate, 1) as u32);
    }

    /// The puddle has soaked away: the Mould is moved to its re-form site, facing its target,
    /// and rises.
    fn finish_melt(&mut self, e: &mut Enemy, target: Option<&PlayerView>) {
        if let Some(site) = e.reform_at.take() {
            e.position = site;
            e.velocity = Vec3::ZERO;
            e.on_ground = true;
            e.trail.clear();
            if let Some(tp) = target
                && let Some(yaw) = yaw_towards(tp.position - site)
            {
                e.yaw = yaw;
            }
        }
        e.nav.reset();
        e.stranded = 0;
        e.progress_from = e.feet();
        e.progress_in = 0;
        e.progressing = true;
        let rise = secs_to_ticks(e.def.melt.rise, self.tick_rate, 1) as u32;
        e.set_state(AiState::Reform, rise);
    }

    /// Starts jump `jump` of its route (landing on waypoint `landing`): a leap crouches first
    /// (`LeapDef::crouch`), a pad ride launches at once.
    fn begin_leap(
        &mut self,
        t: &Tuning,
        e: &mut Enemy,
        jump: u32,
        landing: usize,
        cx: &mut NavCtx,
    ) {
        let Some(nav) = cx.nav.as_deref() else {
            return;
        };
        let Some(j) = nav.jump(jump).copied() else {
            return;
        };
        let pad = match j.kind {
            JumpKind::Pad(i) => Some(i),
            JumpKind::Leap => None,
        };
        e.leap = Some(LeapPlan {
            jump,
            waypoint: landing,
            to: nav.nodes[j.to as usize].pos,
            pad,
            launched: false,
            speed: 0.0,
        });
        e.velocity = Vec3::new(0.0, e.velocity.y, 0.0);
        if pad.is_some() {
            cx.stats.pad_rides += 1;
            self.launch(t, e);
        } else {
            cx.stats.leaps += 1;
            let crouch = e.def.leap.map_or(0.3, |l| l.crouch);
            e.set_state(
                AiState::Leap,
                secs_to_ticks(crouch, self.tick_rate, 1) as u32,
            );
        }
    }

    /// Is another living Mould standing on `to` (bodies overlapping it) or in the air on a
    /// leap landing there?
    fn landing_taken(&self, e: &Enemy, to: Vec3) -> bool {
        self.enemies.values().any(|o| {
            o.id != e.id
                && o.is_alive()
                && !o.is_flying()
                && (o
                    .leap
                    .is_some_and(|l| l.launched && l.to.distance(to) < e.radius() + o.radius())
                    || (horizontal(o.feet() - to).length() < e.radius() + o.radius()
                        && (o.feet().y - to.y).abs() < 1.5))
        })
    }

    /// Leaves the ground: the leap arc's launch velocity from where it actually stands to the
    /// landing (`nav::leap_velocity`, so it flies the arc the nav build swept), or the pad's
    /// velocity (set, as `movement::step` does for players).
    fn launch(&mut self, t: &Tuning, e: &mut Enemy) {
        let Some(plan) = e.leap.as_mut() else {
            return;
        };
        let g = t.movement.gravity.max(1.0);
        let feet = e.position;
        let (v, time) = match plan.pad.and_then(|i| self.level.jump_pads.get(i as usize)) {
            Some(pad) => {
                let v = pad.velocity;
                let rise = plan.to.y - feet.y;
                let disc = v.y * v.y - 2.0 * g * rise;
                let time = if disc > 0.0 {
                    (v.y + disc.sqrt()) / g
                } else {
                    2.0 * v.y.max(1.0) / g
                };
                (v, time)
            }
            None => leap_velocity(feet, plan.to, g),
        };
        plan.launched = true;
        plan.speed = horizontal(v).length();
        if let Some(yaw) = yaw_towards(plan.to - feet) {
            e.yaw = yaw;
        }
        e.velocity = v;
        e.on_ground = false;
        e.set_state(AiState::Leap, secs_to_ticks(time, self.tick_rate, 1) as u32);
    }

    /// One tick of a jump: the crouch (facing the landing; knocked about or off the ground, the
    /// leap is off), then the flight — ballistic, except that a leap whose horizontal speed a
    /// wall or a lip stole is re-aimed at the landing — until it lands: on the planned spot it
    /// follows its route on, anywhere else it re-plans. Returns where to face.
    fn step_leap(&mut self, t: &Tuning, e: &mut Enemy) -> Option<Vec3> {
        let Some(plan) = e.leap else {
            e.set_state(AiState::Chase, 0);
            return None;
        };
        if !plan.launched {
            if e.stagger > 0 || !e.on_ground {
                e.leap = None;
                e.nav.reset();
                e.set_state(AiState::Chase, 0);
                return None;
            }
            e.velocity = Vec3::new(0.0, e.velocity.y, 0.0);
            if e.state_ticks >= e.state_len {
                // Somebody standing on (or flying to) the landing: hold the crouch a little
                // (up to 0.75 s), then pick another way.
                if self.landing_taken(e, plan.to) {
                    if e.state_ticks >= e.state_len + self.tick_rate * 3 / 4 {
                        e.leap = None;
                        e.nav.reset();
                        e.set_state(AiState::Chase, 0);
                        return None;
                    }
                } else {
                    self.launch(t, e);
                }
            }
            return Some(plan.to);
        }
        let dt = self.dt();
        let flat = horizontal(plan.to - e.feet());
        if !e.on_ground
            && plan.pad.is_none()
            && e.stagger == 0
            && flat.length() > 0.3
            && horizontal(e.velocity).length() < plan.speed * 0.5
        {
            let left = e.state_len.saturating_sub(e.state_ticks) as f32 * dt;
            let want = (flat / left.max(0.15)).clamp_length_max(plan.speed.max(2.0) * 1.5);
            e.velocity.x = want.x;
            e.velocity.z = want.z;
        }
        let landed = e.on_ground && e.state_ticks >= 2;
        let lost = e.state_ticks > e.state_len + 2 * self.tick_rate;
        if landed || lost {
            e.leap = None;
            let feet = e.feet();
            let on_spot =
                horizontal(plan.to - feet).length() < 1.5 && (plan.to.y - feet.y).abs() < 1.0;
            if landed && on_spot {
                e.nav.landed(plan.waypoint);
            } else {
                e.nav.reset();
            }
            e.set_state(AiState::Chase, 0);
        }
        None
    }

    fn begin_windup(&mut self, e: &mut Enemy, attack: usize, tp: &PlayerView) {
        let rate = self.tick_rate;
        let def = e.def.clone();
        let Some(a) = def.attacks.get(attack) else {
            return;
        };
        let len = secs_to_ticks(a.windup, rate, 1) as u32;
        let parry = a.parry_window;
        // Attacking: not stranded.
        e.stranded = 0;
        e.attack = attack;
        e.attack_seq = e.attack_seq.wrapping_add(1);
        e.shots_fired = 0;
        e.lunge_hits.clear();
        e.interrupt_damage = 0.0;
        e.set_state(AiState::Windup, len);
        // A laser starts right on the target.
        e.aim_locked = false;
        if a.laser.is_some() {
            e.aim = (tp.chest - Self::muzzle(e, a)).normalize_or(e.forward());
            if let Some(yaw) = yaw_towards(e.aim) {
                e.yaw = yaw;
            }
        }
        e.parry_window = parry.map(|w| {
            let wt = (secs_to_ticks(w, rate, 1) as u32).min(len);
            ParryWindow {
                start: self.tick + len - wt,
                end: self.tick + len,
                seq: e.attack_seq,
            }
        });
        self.events.push(SimEvent::EnemyAttack {
            id: e.id,
            attack: attack.min(255) as u8,
            phase: super::super::EnemyAttackPhase::Windup,
            parryable: parry.is_some() || a.parry_strike,
        });
    }

    /// Melee hits of attack `a` from `e` right now (players in reach, in the arc, in sight).
    fn melee_hits(
        &self,
        e: &Enemy,
        a: &super::AttackDef,
        attack: Attack,
        players: &[PlayerView],
        skip: &[PlayerId],
    ) -> Vec<(PlayerId, DamageInfo)> {
        let world = &self.collision.world;
        let half_arc = (a.arc * 0.5).to_radians();
        let fwd = e.forward();
        let feet = e.feet();
        let damage = a.damage * self.fury(e).damage;
        let mut hits = Vec::new();
        // A lunge's `range` is how far it leaps from; it rakes what is right in front.
        let reach = if a.lunge > 0.0 { LUNGE_REACH } else { a.range };
        for p in players
            .iter()
            .filter(|p| p.targetable && !skip.contains(&p.id))
        {
            let to = horizontal(p.position - feet);
            let dist = to.length();
            if dist - e.radius() - p.radius > reach + 0.25 {
                continue;
            }
            let vertical =
                p.position.y < feet.y + e.height() && p.position.y + p.height > feet.y - 0.3;
            if !vertical {
                continue;
            }
            // Low sweeps miss whoever jumped over them.
            if a.jumpable.is_some_and(|h| p.position.y - feet.y >= h) {
                continue;
            }
            if dist > 0.3 && to.normalize_or_zero().angle_between(fwd) > half_arc {
                continue;
            }
            if world.line_blocked(e.center(), p.chest, MASK_SHOTS) {
                continue;
            }
            let dir = to.normalize_or(fwd);
            let kb = (dir + Vec3::Y * a.knockback_up).normalize_or(Vec3::Y) * a.knockback;
            let info = DamageInfo::new(damage, Owner::Enemy(e.id), attack)
                .at(p.chest, dir)
                .knockback(kb)
                .flags(DamageFlags::MELEE);
            hits.push((p.id, info));
        }
        hits
    }

    /// The strike moment: melee arc / slam radius damage, the lunge / pounce leap, the dive,
    /// or the first projectile.
    fn strike(
        &mut self,
        t: &Tuning,
        e: &mut Enemy,
        players: &[PlayerView],
        target: Option<&PlayerView>,
        nav: Option<&crate::sim::nav::NavGrid>,
    ) {
        let rate = self.tick_rate;
        let def = e.def.clone();
        let idx = e.attack.min(def.attacks.len().saturating_sub(1));
        let Some(a) = def.attacks.get(idx) else {
            e.set_state(AiState::Chase, 0);
            return;
        };
        let attack = Attack::Enemy {
            kind: def.net_kind,
            attack: idx.min(255) as u8,
        };
        let source = Owner::Enemy(e.id);
        let parryable = a.parry_window.is_some();
        let mut hits: Vec<(PlayerId, DamageInfo)> = Vec::new();
        match a.kind {
            AttackKind::Melee if a.lunge > 0.0 => {
                // Leap at where the target will be when the leap reaches it; hits are checked
                // every tick of the strike.
                let feet = e.feet();
                let led = target.map(|tp| {
                    let to = horizontal(tp.position - feet);
                    let time = (to.length() - e.radius() - tp.radius).max(0.0) / a.lunge;
                    horizontal(tp.position + tp.velocity * time - feet).normalize_or(e.forward())
                });
                let direct =
                    target.map(|tp| horizontal(tp.position - feet).normalize_or(e.forward()));
                // The airborne part of a pounce is beyond the edge guard: only leap where the
                // whole arc lands on safe ground (led, else straight at the target, else run).
                let airborne =
                    a.lunge * 2.0 * a.lunge_up / t.movement.gravity.max(1.0) + e.radius() + 0.5;
                let safe = |dir: Vec3| {
                    nav.is_none_or(|n| {
                        n.is_empty()
                            || [0.33, 0.66, 1.0].iter().all(|f| {
                                n.ground_ok(feet + dir * airborne * f, feet.y, STEP_UP + 0.1)
                            })
                    })
                };
                let leap = [led, direct]
                    .into_iter()
                    .flatten()
                    .find(|d| a.lunge_up <= 0.0 || safe(*d));
                let dir = leap.or(direct).unwrap_or(e.forward());
                if let Some(yaw) = yaw_towards(dir) {
                    e.yaw = yaw;
                }
                let up = if a.lunge_up > 0.0 && leap.is_some() {
                    e.on_ground = false;
                    a.lunge_up
                } else {
                    e.velocity.y
                };
                e.velocity = Vec3::new(dir.x * a.lunge, up, dir.z * a.lunge);
            }
            AttackKind::Dive => {
                // A straight line at where the target will be (small lead).
                let center = e.center();
                let goal = target.map_or(center + e.forward() * 10.0 - Vec3::Y * 5.0, |tp| {
                    tp.chest + tp.velocity * a.dive_lead
                });
                e.aim = (goal - center).normalize_or(e.forward());
                if let Some(yaw) = yaw_towards(e.aim) {
                    e.yaw = yaw;
                }
                e.velocity = e.aim * a.lunge;
            }
            AttackKind::Melee => hits = self.melee_hits(e, a, attack, players, &[]),
            // The bell rings: every ally in reach is frenzied.
            AttackKind::Toll => self.toll(e, a),
            // The line laid at the wind-up ignites on its own this tick; it is committed now.
            AttackKind::Pour => e.pour_line = None,
            AttackKind::Slam => {
                let damage = a.damage * self.fury(e).damage;
                let center = e.feet() + e.forward() * (e.radius() + 0.5);
                for p in players.iter().filter(|p| p.targetable) {
                    let to = p.position - center;
                    let d = horizontal(to).length();
                    if d > a.shockwave_radius || to.y.abs() > 2.0 {
                        continue;
                    }
                    let frac = 1.0 - 0.5 * (d / a.shockwave_radius.max(0.1));
                    let dir = horizontal(to).normalize_or(e.forward());
                    let kb = (dir + Vec3::Y * 0.8).normalize_or(Vec3::Y) * (a.knockback * frac);
                    let info = DamageInfo::new(damage * frac, source, attack)
                        .at(p.chest, dir)
                        .knockback(kb)
                        .flags(DamageFlags::SLAM);
                    hits.push((p.id, info));
                }
                self.events.push(SimEvent::EnemyShockwave {
                    id: e.id,
                    position: center,
                    radius: a.shockwave_radius,
                });
            }
            AttackKind::Projectile => {}
        }
        self.deal_hits(t, e, parryable, hits);
        self.events.push(SimEvent::EnemyAttack {
            id: e.id,
            attack: idx.min(255) as u8,
            phase: super::super::EnemyAttackPhase::Strike,
            parryable,
        });
        let mut len = secs_to_ticks(a.strike, rate, 1) as u32;
        if let Some(pa) = &a.projectile {
            let burst = secs_to_ticks(pa.interval, rate, 1) as u32 * pa.count.saturating_sub(1);
            len = len.max(burst + 1);
        }
        if a.parry_strike {
            // Parryable for the whole strike (punch a diving Harrier: the Swat).
            e.parry_window = Some(ParryWindow {
                start: self.tick,
                end: self.tick + len,
                seq: e.attack_seq,
            });
        }
        // (The parry window, if any, survives: late parries still cancel the pending hits.)
        e.set_state(AiState::Strike, len);
        if a.kind == AttackKind::Projectile {
            self.fire_enemy_projectile(t, e, target);
        }
    }

    fn deal_hits(
        &mut self,
        t: &Tuning,
        e: &Enemy,
        parryable: bool,
        hits: Vec<(PlayerId, DamageInfo)>,
    ) {
        for (victim, info) in hits {
            if parryable {
                self.queue_pending_hit(t, ParryKey::Melee(e.id, e.attack_seq), victim, info);
            } else {
                self.apply_damage(t, HitTarget::Player(victim), info);
            }
        }
    }

    /// Active frames of a lunge: everyone reached (once each).
    fn lunge_hits(&mut self, t: &Tuning, e: &mut Enemy, players: &[PlayerView]) {
        let def = e.def.clone();
        let Some(a) = def.attacks.get(e.attack) else {
            return;
        };
        let attack = Attack::Enemy {
            kind: def.net_kind,
            attack: e.attack.min(255) as u8,
        };
        let hits = self.melee_hits(e, a, attack, players, &e.lunge_hits);
        e.lunge_hits.extend(hits.iter().map(|h| h.0));
        if let Some(latch) = &a.latch {
            // A latching leap clings to the first player it may hold instead of raking them
            // (a dash through it, i-frames, spawn protection or a Snuffer already on them: it
            // just misses).
            let reached: Vec<PlayerId> = hits.iter().map(|h| h.0).collect();
            if let Some(victim) = self.latch_victim(&reached) {
                self.latch_onto(t, e, a, latch, victim);
            }
            return;
        }
        self.deal_hits(t, e, a.parry_window.is_some(), hits);
    }

    /// Active frames of a dive: every player the flyer passes within reach (in 3D), once.
    fn dive_hits(&mut self, t: &Tuning, e: &mut Enemy, players: &[PlayerView]) {
        let def = e.def.clone();
        let Some(a) = def.attacks.get(e.attack) else {
            return;
        };
        let attack = Attack::Enemy {
            kind: def.net_kind,
            attack: e.attack.min(255) as u8,
        };
        let center = e.center();
        let damage = a.damage * self.fury(e).damage;
        let mut hits = Vec::new();
        for p in players
            .iter()
            .filter(|p| p.targetable && !e.lunge_hits.contains(&p.id))
        {
            let lo = p.position + Vec3::Y * p.radius;
            let hi = p.position + Vec3::Y * (p.height - p.radius).max(p.radius);
            let closest = closest_on_segment(lo, hi, center);
            if closest.distance(center) - p.radius - e.radius() > DIVE_REACH {
                continue;
            }
            let flat = horizontal(e.aim).normalize_or(e.forward());
            let kb = (flat + Vec3::Y * a.knockback_up).normalize_or(Vec3::Y) * a.knockback;
            let info = DamageInfo::new(damage, Owner::Enemy(e.id), attack)
                .at(closest, e.aim)
                .knockback(kb)
                .flags(DamageFlags::MELEE);
            hits.push((p.id, info));
        }
        e.lunge_hits.extend(hits.iter().map(|h| h.0));
        let parryable = a.parry_window.is_some() || a.parry_strike;
        self.deal_hits(t, e, parryable, hits);
    }

    /// Fires the remaining projectiles of a burst at their intervals.
    fn continue_burst(&mut self, t: &Tuning, e: &mut Enemy, target: Option<&PlayerView>) {
        let Some(a) = e.def.attacks.get(e.attack) else {
            return;
        };
        let Some(pa) = &a.projectile else {
            return;
        };
        if a.kind != AttackKind::Projectile || e.shots_fired >= pa.count {
            return;
        }
        let interval = secs_to_ticks(pa.interval, self.tick_rate, 1) as u32;
        if e.state_ticks >= e.shots_fired * interval {
            self.fire_enemy_projectile(t, e, target);
        }
    }

    fn fire_enemy_projectile(&mut self, t: &Tuning, e: &mut Enemy, target: Option<&PlayerView>) {
        let def = e.def.clone();
        let idx = e.attack.min(def.attacks.len().saturating_sub(1));
        let Some(a) = def.attacks.get(idx) else {
            return;
        };
        let Some(pa) = &a.projectile else {
            return;
        };
        let fury = self.fury(e);
        let speed = pa.speed * fury.projectile_speed;
        let muzzle = e.local_to_world(pa.muzzle);
        let velocity = if a.laser.is_some() {
            // Along the (locked) laser.
            e.aim * speed
        } else if pa.lob {
            // A ballistic arc onto the target's predicted feet, fanned across the spread.
            // Faster projectiles (difficulty, frenzy) fly a flatter arc at that much more than
            // the least speed.
            let g = pa.kind.gravity(&t.combat);
            let feet = match target {
                Some(tp) => tp.position,
                None => e.feet() + e.forward() * 15.0,
            };
            let lead = target.map_or(Vec3::ZERO, |tp| horizontal(tp.velocity) * pa.lead);
            let mut goal = feet + Vec3::Y * 0.3;
            let mut v = Vec3::ZERO;
            for _ in 0..3 {
                let (mut vel, mut time) = lob_velocity(muzzle, goal, g, pa.speed);
                if fury.projectile_speed > 1.0
                    && let Some(fast) =
                        lob_low_arc(muzzle, goal, g, vel.length() * fury.projectile_speed)
                {
                    (vel, time) = fast;
                }
                v = vel;
                goal = feet + Vec3::Y * 0.3 + lead * time;
            }
            glam::Quat::from_rotation_y(fan_offset(e.shots_fired, pa.count, pa.spread)) * v
        } else {
            let aim_point = match target {
                Some(tp) => {
                    let time = tp.chest.distance(muzzle) / speed.max(1.0);
                    tp.chest + tp.velocity * (time * pa.lead)
                }
                None => muzzle + e.forward() * 10.0,
            };
            let mut dir = (aim_point - muzzle).normalize_or(e.forward());
            if pa.spread > 0.0 {
                let mut rng = SpreadRng::new(self.tick, PlayerId(0), e.id.0 ^ e.shots_fired);
                dir = spread_dir(dir, pa.spread, &mut rng);
            }
            dir * speed
        };
        let attack = Attack::Enemy {
            kind: def.net_kind,
            attack: idx.min(255) as u8,
        };
        let damage = a.damage * fury.damage;
        self.spawn_enemy_projectile(t, pa.kind, e.id, attack, muzzle, velocity, damage);
        e.shots_fired += 1;
        self.events.push(SimEvent::EnemyFired {
            id: e.id,
            attack: idx.min(255) as u8,
            position: muzzle,
        });
    }

    /// Hard separation: overlapping living enemies are pushed apart (heavier ones move less),
    /// with a capsule sweep so nobody is pushed into a wall, and never over lava / a ledge.
    /// A few relaxation passes so crowds settle too.
    fn push_enemies_apart(&mut self, nav: Option<&crate::sim::nav::NavGrid>) {
        for _ in 0..PUSH_PASSES {
            if !self.push_pass(nav) {
                break;
            }
        }
    }

    /// One push-apart pass; false when nothing overlapped.
    fn push_pass(&mut self, nav: Option<&crate::sim::nav::NavGrid>) -> bool {
        // A Mould in the air on a leap isn't shoved off its arc (into the water it is
        // crossing), and a puddle isn't shoved at all.
        let bodies: Vec<(NetId, Vec3, f32, f32, f32)> = self
            .enemies
            .values()
            .filter(|e| {
                e.is_alive()
                    && !matches!(e.state, AiState::Melt | AiState::Reform)
                    && !e.leap.is_some_and(|l| l.launched)
            })
            .map(|e| (e.id, e.feet(), e.radius(), e.height(), e.mass()))
            .collect();
        let mut pushes: Vec<(NetId, Vec3)> = Vec::new();
        for (i, a) in bodies.iter().enumerate() {
            for b in &bodies[i + 1..] {
                let overlap_y = a.1.y < b.1.y + b.3 && b.1.y < a.1.y + a.3;
                if !overlap_y {
                    continue;
                }
                let d = horizontal(a.1 - b.1);
                let min = a.2 + b.2;
                let len = d.length();
                if len >= min {
                    continue;
                }
                let dir = if len > 1e-4 {
                    d / len
                } else {
                    // Exactly stacked: split deterministically.
                    let ang = (a.0.0 as f32) * 2.399;
                    Vec3::new(ang.cos(), 0.0, ang.sin())
                };
                let overlap = (min - len).min(0.5);
                let total = (a.4 + b.4).max(1e-3);
                pushes.push((a.0, dir * overlap * (b.4 / total)));
                pushes.push((b.0, -dir * overlap * (a.4 / total)));
            }
        }
        if pushes.is_empty() {
            return false;
        }
        let world = &self.collision.world;
        for (id, push) in pushes {
            let Some(e) = self.enemies.get_mut(&id) else {
                continue;
            };
            let feet = e.feet();
            if !e.is_flying()
                && e.on_ground
                && nav.is_some_and(|n| {
                    !n.is_empty()
                        && !n.ground_ok(
                            feet + push.normalize_or_zero() * e.def.radius,
                            feet.y,
                            STEP_UP + 0.1,
                        )
                })
            {
                continue;
            }
            let r = e.def.radius;
            let h = e.def.height.max(2.0 * r);
            let half = (h * 0.5 - r).max(0.0);
            let center = feet + Vec3::Y * (h * 0.5);
            let frac = world
                .cast_capsule(center, half, r, push, SKIN, MASK_MOVEMENT)
                .map_or(1.0, |hit| hit.fraction);
            e.position += push * frac;
        }
        true
    }

    /// Enemies whose feet enter a kill volume (lava) or fall out of the level die; corpses
    /// that fall out are removed.
    fn enemy_world_rules(&mut self, t: &Tuning) {
        let kill_floor = self.kill_floor;
        let mut kill = Vec::new();
        for e in self.enemies.values_mut() {
            let feet = e.feet();
            let out = !feet.is_finite() || feet.y < kill_floor;
            if !e.is_alive() {
                if out {
                    e.state_ticks = e.state_len;
                    e.velocity = Vec3::ZERO;
                }
                continue;
            }
            if out || self.level.kill_volumes.iter().any(|v| v.contains(feet)) {
                kill.push(e.id);
            }
        }
        for id in kill {
            let Some(e) = self.enemies.get(&id) else {
                continue;
            };
            let info = DamageInfo::new(e.health.max(1.0) * 10.0, Owner::World, Attack::World)
                .at(e.center(), Vec3::NEG_Y);
            self.apply_damage(t, HitTarget::Entity(id), info);
        }
    }
}

/// Kinematic enemy movement: accelerate towards `desired` (ground: horizontally, with
/// gravity), collide-and-slide against the level with a step-up, snap to the ground. Returns
/// whether the move ran into the level (flight: anything; ground: a wall, not a floor or a
/// step).
pub(crate) fn move_enemy(
    world: &CollisionWorld,
    e: &mut Enemy,
    desired: Vec3,
    dt: f32,
    gravity: f32,
) -> bool {
    let def = e.def.clone();
    let r = def.radius;
    let h = def.height.max(2.0 * r);
    let half = (h * 0.5 - r).max(0.0);
    // Feet → capsule centre.
    let lift = Vec3::Y * (h * 0.5);
    // Flight physics while holding itself up; a dead or stunned flyer falls like the rest.
    let flying = e.hovering();
    let alive = e.is_alive();
    let staggered = e.stagger > 0;

    if flying {
        let accel = if staggered {
            def.acceleration * 0.3
        } else {
            def.acceleration
        };
        e.velocity = move_towards(e.velocity, desired, accel * dt);
        e.on_ground = false;
    } else {
        if e.on_ground && e.velocity.y > 0.5 {
            e.on_ground = false;
        }
        if e.on_ground {
            let target = horizontal(desired);
            let accel = if alive && !staggered {
                def.acceleration
            } else {
                def.acceleration * 0.35
            };
            e.velocity = move_towards(horizontal(e.velocity), target, accel * dt);
        } else {
            e.velocity.y -= gravity * dt;
        }
    }

    let mut center = e.position + lift;
    // Depenetrate (spawned in a wall, pushed by a door...) and keep the skin distance from
    // every surface: resting inside the skin band would make sweeps along the floor report
    // zero-distance hits and eat the motion (see `movement::collide`).
    for _ in 0..2 {
        let contacts = world.capsule_contacts(center, half, r, SKIN, MASK_MOVEMENT);
        let mut moved = false;
        for c in contacts {
            if c.distance < SKIN {
                // The contact query can report a deep, wrong-way penetration for a big
                // capsule exactly touching a face (a Gaoler resting on the floor was "pushed
                // out" 4.7 m downwards): deep pushes are only trusted when a slightly shrunk
                // capsule really overlaps the level.
                if c.distance < -DEEP_CONTACT
                    && !world.intersects_capsule(
                        center,
                        half,
                        (r - DEEP_CONTACT).max(0.01),
                        MASK_MOVEMENT,
                    )
                {
                    continue;
                }
                center += c.normal * (SKIN + NUDGE - c.distance);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    let mut motion = e.velocity * dt;
    let mut touched = false;
    for _ in 0..4 {
        if motion.length_squared() < 1e-10 {
            break;
        }
        let Some(hit) = world.cast_capsule(center, half, r, motion, SKIN, MASK_MOVEMENT) else {
            center += motion;
            break;
        };
        touched |= flying;
        center += motion * hit.fraction;
        let mut remaining = motion * (1.0 - hit.fraction);
        let n = hit.normal;
        if !flying && n.y > 0.7 && e.velocity.y <= 0.5 {
            // Walkable ground (a ramp, a bump, a landing): follow it without losing speed.
            e.velocity.y = 0.0;
            e.on_ground = true;
            remaining -= n * remaining.dot(n);
            center += n * NUDGE;
            motion = remaining;
            continue;
        }
        // Step up small ledges while walking.
        if e.on_ground && !flying && n.y.abs() < 0.3 && def.step_height > 0.0 {
            let up = Vec3::Y * def.step_height;
            let flat = horizontal(remaining);
            if world
                .cast_capsule(center, half, r, up, SKIN, MASK_MOVEMENT)
                .is_none()
                && world
                    .cast_capsule(center + up, half, r, flat, SKIN, MASK_MOVEMENT)
                    .is_none()
            {
                center += up + flat;
                remaining = Vec3::ZERO;
                motion = remaining;
                continue;
            }
        }
        touched = true;
        remaining -= n * remaining.dot(n);
        let vn = e.velocity.dot(n);
        if vn < 0.0 {
            e.velocity -= n * vn;
        }
        // Step out of the skin band so sliding along this surface isn't a zero-distance hit.
        center += n * NUDGE;
        motion = remaining;
    }
    // Sliding along the floor must not turn into tiny hops (which would cost a tick of ground
    // control each time).
    if e.on_ground && !flying && e.velocity.y > 0.0 && e.velocity.y < 0.5 {
        e.velocity.y = 0.0;
    }

    if !flying {
        if e.velocity.y <= 0.0 {
            let probe = if e.on_ground {
                def.step_height + 0.05
            } else {
                0.05
            } + SKIN;
            match world.cast_capsule(center, half, r, Vec3::NEG_Y * probe, SKIN, MASK_MOVEMENT) {
                Some(hit) if hit.normal.y > 0.7 => {
                    // Rest just outside the skin band (stable, and sweeps along the floor stay clean).
                    center += Vec3::NEG_Y * (probe * hit.fraction) + Vec3::Y * NUDGE;
                    e.on_ground = true;
                    e.velocity.y = 0.0;
                }
                _ => e.on_ground = false,
            }
        } else {
            e.on_ground = false;
        }
    }
    if center.is_finite() {
        e.position = center - lift;
    } else {
        e.velocity = Vec3::ZERO;
    }
    touched
}
