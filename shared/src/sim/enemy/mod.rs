//! Server-run enemies: data-driven definitions ([`EnemyDef`], `assets/enemies/*.ron`), the
//! runtime [`Enemy`], damage (weak points, armour, knockback, aggro, interrupts), elites and
//! frenzies, death and replication. The AI state machine lives in `ai.rs`, navigation / flight
//! steering in `steer.rs` (on top of `sim::nav`), the latch / toll / pour / training-dummy
//! behaviours of the third roster in `special.rs`, slag lines in `hazard.rs`. Overview:
//! `docs/enemies.md`.
//!
//! Replicated as entity `kind = def.net_kind` (Gnawer 20, Censer 21, Harrier 22, Gaoler 23,
//! Heron 24, Bellwether 25, Snuffer 26, Smelter 27, Effigy 28; docs/enemies.md): `owner` =
//! target player id (`OWNER_NONE` without a target; a latched Snuffer's victim), `position` =
//! feet (bottom of the collision capsule; a Harrier's hover centre is half its height above),
//! `velocity`, `yaw` (+ `pitch` and exact yaw while `AIMING` a laser), `flags` =
//! [`crate::protocol::enemy_flags`], `state` = [`AiState`] as u16, `health` = current health
//! (rounded up), `aux` = state progress (bits 0–7, 0..255 through timed states) | attack index
//! (bits 8–15) | max health (bits 16–31).

mod ai;
mod def;
mod hazard;
mod special;
mod steer;

pub use ai::{is_targetable, lob_velocity};
pub use def::*;
pub use hazard::{SlagLine, trace_floor};
pub use steer::{AiStats, AiWorld, NavAgent, PathMode};

use std::sync::Arc;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::combat::{Blocked, DamageEvent, DamageFlags, DamageInfo, KillInfo, Owner};
use super::nav::NavClass;
use super::trail::Trail;
use super::{HitPart, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::input::look_dir;
use crate::protocol::{EntityState, NetId, OWNER_NONE, enemy_flags};
use crate::tuning::Tuning;

/// Difficulty scaling applied to every enemy's behaviour (`SimWorld::enemy_scale`). The game
/// mode sets it from the PvE difficulty and the player count (`docs/expansion.md`); health is
/// scaled at spawn by the encounter / grind code instead. All 1.0 = the enemy definitions.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemyScale {
    /// Multiplier on the damage enemies deal.
    pub damage: f32,
    /// Attack cooldowns and recoveries are divided by this (wind-ups keep their length so
    /// attacks stay readable).
    pub aggression: f32,
    /// Multiplier on enemy projectile speeds.
    pub projectile_speed: f32,
}

impl Default for EnemyScale {
    fn default() -> Self {
        Self {
            damage: 1.0,
            aggression: 1.0,
            projectile_speed: 1.0,
        }
    }
}

/// How to spawn an enemy ([`SimWorld::spawn_enemy_with`]): the API the game modes use for
/// waves (grind, encounters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnOptions {
    /// Health multiplier (difficulty, co-op scaling, boss waves). The elite bonus
    /// (`EnemyDef::elite.health`) multiplies on top.
    pub health_scale: f32,
    /// Spawn as an elite (`EnemyDef::elite`; replicated `enemy_flags::ELITE`).
    pub elite: bool,
    /// Knows where the players are (arena / wave spawns: never idles, never loses its target).
    pub aware: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            health_scale: 1.0,
            elite: false,
            aware: false,
        }
    }
}

/// The multipliers an enemy acts with right now: `SimWorld::enemy_scale` (difficulty) ×
/// its frenzy (while frenzied) × its elite bonus. Wind-ups are never scaled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fury {
    pub damage: f32,
    /// Cooldowns and recoveries are divided by this.
    pub aggression: f32,
    pub projectile_speed: f32,
    /// Walking / flying speed.
    pub speed: f32,
}

/// AI / animation state (replicated in the entity's `state`; values are protocol).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AiState {
    /// No target: stands (or wanders, later).
    #[default]
    Idle = 0,
    /// Noticed a player: turns towards them for `reaction_time` (alert bark).
    Alert = 1,
    /// Moving towards / around the target.
    Chase = 2,
    /// Attack telegraph (`aux` progress 0..255, attack index in `aux` bits 8–15).
    Windup = 3,
    /// The hit itself.
    Strike = 4,
    Recover = 5,
    /// Parried / staggered: helpless.
    Stunned = 6,
    /// Dead; the corpse stays for `death_time` (no hitboxes), then the entity is removed.
    Dying = 7,
    /// Clinging to a player (a Snuffer's latch; `owner` = the victim, progress over the
    /// latch's `max_time`).
    Latched = 8,
    /// A jump on its nav path: the crouch (progress over `LeapDef::crouch`, on the ground),
    /// then airborne (`AIRBORNE` flag; progress over the planned flight) until it lands. A pad
    /// ride skips the crouch.
    Leap = 9,
    /// Stranded: slumping into a wax puddle (progress over `MeltDef::sink`); no attacks.
    Melt = 10,
    /// Rising out of a puddle where it re-formed (progress over `MeltDef::rise`); no attacks.
    Reform = 11,
}

