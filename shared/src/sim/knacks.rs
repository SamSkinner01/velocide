//! Knacks: the utility on Q (`Buttons::THROW`, the KNACK control), one per loadout
//! ([`Loadout::knack`](crate::loadout::Loadout), docs/arsenal.md "Knacks"). `step_weapons`
//! spends the charges (predicted like everything a weapon decides) and fires
//! `FireIntent::Knack`; [`SimWorld::use_knack`] (server) does the rest:
//!
//! | knack | what it does |
//! |---|---|
//! | Thorn | the relay spike (`thorn.rs`). PvE only |
//! | Kindle Flask | a lobbed flask; where it lands a [`KindlePool`] heals the Shrikes inside |
//! | Snare | a wax trap thrown to the floor; armed, it pins the first hostile close by |
//! | Soot Pot | bursts into a [`SootCloud`]: Moulds can't see into, out of or through it |
//! | Tallow Skin | a few seconds of half damage and no knockback (`TALLOW` player flag) |
//! | Surge | an impulse along the look, mostly horizontal |
//!
//! In a PvP world a knack that isn't `Knack::pvp_allowed` is replaced by `Knack::PVP_FALLBACK`
//! when the loadout applies (`SimWorld::fit_loadout`, `loadouts.rs`).
//!
//! Replication: the flask, snare and pot fly as projectiles (`kind::KINDLE_FLASK`, `SNARE`,
//! `SOOT_POT`); pools and clouds are `kind::KINDLE_POOL` / `SOOT_CLOUD` entities; Tallow Skin is
//! the `TALLOW` player flag; every use but the Thorn's is a [`SimEvent::KnackUsed`] (`Fx::Knack`
//! for the others).

use std::collections::BTreeMap;

use glam::Vec3;

use super::combat::{Owner, secs_to_ticks};
use super::projectiles::ProjectileKind;
use super::{PlayerId, SimEvent, SimWorld};
use crate::input::PlayerInput;
use crate::loadout::Knack;
use crate::protocol::{EntityState, NetId, kind, owner_code};
use crate::tuning::{CombatTuning, Tuning};

impl Knack {
    /// Charges it holds (the Thorn: `thorn.charges`).
    pub fn charges(self, t: &CombatTuning) -> u32 {
        let k = &t.knacks;
        match self {
            Knack::Thorn => t.thorn.charges,
            Knack::KindleFlask => k.kindle_flask.charges,
            Knack::Snare => k.snare.charges,
            Knack::SootPot => k.soot_pot.charges,
            Knack::TallowSkin => k.tallow_skin.charges,
            Knack::Surge => k.surge.charges,
        }
    }

    /// Seconds per recharged charge.
    pub fn recharge_time(self, t: &CombatTuning) -> f32 {
        let k = &t.knacks;
        match self {
            Knack::Thorn => t.thorn.recharge_time,
            Knack::KindleFlask => k.kindle_flask.recharge_time,
            Knack::Snare => k.snare.recharge_time,
            Knack::SootPot => k.soot_pot.recharge_time,
            Knack::TallowSkin => k.tallow_skin.recharge_time,
            Knack::Surge => k.surge.recharge_time,
        }
    }
}

/// A Kindle Flask's healing pool on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KindlePool {
    pub id: NetId,
    pub owner: PlayerId,
    /// Centre, on the ground.
    pub position: Vec3,
    pub radius: f32,
    pub expire_tick: u32,
}

impl KindlePool {
    /// Replicated state: `state` = radius (dm), `aux` = tick it dries up, `owner` = thrower.
    pub fn entity_state(&self) -> EntityState {
        EntityState {
            id: self.id,
            kind: kind::KINDLE_POOL,
            owner: owner_code(Owner::Player(self.owner)),
            position: self.position,
            state: radius_dm(self.radius),
            aux: self.expire_tick,
            ..Default::default()
        }
    }
}

/// A Soot Pot's cloud.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SootCloud {
    pub id: NetId,
    pub owner: PlayerId,
    pub position: Vec3,
    pub radius: f32,
    pub expire_tick: u32,
}

impl SootCloud {
    /// Replicated state: `state` = radius (dm), `aux` = tick it clears, `owner` = thrower.
    pub fn entity_state(&self) -> EntityState {
        EntityState {
            id: self.id,
            kind: kind::SOOT_CLOUD,
            owner: owner_code(Owner::Player(self.owner)),
            position: self.position,
            state: radius_dm(self.radius),
            aux: self.expire_tick,
            ..Default::default()
        }
    }
}

fn radius_dm(radius: f32) -> u16 {
    (radius * 10.0).round().clamp(0.0, u16::MAX as f32) as u16
}

