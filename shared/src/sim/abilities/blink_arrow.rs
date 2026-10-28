//! Blink Arrow (Draw, move; `docs/rework.md` §1): armed like the other arrows, the next
//! release looses it with the draw (`ProjectileKind::BlinkArrow`, its own hit
//! `blink_arrow.damage_scale` × a plain arrow's). ALT while it flies, or within
//! `blink_arrow.window` after it sticks or strikes (`AbilityPhase::Recast`), steps you to it:
//!
//! * **where**: in the level, just in front of the surface (on a floor: standing on it); in a
//!   target, just in front of where it struck; in flight, where it is now — then the nearest
//!   spot back along the arrow's path where the body fits (standing, else crouched);
//! * **how**: the body is moved there (server-side, like the Lunge's impulse: the owner's
//!   client reconciles; `SimEvent::Blink` → `Fx::Blink` for everyone), its speed set to
//!   `exit_speed` along the arrow's flight.
//!
//! The arrow is live (`WeaponState::live`): its cooldown starts at the blink or when the window
//! passes (the server shortens the live time to the window once the arrow sticks or strikes).

use std::collections::BTreeMap;

use engine::physics::{MASK_MOVEMENT, SweepHit};
use glam::Vec3;

use super::super::projectiles::{Projectile, Stuck};
use super::super::{HitPart, HitTarget, PlayerId, ProjectileKind, SimEvent, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::movement::MoveState;
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// The arrow's path is kept this coarsely (metres between kept points).
const PATH_STEP: f32 = 0.5;
/// The search steps back along the path this far at most (m).
const SEARCH: f32 = 30.0;

/// A Blink Arrow out.
#[derive(Clone, Debug, PartialEq)]
pub struct BlinkArrow {
    /// The arrow (`None` once it struck a target and is gone).
    pub arrow: Option<NetId>,
    /// The bow's slot (its live ability).
    pub slot: u8,
    /// Where it has been, oldest first (from the bow).
    pub path: Vec<Vec3>,
    /// Its flight direction (the latest).
    pub dir: Vec3,
    /// Where it stuck or struck: the spot to step to and the surface normal there (`None`:
    /// still flying).
    pub landed: Option<(Vec3, Vec3)>,
}

/// Blink arrows out, per owner (one at a time: the bow's live ability).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlinkArrows {
    pub out: BTreeMap<PlayerId, BlinkArrow>,
}

impl BlinkArrows {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.out.remove(&id);
    }
}

impl SimWorld {
    /// The release looses the arrow; the recast steps to it.
    pub(crate) fn cast_blink_arrow(&mut self, c: &AbilityCast) {
        match c.phase {
            AbilityPhase::Use => {
                let scale = c.t.combat.blink_arrow.damage_scale;
                let Some(arrow) = self.loose_ability_arrow(c, ProjectileKind::BlinkArrow, scale)
                else {
                    return;
                };
                let Some(p) = self.projectiles.get(&arrow) else {
                    return;
                };
                let slot = self.players.get(&c.id).map_or(0, |p| p.combat.weapons.slot);
                let blink = BlinkArrow {
                    arrow: Some(arrow),
                    slot,
                    path: vec![p.position],
                    dir: p.velocity.normalize_or(Vec3::NEG_Z),
                    landed: None,
                };
                // A new arrow replaces an old one still out.
                if let Some(old) = self.abilities.blink.out.insert(c.id, blink)
                    && let Some(id) = old.arrow
                {
                    self.projectiles.remove(&id);
                }
            }
            AbilityPhase::Recast => self.blink(c.t, c.id),
            AbilityPhase::Release => {}
        }
    }

    /// The step: player `id` to their arrow.
    fn blink(&mut self, t: &Tuning, id: PlayerId) {
        let Some(b) = self.abilities.blink.out.remove(&id) else {
            return;
        };
        // In flight: where it is now (then it is gone either way).
        let flying = b
            .arrow
            .and_then(|arrow| self.projectiles.remove(&arrow))
            .map(|a| a.position);
        let Some(p) = self.players.get(&id).filter(|p| !p.combat.dead) else {
            return;
        };
        let from = p.movement.position;
        let mut body = p.movement;
        let (spot, normal) = match (b.landed, flying) {
            (Some(landed), _) => landed,
            (None, Some(at)) => (at, Vec3::ZERO),
            (None, None) => (*b.path.last().unwrap_or(&from), Vec3::ZERO),
        };
        let Some((feet, crouched)) = self.blink_spot(t, &body, spot, normal, &b.path) else {
            return;
        };
        body.position = feet;
        body.crouched = crouched;
        body.velocity = b.dir * t.combat.blink_arrow.exit_speed;
        body.on_ground = false;
        body.dash_time = 0.0;
        if let Some(p) = self.players.get_mut(&id) {
            p.movement = body;
        }
        self.events.push(SimEvent::Blink {
            player: id,
            from,
            to: feet,
        });
    }

