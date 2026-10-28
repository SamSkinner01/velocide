//! Kite (Explosive, damage; `docs/rework.md` §1): an ember kite (`ProjectileKind::Kite`,
//! `kite.speed`, no gravity) its owner steers with the crosshair while ALT is held: it turns
//! towards the point under the crosshair at up to `kite.turn_degrees` per second (at most
//! `kite.steer_time`), then flies straight; it explodes on contact (`kite.explosion_radius`,
//! `damage`, `knockback`) or bursts where it is after `kite.flight_time`. Movement and the
//! primary are free while steering.
//!
//! Steering is server-side ([`SimWorld::kite_tick`], before the kite moves each tick): it
//! reads the owner's latest input (`SimPlayer::last_input`: `ALT_FIRE` held with the Kite the
//! selected ability, and the view), finds the point under the crosshair (the first hostile or
//! the level along the look, else `hitscan_range` out) and turns the kite's velocity towards it,
//! speed kept. Let go (or switched away, dead, the steer time up, relayed off a thorn) and it
//! flies straight for good. `projectile_flags::GUIDED` while it is steered; every tick it turns
//! the trajectory is re-launched (an entity update), so others see the curve.

use std::collections::BTreeMap;

use engine::physics::MASK_SHOTS;
use glam::{Quat, Vec3};

use super::super::combat::{Owner, secs_to_ticks};
use super::super::projectiles::{Projectile, ProjectileKind};
use super::super::{Hitbox, PlayerId, SimWorld};
use super::AbilityCast;
use crate::input::{Buttons, look_dir};
use crate::loadout::Ability;
use crate::protocol::{NetId, projectile_flags};
use crate::tuning::Tuning;

/// Kites being steered: the tick each one's steering runs out (by projectile id).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kites {
    pub steered: BTreeMap<NetId, KiteSteer>,
}

/// A kite under its owner's crosshair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KiteSteer {
    pub owner: PlayerId,
    /// It flies straight from this tick on.
    pub until: u32,
}

impl Kites {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.steered.retain(|_, k| k.owner != id);
    }
}

/// `from` turned towards `to` (both unit) by at most `max` radians.
pub(crate) fn turn_towards(from: Vec3, to: Vec3, max: f32) -> Vec3 {
    let angle = from.angle_between(to);
    if !angle.is_finite() || angle <= max {
        return to;
    }
    let mut axis = from.cross(to);
    if axis.length_squared() < 1e-10 {
        // Straight behind: turn about any axis across the flight.
        axis = from.cross(Vec3::Y);
        if axis.length_squared() < 1e-10 {
            axis = from.cross(Vec3::X);
        }
    }
    (Quat::from_axis_angle(axis.normalize(), max) * from).normalize_or(from)
}

impl SimWorld {
    /// A Kite launch: along the aim, steered from the next tick while ALT stays down.
    pub(crate) fn cast_kite(&mut self, c: &AbilityCast) {
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&c.t.movement);
        let aim = c.input.look_dir();
        let start = self.safe_launch_point(eye, eye + aim * 0.6);
        let k = &c.t.combat.kite;
        let spawned = self.spawn_projectile(
            c.t,
            ProjectileKind::Kite,
            Owner::Player(c.id),
            start,
            aim * k.speed,
        );
        if let Some(id) = spawned {
            if let Some(proj) = self.projectiles.get_mut(&id) {
                proj.ability_flags |= projectile_flags::GUIDED;
            }
            let until = self.tick + secs_to_ticks(k.steer_time, self.tick_rate, 0) as u32;
            self.abilities
                .kite
                .steered
                .insert(id, KiteSteer { owner: c.id, until });
        }
        self.push_fired(c.id, c.ability.attack(), eye);
    }

    /// Where player `id` is steering a kite to: the first hostile or the level along their
    /// look (current shapes), else `hitscan_range` out. `None` when they aren't steering (ALT
    /// up, another ability selected, dead).
    fn kite_steer_point(&self, t: &Tuning, id: PlayerId, boxes: &[Hitbox]) -> Option<Vec3> {
        let p = self.players.get(&id).filter(|p| !p.combat.dead)?;
        let input = &p.last_input;
        let w = &p.combat.weapons;
        if !input.buttons.contains(Buttons::ALT_FIRE)
            || w.held().is_none()
            || w.ability() != Ability::Kite
        {
            return None;
        }
        let eye = p.movement.eye_position(&t.movement);
        let aim = look_dir(input.yaw, input.pitch);
        let range = t.combat.hitscan_range;
        let world_t = self
            .collision
            .world
            .cast_ray(eye, aim, range, MASK_SHOTS)
            .map_or(range, |h| h.distance);
        let owner = Owner::Player(id);
        let hit_t = self
            .first_target_on_ray(t, owner, boxes, eye, aim, world_t, None)
            .map_or(world_t, |h| h.0);
        Some(eye + aim * hit_t)
    }

    /// Every tick of a kite in flight, before it moves: steered towards its owner's crosshair
    /// while they hold it (see the module docs). Never removes it.
    pub(crate) fn kite_tick(&mut self, t: &Tuning, p: &mut Projectile, boxes: &[Hitbox]) -> bool {
        let Some(steer) = self.abilities.kite.steered.get(&p.id).copied() else {
            return true;
        };
        let point = (self.tick < steer.until && p.ricochets == 0)
            .then(|| self.kite_steer_point(t, steer.owner, boxes))
            .flatten();
        let Some(point) = point else {
            // Let go: straight on for good.
            self.abilities.kite.steered.remove(&p.id);
            p.ability_flags &= !projectile_flags::GUIDED;
            p.relaunch(self.launch_tick_now());
            return true;
        };
        let speed = p.velocity.length().max(1e-3);
        let along = p.velocity / speed;
        let want = (point - p.position).normalize_or(along);
        let max = t.combat.kite.turn_degrees.max(0.0).to_radians() * self.dt();
        let dir = turn_towards(along, want, max);
        if dir.angle_between(along) > 1e-4 {
            p.velocity = dir * speed;
            // The step moves it on this tick: launched one tick ago, from here.
            p.relaunch(self.launch_tick_now());
        }
        true
    }

    /// Per tick: steering of kites that are gone is forgotten.
    pub(crate) fn step_kite(&mut self, _t: &Tuning) {
        let live = &self.projectiles;
        self.abilities
            .kite
            .steered
            .retain(|id, _| live.contains_key(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_is_capped_and_lands_on_the_target_direction() {
        let a = Vec3::NEG_Z;
        let b = Vec3::X;
        let r = turn_towards(a, b, 10f32.to_radians());
        assert!((r.angle_between(a) - 10f32.to_radians()).abs() < 1e-4);
        assert!(r.dot(b) > 0.0, "towards it");
        assert_eq!(turn_towards(a, b, 2.0), b);
        // Straight behind: still a capped turn.
        let r = turn_towards(a, Vec3::Z, 0.1);
        assert!((r.angle_between(a) - 0.1).abs() < 1e-4);
    }
}
