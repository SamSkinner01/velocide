//! Thorns: barbed glass relay spikes thrown with `THROW` (they replace the first arsenal's
//! prisms). A thorn flies nearly straight (`thorn.gravity`) and **plants itself** in the first
//! wall, floor or ceiling it hits, oriented along its flight, and lives `thorn.lifetime` there.
//! Hitting an enemy / hostile player in flight deals `thorn.damage` (hit assist applies); the
//! thorn then drops to the floor and lies there as a live thorn. Quill shots passing close to a
//! live thorn (flying, dropping, planted or lying) shatter it and ricochet (`hitscan.rs`).
//!
//! Replicated as `kind::THORN` by trajectory like projectiles (`position`/`velocity` at launch
//! tick `aux`, gravity `thorn.gravity`); while `PLANTED` (planted or lying) `position` is where
//! it sits and `velocity` its orientation. `owner` = thrower.

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::trail::Trail;
use super::{HitTarget, PlayerId, SimEvent, SimWorld};
use crate::protocol::{EntityState, NetId, kind, thorn_flags};
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

    /// Removes a thorn (a shot ricocheted off it).
    pub fn shatter_thorn(&mut self, id: NetId) {
        if let Some(th) = self.thorns.remove(&id) {
            self.events.push(SimEvent::ThornShattered {
                id,
                position: th.position,
            });
        }
    }
}
