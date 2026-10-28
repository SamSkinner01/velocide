//! Knacks: the utility on Q (`Buttons::THROW`, the KNACK control), one per loadout
//! ([`Loadout::knack`](crate::loadout::Loadout), docs/arsenal.md "Knacks"). `step_weapons`
//! spends the charges (predicted like everything a weapon decides) and fires
//! `FireIntent::Knack`; [`SimWorld::use_knack`] (server) does the rest:
//!
//! | knack | what it does |
//! |---|---|
//! | Thorn | the relay spike (`thorn.rs`). PvE only |
//! | Kindle Flask | drunk at once: a few seconds of haste and a heal over time ([`KindleDraught`]) |
//! | Snare | a wax trap thrown to the floor; armed, it pins the first hostile close by |
//! | Soot Pot | bursts into a [`SootCloud`]: Moulds can't see into, out of or through it |
//! | Tallow Skin | a few seconds of half damage and no knockback (`TALLOW` player flag) |
//! | Surge | an impulse along the look, mostly horizontal |
//!
//! In a PvP world a knack that isn't `Knack::pvp_allowed` is replaced by `Knack::PVP_FALLBACK`
//! when the loadout applies (`SimWorld::fit_loadout`, `loadouts.rs`).
//!
//! * **Kindle Flask**: drinking it (instant, nothing thrown) hastes the drinker for `duration`
//!   — `MoveState::haste_time` / `haste_scale`, next to the Pinned slow, so the owning client
//!   predicts it like the rest of its movement (it learns of it through reconciliation, like
//!   the slow) — and heals them `heal_rate` × `heal_interval` every `heal_interval` through
//!   `PlayerCombat::heal` (the normal rules: never above the effective max, so hard damage
//!   still caps it), the first step one interval after drinking. Only the drinker, in every
//!   mode. `SimEvent::Healed` with `HealSource::KindleFlask`. Death ends it.
//! * **Snare**: it lies `STUCK` where it settled on the floor (`payload.rs`); `arm_time` after
//!   landing it is armed, and the nearest hostile whose feet come within `trigger_radius` of it
//!   (and at most 2 m above) springs it: `damage` as `Attack::Snare` and Pinned
//!   (`SimWorld::apply_pin`, the Pinning Arrow's times). A throw with `max_live` of yours out
//!   retires the oldest.
//! * **Soot cloud**: [`SimWorld::soot_blocks`] — a Mould whose eye or target is inside a cloud,
//!   or whose line to it crosses one, doesn't see it: it isn't picked as a target, a target
//!   hidden this way is lost and a Heron's laser wind-up breaks off (`enemy/ai.rs`). Players'
//!   shots pass through (the cloud only hides).
//!
//! Replication: the snare and pot fly as projectiles (`kind::SNARE`, `SOOT_POT`; an armed snare
//! carries `projectile_flags::ARMED`); clouds are `kind::SOOT_CLOUD` entities; Tallow Skin is
//! the `TALLOW` player flag, the Kindle Flask's haste `HASTE` (and the drinker's own
//! `MoveState`); every use but the Thorn's is a [`SimEvent::KnackUsed`] (`Fx::Knack` for the
//! others), a sprung snare a [`SimEvent::SnareSprung`].

use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;

use super::combat::{Attack, DamageInfo, Owner, secs_to_ticks};
use super::projectiles::ProjectileKind;
use super::{HealSource, HitPart, PlayerId, SimEvent, SimWorld};
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

/// A Kindle Flask being drunk: its heal over time (the haste is in the drinker's
/// `MoveState`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KindleDraught {
    /// HP per heal step.
    pub heal: f32,
    /// Ticks between heal steps.
    pub step: u32,
    /// Next heal step.
    pub next_heal: u32,
    /// Tick of the last heal step.
    pub last_heal: u32,
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
    pub clouds: BTreeMap<NetId, SootCloud>,
    /// Kindle Flasks being drunk, per player.
    pub kindle: BTreeMap<PlayerId, KindleDraught>,
    /// Tallow Skin: the tick it wears off, per player.
    pub tallow: BTreeMap<PlayerId, u32>,
    /// Snares (projectiles) armed and waiting: replicated as `projectile_flags::ARMED`.
    pub armed: BTreeSet<NetId>,
}

impl KnackWorld {
    /// Live entities (for the id allocator).
    pub(crate) fn len(&self) -> usize {
        self.clouds.len()
    }

