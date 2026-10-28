//! Lantern (Rapid, set up; `docs/rework.md` §1): tossed (`lantern.toss_speed`,
//! `ProjectileKind::LanternToss`), it settles on the floor (never over a pit) as a
//! `kind::LANTERN` that for `lantern.duration` fires the carrier's primary at the nearest
//! hostile within `lantern.range` in line of sight, at `rate_scale` × its rate and
//! `damage_scale` × its damage (Arclash: chaining zaps, Riveter: rivets, Kiln: a `kiln_range`
//! cone). Moulds may attack it (`lantern.health`). One at a time; a new one replaces the old.
//! Its shots are `Fx::LanternShot`.
//!
//! Groundwork stub (arsenal-a M1): the toss flies and vanishes; the lantern is arsenal-b's.

use engine::physics::SweepHit;

use super::super::combat::Owner;
use super::super::projectiles::{Projectile, ProjectileKind};
use super::super::{PlayerId, SimWorld};
use super::AbilityCast;
use crate::protocol::{NetFrame, NetId};
use crate::tuning::Tuning;

/// Lanterns standing on the floor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lanterns {}

impl Lanterns {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}

    pub(crate) fn len(&self) -> usize {
        0
    }

    pub(crate) fn holds(&self, _id: NetId) -> bool {
        false
    }
}

impl SimWorld {
    /// A Lantern toss.
    pub(crate) fn cast_lantern(&mut self, c: &AbilityCast) {
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&c.t.movement);
        let aim = c.input.look_dir();
        let start = self.safe_launch_point(eye, eye + aim * 0.5);
        let speed = c.t.combat.lantern.toss_speed;
        let spawned = self.spawn_projectile(
            c.t,
            ProjectileKind::LanternToss,
            Owner::Player(c.id),
            start,
            aim * speed,
        );
        // The toss carries the carrier (what the lantern will shoot) as its `state`.
        if let Some(proj) = spawned.and_then(|id| self.projectiles.get_mut(&id)) {
            proj.bounces_left = c.weapon.id() as u32;
        }
        self.push_fired(c.id, c.ability.attack(), eye);
    }

    /// The toss hit the level (`Some(keep)`: handled; `None`: the generic rule).
    pub(crate) fn lantern_hit_world(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _hit: SweepHit,
    ) -> Option<bool> {
        None
    }

    /// The lanterns as entities.
    pub(crate) fn replicate_lanterns(&self, _t: &Tuning, _frame: &mut NetFrame) {}

    /// Per tick.
    pub(crate) fn step_lantern(&mut self, _t: &Tuning) {}
}
