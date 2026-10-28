//! Blink Arrow (Draw, move; `docs/rework.md` §1): armed like the other arrows, the next
//! release looses it with the draw (`ProjectileKind::BlinkArrow`, its own hit
//! `blink_arrow.damage_scale` × a plain arrow's). ALT while it flies, or within
//! `blink_arrow.window` after it sticks (`AbilityPhase::Recast`), steps you to it: just in
//! front of the surface (or the target it struck), at the nearest spot your body fits
//! (searched back along its path), your speed set to `exit_speed` along its flight. Server-side
//! like the Lunge's impulse (the owner reconciles; `Fx::Blink`). The cooldown starts at the
//! blink or when the window passes (`WeaponState::live`).

use engine::physics::SweepHit;
use glam::Vec3;

use super::super::projectiles::Projectile;
use super::super::{HitPart, HitTarget, PlayerId, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::tuning::Tuning;

/// Blink arrows out, per owner.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlinkArrows {}

impl BlinkArrows {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The release looses the arrow; the recast steps to it.
    pub(crate) fn cast_blink_arrow(&mut self, c: &AbilityCast) {
        if c.phase == AbilityPhase::Use {
            let scale = c.t.combat.blink_arrow.damage_scale;
            self.loose_ability_arrow(c, super::super::ProjectileKind::BlinkArrow, scale);
        }
    }

    /// The arrow struck `target` (`Some(keep)`: handled; `None`: the generic arrow hit).
    pub(crate) fn blink_arrow_hit_target(
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
    pub(crate) fn blink_arrow_hit_world(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _hit: SweepHit,
    ) -> Option<bool> {
        None
    }

    /// Per tick.
    pub(crate) fn step_blink_arrow(&mut self, _t: &Tuning) {}
}
