//! Stoke (Rapid, damage; `docs/rework.md` §1): for `stoke.duration` the weapon's primary fires
//! at `stoke.rate` × its rate and every hit sets `stoke.burn_stacks` Burn stacks (the Kiln,
//! which already burns: `stoke.kiln_reach` × its cone's reach instead); then it overheats for
//! `stoke.overheat` (no primary; the other ability, the fist and the knack still work).
//!
//! The rate and the overheat are predicted: `WeaponState::stoke` / `overheat` per slot,
//! `step_weapons` (`player_flags::STOKED` / `OVERHEATED`). The server side is the Burn on hit.
//!
//! Groundwork stub (arsenal-a M1): the use and the timers work (weapons.rs); the Burn on hit
//! and the Kiln's reach are arsenal-b's.

use super::super::{DamageEvent, PlayerId, SimWorld};
use super::AbilityCast;
use crate::tuning::Tuning;

/// Server-side Stoke state (none yet: the timers are the weapon state's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stokes {}

impl Stokes {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// A Stoke use (`step_weapons` already stoked the weapon).
    pub(crate) fn cast_stoke(&mut self, _c: &AbilityCast) {}

    /// After every damage instance: a stoked weapon's hit sets Burn.
    pub(crate) fn stoke_after_damage(&mut self, _t: &Tuning, _event: &DamageEvent) {}

    /// Per tick.
    pub(crate) fn step_stoke(&mut self, _t: &Tuning) {}
}