    /// Where a body (`body`'s stance) can stand stepping to `spot` (a surface with `normal`,
    /// or zero in the air / at a target): in front of it, else the nearest point back along
    /// `path` (newest first) where it fits, standing or crouched. Feet position, crouched.
    fn blink_spot(
        &self,
        t: &Tuning,
        body: &MoveState,
        spot: Vec3,
        normal: Vec3,
        path: &[Vec3],
    ) -> Option<(Vec3, bool)> {
        let m = &t.movement;
        let fits = |feet: Vec3| -> Option<(Vec3, bool)> {
            for crouched in [false, true] {
                let probe = MoveState {
                    position: feet,
                    crouched,
                    ..*body
                };
                let (center, half, radius) = probe.capsule(m);
                if !self
                    .collision
                    .world
                    .intersects_capsule(center, half, radius, MASK_MOVEMENT)
                {
                    return Some((feet, crouched));
                }
            }
            None
        };
        // A spot for the feet that puts the body's middle at `point` (on a floor: on it).
        let feet_at = |point: Vec3, normal: Vec3| -> Vec3 {
            if normal.y > 0.7 {
                point + Vec3::Y * 0.05
            } else {
                let out = point + normal * (m.radius + 0.1);
                out - Vec3::Y * (m.standing_height * 0.5)
            }
        };
        if let Some(fit) = fits(feet_at(spot, normal)) {
            return Some(fit);
        }
        // Back along the path, a little up too (a low ceiling, a step).
        let mut points = vec![spot];
        points.extend(path.iter().rev().copied());
        let mut walked = 0.0;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let len = a.distance(b);
            let steps = (len / 0.25).ceil().max(1.0) as u32;
            for i in 1..=steps {
                let at = a.lerp(b, i as f32 / steps as f32);
                walked += len / steps as f32;
                for lift in [0.0, 0.4] {
                    if let Some(fit) = fits(feet_at(at, Vec3::ZERO) + Vec3::Y * lift) {
                        return Some(fit);
                    }
                }
                if walked > SEARCH {
                    return None;
                }
            }
        }
        None
    }

    /// The arrow struck `target`: its hit (the generic arrow damage), then it is gone and the
    /// step lands just in front of where it struck.
    pub(crate) fn blink_arrow_hit_target(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        target: HitTarget,
        part: HitPart,
        dir: Vec3,
    ) -> Option<bool> {
        self.projectile_direct_damage(t, p, target, part, dir);
        let owner = p.owner.player()?;
        let window = t.combat.blink_arrow.window;
        let b = self.abilities.blink.out.get_mut(&owner)?;
        if b.arrow == Some(p.id) {
            b.arrow = None;
            b.dir = dir;
            b.landed = Some((p.position - dir * 0.6, -dir));
            let slot = b.slot;
            if let Some(pl) = self.players.get_mut(&owner) {
                pl.combat.weapons.shorten_live(slot, window, self.tick_rate);
            }
        }
        Some(false)
    }

    /// The arrow hit the level: it sticks there for the window.
    pub(crate) fn blink_arrow_hit_world(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        hit: SweepHit,
    ) -> Option<bool> {
        let owner = p.owner.player()?;
        let point = p.position;
        let window = t.combat.blink_arrow.window;
        let now = self.tick;
        let tracked = self
            .abilities
            .blink
            .out
            .get(&owner)
            .is_some_and(|b| b.arrow == Some(p.id));
        if !tracked || p.parried {
            return None;
        }
        let dir = p.velocity.normalize_or(Vec3::NEG_Z);
        p.stuck = Some(Stuck {
            to: None,
            offset: Vec3::ZERO,
        });
        p.velocity = Vec3::ZERO;
        p.expire_tick = now + super::super::combat::secs_to_ticks(window, self.tick_rate, 1) as u32;
        p.relaunch(now);
        if let Some(b) = self.abilities.blink.out.get_mut(&owner) {
            b.dir = dir;
            b.landed = Some((point, hit.normal));
            let slot = b.slot;
            if let Some(pl) = self.players.get_mut(&owner) {
                pl.combat.weapons.shorten_live(slot, window, self.tick_rate);
            }
        }
        Some(true)
    }

    /// Per tick: the arrows' paths are kept; one whose owner can no longer step to it (the
    /// window passed, the bow is gone, they died) is dropped.
    pub(crate) fn step_blink_arrow(&mut self, _t: &Tuning) {
        let ids: Vec<PlayerId> = self.abilities.blink.out.keys().copied().collect();
        for id in ids {
            let live = self.players.get(&id).is_some_and(|p| {
                let b = &self.abilities.blink.out[&id];
                !p.combat.dead
                    && p.combat
                        .weapons
                        .live
                        .get(b.slot as usize)
                        .is_some_and(|l| *l > 0)
            });
            let Some(b) = self.abilities.blink.out.get_mut(&id) else {
                continue;
            };
            if !live {
                let arrow = b.arrow;
                self.abilities.blink.out.remove(&id);
                if let Some(arrow) = arrow {
                    self.projectiles.remove(&arrow);
                }
                continue;
            }
            let flying = b.arrow.and_then(|a| self.projectiles.get(&a));
            match flying {
                Some(a) if a.stuck.is_none() => {
                    if b.path
                        .last()
                        .is_none_or(|l| l.distance(a.position) >= PATH_STEP)
                    {
                        b.path.push(a.position);
                    }
                    b.dir = a.velocity.normalize_or(b.dir);
                }
                Some(_) => {}
                // Gone without landing (its flight ran out): nothing to step to.
                None if b.landed.is_none() => b.arrow = None,
                None => {}
            }
        }
    }
}
