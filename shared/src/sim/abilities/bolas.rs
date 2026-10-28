//! Bolas (Scatter, control; `docs/rework.md` §1): two wax-weighted balls on a wick cord
//! (`ProjectileKind::Bolas`: `bolas.speed`, gravity `bolas.gravity`, passes nothing). The
//! first hostile it touches is bound (Pinned: the Pinning Arrow's times); a second hostile
//! within `bolas.link_range` of the first is bound too, and for `bolas.link_time`
//! `bolas.link_share` of the damage either takes is also dealt to the other (never chained
//! further, never to the source itself). `BOUND` flags, `Fx::BolasBound`.
//!
//! Groundwork stub (arsenal-a M1): the throw flies; the bind is arsenal-b's.

use engine::physics::SweepHit;
use glam::Vec3;

use super::super::combat::Owner;
use super::super::projectiles::{Projectile, ProjectileKind};
use super::super::{DamageEvent, HitPart, HitTarget, PlayerId, SimWorld};
use super::AbilityCast;
use crate::tuning::Tuning;

/// Cords binding hostiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BolasWorld {}

impl BolasWorld {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// A Bolas throw.
    pub(crate) fn cast_bolas(&mut self, c: &AbilityCast) {
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&c.t.movement);
        let aim = c.input.look_dir();
        let start = self.safe_launch_point(eye, eye + aim * 0.5);
        let b = &c.t.combat.bolas;
        self.spawn_projectile(
            c.t,
            ProjectileKind::Bolas,
            Owner::Player(c.id),
            start,
            aim * b.speed,
        );
        self.push_fired(c.id, c.ability.attack(), eye);
    }

    /// The Bolas touched `target` (`Some(keep)`: handled; `None`: the generic rule).
    pub(crate) fn bolas_hit_target(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _target: HitTarget,
        _part: HitPart,
        _dir: Vec3,
    ) -> Option<bool> {
        None
    }

    /// The Bolas hit the level.
    pub(crate) fn bolas_hit_world(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _hit: SweepHit,
    ) -> Option<bool> {
        None
    }

    /// Is `target` bound by a Bolas cord?
    pub fn is_bound(&self, _target: HitTarget) -> bool {
        false
    }

    /// After every damage instance: a bound target shares it with its partner.
    pub(crate) fn bolas_after_damage(&mut self, _t: &Tuning, _event: &DamageEvent) {}

    /// Per tick.
    pub(crate) fn step_bolas(&mut self, _t: &Tuning) {}
}
