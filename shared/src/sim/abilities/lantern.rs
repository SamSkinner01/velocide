//! Lantern (Rapid, set up; `docs/rework.md` §1): tossed (`lantern.toss_speed`,
//! `ProjectileKind::LanternToss`), it settles on the floor (never over a pit) as a
//! `kind::LANTERN` that for `lantern.duration` fires the carrier's primary at the nearest
//! hostile within `lantern.range` in line of sight, at `rate_scale` × its rate and
//! `damage_scale` × its damage (Arclash: chaining zaps, Riveter: rivets, Kiln: a `kiln_range`
//! cone). It can be destroyed (`lantern.health`). One at a time; a new one replaces the old.
//! Its shots are `Fx::LanternShot`.
//!
//! * **Settling**: wherever the toss ends — on the level (it falls past bodies), at the end of
//!   its flight — the lantern stands on the floor below that point (`floor_below`, at most
//!   [`SETTLE_DEPTH`] down); with no floor there (a pit) it is lost.
//! * **Shooting** ([`SimWorld::step_lantern`]): from [`MUZZLE_HEIGHT`] above its foot, at the
//!   nearest hostile of its owner whose (padded) shape is within `range`, aimed at its centre
//!   (current shapes: the lantern is a server thing, nothing to compensate), as `Attack::Lantern`
//!   with the carrier's primary numbers (`ability_power` doesn't scale them: it is the
//!   weapon's own shot). With nothing in range it waits, ready. Every shot is a
//!   [`SimEvent::LanternShot`] (`ends`: the zap's end; the Kiln: the far end of its cone's
//!   axis; rivets: none, they are projectiles) and turns the lantern (`yaw` on the wire).
//! * **Damage**: hostile blasts (Moulds', PvP opponents') hurt it like a body at its centre
//!   ([`SimWorld::blast_lanterns`]); at 0 it is gone.

use std::collections::BTreeMap;

use engine::physics::{MASK_SHOTS, SweepHit};
use glam::Vec3;

use super::super::area::KilnCone;
use super::super::combat::{Attack, Owner, secs_to_ticks};
use super::super::hitscan::ShotSpec;
use super::super::projectiles::{Explosive, Projectile, ProjectileKind};
use super::super::weapons::{SpreadRng, spread_dir};
use super::super::{HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use super::AbilityCast;
use crate::loadout::WeaponKind;
use crate::protocol::{EntityState, NetFrame, NetId, kind, owner_code};
use crate::tuning::Tuning;

/// How far below where the toss ended a floor may be.
pub const SETTLE_DEPTH: f32 = 50.0;
/// Its shots leave this high above its foot.
pub const MUZZLE_HEIGHT: f32 = 0.5;
/// Salt of a lantern's rivet spread.
const LANTERN_SALT: u32 = 0x1A47_0000;

/// A lantern standing on the floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lantern {
    pub id: NetId,
    pub owner: PlayerId,
    /// The carrier: whose primary it fires.
    pub weapon: WeaponKind,
    /// Its foot, on the floor.
    pub position: Vec3,
    pub health: f32,
    /// It goes out at this tick.
    pub until: u32,
    /// Its next shot is due at this tick (it fires as soon as something is in range then).
    pub next_shot: u32,
    /// Where it last aimed.
    pub yaw: f32,
}

impl Lantern {
    /// Where its shots leave.
    pub fn muzzle(&self) -> Vec3 {
        self.position + Vec3::Y * MUZZLE_HEIGHT
    }

    /// Replicated state (`kind::LANTERN`): `owner`, `state` = the carrier's weapon id,
    /// `health`, `aux` = the tick it goes out, `yaw` = where it last aimed.
    pub fn entity_state(&self) -> EntityState {
        EntityState {
            id: self.id,
            kind: kind::LANTERN,
            owner: owner_code(Owner::Player(self.owner)),
            position: self.position,
            yaw: self.yaw,
            state: self.weapon.id() as u16,
            health: self.health.ceil().clamp(0.0, u16::MAX as f32) as u16,
            aux: self.until,
            ..Default::default()
        }
    }
}

/// Lanterns standing on the floor ([`super::AbilityWorld::lantern`]), one per owner at most.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lanterns {
    pub by_id: BTreeMap<NetId, Lantern>,
}

