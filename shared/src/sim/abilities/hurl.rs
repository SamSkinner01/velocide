//! Hurl *(twist)* (Melee; `docs/rework.md` §1): the thrown blade / hammer-head
//! (`ProjectileKind::Hurl`, thrown by `melee.rs`) now comes back. It flies out up to
//! `hurl.max_distance` (or until it hits the level), then returns to the thrower's chest
//! (`projectile_flags::GUIDED | RETURNING`), hitting each hostile at most once per leg:
//! `hurl.damage_scale` × the carrier's swing on the way out, `return_damage_scale` × on the way
//! back. The Trimmer's blade passes through targets; the Tamper's head stops at the first one
//! and comes back from there. Caught (it reaches you): `hurl.catch_refund` of the cooldown
//! back (`Fx::HurlCaught`).

use super::super::projectiles::Projectile;
use super::super::{Hitbox, PlayerId, SimWorld};
use crate::tuning::Tuning;

/// Thrown Hurls (their legs live on the projectiles).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hurls {}

impl Hurls {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// Every tick of a thrown Hurl, before it moves (`false`: it is gone).
    pub(crate) fn hurl_tick(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _boxes: &[Hitbox],
    ) -> bool {
        true
    }

    /// Per tick.
    pub(crate) fn step_hurl(&mut self, _t: &Tuning) {}
}