impl AiState {
    pub fn from_u16(v: u16) -> Option<AiState> {
        Some(match v {
            0 => AiState::Idle,
            1 => AiState::Alert,
            2 => AiState::Chase,
            3 => AiState::Windup,
            4 => AiState::Strike,
            5 => AiState::Recover,
            6 => AiState::Stunned,
            7 => AiState::Dying,
            8 => AiState::Latched,
            9 => AiState::Leap,
            10 => AiState::Melt,
            11 => AiState::Reform,
            _ => return None,
        })
    }
}

/// A melting Mould at depth `d` ([`melt_depth`]) has sunk `MELT_SINK × d` of its drawn height
/// into the floor and sagged to `1 − MELT_SAG × d` of its height (about its feet): fully
/// melted, only a puddle shows. Its hit shapes (`Enemy::hitbox`) and its drawn model
/// (`client::models::enemies::body_transform`) follow the same numbers.
pub const MELT_SINK: f32 = 0.75;
pub const MELT_SAG: f32 = 0.35;

/// How far into its puddle a Mould is from its replicated state and progress (0 = standing, 1
/// = only the puddle left): the slump takes the first three quarters of `Melt`, the rise the
/// last three quarters of `Reform`. Its hit shapes sink with it (`Enemy::hitbox`); clients
/// draw the same curve.
pub fn melt_depth(state: AiState, progress: f32) -> f32 {
    let smooth = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    match state {
        AiState::Melt => smooth(progress / 0.75),
        AiState::Reform => 1.0 - smooth((progress - 0.25) / 0.75),
        _ => 0.0,
    }
}

/// A jump on the nav path being made (`AiState::Leap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeapPlan {
    /// The jump link (`NavGrid::jump`) and the index of the waypoint it lands on.
    pub jump: u32,
    pub waypoint: usize,
    /// Landing feet.
    pub to: Vec3,
    /// A pad ride (`LevelDef::jump_pads` index) instead of a leap.
    pub pad: Option<u16>,
    /// Left the ground (the crouch is over).
    pub launched: bool,
    /// Planned horizontal speed of the flight (m/s).
    pub speed: f32,
    /// Knocked about in the air: no more guiding it onto the landing.
    pub knocked: bool,
}

/// The parryable phase of a telegraphed attack (ticks, inclusive) and its sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParryWindow {
    pub start: u32,
    pub end: u32,
    pub seq: u32,
}

/// A live enemy.
#[derive(Clone, Debug)]
pub struct Enemy {
    pub id: NetId,
    pub def: Arc<EnemyDef>,
    /// Feet.
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub on_ground: bool,
    pub health: f32,
    pub max_health: f32,
    pub state: AiState,
    /// Ticks spent in the current state.
    pub state_ticks: u32,
    /// Length of the current timed state (ticks; 0 = untimed).
    pub state_len: u32,
    /// Current / last attack index.
    pub attack: usize,
    /// Increments every attack (identifies parry-delayed damage).
    pub attack_seq: u32,
    /// Ticks until each attack is ready.
    pub cooldowns: Vec<u32>,
    /// Projectiles fired in the current strike.
    pub shots_fired: u32,
    pub target: Option<PlayerId>,
    pub last_seen_tick: u32,
    pub last_seen_pos: Vec3,
    /// Recent damage per player: (player, damage, tick of the last hit).
    pub aggro: Vec<(PlayerId, f32, u32)>,
    pub parry_window: Option<ParryWindow>,
    pub trail: Trail,
    pub spawn_tick: u32,
    pub retarget_in: u32,
    /// Strafe direction (+1 / −1) for ranged enemies.
    pub strafe_sign: f32,

    // ---- AI / navigation (docs/enemies.md) ----
    /// Path following state (ground enemies).
    pub nav: NavAgent,
    /// Spawned by encounter `(index, wave index)`.
    pub encounter: Option<(u16, u8)>,
    /// Knows where the players are without seeing them (arena spawns): never idles while a
    /// targetable player exists, never loses its target for lack of sight.
    pub aware: bool,
    /// Hurt by this player while it had no target (damage alerts).
    pub alerted_by: Option<PlayerId>,
    /// Ticks of reduced control after a heavy knockback.
    pub stagger: u16,
    /// Players already hit by the current lunging strike.
    pub lunge_hits: Vec<PlayerId>,
    /// Line of sight (eye → chest) to the target this tick.
    pub has_los: bool,
    /// Ticks since the target was last in sight (flyers circle in when it stays hidden).
    pub unseen_ticks: u32,
    /// Flyers: cached height of the ground below and ticks until it is re-measured.
    pub ground_y: f32,
    pub ground_check_in: u8,
    /// Ticks until a strafing enemy may flip its strafe direction again.
    pub strafe_flip_in: u16,
    /// Aim of the current attack (unit): an aim laser's direction from the muzzle (wind-up),
    /// a dive's direction (strike).
    pub aim: Vec3,
    /// The aim laser stopped tracking (the last moments of the wind-up).
    pub aim_locked: bool,

    // ---- the third roster: elites, frenzy, latch, pour, dummies (docs/enemies.md) ----
    /// Spawned as an elite (`EnemyDef::elite`).
    pub elite: bool,
    /// Frenzied by a toll until this tick (exclusive; 0 = never), with these multipliers.
    pub frenzy_until: u32,
    pub frenzy: Frenzy,
    /// Damage taken on the current wind-up's interrupt weak point.
    pub interrupt_damage: f32,
    /// The player a latch attack clings to (`AiState::Latched`).
    pub latched: Option<PlayerId>,
    /// Ticks until the latch's next damage tick.
    pub latch_tick_in: u16,
    /// Tick of the last damage taken (training dummies regenerate after a pause).
    pub last_hurt_tick: u32,
    /// The slag line of the pour being wound up (removed if the pour never happens).
    pub pour_line: Option<NetId>,