impl Lanterns {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.by_id.retain(|_, l| l.owner != id);
    }

    pub(crate) fn len(&self) -> usize {
        self.by_id.len()
    }

    pub(crate) fn holds(&self, id: NetId) -> bool {
        self.by_id.contains_key(&id)
    }

    /// Player `id`'s lantern.
    pub fn of(&self, id: PlayerId) -> Option<&Lantern> {
        self.by_id.values().find(|l| l.owner == id)
    }
}

/// Yaw of a horizontal-ish direction (the look convention: yaw 0 looks along −Z).
fn yaw_of(dir: Vec3) -> f32 {
    (-dir.x).atan2(-dir.z)
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

    /// The toss ended at `at`: the lantern stands on the floor below (replacing its owner's
    /// old one), or is lost over a pit.
    fn lantern_settle(&mut self, t: &Tuning, p: &Projectile, at: Vec3) {
        self.events.push(SimEvent::ProjectileImpact {
            id: p.id,
            kind: p.kind,
            position: at,
            normal: Vec3::Y,
        });
        let Owner::Player(owner) = p.owner else {
            return;
        };
        let Some(floor) = self.floor_below(at, SETTLE_DEPTH) else {
            return;
        };
        self.abilities.lantern.forget(owner);
        let Some(id) = self.alloc_net_id() else {
            return;
        };
        let l = &t.combat.lantern;
        let weapon = WeaponKind::from_id(p.bounces_left as u8).unwrap_or(WeaponKind::Arclash);
        let now = self.tick;
        let yaw = yaw_of(p.velocity);
        self.abilities.lantern.by_id.insert(
            id,
            Lantern {
                id,
                owner,
                weapon,
                position: floor,
                health: l.health,
                until: now + secs_to_ticks(l.duration, self.tick_rate, 1) as u32,
                next_shot: now + self.lantern_interval(t, weapon),
                yaw,
            },
        );
    }

    /// Ticks between a lantern's shots: its carrier's primary interval / `rate_scale`.
    fn lantern_interval(&self, t: &Tuning, weapon: WeaponKind) -> u32 {
        let c = &t.combat;
        let secs = weapon.primary_interval(c) / c.lantern.rate_scale.max(0.05);
        secs_to_ticks(secs, self.tick_rate, 1) as u32
    }

    /// The toss hit the level: it settles below where it struck.
    pub(crate) fn lantern_hit_world(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        hit: SweepHit,
    ) -> Option<bool> {
        let at = p.position + hit.normal * 0.05;
        self.lantern_settle(t, p, at);
        Some(false)
    }

    /// The toss's flight ran out in the air: it settles below.
    pub(crate) fn lantern_expired(&mut self, t: &Tuning, p: &mut Projectile) -> bool {
        let at = p.position;
        self.lantern_settle(t, p, at);
        true
    }

    /// The lanterns as entities.
    pub(crate) fn replicate_lanterns(&self, _t: &Tuning, frame: &mut NetFrame) {
        for l in self.abilities.lantern.by_id.values() {
            frame.insert(&l.entity_state());
        }
    }

    /// A blast of `owner` at `center` hurts the lanterns of its owner's enemies within its
    /// radius (linear falloff like a body's, line of sight from the centre).
    pub(crate) fn blast_lanterns(
        &mut self,
        t: &Tuning,
        owner: Owner,
        center: Vec3,
        spec: Explosive,
    ) {
        if self.abilities.lantern.by_id.is_empty() {
            return;
        }
        let edge = t.combat.explosion_edge_fraction;
        let hits: Vec<(NetId, f32)> = self
            .abilities
            .lantern
            .by_id
            .values()
            .filter(|l| {
                owner.as_target() != Some(HitTarget::Player(l.owner))
                    && self.is_hostile(owner, HitTarget::Player(l.owner))
            })
            .filter_map(|l| {
                let at = l.muzzle();
                let d = at.distance(center);
                let r = spec.radius.max(1e-3);
                if d > r || self.collision.world.line_blocked(center, at, MASK_SHOTS) {
                    return None;
                }
                let frac = 1.0 - (1.0 - edge) * (d / r).clamp(0.0, 1.0);
                Some((l.id, spec.damage * frac))
            })
            .collect();
        for (id, damage) in hits {
            if let Some(l) = self.abilities.lantern.by_id.get_mut(&id) {
                l.health -= damage;
            }
        }
    }

    /// The nearest hostile of `owner` within `range` of `from` (padded shape), in line of
    /// sight of its centre: (target, aim point).
    fn lantern_target(
        &self,
        t: &Tuning,
        owner: PlayerId,
        boxes: &[Hitbox],
        from: Vec3,
        range: f32,
    ) -> Option<(HitTarget, Vec3)> {
        let me = Owner::Player(owner);
        let world = &self.collision.world;
        boxes
            .iter()
            .filter(|h| self.can_hit(me, h.target))
            .filter_map(|h| {
                let pads = self.hit_pads(t, me, h.target);
                let (d, _) = h.distance_to_point_padded(from, pads)?;
                let aim = h.center()?;
                (d <= range && !world.line_blocked(from, aim, MASK_SHOTS))
                    .then_some((d, h.target, aim))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, target, aim)| (target, aim))
    }

    /// Lantern `l` fires its carrier's primary at `aim` (see the module docs). Returns the
    /// event's ray ends.
    fn lantern_fire(&mut self, t: &Tuning, l: &Lantern, aim: Vec3, boxes: &[Hitbox]) -> Vec<Vec3> {
        let c = &t.combat;
        let ln = &c.lantern;
        let scale = ln.damage_scale.max(0.0);
        let from = l.muzzle();
        let dir = (aim - from).normalize_or(Vec3::NEG_Z);
        let me = Owner::Player(l.owner);
        let attack = Attack::Lantern;
        match l.weapon {
            WeaponKind::Riveter => {
                let r = &c.riveter;
                let mut rng = SpreadRng::new(self.tick, l.owner, LANTERN_SALT ^ l.id.0 as u32);
                let d = spread_dir(dir, r.spread_degrees, &mut rng);
                let start = self.safe_launch_point(from, from + dir * 0.3);
                self.spawn_player_projectile(
                    t,
                    ProjectileKind::Rivet,
                    l.owner,
                    start,
                    d * r.rivet_speed,
                    r.damage * scale,
                    attack,
                );
                Vec::new()
            }
            WeaponKind::Kiln => {
                let cone = KilnCone {
                    eye: from,
                    aim: dir,
                    range: ln.kiln_range,
                    damage: c.kiln.damage * scale,
                    attack,
                };
                self.kiln_cone(t, me, boxes, cone);
                vec![from + dir * ln.kiln_range]
            }
            // The Arclash (and anything else of its class): a chaining zap.
            _ => {
                let a = &c.arclash;
                let damage = a.zap_damage * scale;
                let range = a.zap_range.max(ln.range + 2.0);
                let spec = ShotSpec::new(from, dir, range, damage, attack);
                let r = self.trace_shot(t, me, boxes, None, &spec, 0);
                if let Some(impact) = r.impact
                    && let Some((target, _)) = impact.target
                {
                    self.arc_chain(t, me, boxes, target, impact.point, damage);
                }
                vec![r.end]
            }
        }
    }

    /// Per tick: lanterns go out (their time, destroyed, their owner gone) and fire when due
    /// at the nearest hostile in range.
    pub(crate) fn step_lantern(&mut self, t: &Tuning) {
        let now = self.tick;
        let players = &self.players;
        self.abilities
            .lantern
            .by_id
            .retain(|_, l| now < l.until && l.health > 0.0 && players.contains_key(&l.owner));
        let due: Vec<Lantern> = self
            .abilities
            .lantern
            .by_id
            .values()
            .filter(|l| now >= l.next_shot)
            .copied()
            .collect();
        if due.is_empty() {
            return;
        }
        let boxes = self.current_hitboxes(t);
        let range = t.combat.lantern.range;
        for l in due {
            let Some((_, aim)) = self.lantern_target(t, l.owner, &boxes, l.muzzle(), range) else {
                continue;
            };
            let ends = self.lantern_fire(t, &l, aim, &boxes);
            let next = now + self.lantern_interval(t, l.weapon);
            if let Some(lantern) = self.abilities.lantern.by_id.get_mut(&l.id) {
                lantern.next_shot = next;
                lantern.yaw = yaw_of(aim - l.muzzle());
            }
            self.events.push(SimEvent::LanternShot {
                lantern: l.id,
                origin: l.muzzle(),
                ends,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_follows_the_look_convention() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(yaw_of(Vec3::NEG_Z), 0.0));
        let look = crate::input::look_dir(yaw_of(Vec3::X), 0.0);
        assert!(look.distance(Vec3::X) < 1e-5, "{look}");
    }
}
