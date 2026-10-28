//! Tally (Precision, set up; `docs/rework.md` §1): hold ALT for up to `tally.hold_time`;
//! every hostile the crosshair passes over (within `tally.range`, line of sight,
//! lag-compensated, `tally.assist_degrees` of assist) is tallied, up to `tally.max_targets`.
//! The release shoots each tallied target still in sight, at the part the crosshair passed
//! over. The server keeps the tally; the owner sees it as `PlayerCombat::tallied`.

use super::super::{HitboxHistory, PlayerId, SimWorld};
use super::AbilityCast;
use crate::input::PlayerInput;
use crate::tuning::Tuning;

/// Most marks a tally holds (the owner's `PlayerCombat::tallied`).
pub const MAX_TALLY: usize = 4;

/// Tallies being held, per player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tallies {}

impl Tallies {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The press (starts a tally) and the release (fires it).
    pub(crate) fn cast_tally(&mut self, _c: &AbilityCast) {}

    /// Every input of a player holding a Tally: what the crosshair passes over is tallied.
    pub(crate) fn tally_sweep(
        &mut self,
        _t: &Tuning,
        _history: Option<&dyn HitboxHistory>,
        _id: PlayerId,
        _input: &PlayerInput,
    ) {
    }

    /// Per tick.
    pub(crate) fn step_tally(&mut self, _t: &Tuning) {}
}
