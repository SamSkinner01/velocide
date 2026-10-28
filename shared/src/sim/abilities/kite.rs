//! Kite (Explosive, damage; `docs/rework.md` §1): an ember kite (`ProjectileKind::Kite`,
//! `kite.speed`, no gravity) its owner steers with the crosshair while ALT is held: it turns
//! towards the point under the crosshair at up to `kite.turn_degrees` per second (at most
//! `kite.steer_time`), then flies straight; it explodes on contact (`kite.explosion_radius`,
//! `damage`, `knockback`). Movement and the primary are free while steering.
//! `projectile_flags::GUIDED` while it turns (re-launched every tick it does).
//!
//! Groundwork stub (arsenal-a M1): the kite flies straight and explodes; the steering is
//! arsenal-b's (`kite_tick`, reading the owner's `last_input`).

use super::super::combat::Owner;
use super::super::projectiles::{Projectile, ProjectileKind};
use super::super::{Hitbox, PlayerId, SimWorld};
use super::AbilityCast;
use crate::tuning::Tuning;

/// Kites being steered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kites {}

impl Kites {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// A Kite launch.
    pub(crate) fn cast_kite(&mut self, c: &AbilityCast) {
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&c.t.movement);
        let aim = c.input.look_dir();
        let start = self.safe_launch_point(eye, eye + aim * 0.6);
        let speed = c.t.combat.kite.speed;
        self.spawn_projectile(
            c.t,
            ProjectileKind::Kite,
            Owner::Player(c.id),
            start,
            aim * speed,
        );
        self.push_fired(c.id, c.ability.attack(), eye);
    }

    /// Every tick of a kite in flight, before it moves (`false`: it is gone).
    pub(crate) fn kite_tick(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _boxes: &[Hitbox],
    ) -> bool {
        true
    }

    /// Per tick.
    pub(crate) fn step_kite(&mut self, _t: &Tuning) {}
}
