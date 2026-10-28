//! Wisp *(twist)* (Rapid, set up; `docs/rework.md` §1): the wisp orbits its caster
//! (`arclash.wisp_orbit_radius` out at `wisp_orbit_height`, `wisp_orbit_rate` degrees a
//! second, for `wisp_orbit_time`), zapping the nearest hostile within `wisp_zap_range` (line of
//! sight) every `wisp_zap_interval` for `wisp_zap_damage`. ALT again while it orbits
//! (`AbilityPhase::Recast`) flings it along the aim at `wisp_speed`: it pops on the first
//! contact — a hostile or the level — (`wisp_pop_*`) or after `wisp_lifetime`. The cooldown
//! starts when it is gone (`WeaponState::live`; a fling that pops early starts it then: the
//! server re-bases the owner's weapon state). `projectile_flags::ORBITING` while it orbits.
//!
//! * **Orbiting** ([`SimWorld::wisp_tick`]): the wisp is placed on its circle round the
//!   owner's feet + `wisp_orbit_height` every tick (pulled in short of a wall), velocity zero;
//!   it zaps (`SimWorld::wisp_zap`), never touches anything and never relays off a thorn. It
//!   is gone, quietly, when the orbit time is up or its owner dies or leaves. Replicated
//!   `ORBITING` with `position` current and `aux` the tick the orbit started.
//! * **Flung**: a straight projectile (no gravity) that hits targets: the first hostile its
//!   sweep touches before the level is a direct hit of the pop ([`SimWorld::explode`] with the
//!   hostile as the direct target); the level and the end of its flight pop it where it is.

use std::collections::BTreeMap;

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::super::combat::{Owner, secs_to_ticks};
use super::super::projectiles::{Projectile, ProjectileKind, projectile_target_pad};
use super::super::{Hitbox, PlayerId, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::protocol::{NetId, projectile_flags};
use crate::tuning::Tuning;

/// A player's wisp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WispState {
    pub id: NetId,
    /// The loadout slot that cast it (its cooldown).
    pub slot: u8,
    /// The tick the orbit started ...
    pub start: u32,
    /// ... and the tick it ends (orbiting).
    pub orbit_until: u32,
    /// Flung (no longer orbiting).
    pub flung: bool,
}

/// Wisps of every player ([`super::AbilityWorld::wisp`]), one each at most.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wisps {
    pub by_owner: BTreeMap<PlayerId, WispState>,
}

impl Wisps {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.by_owner.remove(&id);
    }

    /// The owner of wisp projectile `id`.
    fn owner_of(&self, id: NetId) -> Option<(PlayerId, WispState)> {
        self.by_owner
            .iter()
            .find(|(_, w)| w.id == id)
            .map(|(owner, w)| (*owner, *w))
    }
}

impl SimWorld {
    /// The use (the wisp comes out, orbiting) and the recast (flung).
    pub(crate) fn cast_wisp(&mut self, c: &AbilityCast) {
        match c.phase {
            AbilityPhase::Use => self.wisp_orbit(c),
            AbilityPhase::Recast => self.wisp_fling(c),
            AbilityPhase::Release => {}
        }
    }

    /// Where player `id`'s orbiting wisp is at `tick` (started at `start`): on its circle,
    /// pulled in short of a wall.
    fn orbit_point(&self, t: &Tuning, id: PlayerId, start: u32, tick: u32) -> Option<Vec3> {
        let p = self.players.get(&id).filter(|p| !p.combat.dead)?;
        let a = &t.combat.arclash;
        let center = p.movement.position + Vec3::Y * a.wisp_orbit_height;
        let secs = tick.wrapping_sub(start) as f32 / self.tick_rate.max(1) as f32;
        let angle = (a.wisp_orbit_rate * secs).to_radians();
        let out = Vec3::new(angle.cos(), 0.0, angle.sin());
        let r = a.wisp_orbit_radius.max(0.0);
        let reach = self
            .collision
            .world
            .cast_ray(center, out, r + a.wisp_radius, MASK_SHOTS)
            .map_or(r, |h| (h.distance - a.wisp_radius).clamp(0.0, r));
        Some(center + out * reach)
    }

    /// The wisp comes out and orbits its caster (a wisp still out is replaced).
    fn wisp_orbit(&mut self, c: &AbilityCast) {
        if let Some(old) = self.abilities.wisp.by_owner.remove(&c.id) {
            self.projectiles.remove(&old.id);
        }
        let now = self.tick;
        let Some(at) = self.orbit_point(c.t, c.id, now, now) else {
            return;
        };
        let Some(slot) = self.players.get(&c.id).map(|p| p.combat.weapons.slot) else {
            return;
        };
        let owner = Owner::Player(c.id);
        let Some(id) = self.spawn_projectile(c.t, ProjectileKind::Wisp, owner, at, Vec3::ZERO)
        else {
            return;
        };
        let orbit = secs_to_ticks(c.t.combat.arclash.wisp_orbit_time, self.tick_rate, 1) as u32;
        if let Some(proj) = self.projectiles.get_mut(&id) {
            proj.ability_flags |= projectile_flags::ORBITING;
            // It ends with its orbit (`wisp_tick`), not by the generic expiry's pop.
            proj.expire_tick = now + orbit + 1;
            proj.launch_tick = now;
        }
        self.abilities.wisp.by_owner.insert(
            c.id,
            WispState {
                id,
                slot,
                start: now,
                orbit_until: now + orbit,
                flung: false,
            },
        );
        let eye = self.players[&c.id].movement.eye_position(&c.t.movement);
        self.push_fired(c.id, c.ability.attack(), eye);
    }

