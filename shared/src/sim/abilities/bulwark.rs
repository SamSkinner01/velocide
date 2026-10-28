//! Bulwark (Scatter, defend; `docs/rework.md` §1): hold ALT to raise a slab of hardened wax
//! in front of you (`bulwark.width` × `height`, `distance` out, turning with the aim's yaw,
//! pitch clamped to `pitch_limit_degrees`), up to `bulwark.hold_time`. The shared guard
//! (`guard.rs`) absorbs hostile hits from its `cone_degrees` front until it has absorbed
//! `bulwark.absorb`, then it shatters. Your own shots pass; you move at `move_scale`. The
//! cooldown runs from the drop (`broken_cooldown` if it shattered).
//!
//! The guard (`guard.rs`) goes up on the press, down on the release, and shatters when it is
//! spent. The slab itself ([`Slab`], [`SimWorld::bulwark_slab`]) is a rectangle `distance` in
//! front of its holder's chest, facing the guard: it stops what reaches it from the front so
//! nothing goes on to what is behind it either —
//!
//! * a hostile **projectile** about to hit the holder from inside the guard's cone is
//!   destroyed at the slab (no blast, no parry delay; [`SimWorld::bulwark_meets_projectile`]):
//!   its damage (direct, or its blast's) is what the slab absorbs;
//! * a hostile **ray** (another player's, in PvP) crossing the slab from the front before it
//!   reaches anything else ends there ([`SimWorld::slab_on_ray`], asked by `hitscan.rs`), its
//!   damage absorbed.
//!
//! Both book the hit through `SimWorld::guard_absorbed` like any covered hit (the strength
//! wears, `Fx::GuardBlock`, a shattering past `absorb`). Blasts centred in front and blows are
//! the guard's ordinary cover (`guard_covers`).

use glam::Vec3;

use super::super::combat::{DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::super::projectiles::Projectile;
use super::super::{HitTarget, PlayerId, SimEvent, SimWorld};
use super::guard::Guard;
use super::{AbilityCast, AbilityPhase};
use crate::loadout::Ability;
use crate::tuning::Tuning;

/// Height of the slab's centre below the holder's eye (about the chest).
pub const SLAB_DROP: f32 = 0.4;

/// A raised Bulwark's slab: a rectangle facing `normal` (the guard's facing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slab {
    pub center: Vec3,
    /// Out of its front (unit).
    pub normal: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub half_width: f32,
    pub half_height: f32,
}

impl Slab {
    /// Where a ray from `origin` along `dir` (unit) crosses the slab from the front, within
    /// `max_t`: its distance along the ray.
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<f32> {
        let denom = dir.dot(self.normal);
        if denom >= -1e-6 {
            return None;
        }
        let d = (self.center - origin).dot(self.normal) / denom;
        if !(0.0..=max_t).contains(&d) {
            return None;
        }
        let off = origin + dir * d - self.center;
        (off.dot(self.right).abs() <= self.half_width && off.dot(self.up).abs() <= self.half_height)
            .then_some(d)
    }

    /// The point of the slab nearest `from` (where a projectile reaching it is stopped).
    pub fn stop_point(&self, from: Vec3) -> Vec3 {
        let off = from - self.center;
        let x = off.dot(self.right).clamp(-self.half_width, self.half_width);
        let y = off.dot(self.up).clamp(-self.half_height, self.half_height);
        self.center + self.right * x + self.up * y
    }
}

/// Slabs being held (their guard is `guard.rs`'s).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bulwarks {}

impl Bulwarks {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The press raises the slab, the release drops it.
    pub(crate) fn cast_bulwark(&mut self, c: &AbilityCast) {
        let b = &c.t.combat.bulwark;
        match c.phase {
            AbilityPhase::Use => {
                let Some(p) = self.players.get(&c.id) else {
                    return;
                };
                let until = self.tick + secs_to_ticks(b.hold_time, self.tick_rate, 1) as u32;
                let guard = Guard {
                    ability: c.ability,
                    weapon: c.weapon,
                    slot: p.combat.weapons.slot,
                    facing: c.input.look_dir(),
                    half_angle: (b.cone_degrees * 0.5).to_radians(),
                    absorb_left: b.absorb,
                    absorb: b.absorb,
                    until,
                    blocks: 0,
                    absorbed: 0.0,
                    pitch_limit: Some(b.pitch_limit_degrees.to_radians()),
                };
                self.raise_guard(c.id, guard, b.move_scale);
            }
            AbilityPhase::Release | AbilityPhase::Recast => {
                self.drop_guard(c.id);
            }
        }
    }