    // ---- getting there: leaps, melting, vantage points, detours (docs/enemies.md) ----
    /// The jump being made (`AiState::Leap`).
    pub leap: Option<LeapPlan>,
    /// Where it re-forms (feet) once its melt is over (`AiState::Melt`).
    pub reform_at: Option<Vec3>,
    /// Stranded time (ticks; off the nav grid counts triple): melts at `MeltDef::after`.
    pub stranded: u32,
    /// Progress check: where it stood when the current window started, ticks left in it, and
    /// whether the last window moved it (≥ 1 m).
    pub progress_from: Vec3,
    pub progress_in: u32,
    pub progressing: bool,
    /// Ranged enemies without sight of their target: a spot (feet) to walk to that has it, and
    /// the tick to give up on it.
    pub vantage: Option<(Vec3, u32)>,
    /// Tick of the next vantage / detour search allowed.
    pub search_after: u32,
    /// Flyers: a detour point (hover centre) to fly to round or over what hides the target,
    /// and the tick to give up on it; the tick of the last orbit flip at a wall.
    pub detour: Option<(Vec3, u32)>,
    pub last_flip: u32,
    /// Melt on its next chasing tick, stranded or not (dev `/melt`, `SimWorld::melt_enemies`).
    pub melt_now: bool,
}

impl Enemy {
    pub fn new(
        id: NetId,
        def: Arc<EnemyDef>,
        position: Vec3,
        yaw: f32,
        health: f32,
        tick: u32,
    ) -> Self {
        let attacks = def.attacks.len();
        Self {
            id,
            def,
            position,
            velocity: Vec3::ZERO,
            yaw,
            on_ground: false,
            health,
            max_health: health,
            state: AiState::Idle,
            state_ticks: 0,
            state_len: 0,
            attack: 0,
            attack_seq: 0,
            cooldowns: vec![0; attacks],
            shots_fired: 0,
            target: None,
            last_seen_tick: 0,
            last_seen_pos: position,
            aggro: Vec::new(),
            parry_window: None,
            trail: Trail::new(),
            spawn_tick: tick,
            retarget_in: (id.0 % 7),
            strafe_sign: if id.0.is_multiple_of(2) { 1.0 } else { -1.0 },
            nav: NavAgent::default(),
            encounter: None,
            aware: false,
            alerted_by: None,
            stagger: 0,
            lunge_hits: Vec::new(),
            has_los: false,
            unseen_ticks: 0,
            ground_y: position.y,
            ground_check_in: 0,
            strafe_flip_in: 0,
            aim: look_dir(yaw, 0.0),
            aim_locked: false,
            elite: false,
            frenzy_until: 0,
            frenzy: Frenzy::default(),
            interrupt_damage: 0.0,
            latched: None,
            latch_tick_in: 0,
            last_hurt_tick: 0,
            pour_line: None,
            leap: None,
            reform_at: None,
            stranded: 0,
            progress_from: position,
            progress_in: 0,
            progressing: true,
            vantage: None,
            search_after: 0,
            detour: None,
            last_flip: 0,
            melt_now: false,
        }
    }

