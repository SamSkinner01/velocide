//! Hurl *(twist)* (Melee; `docs/rework.md` §1): the thrown blade / hammer-head
//! (`ProjectileKind::Hurl`, thrown by `melee.rs`) now comes back.
//!
//! * **Out**: it flies (gravity `hurl.gravity`) up to `hurl.max_distance` from where it left
//!   the hand, or until it hits the level, then turns back. The Trimmer's blade passes through
//!   what it hits (each hostile once), the Tamper's head turns back at the first one.
//! * **Back**: it homes on the thrower's chest at `hurl.return_speed`
//!   (`projectile_flags::GUIDED | RETURNING`: no gravity, re-launched every tick), hitting
//!   each hostile it passes once more for `return_damage_scale` / `damage_scale` of its throw;
//!   the level stops it (it is gone). Reaching the thrower it is **caught**:
//!   `hurl.catch_refund` of the cooldown back (server-side: their client re-bases) and
//!   `SimEvent::HurlCaught` → `Fx::HurlCaught`.
//!
//! The Hurl tests targets itself every tick ([`SimWorld::hurl_tick`], the projectile's own
//! sweep passes them: `ProjectileKind::hits_targets`), on the body like every melee hit (no
//! weak points), against the current hit shapes like every projectile.

use std::collections::BTreeMap;

use glam::Vec3;

use super::super::combat::{DamageFlags, DamageInfo, secs_to_ticks};
use super::super::projectiles::Projectile;
use super::super::{HitPart, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::loadout::{Ability, WeaponKind};
use crate::protocol::{NetId, projectile_flags};
use crate::tuning::Tuning;

/// Within this of the thrower's chest (m) it is caught.
const CATCH: f32 = 1.0;

/// A thrown Hurl's leg.
#[derive(Clone, Debug, PartialEq)]
pub struct Thrown {
    pub owner: PlayerId,
    /// Where it left the hand.
    pub from: Vec3,
    pub returning: bool,
    /// Targets struck on this leg.
    pub struck: Vec<HitTarget>,
}

/// Thrown Hurls, by their projectile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hurls {
    pub thrown: BTreeMap<NetId, Thrown>,
}

impl Hurls {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.thrown.retain(|_, h| h.owner != id);
    }
}

impl SimWorld {
    /// `melee.rs` threw Hurl `id` for player `owner` from `from`.
    pub(crate) fn hurl_thrown(&mut self, id: NetId, owner: PlayerId, from: Vec3) {
        self.abilities.hurl.thrown.insert(
            id,
            Thrown {
                owner,
                from,
                returning: false,
                struck: Vec::new(),
            },
        );
    }