    /// ALT again: the orbiting wisp is flung along the aim.
    fn wisp_fling(&mut self, c: &AbilityCast) {
        let Some(w) = self.abilities.wisp.by_owner.get_mut(&c.id) else {
            return;
        };
        if w.flung {
            return;
        }
        w.flung = true;
        let id = w.id;
        let a = &c.t.combat.arclash;
        let aim = c.input.look_dir();
        let life = secs_to_ticks(a.wisp_lifetime, self.tick_rate, 1) as u32;
        let now = self.tick;
        let launch = self.launch_tick_now();
        let Some(proj) = self.projectiles.get_mut(&id) else {
            return;
        };
        proj.ability_flags &= !projectile_flags::ORBITING;
        proj.velocity = aim * a.wisp_speed;
        proj.speed = a.wisp_speed;
        proj.expire_tick = now + life;
        proj.relaunch(launch);
        let from = proj.position;
        self.push_fired(c.id, c.ability.attack(), from);
    }

    /// Every tick of a wisp in flight or orbit, before it moves (`false`: it is gone; see the
    /// module docs).
    pub(crate) fn wisp_tick(&mut self, t: &Tuning, p: &mut Projectile, boxes: &[Hitbox]) -> bool {
        let now = self.tick;
        let mine = self
            .abilities
            .wisp
            .owner_of(p.id)
            .filter(|(owner, _)| p.owner == Owner::Player(*owner));
        let Some((owner, w)) = mine else {
            // Deflected (a Gust turned it: someone else's now): it flies and pops like any
            // projectile.
            if let Some((owner, _)) = self.abilities.wisp.owner_of(p.id) {
                self.abilities.wisp.by_owner.remove(&owner);
                p.ability_flags &= !projectile_flags::ORBITING;
            }
            return true;
        };
        if !w.flung {
            let at = (now < w.orbit_until)
                .then(|| self.orbit_point(t, owner, w.start, now))
                .flatten();
            let Some(at) = at else {
                // The orbit is over (or its owner is down): gone without a pop.
                self.abilities.wisp.by_owner.remove(&owner);
                if let Some(pl) = self.players.get_mut(&owner) {
                    let rate = self.tick_rate;
                    pl.combat.weapons.end_live(w.slot, &t.combat, rate);
                }
                return false;
            };
            p.position = at;
            p.velocity = Vec3::ZERO;
            // `ORBITING`: `position` is current, `aux` the orbit's start.
            p.launch_position = at;
            p.launch_velocity = Vec3::ZERO;
            p.launch_tick = w.start;
            self.wisp_zap(t, p, boxes);
            return true;
        }
        // Flung: the first hostile it touches before the level is a direct pop.
        let dt = self.dt();
        let motion = p.velocity * dt;
        let world_f = self
            .collision
            .world
            .cast_sphere(p.position, p.radius, motion, MASK_SHOTS)
            .map_or(1.0, |h| h.fraction);
        let reach = p.radius + projectile_target_pad(&t.combat, p.owner);
        let first = boxes
            .iter()
            .filter(|h| self.can_hit(p.owner, h.target))
            .filter_map(|h| {
                let pads = self.hit_pads(t, p.owner, h.target);
                h.sweep_sphere_padded(p.position, motion, reach, pads)
                    .map(|(f, _)| (f, h.target))
            })
            .filter(|(f, _)| *f <= world_f)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((f, target)) = first {
            p.position += motion * f;
            self.explode_projectile(t, p, Some(target));
            self.wisp_gone(t, owner);
            return false;
        }
        true
    }

    /// A flung wisp of `owner` is gone (popped): its cooldown runs from now.
    fn wisp_gone(&mut self, t: &Tuning, owner: PlayerId) {
        let Some(w) = self.abilities.wisp.by_owner.remove(&owner) else {
            return;
        };
        let rate = self.tick_rate;
        if let Some(pl) = self.players.get_mut(&owner) {
            pl.combat
                .weapons
                .set_alt_cooldown(w.slot, t.combat.arclash.wisp_cooldown, rate);
        }
    }

    /// Per tick: a wisp that popped against the level or ran out (the projectile is gone) or
    /// whose owner died lets its cooldown start.
    pub(crate) fn step_wisp(&mut self, t: &Tuning) {
        let gone: Vec<(PlayerId, WispState)> = self
            .abilities
            .wisp
            .by_owner
            .iter()
            .filter(|(owner, w)| {
                !self.projectiles.contains_key(&w.id)
                    || self.players.get(owner).is_none_or(|p| p.combat.dead)
            })
            .map(|(owner, w)| (*owner, *w))
            .collect();
        for (owner, w) in gone {
            self.projectiles.remove(&w.id);
            if w.flung {
                self.wisp_gone(t, owner);
            } else {
                self.abilities.wisp.by_owner.remove(&owner);
                let rate = self.tick_rate;
                if let Some(pl) = self.players.get_mut(&owner) {
                    pl.combat.weapons.end_live(w.slot, &t.combat, rate);
                }
            }
        }
    }
}