    /// Progress (0..1) through the current timed state (0 for untimed states).
    pub fn state_progress(&self) -> f32 {
        if self.state_len > 0 {
            (self.state_ticks as f32 / self.state_len as f32).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// How far into its puddle it is ([`melt_depth`]).
    pub fn melt_depth(&self) -> f32 {
        melt_depth(self.state, self.state_progress())
    }

    /// Frenzied by a toll at `tick`.
    pub fn frenzied(&self, tick: u32) -> bool {
        tick < self.frenzy_until && self.is_alive()
    }

    /// A training dummy (never targets, attacks, moves or dies).
    pub fn is_dummy(&self) -> bool {
        self.def.dummy.is_some()
    }

    /// Can be frenzied by a toll: a living fighter that doesn't toll itself (two ringers
    /// would keep each other frenzied forever) and isn't a dummy.
    pub fn frenzies(&self) -> bool {
        self.is_alive()
            && !self.is_dummy()
            && !self.def.attacks.iter().any(|a| a.kind == AttackKind::Toll)
    }

    pub fn is_flying(&self) -> bool {
        matches!(self.def.locomotion, Locomotion::Flying { .. })
    }

    /// Holding itself up right now: a living flyer that isn't stunned (a swatted Harrier
    /// falls).
    pub fn hovering(&self) -> bool {
        self.is_flying() && self.is_alive() && self.state != AiState::Stunned
    }

    /// The current attack, if any.
    pub fn current_attack(&self) -> Option<&AttackDef> {
        self.def.attacks.get(self.attack)
    }

    /// Showing an aim laser (a laser attack's wind-up).
    pub fn aiming(&self) -> bool {
        self.state == AiState::Windup && self.current_attack().is_some_and(|a| a.laser.is_some())
    }

    /// Bottom of the collision capsule (= `position` for every enemy, flyers included; a
    /// Harrier's hover centre is `center()`).
    pub fn feet(&self) -> Vec3 {
        self.position
    }

    /// Navigation class (ground enemies): its size, its leap limits (`EnemyDef::leap`) and
    /// whether it rides pads.
    pub fn nav_class(&self) -> NavClass {
        let class = NavClass::for_size(self.def.radius, self.def.height).with_pads(self.def.pads);
        match self.def.leap {
            Some(l) => class.with_leaps(l.distance, l.up, l.down),
            None => class,
        }
    }

    /// Relative weight when enemies push each other apart (a training dummy doesn't budge).
    pub fn mass(&self) -> f32 {
        if self.is_dummy() {
            return 1e6;
        }
        self.def.radius * self.def.radius * self.def.height.max(0.1)
    }

    pub fn is_alive(&self) -> bool {
        self.state != AiState::Dying
    }

    pub fn net_kind(&self) -> u16 {
        self.def.net_kind
    }

    pub fn height(&self) -> f32 {
        self.def.height
    }

    pub fn radius(&self) -> f32 {
        self.def.radius
    }

    pub fn forward(&self) -> Vec3 {
        look_dir(self.yaw, 0.0)
    }

    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * self.def.eye_height
    }

    pub fn center(&self) -> Vec3 {
        self.feet() + Vec3::Y * (self.def.height * 0.5)
    }

    /// Local offset (+X right, +Y up, −Z forward) to world space.
    pub fn local_to_world(&self, offset: Vec3) -> Vec3 {
        self.position + Quat::from_rotation_y(self.yaw) * offset
    }

    /// Body capsule + extra body shapes + weak point spheres (`HitPart::Weak(i)`), sized to
    /// the drawn model rather than the movement capsule ([`EnemyDef::hitbox`]). A melting /
    /// re-forming Mould stays hittable, its shapes sinking with it into the floor as its model
    /// does ([`MELT_SINK`], [`MELT_SAG`]): what is still above the floor can be shot, down to
    /// the puddle.
    pub fn hitbox(&self) -> Hitbox {
        let mut hitbox = self
            .def
            .hitbox(HitTarget::Entity(self.id), self.feet(), self.yaw);
        let depth = self.melt_depth();
        if depth > 0.0 {
            let sag = 1.0 - MELT_SAG * depth;
            let sink = MELT_SINK * depth * self.def.height * self.def.draw_scale();
            let feet = self.feet().y;
            for s in &mut hitbox.shapes {
                s.a.y = feet + (s.a.y - feet) * sag - sink;
                s.b.y = feet + (s.b.y - feet) * sag - sink;
            }
        }
        hitbox
    }

    /// Changes state. The last attack's parry window survives Strike/Recover/Chase: a lagging
    /// player may still parry it while its damage is pending (see `parry.rs`); it is replaced
    /// by the next wind-up and dropped when stunned or dead.
    pub(crate) fn set_state(&mut self, state: AiState, len: u32) {
        self.state = state;
        self.state_ticks = 0;
        self.state_len = len;
        if matches!(state, AiState::Stunned | AiState::Dying | AiState::Idle) {
            self.parry_window = None;
        }
    }

    /// Stuns (parry counter / dev). Cancels the current attack.
    pub fn stun(&mut self, ticks: u16) {
        if self.is_alive() {
            self.set_state(AiState::Stunned, ticks as u32);
            self.parry_window = None;
        }
    }

    /// (stun seconds, counter damage, attack seq) of the current attack.
    pub fn parry_stun(&self) -> (f32, f32, u32) {
        let a = self.def.attacks.get(self.attack);
        (
            a.map_or(2.5, |a| a.parry_stun),
            a.map_or(150.0, |a| a.parry_damage),
            self.attack_seq,
        )
    }

    /// Replicated state (see the module docs).
    pub fn entity_state(&self, tick: u32) -> EntityState {
        let mut flags = 0;
        if self.target.is_some() {
            flags |= enemy_flags::ALERT;
        }
        if self
            .parry_window
            .is_some_and(|w| tick >= w.start && tick <= w.end)
        {
            flags |= enemy_flags::PARRYABLE;
        }
        if self.state == AiState::Stunned {
            flags |= enemy_flags::STUNNED;
        }
        if self.frenzied(tick) {
            flags |= enemy_flags::FRENZIED;
        }
        if self.elite {
            flags |= enemy_flags::ELITE;
        }
        if self.state == AiState::Latched {
            flags |= enemy_flags::LATCHED;
        }
        if !self.is_alive() {
            flags |= enemy_flags::DYING;
        }
        if !self.on_ground {
            flags |= enemy_flags::AIRBORNE;
        }
        if self.def.armour.is_some() {
            flags |= enemy_flags::ARMOURED;
        }
        let aiming = self.aiming();
        if aiming {
            flags |= enemy_flags::AIMING;
            if self.aim_locked {
                flags |= enemy_flags::AIM_LOCKED;
            }
        }
        let progress = if self.state_len > 0 {
            ((self.state_ticks as f32 / self.state_len as f32).clamp(0.0, 1.0) * 255.0) as u32
        } else {
            0
        };
        let attack = (self.attack.min(255) as u32) << 8;
        let max = (self.max_health.ceil().clamp(0.0, 65535.0) as u32) << 16;
        // Velocity only drives client extrapolation and animation speed: 1/4 m/s steps keep
        // steering jitter out of the deltas (positions stay exact).
        let velocity = (self.velocity * 4.0).round() / 4.0;
        // Yaw in ~0.7° steps for the same reason, except while aiming a laser: every client
        // draws it from the exact aim (yaw + pitch).
        let (yaw, pitch) = if aiming {
            (self.yaw, self.aim.y.clamp(-1.0, 1.0).asin())
        } else {
            ((self.yaw * 80.0).round() / 80.0, 0.0)
        };
        EntityState {
            id: self.id,
            kind: self.def.net_kind,
            owner: self
                .latched
                .or(self.target)
                .map_or(OWNER_NONE, |p| p.0 as u16),
            position: self.position,
            velocity,
            yaw,
            pitch,
            flags,
            state: self.state as u16,
            health: self.health.max(0.0).ceil().min(65535.0) as u16,
            aux: progress | attack | max,
        }
    }
}

impl SimWorld {
    /// Co-op health multiplier for newly spawned enemies: `1 + 0.5 (players − 1)` when
    /// `rules.coop_enemy_scaling` (the encounter/modes code may replace this hook).
    pub fn enemy_health_scale(&self) -> f32 {
        if self.rules.coop_enemy_scaling {
            1.0 + 0.5 * (self.players.len().max(1) as f32 - 1.0)
        } else {
            1.0
        }
    }

