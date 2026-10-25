//! `--bot`: an auto-pilot that plays like a (slightly unhinged) human so multi-client tests
//! exercise the whole move set: it runs, wanders, steers away from walls, jumps, dashes,
//! slides, slams, wall-jumps when it gets the chance and switches weapons. Deterministic for a
//! given seed (the player name), driven once per fixed tick.

use engine::physics::{CollisionWorld, MASK_MOVEMENT};
use glam::Vec3;
use shared::input::{Buttons, look_dir};
use shared::movement::{MoveMode, MoveState};

use crate::controls::Controls;

#[derive(Clone, Debug)]
pub struct Bot {
    seed: f32,
    t: f32,
    tick: u32,
    crouch_ticks: u32,
    turn: f32,
}

fn hash_str(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

impl Bot {
    pub fn new(seed_name: &str) -> Self {
        Self {
            seed: (hash_str(seed_name) % 10_000) as f32 * 0.137,
            t: 0.0,
            tick: 0,
            crouch_ticks: 0,
            turn: 0.0,
        }
    }

    /// Free distance along `dir` from `from` (capped at `max`).
    fn clearance(world: &CollisionWorld, from: Vec3, dir: Vec3, max: f32) -> f32 {
        world
            .cast_ray(from, dir, max, MASK_MOVEMENT)
            .map_or(max, |h| h.distance)
    }

    /// One fixed tick: steers `controls` (view angles, movement override, weapon switching).
    /// `target` is the nearest other player: the bot hunts it (circling when close) so test
    /// clients keep meeting each other.
    pub fn tick(
        &mut self,
        dt: f32,
        state: Option<&MoveState>,
        world: Option<&CollisionWorld>,
        target: Option<Vec3>,
        controls: &mut Controls,
    ) {
        self.t += dt;
        self.tick = self.tick.wrapping_add(1);
        let t = self.t + self.seed;
        let period = |secs: f32| (secs / dt).round().max(1.0) as u32;
        let phase = (self.seed * 37.0) as u32;
        let on = |secs: f32, offset: u32| (self.tick + phase + offset).is_multiple_of(period(secs));

        // Wander: smooth pseudo-random turning.
        let mut turn = (t * 0.71).sin() * 1.3 + (t * 0.23 + 1.7).sin() * 0.9;
        let mut buttons = Buttons::NONE;
        let mut right = (t * 0.53).sin() * 0.35;

        if let (Some(state), Some(target)) = (state, target) {
            // Hunt: turn towards the other player, orbiting once close.
            let to = target - state.position;
            let dist = Vec3::new(to.x, 0.0, to.z).length();
            if dist > 1.0 {
                let mut want = (-to.x).atan2(-to.z);
                if dist < 14.0 {
                    want += 1.2; // circle-strafe instead of ramming
                }
                let err = (want - controls.yaw + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                turn = turn * 0.3 + err.clamp(-1.0, 1.0) * 3.0;
            }
        }
        if let (Some(state), Some(world)) = (state, world) {
            // Avoid walls: look ahead along the facing direction at chest height and turn
            // towards whichever side is more open.
            let speed = Vec3::new(state.velocity.x, 0.0, state.velocity.z).length();
            let chest = state.position + Vec3::Y * 1.0;
            let fwd = look_dir(controls.yaw, 0.0);
            let reach = 4.0 + speed * 0.45;
            let ahead = Self::clearance(world, chest, fwd, reach);
            if ahead < reach {
                let left = look_dir(controls.yaw + 0.9, 0.0);
                let rightd = look_dir(controls.yaw - 0.9, 0.0);
                let l = Self::clearance(world, chest, left, 30.0);
                let r = Self::clearance(world, chest, rightd, 30.0);
                let urgency = 1.0 - ahead / reach;
                turn = if l >= r { 1.0 } else { -1.0 } * (2.5 + 5.0 * urgency);
                right *= 0.3;
            }
            // Moves.
            if on(1.35, 0) {
                buttons.set(Buttons::JUMP, true);
            }
            if on(2.6, 17) && state.stamina_used < 2.0 {
                buttons.set(Buttons::DASH, true);
            }
            if state.on_ground && on(4.1, 40) {
                self.crouch_ticks = period(0.9);
            }
            if !state.on_ground && state.air_time > 0.45 && on(5.3, 7) {
                // Slam (a crouch press in the air).
                self.crouch_ticks = 2;
            }
            if state.mode == MoveMode::Slide && on(3.0, 5) {
                buttons.set(Buttons::JUMP, true); // slide-jump
            }
            if state.wall_normal != Vec3::ZERO && state.wall_jumps_used < 2 && on(0.5, 3) {
                buttons.set(Buttons::JUMP, true); // wall jump when near a wall
            }
        }
        if self.crouch_ticks > 0 {
            self.crouch_ticks -= 1;
            buttons.set(Buttons::CROUCH, true);
        }
        if on(3.3, 11) {
            let slot = ((t * 3.1).sin().abs() * 4.0) as u8 % 4;
            controls.weapons.press_slot(slot);
        }
        // Smooth the turn rate so the camera doesn't jerk.
        self.turn += (turn - self.turn) * (dt * 6.0).min(1.0);
        controls.yaw += self.turn * dt;
        let pitch = (t * 0.4).sin() * 0.15 - 0.05;
        controls.set_view(controls.yaw, pitch);
        controls.override_move = Some((1.0, right, buttons));
    }
}
