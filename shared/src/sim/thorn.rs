//! Thorns: barbed glass relay spikes thrown with `THROW` (they replace the first arsenal's
//! prisms). A thorn flies nearly straight (`thorn.gravity`) and **plants itself** in the first
//! wall, floor or ceiling it hits, oriented along its flight, and lives `thorn.lifetime` there.
//! Hitting an enemy / hostile player in flight deals `thorn.damage` (hit assist applies); the
//! thorn then drops to the floor and lies there as a live thorn.
//!
//! **Relays** (PvE only, as the Thorn is): the first player attack that reaches a live thorn
//! (flying, dropping, planted or lying) shatters it and is relayed to the thorn's
//! [`SimWorld::ricochet_target`] — another live thorn within `chain_range` first, then a
//! branded hostile, then the nearest hostile, weak point / head if visible:
//!
//! * **rays** (every hitscan primary and ability, each pellet and zap) ricochet off it
//!   (`hitscan.rs`; a pellet spread uses it once);
//! * **projectiles** ([`SimWorld::relay_projectile`]) passing within `ricochet_radius` (beyond
//!   their own radius) before the level or a target stops them turn there onto a straight line
//!   at the target, keeping their payload and speed;
//! * **swings, cones, novas and blasts** whose area holds it fling it as a ray at the target
//!   ([`SimWorld::fling_thorns`]): of the swing's damage (melee), the per-hit damage (a cone or
//!   a nova), `relay_damage` (an explosion).
//!
//! Each relay multiplies the damage by `damage_multiplier` and makes it a ricochet hit
//! (`DamageFlags::RICOCHET`, `DamageInfo::ricochets`: style's RICOCHET); every relay is a
//! [`SimEvent::Ricochet`] (`projectile` set for a turned projectile).
//!
//! Replicated as `kind::THORN` by trajectory like projectiles (`position`/`velocity` at launch
//! tick `aux`, gravity `thorn.gravity`); while `PLANTED` (planted or lying) `position` is where
//! it sits and `velocity` its orientation. `owner` = thrower. A projectile relayed off one
//! carries `projectile_flags::RELAYED` (no gravity).

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::hitscan::ShotSpec;
use super::projectiles::{Projectile, ProjectileKind, projectile_target_pad};
use super::trail::Trail;
use super::{HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::input::PlayerInput;
use crate::protocol::{EntityState, NetId, kind, projectile_flags, thorn_flags};
use crate::tuning::Tuning;

/// What a thorn is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThornState {
    /// Thrown, flying.
    Flying,
    /// Hit a target and is falling to the floor.
    Dropping,
    /// Stuck in a surface.
    Planted,
    /// Lying on the floor (after hitting a target).
    Lying,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thorn {
    pub id: NetId,
    pub owner: PlayerId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub launch_tick: u32,
    pub launch_position: Vec3,
    pub launch_velocity: Vec3,
    pub expire_tick: u32,
    pub state: ThornState,
    /// Orientation while planted / lying (unit; planted: the flight direction).
    pub facing: Vec3,
    /// Positions at the end of each tick (ricochet rewinding).
    pub trail: Trail,
}

impl Thorn {
    /// Planted in a surface or lying on the floor: not moving.
    pub fn is_planted(&self) -> bool {
        matches!(self.state, ThornState::Planted | ThornState::Lying)
    }

    pub fn entity_state(&self) -> EntityState {
        let (position, velocity, mut flags) = if self.is_planted() {
            (self.position, self.facing, thorn_flags::PLANTED)
        } else {
            (self.launch_position, self.launch_velocity, 0)
        };
        match self.state {
            ThornState::Lying => flags |= thorn_flags::LYING,
            ThornState::Dropping => flags |= thorn_flags::DROPPING,
            _ => {}
        }
        EntityState {
            id: self.id,
            kind: kind::THORN,
            owner: self.owner.0 as u16,
            position,
            velocity,
            flags,
            aux: self.launch_tick,
            ..Default::default()
        }
    }

    fn relaunch(&mut self, tick: u32) {
        self.launch_tick = tick;
        self.launch_position = self.position;
        self.launch_velocity = self.velocity;
    }
}

impl SimWorld {
    /// Throws a thorn for `owner` (the charge was already spent by the weapon state). `None`
    /// if no entity id is free.
    pub fn spawn_thorn(
        &mut self,
        t: &Tuning,
        owner: PlayerId,
        position: Vec3,
        velocity: Vec3,
    ) -> Option<NetId> {
        let id = self.alloc_net_id()?;
        let launch_tick = self.launch_tick_now();
        let life = secs_to_ticks(t.combat.thorn.flight_time, self.tick_rate, 1) as u32;
        // Don't start inside a wall right in front of the thrower.
        let start = match self.players.get(&owner) {
            Some(p) => self.safe_launch_point(p.movement.eye_position(&t.movement), position),
            None => position,
        };
        self.thorns.insert(
            id,
            Thorn {
                id,
                owner,
                position: start,
                velocity,
                launch_tick,
                launch_position: start,
                launch_velocity: velocity,
                expire_tick: self.tick + life,
                state: ThornState::Flying,
                facing: velocity.normalize_or(Vec3::NEG_Z),
                trail: Trail::new(),
            },
        );
        self.events
            .push(SimEvent::ThornThrown { player: owner, id });
        Some(id)
    }

    pub(crate) fn step_thorns(&mut self, t: &Tuning) {
        let now = self.tick;
        let expired: Vec<NetId> = self
            .thorns
            .values()
            .filter(|th| now >= th.expire_tick)
            .map(|th| th.id)
            .collect();
        for id in expired {
            self.thorns.remove(&id);
            self.events.push(SimEvent::ThornExpired { id });
        }
        let ids: Vec<NetId> = self
            .thorns
            .values()
            .filter(|th| !th.is_planted())
            .map(|th| th.id)
            .collect();
        if !ids.is_empty() {
            let boxes = self.current_hitboxes(t);
            for id in ids {
                let Some(mut th) = self.thorns.remove(&id) else {
                    continue;
                };
                self.step_thorn(t, &mut th, &boxes);
                self.thorns.insert(id, th);
            }
        }
        for th in self.thorns.values_mut() {
            th.trail.record(now, th.position);
        }
    }

    /// One tick of a flying / dropping thorn.
    fn step_thorn(&mut self, t: &Tuning, th: &mut Thorn, boxes: &[super::Hitbox]) {
        let now = self.tick;
        let dt = self.dt();
        let tt = &t.combat.thorn;
        let motion = th.velocity * dt - Vec3::Y * (0.5 * tt.gravity * dt * dt);
        let new_velocity = th.velocity - Vec3::Y * (tt.gravity * dt);
        let world_hit =
            self.collision
                .world
                .cast_sphere(th.position, tt.radius, motion, MASK_SHOTS);
        let world_f = world_hit.map_or(1.0, |h| h.fraction);
        let dir = motion.normalize_or(th.velocity.normalize_or(Vec3::NEG_Z));

        if th.state == ThornState::Flying {
            let thrower = Owner::Player(th.owner);
            let first = boxes
                .iter()
                .filter(|h| self.can_hit(thrower, h.target))
                .filter_map(|h| {
                    let pads = self.hit_pads(t, thrower, h.target);
                    h.sweep_sphere_padded(th.position, motion, tt.radius, pads)
                        .map(|(f, part)| (f, h.target, part))
                })
                .filter(|(f, _, _)| *f <= world_f)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((f, target, part)) = first {
                th.position += motion * f;
                self.thorn_hit_target(t, th, target, part, dir);
                return;
            }
        }

        let Some(hit) = world_hit else {
            th.position += motion;
            th.velocity = new_velocity;
            return;
        };
        th.position += motion * hit.fraction + hit.normal * 0.01;
        match th.state {
            ThornState::Flying => {
                // Plant: the tip goes into the surface along the flight.
                th.state = ThornState::Planted;
                th.facing = dir;
                th.velocity = Vec3::ZERO;
                th.expire_tick = now + secs_to_ticks(tt.lifetime, self.tick_rate, 1) as u32;
                th.relaunch(now);
                self.events.push(SimEvent::ThornPlanted {
                    id: th.id,
                    position: th.position,
                });
            }
            _ if hit.normal.y > 0.5 => {
                // Dropped onto a floor: lies there, pointing the way it was falling.
                th.state = ThornState::Lying;
                let flat = Vec3::new(new_velocity.x, 0.0, new_velocity.z);
                th.facing = flat.normalize_or(Vec3::X);
                th.velocity = Vec3::ZERO;
                th.expire_tick = now + secs_to_ticks(tt.lifetime, self.tick_rate, 1) as u32;
                th.relaunch(now);
            }
            _ => {
                // Dropping against a wall: slide off it and keep falling.
                let n = hit.normal;
                let v = new_velocity - n * (2.0 * new_velocity.dot(n));
                th.velocity = v * 0.3;
                th.relaunch(now);
            }
        }
    }

    /// A flying thorn hit `target`: damage, then it drops to the floor.
    fn thorn_hit_target(
        &mut self,
        t: &Tuning,
        th: &mut Thorn,
        target: HitTarget,
        part: super::HitPart,
        dir: Vec3,
    ) {
        let now = self.tick;
        let tt = &t.combat.thorn;
        let info = DamageInfo::new(tt.damage, Owner::Player(th.owner), Attack::Thorn)
            .at(th.position, dir)
            .part(part)
            .flags(DamageFlags::PROJECTILE);
        self.apply_damage(t, target, info);
        // Bounce off a little and fall; it stays live for its lifetime from now.
        let flat = Vec3::new(dir.x, 0.0, dir.z);
        th.state = ThornState::Dropping;
        th.velocity = -flat * 2.0 + Vec3::Y * 2.5;
        th.expire_tick = now + secs_to_ticks(tt.lifetime, self.tick_rate, 1) as u32;
        th.relaunch(now);
    }

    /// Removes a thorn (an attack was relayed off it).
    pub fn shatter_thorn(&mut self, id: NetId) {
        if let Some(th) = self.thorns.remove(&id) {
            self.events.push(SimEvent::ThornShattered {
                id,
                position: th.position,
            });
        }
    }

    // ---- relays (see the module docs) ----------------------------------------------------

    /// Live thorns at `time` (ticks: a shooter's view, through the thorns' trails; `None`: now)
    /// whose position passes `inside`: (id, position).
    pub(crate) fn thorns_where(
        &self,
        time: Option<f64>,
        inside: impl Fn(Vec3) -> bool,
    ) -> Vec<(NetId, Vec3)> {
        self.thorns
            .values()
            .filter_map(|th| {
                let pos = time
                    .and_then(|time| th.trail.at(time))
                    .unwrap_or(th.position);
                inside(pos).then_some((th.id, pos))
            })
            .collect()
    }

    /// Live thorns (at `time`) inside the cone from `eye` along `aim`, `reach` long and
    /// `half_angle` radians wide, in line of sight of the eye. A thorn counts as a ball of
    /// `ricochet_radius` (like a ray passing it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn thorns_in_cone(
        &self,
        t: &Tuning,
        time: Option<f64>,
        eye: Vec3,
        aim: Vec3,
        reach: f32,
        half_angle: f32,
    ) -> Vec<(NetId, Vec3)> {
        let pad = t.combat.thorn.ricochet_radius.max(0.0);
        let world = &self.collision.world;
        self.thorns_where(time, |pos| {
            let to = pos - eye;
            let d = to.length();
            if d - pad > reach {
                return false;
            }
            let covers = if d > pad {
                (pad / d).asin()
            } else {
                std::f32::consts::PI
            };
            to.normalize_or(aim).angle_between(aim) - covers <= half_angle
                && !world.line_blocked(eye, pos, MASK_SHOTS)
        })
    }

    /// Live thorns (at `time`) within `radius` of `center` (their `ricochet_radius` counts),
    /// in line of sight of it.
    pub(crate) fn thorns_in_sphere(
        &self,
        t: &Tuning,
        time: Option<f64>,
        center: Vec3,
        radius: f32,
    ) -> Vec<(NetId, Vec3)> {
        let reach = radius + t.combat.thorn.ricochet_radius.max(0.0);
        let world = &self.collision.world;
        self.thorns_where(time, |pos| {
            pos.distance_squared(center) <= reach * reach
                && !world.line_blocked(center, pos, MASK_SHOTS)
        })
    }

    /// An area attack of `shooter` (a swing, a cone, a nova, a blast from `from`) reached
    /// `thorns`: each one still live shatters and is flung at its ricochet target as a ray of
    /// `damage` (one bounce: × `damage_multiplier`, a ricochet hit) as `attack` — without a
    /// target, on along the line from `from` through it. A [`SimEvent::Ricochet`] each.
    /// Returns the hits.
    pub(crate) fn fling_thorns(
        &mut self,
        t: &Tuning,
        shooter: Owner,
        attack: Attack,
        damage: f32,
        from: Vec3,
        thorns: &[(NetId, Vec3)],
    ) -> u32 {
        let mut hits = 0;
        for &(id, at) in thorns {
            // An earlier one's ray may have chained through it.
            if !self.thorns.contains_key(&id) {
                continue;
            }
            let dir = (at - from).normalize_or(Vec3::NEG_Z);
            let spec = ShotSpec::new(at, dir, t.combat.hitscan_range, damage, attack);
            let r = self.relay_leg(t, shooter, id, at, dir, &spec, 0);
            hits += r.hits;
            self.events.push(SimEvent::Ricochet {
                shooter,
                attack,
                points: r.ricochet,
                projectile: None,
            });
        }
        hits
    }

    /// Player `id`'s swing, spray or gust (`attack`, `damage` per hit) along their aim, as they
    /// saw it (`input`'s view time): the thorns in its cone (`reach`, full aperture
    /// `cone_degrees`) are flung on. Returns the hits.
    pub(crate) fn relay_cone(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        input: &PlayerInput,
        (reach, cone_degrees): (f32, f32),
        damage: f32,
        attack: Attack,
    ) -> u32 {
        let Some(p) = self.players.get(&id) else {
            return 0;
        };
        if self.thorns.is_empty() {
            return 0;
        }
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let time = self.view_time(t, id, input);
        let half = (cone_degrees * 0.5).to_radians();
        let thorns = self.thorns_in_cone(t, Some(time), eye, aim, reach, half);
        self.fling_thorns(t, Owner::Player(id), attack, damage, eye, &thorns)
    }

    /// Player `id`'s nova or Whirl (`attack`, `damage` per hit) around `center`, as they saw
    /// it: the thorns within `radius` are flung on. Returns the hits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn relay_nova(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        input: &PlayerInput,
        center: Vec3,
        radius: f32,
        damage: f32,
        attack: Attack,
    ) -> u32 {
        if self.thorns.is_empty() {
            return 0;
        }
        let time = self.view_time(t, id, input);
        let thorns = self.thorns_in_sphere(t, Some(time), center, radius);
        self.fling_thorns(t, Owner::Player(id), attack, damage, center, &thorns)
    }

    /// A blast of `owner` at `center`: a player's flings the thorns within `radius` on as rays
    /// of `thorn.relay_damage` (current positions, like the blast's).
    pub(crate) fn relay_blast(
        &mut self,
        t: &Tuning,
        owner: Owner,
        attack: Attack,
        center: Vec3,
        radius: f32,
    ) {
        if !matches!(owner, Owner::Player(_)) || self.thorns.is_empty() {
            return;
        }
        let thorns = self.thorns_in_sphere(t, None, center, radius);
        let damage = t.combat.thorn.relay_damage;
        self.fling_thorns(t, owner, attack, damage, center, &thorns);
    }

    /// Projectile step: does player projectile `p`, moving `motion` this tick (the level
    /// `world_f` of the way along), come within `ricochet_radius` (+ its radius) of a live
    /// thorn before the level and before a target it would hit? Then the thorn shatters and `p`
    /// turns where it reached it onto a straight line at the thorn's ricochet target (on
    /// straight without one) at its speed, payload kept: no gravity, one ricochet more (a bomb
    /// goes off at its next contact). The rest of the tick's motion is dropped. `true` if it
    /// turned.
    pub(crate) fn relay_projectile(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        motion: Vec3,
        world_f: f32,
        boxes: &[Hitbox],
    ) -> bool {
        // (An orbiting Wisp circles its owner: it reaches nothing.)
        if self.thorns.is_empty()
            || !matches!(p.owner, Owner::Player(_))
            || !p.kind.relays_off_thorns()
            || p.ability_flags & projectile_flags::ORBITING != 0
        {
            return false;
        }
        let reach = t.combat.thorn.ricochet_radius.max(0.0) + p.radius;
        let first = self
            .thorns
            .values()
            .filter_map(|th| {
                let f = sphere_entry(p.position, motion, th.position, reach)?;
                (f <= world_f).then_some((f, th.id, th.position))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((f, id, at)) = first else {
            return false;
        };
        // A target it would hit before the thorn takes the hit (the normal test).
        if p.kind.hits_targets() {
            let target_radius = p.radius + projectile_target_pad(&t.combat, p.owner);
            let before = motion * f;
            let hit_first = boxes
                .iter()
                .filter(|h| self.can_hit(p.owner, h.target))
                .any(|h| {
                    let pads = self.hit_pads(t, p.owner, h.target);
                    h.sweep_sphere_padded(p.position, before, target_radius, pads)
                        .is_some()
                });
            if hit_first {
                return false;
            }
        }
        let from = p.position + motion * f;
        let along = motion.normalize_or(p.velocity.normalize_or(Vec3::NEG_Z));
        self.shatter_thorn(id);
        let aim = self.ricochet_target(t, at, p.owner);
        let dir = aim.map_or(along, |aim| (aim - from).normalize_or(along));
        let speed = p.velocity.length().max(1.0);
        p.position = from;
        p.velocity = dir * speed;
        p.gravity = 0.0;
        p.ricochets = p.ricochets.saturating_add(1);
        if p.kind == ProjectileKind::Bomb {
            p.bounces_left = 0;
        }
        p.contacts.clear();
        p.relaunch(self.tick);
        self.events.push(SimEvent::Ricochet {
            shooter: p.owner,
            attack: p.attack,
            points: vec![at, aim.unwrap_or(from + dir * 10.0)],
            projectile: Some(p.id),
        });
        true
    }
}

impl ProjectileKind {
    /// Turns at a live thorn it reaches (every player attack; not the thrown knacks).
    pub fn relays_off_thorns(self) -> bool {
        !matches!(self, ProjectileKind::Snare | ProjectileKind::SootPot)
    }
}

/// Fraction of `motion` (from `from`) at which a ball of `radius` round `center` is first
/// reached (0: already inside); `None` if this segment never reaches it.
fn sphere_entry(from: Vec3, motion: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let to = center - from;
    let c = to.length_squared() - radius * radius;
    if c <= 0.0 {
        return Some(0.0);
    }
    let len2 = motion.length_squared();
    let b = to.dot(motion);
    if len2 < 1e-12 || b <= 0.0 {
        return None;
    }
    let disc = b * b - len2 * c;
    if disc < 0.0 {
        return None;
    }
    let f = (b - disc.sqrt()) / len2;
    (f <= 1.0).then_some(f.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spheres_are_entered_where_the_path_first_reaches_them() {
        let c = Vec3::new(0.0, 0.0, -10.0);
        let m = Vec3::new(0.0, 0.0, -20.0);
        // Straight at it: reached 1 m short of the centre.
        let f = sphere_entry(Vec3::ZERO, m, c, 1.0).unwrap();
        assert!((f - 9.0 / 20.0).abs() < 1e-5, "{f}");
        // Already inside: at once. Passing beside it within the radius; past it; short of it;
        // moving away.
        assert_eq!(sphere_entry(c, m, c, 1.0), Some(0.0));
        assert!(sphere_entry(Vec3::new(0.8, 0.0, 0.0), m, c, 1.0).is_some());
        assert!(sphere_entry(Vec3::new(1.2, 0.0, 0.0), m, c, 1.0).is_none());
        assert!(sphere_entry(Vec3::ZERO, m * 0.4, c, 1.0).is_none());
        assert!(sphere_entry(Vec3::ZERO, -m, c, 1.0).is_none());
    }
}