    /// Spawns an enemy by definition name. `None` if the name is unknown.
    pub fn spawn_enemy(&mut self, name: &str, position: Vec3, yaw: f32) -> Option<NetId> {
        let scale = self.enemy_health_scale();
        self.spawn_enemy_scaled(name, position, yaw, scale)
    }

    /// Spawns an enemy with an explicit health multiplier (encounters scale by the player
    /// count at wave start). `yaw` in radians. `None` if the name is unknown or the world is
    /// full ([`super::MAX_ENEMIES`]).
    pub fn spawn_enemy_scaled(
        &mut self,
        name: &str,
        position: Vec3,
        yaw: f32,
        health_scale: f32,
    ) -> Option<NetId> {
        self.spawn_enemy_with(
            name,
            position,
            yaw,
            SpawnOptions {
                health_scale,
                ..Default::default()
            },
        )
    }

    /// Spawns an enemy with explicit [`SpawnOptions`] (health scale, elite, aware): the game
    /// modes' spawn call. `yaw` in radians. `None` if the name is unknown or the world is full
    /// ([`super::MAX_ENEMIES`]).
    pub fn spawn_enemy_with(
        &mut self,
        name: &str,
        position: Vec3,
        yaw: f32,
        options: SpawnOptions,
    ) -> Option<NetId> {
        let def = self.enemy_defs.get(name)?;
        if self.enemies.len() >= super::MAX_ENEMIES {
            log::warn!(
                "enemy limit ({}) reached: '{name}' not spawned",
                super::MAX_ENEMIES
            );
            return None;
        }
        let id = self.alloc_net_id()?;
        let health = def.health * options.health_scale.max(0.01);
        let mut enemy = Enemy::new(id, def, position, yaw, health, self.tick);
        enemy.aware = options.aware;
        let kind = enemy.net_kind();
        self.enemies.insert(id, enemy);
        if options.elite {
            self.make_elite(id);
        }
        self.events
            .push(SimEvent::EnemySpawned { id, kind, position });
        Some(id)
    }

    /// Turns a living enemy into an elite (health and max health × `EnemyDef::elite.health`,
    /// faster attacks, replicated `ELITE`). Training dummies can't be elites. Returns whether
    /// it is one now.
    pub fn make_elite(&mut self, id: NetId) -> bool {
        let Some(e) = self.enemies.get_mut(&id) else {
            return false;
        };
        if e.elite {
            return true;
        }
        if !e.is_alive() || e.is_dummy() {
            return false;
        }
        let bonus = e.def.elite.health.max(0.01);
        e.elite = true;
        e.max_health *= bonus;
        e.health *= bonus;
        true
    }

    /// The multipliers enemy `e` acts with now ([`Fury`]).
    pub fn fury(&self, e: &Enemy) -> Fury {
        let s = self.enemy_scale;
        let mut f = Fury {
            damage: s.damage.max(0.0),
            aggression: s.aggression.max(0.05),
            projectile_speed: s.projectile_speed.max(0.05),
            speed: 1.0,
        };
        if e.frenzied(self.tick) {
            let z = e.frenzy;
            f.damage *= z.damage.max(0.0);
            f.aggression *= z.aggression.max(0.05);
            f.projectile_speed *= z.projectile_speed.max(0.05);
            f.speed *= z.speed.max(0.05);
        }
        if e.elite {
            let x = e.def.elite;
            f.aggression *= x.aggression.max(0.05);
            f.speed *= x.speed.max(0.05);
        }
        f
    }

