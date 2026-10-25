//! Ricochet prisms: mirrored shards thrown with `THROW`. Ballistic, bounce off the level
//! (losing most of their speed) and come to rest; shatter when a hitscan shot ricochets off
//! them (see `hitscan.rs`) or after `prism.lifetime`.
//!
//! Replicated as `kind::PRISM` by trajectory like projectiles (`position`/`velocity` at launch
//! tick `aux`, gravity `prism.gravity`, none while `RESTING`); `owner` = thrower.

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::secs_to_ticks;
use super::trail::Trail;
use super::{PlayerId, SimEvent, SimWorld};
use crate::protocol::{EntityState, NetId, kind, prism_flags};
use crate::tuning::Tuning;

#[derive(Clone, Debug, PartialEq)]
pub struct Prism {
    pub id: NetId,
    pub owner: PlayerId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub launch_tick: u32,
    pub launch_position: Vec3,
    pub launch_velocity: Vec3,
    pub expire_tick: u32,
    /// Came to rest on the ground (no gravity, no movement).
    pub resting: bool,
    pub trail: Trail,
}

impl Prism {
    pub fn entity_state(&self) -> EntityState {
        EntityState {
            id: self.id,
            kind: kind::PRISM,
            owner: self.owner.0 as u16,
            position: self.launch_position,
            velocity: self.launch_velocity,
            flags: if self.resting {
                prism_flags::RESTING
            } else {
                0
            },
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
    /// Throws a prism for `owner` (the charge was already spent by the weapon state).
    pub fn spawn_prism(
        &mut self,
        t: &Tuning,
        owner: PlayerId,
        position: Vec3,
        velocity: Vec3,
    ) -> NetId {
        let id = self.alloc_net_id();
        let launch_tick = self.launch_tick_now();
        let life = secs_to_ticks(t.combat.prism.lifetime, self.tick_rate, 1) as u32;
        // Don't start inside a wall right in front of the thrower.
        let start = match self.players.get(&owner) {
            Some(p) => {
                let eye = p.movement.eye_position(&t.movement);
                if self.collision.world.line_blocked(eye, position, MASK_SHOTS) {
                    eye
                } else {
                    position
                }
            }
            None => position,
        };
        self.prisms.insert(
            id,
            Prism {
                id,
                owner,
                position: start,
                velocity,
                launch_tick,
                launch_position: start,
                launch_velocity: velocity,
                expire_tick: self.tick + life,
                resting: false,
                trail: Trail::new(),
            },
        );
        self.events
            .push(SimEvent::PrismThrown { player: owner, id });
        id
    }

    pub(crate) fn step_prisms(&mut self, t: &Tuning) {
        let now = self.tick;
        let dt = self.dt();
        let pt = &t.combat.prism;
        let expired: Vec<NetId> = self
            .prisms
            .values()
            .filter(|p| now >= p.expire_tick)
            .map(|p| p.id)
            .collect();
        for id in expired {
            self.prisms.remove(&id);
            self.events.push(SimEvent::PrismExpired { id });
        }
        let world = &self.collision.world;
        for p in self.prisms.values_mut() {
            if !p.resting {
                let motion = p.velocity * dt - Vec3::Y * (0.5 * pt.gravity * dt * dt);
                let new_velocity = p.velocity - Vec3::Y * (pt.gravity * dt);
                match world.cast_sphere(p.position, pt.radius, motion, MASK_SHOTS) {
                    Some(hit) => {
                        p.position += motion * hit.fraction + hit.normal * 0.02;
                        let n = hit.normal;
                        let v = new_velocity - n * (2.0 * new_velocity.dot(n));
                        p.velocity = v * pt.restitution;
                        if n.y > 0.7 && p.velocity.length() < 2.0 {
                            p.velocity = Vec3::ZERO;
                            p.resting = true;
                        }
                        p.relaunch(now);
                    }
                    None => {
                        p.position += motion;
                        p.velocity = new_velocity;
                    }
                }
            }
            p.trail.record(now, p.position);
        }
    }

    /// Removes a prism (a shot ricocheted off it).
    pub fn shatter_prism(&mut self, id: NetId) {
        if let Some(p) = self.prisms.remove(&id) {
            self.events.push(SimEvent::PrismShattered {
                id,
                position: p.position,
            });
        }
    }
}
