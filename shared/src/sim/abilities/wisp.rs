//! Wisp *(twist)* (Rapid, set up; `docs/rework.md` §1): the wisp orbits its caster
//! (`arclash.wisp_orbit_radius` out at `wisp_orbit_height`, for `wisp_orbit_time`), zapping the
//! nearest hostile within `wisp_zap_range` (line of sight) every `wisp_zap_interval`. ALT
//! again while it orbits (`AbilityPhase::Recast`) flings it along the aim at `wisp_speed`: it
//! pops on the first contact (`wisp_pop_*`) or after `wisp_lifetime`. The cooldown starts when
//! it is gone (`WeaponState::live`). `projectile_flags::ORBITING` while it orbits.
//!
//! Groundwork stub (arsenal-a M1): the use still throws the old drifting wisp; the orbit and
//! the fling are arsenal-b's.

use super::super::combat::Owner;
use super::super::projectiles::{Projectile, ProjectileKind};
use super::super::{Hitbox, PlayerId, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::tuning::Tuning;

/// Orbiting wisps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wisps {}

impl Wisps {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The use (the wisp comes out) and the recast (flung).
    pub(crate) fn cast_wisp(&mut self, c: &AbilityCast) {
        if c.phase != AbilityPhase::Use {
            return;
        }
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&c.t.movement);
        let aim = c.input.look_dir();
        let start = self.safe_launch_point(eye, eye + aim * 0.6);
        let speed = c.t.combat.arclash.wisp_speed;
        self.spawn_projectile(
            c.t,
            ProjectileKind::Wisp,
            Owner::Player(c.id),
            start,
            aim * speed,
        );
        self.push_fired(c.id, c.ability.attack(), eye);
    }

    /// Every tick of a wisp in flight or orbit, before it moves (`false`: it is gone).
    pub(crate) fn wisp_tick(&mut self, t: &Tuning, p: &mut Projectile, boxes: &[Hitbox]) -> bool {
        self.wisp_zap(t, p, boxes);
        true
    }

    /// Per tick.
    pub(crate) fn step_wisp(&mut self, _t: &Tuning) {}
}