    /// The slab caught a hit (`guard.rs` booked it). Returns whether it shattered: spent, it
    /// drops and the longer cooldown starts (a server-side change of the owner's weapon state:
    /// its client re-bases).
    pub(crate) fn bulwark_blocked(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        guard: &Guard,
        _info: &DamageInfo,
    ) -> bool {
        if guard.absorb_left > 0.0 {
            return false;
        }
        self.drop_guard(id);
        let rate = self.tick_rate;
        if let Some(p) = self.players.get_mut(&id) {
            let w = &mut p.combat.weapons;
            let slot = guard.slot as usize;
            if w.slot == guard.slot {
                w.charge = 0;
            }
            if let Some(cd) = w.alt_cooldown.get_mut(slot) {
                *cd = secs_to_ticks(t.combat.bulwark.broken_cooldown, rate, 1);
            }
        }
        true
    }

    /// Player `id`'s raised Bulwark slab (see the module docs).
    pub fn bulwark_slab(&self, t: &Tuning, id: PlayerId) -> Option<Slab> {
        let g = self.guard(id)?;
        if g.ability != Ability::Bulwark || self.tick >= g.until || g.absorb_left <= 0.0 {
            return None;
        }
        let p = self.players.get(&id).filter(|p| !p.combat.dead)?;
        let normal = self.guard_facing(id)?;
        let b = &t.combat.bulwark;
        let chest = p.movement.eye_position(&t.movement) - Vec3::Y * SLAB_DROP;
        let right = normal.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(normal).normalize_or(Vec3::Y);
        Some(Slab {
            center: chest + normal * b.distance,
            normal,
            right,
            up,
            half_width: b.width * 0.5,
            half_height: b.height * 0.5,
        })
    }

    /// The first raised slab of a player hostile to `shooter` that a ray from `origin` along
    /// `dir` crosses from the front within `max_t`: (distance, holder). Your own shots pass
    /// through yours (and teammates' in co-op).
    pub(crate) fn slab_on_ray(
        &self,
        t: &Tuning,
        shooter: Owner,
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
    ) -> Option<(f32, PlayerId)> {
        if self.abilities.guards.raised.is_empty() {
            return None;
        }
        self.abilities
            .guards
            .raised
            .keys()
            .filter(|id| {
                shooter.as_target() != Some(HitTarget::Player(**id))
                    && self.is_hostile(shooter, HitTarget::Player(**id))
            })
            .filter_map(|id| {
                let slab = self.bulwark_slab(t, *id)?;
                Some((slab.ray_hit(origin, dir, max_t)?, *id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// A ray of `info` ended on player `id`'s slab: the slab absorbs its damage.
    pub(crate) fn slab_absorbs(&mut self, t: &Tuning, id: PlayerId, info: &DamageInfo) {
        self.guard_absorbed(t, id, info, info.amount.max(0.0));
    }

    /// A projectile is about to hit player `id` behind a raised slab (`guard.rs` asks): one
    /// coming from inside the guard's cone is destroyed at the slab, its damage (direct or its
    /// blast's) absorbed. `Some(false)`: gone; `None`: not the slab's (from the side or back).
    pub(crate) fn bulwark_meets_projectile(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        guard: &Guard,
        p: &mut Projectile,
        dir: Vec3,
    ) -> Option<bool> {
        let slab = self.bulwark_slab(t, id)?;
        let from = -dir.normalize_or(Vec3::NEG_Z);
        if p.owner == Owner::Player(id) || from.angle_between(slab.normal) > guard.half_angle {
            return None;
        }
        let at = slab.stop_point(p.position);
        let amount = p.explosion.map_or(p.damage, |e| e.damage.max(p.damage));
        let info = DamageInfo::new(amount, p.owner, p.attack)
            .at(at, dir)
            .flags(DamageFlags::PROJECTILE);
        p.position = at;
        self.events.push(SimEvent::ProjectileImpact {
            id: p.id,
            kind: p.kind,
            position: at,
            normal: slab.normal,
        });
        self.guard_absorbed(t, id, &info, amount);
        Some(false)
    }

    /// Per tick.
    pub(crate) fn step_bulwark(&mut self, _t: &Tuning) {}
}
