//! Bolas (Scatter, control; `docs/rework.md` §1): two wax-weighted balls on a wick cord
//! (`ProjectileKind::Bolas`: `bolas.speed`, gravity `bolas.gravity`, passes nothing). The
//! first hostile it touches is bound (Pinned: the Pinning Arrow's times); a second hostile
//! within `bolas.link_range` of the first is bound too, and for `bolas.link_time`
//! `bolas.link_share` of the damage either takes is also dealt to the other (never chained
//! further, never to the source itself). `BOUND` flags, `Fx::BolasBound`.
//!
//! * **The bind** ([`SimWorld::bolas_hit_target`]): `bolas.damage` (if any) as
//!   `Attack::Bolas`, then `SimWorld::apply_pin` on the first; the second is the nearest other
//!   hostile of the thrower whose (padded) shape is within `link_range` of the first's centre,
//!   in line of sight of it (current shapes). Off the level the throw just drops.
//! * **The cord** ([`SimWorld::bolas_after_damage`]): damage that went through on one end
//!   (its `dealt`, after every rule) is dealt × `link_share` to the other as the same source
//!   and attack (kill credit stays the attacker's), on the body, no knockback, no flags — and
//!   that share is never shared back or on (`BolasWorld::sharing`), nor dealt to the one who
//!   caused it. A cord ends when its time is up or either end dies.
//! * `BOUND` (player and enemy flags) while a target is pinned by a bolas or on a cord.

use std::collections::BTreeMap;

use engine::physics::{MASK_SHOTS, SweepHit};
use glam::Vec3;

use super::super::combat::{DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::super::projectiles::{Projectile, ProjectileKind};
use super::super::{DamageEvent, HitPart, HitTarget, PlayerId, SimEvent, SimWorld};
use super::AbilityCast;
use crate::tuning::Tuning;

/// A Waxwork's double (its decoy id: `waxwork.rs`)?
fn is_decoy_target(target: HitTarget) -> bool {
    matches!(target, HitTarget::Player(id) if super::waxwork::decoy_owner(id).is_some())
}

/// A cord binding two hostiles: damage either takes is shared with the other.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cord {
    /// Who threw it.
    pub owner: PlayerId,
    pub ends: [HitTarget; 2],
    /// It goes slack at this tick.
    pub until: u32,
}

impl Cord {
    /// The other end from `target` (`None`: not on this cord).
    pub fn other(&self, target: HitTarget) -> Option<HitTarget> {
        match self.ends {
            [a, b] if a == target => Some(b),
            [a, b] if b == target => Some(a),
            _ => None,
        }
    }
}

/// Bolas binds and cords ([`super::AbilityWorld::bolas`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BolasWorld {
    /// Bound targets (pinned by a bolas or on a cord) until this tick.
    pub bound: BTreeMap<HitTarget, u32>,
    pub cords: Vec<Cord>,
    /// A cord's share is being dealt: it isn't shared again (nor does it stoke a Burn).
    pub sharing: bool,
}

