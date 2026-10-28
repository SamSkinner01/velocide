//! Wickline (Precision, move; `docs/rework.md` §1): a burning wick-thread, hitscan to
//! `wickline.range`, lag-compensated. Anchored in the level it zips you towards the anchor; a
//! light hostile is yanked to you (and takes `wickline.damage`), a heavy one yanks you to it.
//! A miss serves only part of the cooldown. The line is `Fx::Wickline`.

use super::super::{PlayerId, SimWorld};
use super::AbilityCast;
use crate::tuning::Tuning;

/// Wickline state between ticks (none yet: the line is an effect).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wicklines {}

impl Wicklines {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// A Wickline cast.
    pub(crate) fn cast_wickline(&mut self, _c: &AbilityCast) {}

    /// Per tick.
    pub(crate) fn step_wickline(&mut self, _t: &Tuning) {}
}
