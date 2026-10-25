//! Server-run enemies: data-driven definitions ([`EnemyDef`], `assets/enemies/*.ron`), the
//! runtime [`Enemy`], damage (weak points, armour, knockback, aggro), death and replication.
//! The AI state machine lives in `ai.rs`, navigation / flight steering in `steer.rs` (on top of
//! `sim::nav`). Overview: `docs/enemies.md`.
//!
//! Replicated as entity `kind = def.net_kind` (Gnawer 20, Censer 21, Harrier 22, Gaoler 23,
//! Heron 24; docs/roster.md): `owner` = target player id (`OWNER_NONE` without a target),
//! `position` = feet (bottom of the collision capsule; a Harrier's hover centre is half its
//! height above), `velocity`, `yaw` (+ `pitch` and exact yaw while `AIMING` a laser),
//! `flags` = [`crate::protocol::enemy_flags`], `state` = [`AiState`] as u16, `health` =
//! current health (rounded up), `aux` = state progress (bits 0–7, 0..255 through timed states)
//! | attack index (bits 8–15) | max health (bits 16–31).

mod ai;
mod def;
mod steer;

pub use ai::{is_targetable, lob_velocity};
pub use def::*;
pub use steer::{AiStats, AiWorld, NavAgent, PathMode};

use std::sync::Arc;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::combat::{Blocked, DamageEvent, DamageFlags, DamageInfo, KillInfo, Owner};
use super::trail::Trail;
use super::nav::NavClass;
use super::{HitPart, HitShape, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::input::look_dir;
use crate::protocol::{EntityState, NetId, OWNER_NONE, enemy_flags};
use crate::tuning::Tuning;

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
            _ => return None,
        })
    }
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
        }
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

    /// Navigation size class (ground enemies).
    pub fn nav_class(&self) -> NavClass {
        NavClass::for_size(self.def.radius, self.def.height)
    }

    /// Relative weight when enemies push each other apart.
    pub fn mass(&self) -> f32 {
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

    /// Body capsule + weak point spheres (`HitPart::Weak(i)`).
    pub fn hitbox(&self) -> Hitbox {
        let r = self.def.radius;
        let h = self.def.body_top.unwrap_or(self.def.height).max(2.0 * r);
        let feet = self.feet();
        let mut shapes = vec![HitShape::capsule(
            HitPart::Body,
            feet + Vec3::Y * r,
            feet + Vec3::Y * (h - r),
            r,
        )];
        for (i, w) in self.def.weak_points.iter().enumerate() {
            shapes.push(HitShape::sphere(
                HitPart::Weak(i.min(u8::MAX as usize) as u8),
                self.local_to_world(w.offset),
                w.radius,
            ));
        }
        Hitbox {
            target: HitTarget::Entity(self.id),
            shapes,
        }
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
            owner: self.target.map_or(OWNER_NONE, |p| p.0 as u16),
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
        let def = self.enemy_defs.get(name)?;
        if self.enemies.len() >= super::MAX_ENEMIES {
            log::warn!("enemy limit ({}) reached: '{name}' not spawned", super::MAX_ENEMIES);
            return None;
        }
        let id = self.alloc_net_id()?;
        let health = def.health * health_scale.max(0.01);
        let enemy = Enemy::new(id, def, position, yaw, health, self.tick);
        let kind = enemy.net_kind();
        self.enemies.insert(id, enemy);
        self.events
            .push(SimEvent::EnemySpawned { id, kind, position });
        Some(id)
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
        let counter = info.flags.contains(DamageFlags::MELEE | DamageFlags::PARRIED);
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
        let dealt = amount.min(e.health.max(0.0));
        e.health -= amount;
        if info.knockback.length_squared() > 0.0 {
            let kb = info.knockback * def.knockback_scale;
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
        if dealt > 0.0 {
            self.spawn_blood_for_damage(t, &event);
        }
        event
    }

    /// An enemy died: corpse (Dying state), death blood burst, its pending hits cancelled,
    /// stickies on it fall off.
    pub fn on_enemy_died(&mut self, t: &Tuning, id: NetId, _kill: &KillInfo) {
        let rate = self.tick_rate;
        let Some(e) = self.enemies.get_mut(&id) else {
            return;
        };
        let ticks = super::combat::secs_to_ticks(e.def.death_time, rate, 1) as u32;
        e.set_state(AiState::Dying, ticks);
        e.target = None;
        let center = e.center();
        let (pool, radius) = (e.def.drops.blood_pool, e.def.drops.blood_radius);
        self.spawn_blood(t, center, pool, radius, t.combat.blood.death_lifetime, true);
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

    /// Removes every enemy (dev `/killall` uses [`SimWorld::kill_enemy`] instead).
    pub fn clear_enemies(&mut self) {
        self.enemies.clear();
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

    /// The roster contract (docs/roster.md): names, net kinds, capsule sizes, health, speed
    /// and weak points of the built-in definitions.
    #[test]
    fn builtin_roster_matches_the_contract() {
        use crate::protocol::kind;
        let r = EnemyRegistry::builtin();
        assert_eq!(r.len(), 5, "{:?}", r.names().collect::<Vec<_>>());
        // (file, name, kind, radius, height, health, speed, weak points (name, offset, r, x)).
        type Weak = (&'static str, [f32; 3], f32, f32);
        let table: [(&str, &str, u16, f32, f32, f32, f32, &[Weak]); 5] = [
            ("gnawer", "Gnawer", kind::GNAWER, 0.6, 2.0, 45.0, 10.0,
                &[("head", [0.0, 1.62, -0.38], 0.38, 2.0)]),
            ("censer", "Censer", kind::CENSER, 0.6, 2.5, 80.0, 5.0,
                &[("lantern", [0.0, 2.22, -0.1], 0.42, 2.0)]),
            ("harrier", "Harrier", kind::HARRIER, 0.7, 1.3, 40.0, 15.0,
                &[("heart", [0.0, 0.35, -0.15], 0.35, 2.5)]),
            ("gaoler", "Gaoler", kind::GAOLER, 1.3, 3.7, 520.0, 4.5,
                &[("head", [0.0, 3.32, -0.35], 0.45, 1.5), ("lantern", [0.0, 2.2, 0.95], 0.55, 2.5)]),
            ("heron", "Heron", kind::HERON, 0.5, 3.4, 60.0, 7.0,
                &[("head", [0.0, 3.12, -0.4], 0.35, 2.5)]),
        ];
        for (file, name, net_kind, radius, height, health, speed, weak) in table {
            let d = r.get(file).unwrap_or_else(|| panic!("{file} is built in"));
            assert_eq!(d.name, name);
            assert_eq!(d.net_kind, net_kind, "{name}");
            assert_eq!(r.by_kind(net_kind).map(|d| d.name.clone()), Some(name.to_string()));
            assert_eq!((d.radius, d.height), (radius, height), "{name} capsule");
            assert_eq!((d.health, d.speed), (health, speed), "{name}");
            assert_eq!(d.weak_points.len(), weak.len(), "{name} weak points");
            for (w, (wn, off, wr, mult)) in d.weak_points.iter().zip(weak) {
                assert_eq!(w.name, *wn);
                assert_eq!(w.offset, Vec3::from(*off), "{name} {wn}");
                assert_eq!((w.radius, w.multiplier), (*wr, *mult), "{name} {wn}");
            }
            assert!(!d.attacks.is_empty());
            for a in &d.attacks {
                assert!(a.windup > 0.0, "{name} {}: every attack telegraphs", a.name);
            }
            // Biggest dimension at least ~2 m (docs/roster.md "big and readable").
            assert!(height.max(radius * 2.0) >= 1.3, "{name}");
        }
        assert!(r.get("husk").is_none() && r.by_kind(16).is_none(), "the first roster is retired");
        let gaoler = r.get("gaoler").unwrap();
        let armour = gaoler.armour.as_ref().unwrap();
        assert_eq!((armour.front_multiplier, armour.front_arc), (0.15, 120.0));
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
}
