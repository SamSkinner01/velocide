//! What the arsenal's projectiles do when they land (`docs/arsenal.md`): the Flak shell's
//! airburst ring, Limpets sticking and detonating, the Longwick's special arrows (pinning,
//! branding), the refit's thrown knacks (the Kindle Flask's pool, the Soot Pot's cloud, the
//! Snare settling on the floor) and the rework's (the Bolas, the Lantern's toss, the Blink and
//! Beacon arrows: their own files in `abilities/`). `projectiles.rs` calls the hooks below on a
//! target hit, a world hit, the end of a flight and every tick of flight; everything else
//! follows the generic projectile rules there.

use engine::physics::SweepHit;
use glam::Vec3;

use super::combat::{Attack, Owner};
use super::hitscan::ShotSpec;
use super::projectiles::{Projectile, ProjectileKind, Stuck};
use super::weapons::aim_basis;
use super::{HitPart, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::tuning::Tuning;

impl SimWorld {
    /// A projectile hit `target` (`part`, flying along `dir`). `Some(keep)` if its kind has its
    /// own contact rule, `None` for the generic one.
    pub(crate) fn payload_hit_target(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        target: HitTarget,
        part: HitPart,
        dir: Vec3,
    ) -> Option<bool> {
        let now = self.tick;
        match p.kind {
            ProjectileKind::Limpet => {
                // Sticks to whatever it hit and rides along.
                let origin = self.target_origin(target)?;
                p.stuck = Some(Stuck {
                    to: Some(target),
                    offset: p.position - origin,
                });
                p.velocity = Vec3::ZERO;
                p.relaunch(now);
                Some(true)
            }
            ProjectileKind::FlakShell => {
                // A contact burst half a metre short: the ring closes on what it hit.
                let at = p.position - dir * 0.5;
                self.flak_burst(t, p, at, dir);
                Some(false)
            }
            ProjectileKind::PinArrow => {
                self.projectile_direct_damage(t, p, target, part, dir);
                self.apply_pin(t, p.owner, target);
                Some(false)
            }
            ProjectileKind::BrandArrow => {
                self.projectile_direct_damage(t, p, target, part, dir);
                if let Owner::Player(owner) = p.owner {
                    self.brand_target(t, owner, target, p.position);
                }
                Some(false)
            }
            ProjectileKind::Bolas => self.bolas_hit_target(t, p, target, part, dir),
            ProjectileKind::BlinkArrow => self.blink_arrow_hit_target(t, p, target, part, dir),
            ProjectileKind::BeaconArrow => self.beacon_arrow_hit_target(t, p, target, part, dir),
            ProjectileKind::KindleFlask => {
                // Breaks on whoever it hit: the pool at their feet.
                let feet = self.target_origin(target).unwrap_or(p.position);
                self.knack_lands(t, p, feet);
                Some(false)
            }
            ProjectileKind::SootPot => {
                self.knack_lands(t, p, p.position);
                Some(false)
            }
            _ => None,
        }
    }

    /// A projectile hit the level. `Some(keep)` if its kind has its own rule.
    pub(crate) fn payload_hit_world(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        hit: SweepHit,
    ) -> Option<bool> {
        let now = self.tick;
        match p.kind {
            ProjectileKind::Limpet if !p.parried => {
                p.stuck = Some(Stuck {
                    to: None,
                    offset: Vec3::ZERO,
                });
                p.velocity = Vec3::ZERO;
                p.relaunch(now);
                Some(true)
            }
            ProjectileKind::FlakShell => {
                // Off a wall the ring sprays away from it (the flight reflected).
                let d = p.velocity.normalize_or(Vec3::NEG_Z);
                let n = hit.normal;
                let axis = (d - n * (2.0 * d.dot(n))).normalize_or(n);
                let at = p.position + n * 0.1;
                self.flak_burst(t, p, at, axis);
                Some(false)
            }
            ProjectileKind::Bolas => self.bolas_hit_world(t, p, hit),
            ProjectileKind::Hurl => self.hurl_hit_world(t, p),
            ProjectileKind::LanternToss => self.lantern_hit_world(t, p, hit),
            ProjectileKind::BlinkArrow => self.blink_arrow_hit_world(t, p, hit),
            ProjectileKind::BeaconArrow => self.beacon_arrow_hit_world(t, p, hit),
            ProjectileKind::KindleFlask | ProjectileKind::SootPot => {
                let at = p.position + hit.normal * 0.05;
                self.knack_lands(t, p, at);
                Some(false)
            }
            ProjectileKind::Snare => {
                if hit.normal.y > 0.5 {
                    // On the floor: it lies there as a trap (`knacks.rs` arms and springs it).
                    p.stuck = Some(Stuck {
                        to: None,
                        offset: Vec3::ZERO,
                    });
                    p.velocity = Vec3::ZERO;
                } else {
                    // Off a wall or a ceiling: it drops on.
                    let n = hit.normal;
                    p.velocity = (p.velocity - n * (2.0 * p.velocity.dot(n))) * 0.3;
                }
                p.relaunch(now);
                Some(true)
            }
            _ => None,
        }
    }

    /// A projectile's flight time ran out in the air. `true` if its kind has its own rule.
    pub(crate) fn payload_expired(&mut self, t: &Tuning, p: &mut Projectile) -> bool {
        match p.kind {
            ProjectileKind::FlakShell => {
                // The airburst.
                let axis = p.velocity.normalize_or(Vec3::NEG_Z);
                self.flak_burst(t, p, p.position, axis);
                true
            }
            ProjectileKind::KindleFlask | ProjectileKind::SootPot => {
                self.knack_lands(t, p, p.position);
                true
            }
            _ => false,
        }
    }

    /// Every tick of flight, before a projectile moves (the Wisp zaps and orbits, the Kite is
    /// steered, the Hurl turns back). `false`: it is gone.
    pub(crate) fn payload_tick(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        boxes: &[Hitbox],
    ) -> bool {
        match p.kind {
            ProjectileKind::Wisp => self.wisp_tick(t, p, boxes),
            ProjectileKind::Kite => self.kite_tick(t, p, boxes),
            ProjectileKind::Hurl => self.hurl_tick(t, p, boxes),
            _ => true,
        }
    }

    /// A Kindle Flask / Soot Pot broke at `at`: the pool (on the ground below) or the cloud.
    fn knack_lands(&mut self, t: &Tuning, p: &Projectile, at: Vec3) {
        if let Owner::Player(owner) = p.owner {
            match p.kind {
                ProjectileKind::KindleFlask => self.spawn_kindle_pool(t, owner, at),
                ProjectileKind::SootPot => self.spawn_soot_cloud(t, owner, at),
                _ => {}
            }
        }
        self.events.push(SimEvent::ProjectileImpact {
            id: p.id,
            kind: p.kind,
            position: at,
            normal: Vec3::Y,
        });
    }

    /// The Flak ring: `flak.pellets` rays from `origin`, evenly around `axis`, each
    /// `flak.ring_degrees` off it (a cone's rim), `flak.pellet_range` long, the shell's damage
    /// each. Emits [`SimEvent::Burst`].
    fn flak_burst(&mut self, t: &Tuning, p: &Projectile, origin: Vec3, axis: Vec3) {
        let f = &t.combat.flak;
        let n = f.pellets.max(1);
        let (right, up) = aim_basis(axis);
        let (sin, cos) = f.ring_degrees.to_radians().sin_cos();
        let boxes = self.current_hitboxes(t);
        let mut ends = Vec::with_capacity(n as usize);
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            let side = right * a.cos() + up * a.sin();
            let dir = (axis * cos + side * sin).normalize_or(axis);
            let spec = ShotSpec::new(origin, dir, f.pellet_range, p.damage, p.attack);
            let r = self.trace_shot(t, p.owner, &boxes, None, &spec, 0);
            ends.push(r.end);
        }
        self.events.push(SimEvent::Burst {
            shooter: p.owner,
            attack: p.attack,
            origin,
            ends,
        });
    }

    /// Player `id`'s Limpet charges (in flight or stuck).
    pub fn limpets_of(&self, id: PlayerId) -> Vec<crate::protocol::NetId> {
        self.projectiles
            .values()
            .filter(|p| p.kind == ProjectileKind::Limpet && p.owner == Owner::Player(id))
            .map(|p| p.id)
            .collect()
    }

    /// Detonates every Limpet charge of player `id` (each explodes where it is, a direct hit
    /// on what it sticks to). Returns how many went off.
    pub fn detonate_limpets(&mut self, t: &Tuning, id: PlayerId) -> usize {
        let ids = self.limpets_of(id);
        for pid in &ids {
            if let Some(p) = self.projectiles.remove(pid) {
                let direct = p.stuck.and_then(|s| s.to);
                self.explode_projectile(t, &p, direct);
            }
        }
        ids.len()
    }

    /// Player `id` uses Limpet: a tap (`charge` 0) throws a charge from `at` at `vel`, the
    /// hold (`charge` 1) detonates every charge out; so does a throw with `limpet.max_live`
    /// charges already out.
    pub(crate) fn limpet(&mut self, t: &Tuning, id: PlayerId, charge: f32, at: Vec3, vel: Vec3) {
        let live = self.limpets_of(id).len() as u32;
        if charge >= 1.0 || live >= t.combat.limpet.max_live.max(1) {
            self.detonate_limpets(t, id);
            return;
        }
        let owner = Owner::Player(id);
        self.spawn_projectile(t, ProjectileKind::Limpet, owner, at, vel);
        self.push_fired(id, Attack::Limpet, at);
    }
}