    /// Enemy side of the damage pipeline.
    pub(crate) fn damage_enemy(&mut self, t: &Tuning, id: NetId, info: DamageInfo) -> DamageEvent {
        let tick = self.tick;
        let mut event = DamageEvent {
            target: HitTarget::Entity(id),
            info,
            dealt: 0.0,
            hard: 0.0,
            health_after: 0.0,
            killed: false,
            blocked: None,
            tick,
        };
        let Some(e) = self.enemies.get_mut(&id) else {
            event.blocked = Some(Blocked::NoTarget);
            return event;
        };
        if !e.is_alive() {
            event.blocked = Some(Blocked::NoTarget);
            return event;
        }
        if let Owner::Enemy(_) = info.source {
            event.blocked = Some(Blocked::SameTeam);
            event.health_after = e.health;
            return event;
        }
        let def = e.def.clone();
        let mut amount = info.amount.max(0.0);
        let explosive = info.flags.contains(DamageFlags::EXPLOSIVE);
        let mut weak_name = None;
        if let HitPart::Weak(i) = info.part
            && let Some(w) = def.weak_points.get(i as usize)
            && !explosive
        {
            amount *= w.multiplier;
            event.info.flags.insert(DamageFlags::WEAK_POINT);
            if w.kind == WeakKind::Head {
                event.info.flags.insert(DamageFlags::HEADSHOT);
            }
            weak_name = Some(w.name.as_str());
        }
        // A parry counter (telegraphed-melee parry) always lands in full.
        let counter = info
            .flags
            .contains(DamageFlags::MELEE | DamageFlags::PARRIED);
        if let Some(armour) = &def.armour
            && (!explosive || armour.explosions)
            && !counter
            && !weak_name.is_some_and(|n| armour.bypass.iter().any(|b| b == n))
        {
            // Damage travelling against the facing direction comes from the front.
            let from = -info.dir;
            let flat = Vec3::new(from.x, 0.0, from.z).normalize_or_zero();
            if flat != Vec3::ZERO
                && flat.angle_between(e.forward()) <= (armour.front_arc * 0.5).to_radians()
            {
                amount *= armour.front_multiplier;
                event.info.flags.insert(DamageFlags::ARMOURED);
            }
        }
        e.last_hurt_tick = tick;
        // Health actually lost (embers are shed from that).
        let lost;
        let dealt = if e.is_dummy() {
            // A training dummy shows every hit in full but never drops below 1 (and flinches).
            lost = amount.min((e.health - 1.0).max(0.0));
            e.health = (e.health - amount).max(1.0);
            if amount > 0.0 {
                let ticks = super::combat::secs_to_ticks(special::DUMMY_FLINCH, self.tick_rate, 1);
                e.set_state(AiState::Recover, ticks as u32);
            }
            amount
        } else {
            let dealt = amount.min(e.health.max(0.0));
            lost = dealt;
            e.health -= amount;
            dealt
        };
        // Hitting the interrupt weak point hard enough mid-wind-up cancels the attack (the
        // Bellwether's bell).
        if e.state == AiState::Windup
            && e.health > 0.0
            && let Some(int) = def.attacks.get(e.attack).and_then(|a| a.interrupt.as_ref())
            && let HitPart::Weak(i) = info.part
            && def
                .weak_points
                .get(i as usize)
                .is_some_and(|w| w.name == int.weak_point)
        {
            e.interrupt_damage += amount;
            if e.interrupt_damage >= int.damage {
                special::interrupt(e, int, self.tick_rate);
            }
        }
        if info.knockback.length_squared() > 0.0 {
            let elite = if e.elite { def.elite.knockback } else { 1.0 };
            let kb = info.knockback * def.knockback_scale * elite;
            e.velocity += kb;
            if e.velocity.y > 0.5 {
                e.on_ground = false;
            }
            // Heavy hits stagger (brief loss of control) so the knockback reads.
            if kb.length() > ai::STAGGER_KNOCKBACK {
                let ticks = super::combat::secs_to_ticks(ai::STAGGER_TIME, self.tick_rate, 1);
                e.stagger = e.stagger.max(ticks);
            }
        }
        if let Owner::Player(p) = info.source {
            // Getting hurt alerts an enemy that had no target.
            if e.target.is_none() {
                e.alerted_by = Some(p);
            }
            match e.aggro.iter_mut().find(|a| a.0 == p) {
                Some(a) => {
                    a.1 += dealt;
                    a.2 = tick;
                }
                None => e.aggro.push((p, dealt, tick)),
            }
        }
        event.dealt = dealt;
        event.health_after = e.health.max(0.0);
        event.killed = e.health <= 0.0;
        if lost > 0.0 {
            let shed = DamageEvent {
                dealt: lost,
                ..event
            };
            self.spawn_embers_for_damage(t, &shed);
        }
        event
    }

    /// An enemy died: corpse (Dying state), its heat released as a big ember burst, its pending
    /// hits cancelled, stickies on it fall off.
    pub fn on_enemy_died(&mut self, t: &Tuning, id: NetId, _kill: &KillInfo) {
        let rate = self.tick_rate;
        let Some(e) = self.enemies.get_mut(&id) else {
            return;
        };
        let ticks = super::combat::secs_to_ticks(e.def.death_time, rate, 1) as u32;
        e.set_state(AiState::Dying, ticks);
        e.target = None;
        // A latched Snuffer drops off its victim; a pour that never happened leaves no line.
        e.latched = None;
        let line = e.pour_line.take();
        let center = e.center();
        let heat = if e.elite { e.def.elite.heat } else { 1.0 };
        let (pool, radius) = (e.def.drops.heat * heat, e.def.drops.heat_radius);
        if let Some(line) = line {
            self.drop_unlit_line(line);
        }
        self.spawn_embers(
            t,
            center,
            pool,
            radius,
            t.combat.embers.death_lifetime,
            true,
        );
        self.pending_hits
            .retain(|h| !matches!(h.key, super::parry::ParryKey::Melee(e, _) if e == id));
        for p in self.projectiles.values_mut() {
            if let Some(stuck) = &mut p.stuck
                && stuck.to == Some(HitTarget::Entity(id))
            {
                stuck.to = None;
            }
        }
    }

    /// Removes every enemy and the hazards they left (dev `/killall` uses
    /// [`SimWorld::kill_enemy`] instead).
    pub fn clear_enemies(&mut self) {
        self.enemies.clear();
        self.enemy_hazards.clear();
    }

