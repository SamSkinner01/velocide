//! Beacon Arrow (Draw, set up; `docs/rework.md` §1): an arrow with the draw
//! (`ProjectileKind::BeaconArrow`, its own hit `beacon_arrow.damage_scale` × a plain arrow's)
//! that lights a beacon where it sticks (in a target it rides along) for
//! `beacon_arrow.duration`: every hostile within `beacon_arrow.radius` of it is **revealed**
//! to the shooter and their teammates (`REVEALED` flags: a silhouette through walls) and takes
//! `damage_multiplier` × the damage from every player (`apply_damage`, `DamageFlags::REVEALED`).

use engine::physics::SweepHit;
use glam::Vec3;

use super::super::projectiles::Projectile;
use super::super::{HitPart, HitTarget, PlayerId, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::tuning::Tuning;

/// Lit beacons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beacons {}

impl Beacons {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The release looses the arrow.
    pub(crate) fn cast_beacon_arrow(&mut self, c: &AbilityCast) {
        if c.phase == AbilityPhase::Use {
            let scale = c.t.combat.beacon_arrow.damage_scale;
            self.loose_ability_arrow(c, super::super::ProjectileKind::BeaconArrow, scale);
        }
    }

    /// Is `target` revealed by a lit beacon?
    pub fn is_revealed(&self, _target: HitTarget) -> bool {
        false
    }

    /// The arrow struck `target` (`Some(keep)`: handled; `None`: the generic arrow hit).
    pub(crate) fn beacon_arrow_hit_target(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _target: HitTarget,
        _part: HitPart,
        _dir: Vec3,
    ) -> Option<bool> {
        None
    }

    /// The arrow hit the level.
    pub(crate) fn beacon_arrow_hit_world(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _hit: SweepHit,
    ) -> Option<bool> {
        None
    }

    /// Per tick.
    pub(crate) fn step_beacon_arrow(&mut self, _t: &Tuning) {}
}
