//! Beacon Arrow (Draw, set up; `docs/rework.md` §1): an arrow with the draw
//! (`ProjectileKind::BeaconArrow`, its own hit `beacon_arrow.damage_scale` × a plain arrow's)
//! that lights a beacon where it sticks — in the level, or in a target, riding along — for
//! `beacon_arrow.duration`: every hostile (to the shooter) within `beacon_arrow.radius` of it
//! is **revealed** (`REVEALED` entity flags: the shooter and their teammates draw it as a
//! silhouette through walls; the client knows the lit beacons, they are the `STUCK`
//! `BEACON_ARROW` entities) and takes `damage_multiplier` × the damage from every player
//! (`apply_damage`, `DamageFlags::REVEALED`). An arrow that flies out without sticking lights
//! nothing.

use std::collections::BTreeMap;

use engine::physics::SweepHit;
use glam::Vec3;

use super::super::combat::{Owner, secs_to_ticks};
use super::super::projectiles::{Projectile, Stuck};
use super::super::{HitPart, HitTarget, PlayerId, ProjectileKind, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// A lit beacon (its arrow is the projectile, stuck).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beacon {
    pub owner: PlayerId,
    pub radius: f32,
}

/// Lit beacons, by their arrow's id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beacons {
    pub lit: BTreeMap<NetId, Beacon>,
}

impl Beacons {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.lit.retain(|_, b| b.owner != id);
    }
}

impl SimWorld {
    /// The release looses the arrow.
    pub(crate) fn cast_beacon_arrow(&mut self, c: &AbilityCast) {
        if c.phase == AbilityPhase::Use {
            let scale = c.t.combat.beacon_arrow.damage_scale;
            self.loose_ability_arrow(c, ProjectileKind::BeaconArrow, scale);
        }
    }

    /// Is `target` revealed: within a lit beacon's radius, hostile to its owner?
    pub fn is_revealed(&self, target: HitTarget) -> bool {
        if self.abilities.beacon.lit.is_empty() {
            return false;
        }
        let Some(at) = self.target_center(target) else {
            return false;
        };
        self.abilities.beacon.lit.iter().any(|(id, b)| {
            self.projectiles.get(id).is_some_and(|p| {
                p.position.distance(at) <= b.radius
                    && self.is_hostile(Owner::Player(b.owner), target)
                    && target != HitTarget::Player(b.owner)
            })
        })
    }

    /// Lights the beacon on arrow `p` (stuck now) for `beacon_arrow.duration`.
    fn light_beacon(&mut self, t: &Tuning, p: &mut Projectile, stuck: Stuck) {
        let Some(owner) = p.owner.player() else {
            return;
        };
        let b = &t.combat.beacon_arrow;
        let now = self.tick;
        p.stuck = Some(stuck);
        p.velocity = Vec3::ZERO;
        p.expire_tick = now + secs_to_ticks(b.duration, self.tick_rate, 1) as u32;
        p.relaunch(now);
        self.abilities.beacon.lit.insert(
            p.id,
            Beacon {
                owner,
                radius: b.radius,
            },
        );
    }

    /// The arrow struck `target`: its hit, then it sticks in it and lights (riding along).
    pub(crate) fn beacon_arrow_hit_target(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        target: HitTarget,
        part: HitPart,
        dir: Vec3,
    ) -> Option<bool> {
        self.projectile_direct_damage(t, p, target, part, dir);
        // It rides along on what it struck (where it struck: the level if that died).
        let stuck = match self.target_origin(target) {
            Some(origin) => Stuck {
                to: Some(target),
                offset: p.position - origin,
            },
            None => Stuck {
                to: None,
                offset: Vec3::ZERO,
            },
        };
        self.light_beacon(t, p, stuck);
        Some(true)
    }

    /// The arrow hit the level: it sticks there and lights.
    pub(crate) fn beacon_arrow_hit_world(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        _hit: SweepHit,
    ) -> Option<bool> {
        if p.parried {
            return None;
        }
        let stuck = Stuck {
            to: None,
            offset: Vec3::ZERO,
        };
        self.light_beacon(t, p, stuck);
        Some(true)
    }

    /// Per tick: a beacon whose arrow is gone (burnt out) goes out.
    pub(crate) fn step_beacon_arrow(&mut self, _t: &Tuning) {
        let projectiles = &self.projectiles;
        self.abilities
            .beacon
            .lit
            .retain(|id, _| projectiles.get(id).is_some_and(|p| p.stuck.is_some()));
    }
}