    /// Kills an enemy through the damage pipeline (dev commands).
    pub fn kill_enemy(&mut self, t: &Tuning, id: NetId) {
        let Some(e) = self.enemies.get(&id) else {
            return;
        };
        let info = DamageInfo::new(
            e.health.max(1.0) * 10.0,
            Owner::World,
            super::combat::Attack::Dev,
        )
        .at(e.center(), Vec3::NEG_Y);
        self.apply_damage(t, HitTarget::Entity(id), info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_blood_drop_names_still_parse() {
        // Definitions written before the ember rename.
        let d: Drops = ron::from_str("(blood_pool: 40.0, blood_radius: 5.0)").unwrap();
        assert_eq!((d.heat, d.heat_radius), (40.0, 5.0));
    }

    /// The roster contract (docs/roster.md): names, net kinds, capsule sizes, health, speed
    /// and weak points of the built-in definitions.
    #[test]
    fn builtin_roster_matches_the_contract() {
        use crate::protocol::kind;
        let r = EnemyRegistry::builtin();
        assert_eq!(r.len(), 9, "{:?}", r.names().collect::<Vec<_>>());
        // (file, name, kind, radius, height, health, speed, weak points (name, offset, r, x)).
        type Weak = (&'static str, [f32; 3], f32, f32);
        type Row = (
            &'static str,
            &'static str,
            u16,
            f32,
            f32,
            f32,
            f32,
            &'static [Weak],
        );
        let table: [Row; 9] = [
            (
                "gnawer",
                "Gnawer",
                kind::GNAWER,
                0.6,
                2.0,
                45.0,
                10.0,
                &[("head", [0.0, 1.95, -0.65], 0.52, 2.0)],
            ),
            (
                "censer",
                "Censer",
                kind::CENSER,
                0.6,
                2.5,
                80.0,
                5.0,
                &[("lantern", [0.0, 2.68, -0.12], 0.5, 2.0)],
            ),
            (
                "harrier",
                "Harrier",
                kind::HARRIER,
                0.7,
                1.3,
                40.0,
                15.0,
                &[("heart", [0.0, 0.26, -0.2], 0.44, 2.5)],
            ),
            (
                "gaoler",
                "Gaoler",
                kind::GAOLER,
                1.3,
                3.7,
                520.0,
                4.5,
                &[
                    ("head", [0.0, 3.72, -0.35], 0.5, 1.5),
                    ("lantern", [0.0, 2.46, 1.06], 0.6, 2.5),
                ],
            ),
            (
                "heron",
                "Heron",
                kind::HERON,
                0.5,
                3.4,
                60.0,
                7.0,
                &[("head", [0.0, 3.74, -0.48], 0.42, 2.5)],
            ),
            // The third roster (docs/enemies.md).
            (
                "bellwether",
                "Bellwether",
                kind::BELLWETHER,
                0.7,
                3.1,
                220.0,
                5.5,
                &[("bell", [0.0, 3.17, 0.36], 0.72, 2.0)],
            ),
            (
                "snuffer",
                "Snuffer",
                kind::SNUFFER,
                0.7,
                1.4,
                70.0,
                12.5,
                &[("wick", [0.0, 0.82, -1.32], 0.42, 2.5)],
            ),
            (
                "smelter",
                "Smelter",
                kind::SMELTER,
                1.1,
                3.4,
                420.0,
                3.6,
                &[
                    ("hatch", [0.0, 1.62, -1.45], 0.5, 2.5),
                    ("head", [0.0, 3.4, -0.37], 0.45, 1.5),
                ],
            ),
            (
                "effigy",
                "Effigy",
                kind::EFFIGY,
                0.5,
                2.2,
                1000.0,
                0.0,
                &[
                    ("head", [0.0, 2.46, -0.05], 0.37, 2.0),
                    ("sigil", [0.0, 1.69, -0.35], 0.27, 1.5),
                ],
            ),
        ];
        for (file, name, net_kind, radius, height, health, speed, weak) in table {
            let d = r.get(file).unwrap_or_else(|| panic!("{file} is built in"));
            assert_eq!(d.name, name);
            assert_eq!(d.net_kind, net_kind, "{name}");
            assert_eq!(
                r.by_kind(net_kind).map(|d| d.name.clone()),
                Some(name.to_string())
            );
            assert_eq!((d.radius, d.height), (radius, height), "{name} capsule");
            assert_eq!((d.health, d.speed), (health, speed), "{name}");
            assert_eq!(d.weak_points.len(), weak.len(), "{name} weak points");
            for (w, (wn, off, wr, mult)) in d.weak_points.iter().zip(weak) {
                assert_eq!(w.name, *wn);
                assert_eq!(w.offset, Vec3::from(*off), "{name} {wn}");
                assert_eq!((w.radius, w.multiplier), (*wr, *mult), "{name} {wn}");
            }
            // Only the training dummy has no attacks.
            assert_eq!(d.attacks.is_empty(), d.dummy.is_some(), "{name}");
            for a in &d.attacks {
                assert!(a.windup > 0.0, "{name} {}: every attack telegraphs", a.name);
            }
            // Biggest dimension at least ~2 m (docs/roster.md "big and readable").
            assert!(height.max(radius * 2.0) >= 1.3, "{name}");
            // The refit (docs/enemies.md "Sizes and hit shapes"): drawn bigger than built, and
            // the body hit capsule at least as big as the movement capsule it is drawn around.
            assert!(d.draw_scale() >= 1.1, "{name} is drawn bigger");
            let hit_r = d.hit_radius.unwrap_or(d.radius);
            assert!(
                hit_r >= radius,
                "{name}: hit radius {hit_r} < capsule {radius}"
            );
        }
        assert!(
            r.get("husk").is_none() && r.by_kind(16).is_none(),
            "the first roster is retired"
        );
        let gaoler = r.get("gaoler").unwrap();
        let armour = gaoler.armour.as_ref().unwrap();
        assert_eq!((armour.front_multiplier, armour.front_arc), (0.15, 120.0));
        // The specials parse with their parameters; the dummy is never in waves.
        let bell = r.get("bellwether").unwrap();
        let toll = bell
            .attacks
            .iter()
            .find(|a| a.kind == AttackKind::Toll)
            .unwrap();
        assert!(toll.toll.is_some() && toll.interrupt.is_some());
        assert!(r.get("snuffer").unwrap().attacks[0].latch.is_some());
        let smelter = r.get("smelter").unwrap();
        assert!(smelter.attacks.iter().any(|a| a.pour.is_some()));
        assert_eq!(smelter.elite.health, 2.0);
        assert_eq!(gaoler.elite, EliteDef::default());
        let waves: Vec<&str> = r.wave_names().collect();
        assert_eq!(waves.len(), 8);
        assert!(!waves.contains(&"effigy"));
    }

    /// The game modes' spawn (encounters, grind) makes the elites it rolls.
    #[test]
    fn pve_spawns_make_the_rolled_elites() {
        let level = crate::sim::builtin_level(crate::sim::BUILTIN_LEVEL).unwrap();
        let mut w = SimWorld::new(crate::sim::BUILTIN_LEVEL, Arc::new(level), 60);
        let elite = w
            .spawn_pve_enemy("gnawer", Vec3::ZERO, 0.0, 2.0, true)
            .unwrap();
        let plain = w
            .spawn_pve_enemy("gnawer", Vec3::X * 4.0, 0.0, 2.0, false)
            .unwrap();
        let (e, p) = (&w.enemies[&elite], &w.enemies[&plain]);
        assert!(e.elite && e.aware && !p.elite && p.aware);
        assert_eq!((e.max_health, p.max_health), (45.0 * 2.0 * 2.5, 90.0));
        // A training dummy is never an elite.
        let dummy = w
            .spawn_pve_enemy("effigy", Vec3::Z * 4.0, 0.0, 1.0, true)
            .unwrap();
        assert!(!w.enemies[&dummy].elite);
    }

    /// A melting Mould stays hittable while its hit shapes sink with it: part of it above the
    /// floor halfway down, only the puddle's height at the bottom, all of it again once risen.
    #[test]
    fn hit_shapes_sink_into_the_puddle() {
        let r = EnemyRegistry::builtin();
        for name in ["gnawer", "gaoler", "heron", "smelter"] {
            let def = r.get(name).unwrap();
            let mut e = Enemy::new(NetId(1024), def.clone(), Vec3::ZERO, 0.0, 10.0, 0);
            let top = |e: &Enemy| {
                e.hitbox()
                    .shapes
                    .iter()
                    .map(|s| s.a.y.max(s.b.y) + s.radius)
                    .fold(f32::MIN, f32::max)
            };
            let standing = top(&e);
            e.set_state(AiState::Melt, 36);
            e.state_ticks = 12;
            let sinking = top(&e);
            e.state_ticks = 36;
            let puddle = top(&e);
            e.set_state(AiState::Reform, 48);
            e.state_ticks = 48;
            let risen = top(&e);
            assert!(sinking < standing && sinking > 0.5, "{name}: {sinking}");
            assert!(puddle < 0.4 * standing && puddle < 1.4, "{name}: {puddle}");
            assert!((risen - standing).abs() < 1e-4, "{name}");
        }
    }

    #[test]
    fn local_offsets_follow_yaw() {
        let def = Arc::new(EnemyDef::default());
        let mut e = Enemy::new(NetId(1024), def, Vec3::ZERO, 0.0, 10.0, 0);
        assert!((e.local_to_world(Vec3::NEG_Z) - Vec3::NEG_Z).length() < 1e-6);
        e.yaw = std::f32::consts::FRAC_PI_2; // facing -X
        assert!((e.local_to_world(Vec3::NEG_Z) - Vec3::NEG_X).length() < 1e-6);
        assert!((e.forward() - Vec3::NEG_X).length() < 1e-6);
    }

    #[test]
    fn definitions_on_disk_override_and_extend_the_builtins() {
        let dir = std::env::temp_dir().join(format!("velocide-enemies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("gnawer.ron"), "(name: \"Gnawer\", health: 7.0)").unwrap();
        std::fs::write(dir.join("newt.ron"), "(name: \"Newt\", net_kind: 999)").unwrap();
        std::fs::write(dir.join("broken.ron"), "(health: ").unwrap();
        let builtin = EnemyRegistry::builtin();
        let r = EnemyRegistry::load_dir(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            r.len(),
            builtin.len() + 1,
            "the builtins plus newt, not broken"
        );
        assert_eq!(r.get("gnawer").unwrap().health, 7.0);
        assert_ne!(builtin.get("gnawer").unwrap().health, 7.0);
        assert_eq!(r.get("newt").unwrap().name, "Newt");
        assert_eq!(r.get("censer"), builtin.get("censer"));
    }
}