    /// Every tick of a thrown Hurl, before it moves: it turns back at its range, homes on its
    /// thrower on the way back (caught there), and strikes what this tick's motion passes.
    /// `false`: it is gone.
    pub(crate) fn hurl_tick(&mut self, t: &Tuning, p: &mut Projectile, boxes: &[Hitbox]) -> bool {
        let h = &t.combat.hurl;
        let Some(leg) = self.abilities.hurl.thrown.get(&p.id).cloned() else {
            return true;
        };
        let Some(owner) = self.players.get(&leg.owner).filter(|o| !o.combat.dead) else {
            self.abilities.hurl.thrown.remove(&p.id);
            return false;
        };
        let (chest, _, _) = owner.movement.capsule(&t.movement);
        let chest = chest + Vec3::Y * 0.3;
        if !leg.returning && p.position.distance(leg.from) >= h.max_distance {
            self.turn_hurl(t, p);
        }
        let returning = self
            .abilities
            .hurl
            .thrown
            .get(&p.id)
            .is_some_and(|l| l.returning);
        if returning {
            let to = chest - p.position;
            if to.length() <= CATCH {
                self.catch_hurl(t, p);
                return false;
            }
            p.velocity = to.normalize_or(Vec3::Y) * h.return_speed;
            p.relaunch(self.launch_tick_now());
        }
        // What this tick's motion passes (the projectile's own sweep doesn't test targets).
        let dt = self.dt();
        let motion = p.velocity * dt - Vec3::Y * (0.5 * p.gravity * dt * dt);
        let pad = p.radius + super::super::projectiles::projectile_target_pad(&t.combat, p.owner);
        let struck = self
            .abilities
            .hurl
            .thrown
            .get(&p.id)
            .map(|l| l.struck.clone())
            .unwrap_or_default();
        let mut hits: Vec<(f32, HitTarget)> = boxes
            .iter()
            .filter(|b| self.can_hit(p.owner, b.target) && !struck.contains(&b.target))
            .filter_map(|b| {
                let pads = self.hit_pads(t, p.owner, b.target);
                b.sweep_sphere_padded(p.position, motion, pad, pads)
                    .map(|(f, _)| (f, b.target))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let head = WeaponKind::from_id(p.bounces_left as u8) == Some(WeaponKind::Tamper);
        let dir = motion.normalize_or(p.velocity.normalize_or(Vec3::NEG_Z));
        for (f, target) in hits {
            let at = p.position + motion * f;
            let scale = if returning {
                h.return_damage_scale / h.damage_scale.max(1e-3)
            } else {
                1.0
            };
            let mut flags = DamageFlags::PROJECTILE;
            // Turned at a thorn on its way out (thorn.rs): × the multiplier, a ricochet hit.
            let mut damage = p.damage * scale;
            if p.ricochets > 0 {
                flags.insert(DamageFlags::RICOCHET);
                damage *= t.combat.thorn.damage_multiplier.powi(p.ricochets as i32);
            }
            let mut info = DamageInfo::new(damage, p.owner, p.attack)
                .at(at, dir)
                .part(HitPart::Body)
                .knockback(dir * h.knockback)
                .flags(flags);
            info.projectile = Some(p.id);
            info.ricochets = p.ricochets;
            self.apply_damage(t, target, info);
            if let Some(l) = self.abilities.hurl.thrown.get_mut(&p.id) {
                l.struck.push(target);
            }
            // The hammer-head turns back at the first one (from there, past it).
            if head && !returning {
                p.position = at;
                self.turn_hurl(t, p);
                if let Some(l) = self.abilities.hurl.thrown.get_mut(&p.id) {
                    l.struck.push(target);
                }
                break;
            }
        }
        true
    }

    /// The outward leg is over: it comes back now (each hostile may be struck once more).
    fn turn_hurl(&mut self, t: &Tuning, p: &mut Projectile) {
        let Some(l) = self.abilities.hurl.thrown.get_mut(&p.id) else {
            return;
        };
        l.returning = true;
        l.struck.clear();
        p.gravity = 0.0;
        p.ability_flags |= projectile_flags::GUIDED | projectile_flags::RETURNING;
        // A fresh flight for the way back.
        let back = secs_to_ticks(t.combat.hurl.flight_time, self.tick_rate, 1) as u32;
        p.expire_tick = p.expire_tick.max(self.tick + back);
        p.velocity = -p.velocity.normalize_or(Vec3::Z) * t.combat.hurl.return_speed;
        p.relaunch(self.launch_tick_now());
    }

    /// It reached its thrower: part of the cooldown back.
    fn catch_hurl(&mut self, t: &Tuning, p: &Projectile) {
        let Some(l) = self.abilities.hurl.thrown.remove(&p.id) else {
            return;
        };
        let weapon = WeaponKind::from_id(p.bounces_left as u8);
        let rate = self.tick_rate;
        if let Some(owner) = self.players.get_mut(&l.owner) {
            let w = &mut owner.combat.weapons;
            if let Some(slot) = weapon.and_then(|weapon| w.loadout.slot_of(weapon)) {
                let refund = secs_to_ticks(
                    Ability::Hurl.cooldown(&t.combat) * t.combat.hurl.catch_refund,
                    rate,
                    0,
                );
                let cd = &mut w.alt_cooldown[slot as usize];
                *cd = cd.saturating_sub(refund);
            }
        }
        self.events.push(SimEvent::HurlCaught { player: l.owner });
    }

    /// The level stopped it: on the way out it turns back; on the way back it is gone.
    pub(crate) fn hurl_hit_world(&mut self, t: &Tuning, p: &mut Projectile) -> Option<bool> {
        let returning = self.abilities.hurl.thrown.get(&p.id)?.returning;
        if returning {
            self.abilities.hurl.thrown.remove(&p.id);
            return None;
        }
        self.turn_hurl(t, p);
        Some(true)
    }

    /// Per tick: legs whose projectile is gone are forgotten.
    pub(crate) fn step_hurl(&mut self, _t: &Tuning) {
        let projectiles = &self.projectiles;
        self.abilities
            .hurl
            .thrown
            .retain(|id, _| projectiles.contains_key(id));
    }
}