/// What the knacks leave in the world ([`SimWorld::knacks`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnackWorld {
    pub pools: BTreeMap<NetId, KindlePool>,
    pub clouds: BTreeMap<NetId, SootCloud>,
    /// Tallow Skin: the tick it wears off, per player.
    pub tallow: BTreeMap<PlayerId, u32>,
}

impl KnackWorld {
    /// Live entities (for the id allocator).
    pub(crate) fn len(&self) -> usize {
        self.pools.len() + self.clouds.len()
    }

    pub(crate) fn holds(&self, id: NetId) -> bool {
        self.pools.contains_key(&id) || self.clouds.contains_key(&id)
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Forgets everything of player `id` (they left).
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.pools.retain(|_, p| p.owner != id);
        self.clouds.retain(|_, c| c.owner != id);
        self.tallow.remove(&id);
    }
}

impl SimWorld {
    /// `FireIntent::Knack` (server): what a knack other than the Thorn does (the Thorn's throw
    /// is `thorn.rs`).
    pub(crate) fn use_knack(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        input: &PlayerInput,
        knack: Knack,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let k = &t.combat.knacks;
        let owner = Owner::Player(id);
        match knack {
            Knack::Thorn => {
                // Thrown along the aim, inheriting part of the thrower's velocity.
                let c = &t.combat;
                let vel =
                    aim * c.thorn.throw_speed + p.movement.velocity * c.thorn.inherit_velocity;
                self.spawn_thorn(t, id, eye + aim * 0.5, vel);
                // Its own event (`ThornThrown`), not `KnackUsed`.
                return;
            }
            Knack::KindleFlask => {
                let f = &k.kindle_flask;
                let dir = super::weapons::lob_dir(aim, f.angle_degrees);
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                self.spawn_projectile(t, ProjectileKind::KindleFlask, owner, start, dir * f.speed);
            }
            Knack::Snare => {
                let s = &k.snare;
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                self.spawn_projectile(t, ProjectileKind::Snare, owner, start, aim * s.speed);
            }
            Knack::SootPot => {
                let s = &k.soot_pot;
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                self.spawn_projectile(t, ProjectileKind::SootPot, owner, start, aim * s.speed);
            }
            Knack::TallowSkin => {
                let until =
                    self.tick + secs_to_ticks(k.tallow_skin.duration, self.tick_rate, 1) as u32;
                self.knacks.tallow.insert(id, until);
            }
            Knack::Surge => {
                let s = &k.surge;
                let dir = Vec3::new(aim.x, aim.y * s.vertical, aim.z).normalize_or(aim);
                self.apply_impulse(id, dir * s.speed);
            }
        }
        self.events.push(SimEvent::KnackUsed { player: id, knack });
    }

    /// Is Tallow Skin on `id` right now?
    pub fn has_tallow(&self, id: PlayerId) -> bool {
        self.knacks
            .tallow
            .get(&id)
            .is_some_and(|until| self.tick < *until)
    }

    /// A Kindle Flask landed at `at` (on the ground below it, none over a pit): its pool.
    pub(crate) fn spawn_kindle_pool(&mut self, t: &Tuning, owner: PlayerId, at: Vec3) {
        let Some(floor) = self.floor_below(at, 50.0) else {
            return;
        };
        let Some(id) = self.alloc_net_id() else {
            return;
        };
        let f = &t.combat.knacks.kindle_flask;
        let expire_tick = self.tick + secs_to_ticks(f.duration, self.tick_rate, 1) as u32;
        self.knacks.pools.insert(
            id,
            KindlePool {
                id,
                owner,
                position: floor,
                radius: f.radius,
                expire_tick,
            },
        );
    }

    /// A Soot Pot burst at `at`: its cloud.
    pub(crate) fn spawn_soot_cloud(&mut self, t: &Tuning, owner: PlayerId, at: Vec3) {
        let Some(id) = self.alloc_net_id() else {
            return;
        };
        let s = &t.combat.knacks.soot_pot;
        let expire_tick = self.tick + secs_to_ticks(s.duration, self.tick_rate, 1) as u32;
        self.knacks.clouds.insert(
            id,
            SootCloud {
                id,
                owner,
                position: at,
                radius: s.radius,
                expire_tick,
            },
        );
    }

    /// Per tick (pipeline step 4): pools, clouds and Tallow Skin wear off.
    pub(crate) fn step_knacks(&mut self, _t: &Tuning) {
        let now = self.tick;
        self.knacks.pools.retain(|_, p| now < p.expire_tick);
        self.knacks.clouds.retain(|_, c| now < c.expire_tick);
        self.knacks.tallow.retain(|_, until| now < *until);
    }
}