impl BolasWorld {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        let me = HitTarget::Player(id);
        self.bound.remove(&me);
        self.cords
            .retain(|c| c.owner != id && !c.ends.contains(&me));
    }
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

    /// Seconds a Pinning Arrow's pin holds `target` (a Mould, a heavy one, a player): the bind's
    /// `BOUND` lasts as long.
    fn pin_secs(&self, t: &Tuning, target: HitTarget) -> f32 {
        let pa = &t.combat.pinning_arrow;
        match target {
            HitTarget::Player(_) => pa.player_time,
            HitTarget::Entity(id) => match self.enemies.get(&id) {
                Some(e) if e.def.health >= pa.heavy_health => pa.heavy_time,
                _ => pa.enemy_time,
            },
        }
    }

    /// Binds `target` for `secs` (at least): pinned, `BOUND`.
    fn bolas_bind(&mut self, t: &Tuning, by: Owner, target: HitTarget, secs: f32) {
        self.apply_pin(t, by, target);
        let until = self.tick + secs_to_ticks(secs, self.tick_rate, 1) as u32;
        let e = self.abilities.bolas.bound.entry(target).or_insert(until);
        *e = (*e).max(until);
    }

    /// The Bolas touched `target`: it binds it (and a second hostile close by, on a cord) and
    /// is gone (see the module docs).
    pub(crate) fn bolas_hit_target(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        target: HitTarget,
        part: HitPart,
        dir: Vec3,
    ) -> Option<bool> {
        let Owner::Player(owner) = p.owner else {
            return Some(false);
        };
        // A Waxwork's double isn't bound (it stands still anyway).
        if is_decoy_target(target) {
            return None;
        }
        let b = &t.combat.bolas;
        // (`Attack::Bolas` on its own hit, `projectile_direct_damage`.)
        if p.damage > 0.0 {
            self.projectile_direct_damage(t, p, target, part, dir);
        }
        let first_at = self.target_center(target).unwrap_or(p.position);
        let pin = self.pin_secs(t, target);
        // The second end: the nearest other hostile close to the first.
        let mut second: Option<(f32, HitTarget, Vec3)> = None;
        let world = &self.collision.world;
        for h in self.current_hitboxes(t) {
            if h.target == target || is_decoy_target(h.target) || !self.can_hit(p.owner, h.target) {
                continue;
            }
            let pads = self.hit_pads(t, p.owner, h.target);
            let Some((d, _)) = h.distance_to_point_padded(first_at, pads) else {
                continue;
            };
            let Some(at) = h.center() else {
                continue;
            };
            if d > b.link_range || world.line_blocked(first_at, at, MASK_SHOTS) {
                continue;
            }
            if second.is_none_or(|(sd, _, _)| d < sd) {
                second = Some((d, h.target, at));
            }
        }
        if !self.can_hit(p.owner, target) {
            // The hit itself killed it: nothing to bind.
            self.events.push(SimEvent::BolasBound {
                owner,
                positions: vec![first_at],
            });
            return Some(false);
        }
        let mut positions = vec![first_at];
        match second {
            Some((_, other, at)) => {
                let link = b.link_time.max(0.0);
                self.bolas_bind(t, p.owner, target, link.max(pin));
                let other_pin = self.pin_secs(t, other);
                self.bolas_bind(t, p.owner, other, link.max(other_pin));
                let until = self.tick + secs_to_ticks(link, self.tick_rate, 1) as u32;
                // A target is on one cord at a time: the newest.
                self.abilities
                    .bolas
                    .cords
                    .retain(|c| !c.ends.contains(&target) && !c.ends.contains(&other));
                self.abilities.bolas.cords.push(Cord {
                    owner,
                    ends: [target, other],
                    until,
                });
                positions.push(at);
            }
            None => self.bolas_bind(t, p.owner, target, pin),
        }
        self.events.push(SimEvent::BolasBound { owner, positions });
        Some(false)
    }

    /// The Bolas hit the level: the generic rule (it drops there and is gone).
    pub(crate) fn bolas_hit_world(
        &mut self,
        _t: &Tuning,
        _p: &mut Projectile,
        _hit: SweepHit,
    ) -> Option<bool> {
        None
    }

    /// Is `target` bound by a Bolas (pinned by one, or on a cord)?
    pub fn is_bound(&self, target: HitTarget) -> bool {
        self.abilities
            .bolas
            .bound
            .get(&target)
            .is_some_and(|until| self.tick < *until)
    }

    /// The other end of `target`'s cord, while it holds.
    pub fn cord_partner(&self, target: HitTarget) -> Option<HitTarget> {
        let now = self.tick;
        self.abilities
            .bolas
            .cords
            .iter()
            .filter(|c| now < c.until)
            .find_map(|c| c.other(target))
    }

    /// After every damage instance: an end of a cord shares it with the other (see the module
    /// docs).
    pub(crate) fn bolas_after_damage(&mut self, t: &Tuning, event: &DamageEvent) {
        if self.abilities.bolas.sharing || event.dealt <= 0.0 {
            return;
        }
        let Some(other) = self.cord_partner(event.target) else {
            return;
        };
        let source = event.info.source;
        if source.as_target() == Some(other) || !self.can_hit(source, other) {
            return;
        }
        let Some(at) = self.target_center(other) else {
            return;
        };
        let from = self.target_center(event.target).unwrap_or(at);
        let amount = event.dealt * t.combat.bolas.link_share.max(0.0);
        let info = DamageInfo::new(amount, source, event.info.attack)
            .at(at, (at - from).normalize_or(Vec3::Y))
            .part(HitPart::Body)
            .flags(DamageFlags::NONE);
        self.abilities.bolas.sharing = true;
        self.apply_damage(t, other, info);
        self.abilities.bolas.sharing = false;
    }

    /// Per tick: binds wear off, cords go slack (or lose an end).
    pub(crate) fn step_bolas(&mut self, _t: &Tuning) {
        let now = self.tick;
        let bolas = &mut self.abilities.bolas;
        bolas.bound.retain(|_, until| now < *until);
        let alive = |target: &HitTarget| match target {
            HitTarget::Player(id) => self.players.get(id).is_some_and(|p| !p.combat.dead),
            HitTarget::Entity(id) => self.enemies.get(id).is_some_and(|e| e.is_alive()),
        };
        bolas
            .cords
            .retain(|c| now < c.until && c.ends.iter().all(alive));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::NetId;

    #[test]
    fn a_cord_knows_its_other_end() {
        let (a, b) = (HitTarget::Entity(NetId(1)), HitTarget::Entity(NetId(2)));
        let cord = Cord {
            owner: PlayerId(0),
            ends: [a, b],
            until: 10,
        };
        assert_eq!(cord.other(a), Some(b));
        assert_eq!(cord.other(b), Some(a));
        assert_eq!(cord.other(HitTarget::Entity(NetId(3))), None);
    }
}