    pub(crate) fn holds(&self, id: NetId) -> bool {
        self.clouds.contains_key(&id)
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Forgets everything of player `id` (they left).
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.clouds.retain(|_, c| c.owner != id);
        self.tallow.remove(&id);
        self.kindle.remove(&id);
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
            Knack::KindleFlask => self.drink_kindle(t, id),
            Knack::Snare => {
                let s = &k.snare;
                // At most `max_live` out: the oldest goes.
                let mut mine: Vec<(u32, NetId)> = self
                    .projectiles
                    .values()
                    .filter(|p| p.kind == ProjectileKind::Snare && p.owner == owner)
                    .map(|p| (p.spawn_tick, p.id))
                    .collect();
                mine.sort_unstable();
                let excess = (mine.len() + 1).saturating_sub(s.max_live.max(1) as usize);
                for (_, old) in mine.into_iter().take(excess) {
                    self.projectiles.remove(&old);
                    self.knacks.armed.remove(&old);
                }
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

    /// Player `id` drinks a Kindle Flask: hasted (their `MoveState`) and healing over time
    /// (see the module docs). A second one while the first lasts starts over.
    fn drink_kindle(&mut self, t: &Tuning, id: PlayerId) {
        let f = &t.combat.knacks.kindle_flask;
        let rate = self.tick_rate;
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        p.movement.haste_time = f.duration;
        p.movement.haste_scale = f.haste;
        let step = secs_to_ticks(f.heal_interval, rate, 1) as u32;
        let steps = (f.duration / f.heal_interval.max(1e-3)).round().max(1.0) as u32;
        self.knacks.kindle.insert(
            id,
            KindleDraught {
                heal: f.heal_rate * step as f32 / rate as f32,
                step,
                next_heal: self.tick + step,
                last_heal: self.tick + step * steps,
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

    /// Does a soot cloud hide `b` from `a`: either inside one, or the line between them
    /// crossing one? (Moulds' sight, `enemy/ai.rs`.)
    pub fn soot_blocks(&self, a: Vec3, b: Vec3) -> bool {
        let now = self.tick;
        self.knacks
            .clouds
            .values()
            .any(|c| now < c.expire_tick && segment_meets_sphere(a, b, c.position, c.radius))
    }

    /// Per tick (pipeline step 4): clouds and Tallow Skin wear off, Kindle Flasks heal, snares
    /// arm and spring.
    pub(crate) fn step_knacks(&mut self, t: &Tuning) {
        let now = self.tick;
        self.knacks.clouds.retain(|_, c| now < c.expire_tick);
        self.knacks.tallow.retain(|_, until| now < *until);
        self.heal_kindle(t);
        self.step_snares(t);
    }

    /// Every Kindle Flask due a heal step heals its drinker (see the module docs); a dead
    /// drinker's or a finished one is gone.
    fn heal_kindle(&mut self, t: &Tuning) {
        let now = self.tick;
        let players = &self.players;
        self.knacks.kindle.retain(|id, d| {
            d.next_heal <= d.last_heal && players.get(id).is_some_and(|p| !p.combat.dead)
        });
        let due: Vec<(PlayerId, f32)> = self
            .knacks
            .kindle
            .iter_mut()
            .filter(|(_, d)| now >= d.next_heal)
            .map(|(id, d)| {
                d.next_heal += d.step;
                (*id, d.heal)
            })
            .collect();
        for (id, amount) in due {
            let Some(p) = self.players.get_mut(&id) else {
                continue;
            };
            let healed = p.combat.heal(amount);
            if healed > 0.0 {
                // From the drinker's own wick.
                let (position, _, _) = p.movement.capsule(&t.movement);
                self.events.push(SimEvent::Healed {
                    player: id,
                    amount: healed,
                    source: HealSource::KindleFlask,
                    position,
                });
            }
        }
    }

    /// Snares lying on the floor arm `arm_time` after landing; an armed one springs on the
    /// nearest hostile close enough (see the module docs).
    fn step_snares(&mut self, t: &Tuning) {
        let s = &t.combat.knacks.snare;
        let now = self.tick;
        let arm = secs_to_ticks(s.arm_time, self.tick_rate, 0) as u32;
        self.knacks.armed.clear();
        let armed: Vec<(NetId, PlayerId, Vec3)> = self
            .projectiles
            .values()
            .filter(|p| p.kind == ProjectileKind::Snare && p.stuck.is_some())
            .filter(|p| now >= p.launch_tick.saturating_add(arm))
            .filter_map(|p| Some((p.id, p.owner.player()?, p.position)))
            .collect();
        for (id, owner, at) in armed {
            let by = Owner::Player(owner);
            let victim = self
                .targets_in_patch(at, s.trigger_radius)
                .into_iter()
                .filter(|target| self.can_hit(by, *target))
                .filter_map(|target| Some((self.target_origin(target)?.distance(at), target)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((_, target)) = victim else {
                self.knacks.armed.insert(id);
                continue;
            };
            self.projectiles.remove(&id);
            let center = self.target_center(target).unwrap_or(at);
            let info = DamageInfo::new(s.damage, by, Attack::Snare)
                .at(center, Vec3::Y)
                .part(HitPart::Body);
            self.apply_damage(t, target, info);
            self.apply_pin(t, by, target);
            self.events.push(SimEvent::SnareSprung {
                id,
                owner,
                target,
                position: at,
            });
        }
    }
}

/// Does the segment `a`–`b` come within `radius` of `center`?
fn segment_meets_sphere(a: Vec3, b: Vec3, center: Vec3, radius: f32) -> bool {
    let ab = b - a;
    let along = ((center - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
    (a + ab * along).distance_squared(center) <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_meet_spheres_inside_and_through() {
        let c = Vec3::new(0.0, 0.0, -10.0);
        // Through it, from inside it, into it; past it, short of it.
        assert!(segment_meets_sphere(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -20.0),
            c,
            2.0
        ));
        assert!(segment_meets_sphere(c, Vec3::new(30.0, 0.0, 0.0), c, 2.0));
        assert!(segment_meets_sphere(Vec3::ZERO, c + Vec3::X, c, 2.0));
        assert!(!segment_meets_sphere(
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, -20.0),
            c,
            2.0
        ));
        assert!(!segment_meets_sphere(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -7.0),
            c,
            2.0
        ));
        // A zero-length segment is a point.
        assert!(segment_meets_sphere(c, c, c, 0.5));
    }

    #[test]
    fn every_knack_has_charges_and_a_recharge() {
        let t = CombatTuning::default();
        for k in Knack::ALL {
            assert!(k.charges(&t) > 0 && k.recharge_time(&t) > 0.0, "{k:?}");
        }
        assert_eq!(Knack::Thorn.charges(&t), t.thorn.charges);
    }
}
