//! Waxwork (Precision, defend; `docs/rework.md` §1): a wax double of its owner where they
//! stood, posed as they were, for `waxwork.duration` with `waxwork.health`. Moulds within
//! `waxwork.lure_range` that could see it prefer it to its owner (`select_target`); PvP
//! opponents see a still player (`kind::WAXWORK`). Destroyed or at its end it bursts in hot
//! wax. One at a time; the cooldown starts at its end (`WeaponState::live`).

use super::super::{DamageEvent, DamageInfo, PlayerId, SimWorld};
use super::AbilityCast;
use crate::protocol::{NetFrame, NetId};
use crate::tuning::Tuning;

/// Standing Waxworks, one per owner at most.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Waxworks {}

impl Waxworks {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}

    pub(crate) fn len(&self) -> usize {
        0
    }

    pub(crate) fn holds(&self, _id: NetId) -> bool {
        false
    }
}

impl SimWorld {
    /// A Waxwork cast.
    pub(crate) fn cast_waxwork(&mut self, _c: &AbilityCast) {}

    /// Per tick.
    pub(crate) fn step_waxwork(&mut self, _t: &Tuning) {}

    /// Damage aimed at `id` that is really aimed at a Waxwork's double (`apply_damage` asks
    /// first): `Some` = it was the double's, here is what happened.
    pub(crate) fn damage_waxwork(
        &mut self,
        _t: &Tuning,
        _id: PlayerId,
        _info: DamageInfo,
    ) -> Option<DamageEvent> {
        None
    }

    /// Is enemy `id` lured by a Waxwork (its target is the double)?
    pub fn is_lured(&self, _id: NetId) -> bool {
        false
    }

    /// The doubles as entities.
    pub(crate) fn replicate_waxworks(&self, _t: &Tuning, _frame: &mut NetFrame) {}
}
