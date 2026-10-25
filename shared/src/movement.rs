//! Deterministic kinematic character controller.
//!
//! PLACEHOLDER: walk + jump + gravity with collide-and-slide. The full move set (dash, slide,
//! slam, wall jumps, coyote time...) replaces this, keeping the public API:
//! [`MoveState`], [`MoveEnv`], [`MoveEvents`], [`step`] and the `MoveState` helper methods.
//!
//! External impulses (explosion knockback, enemy hits) are applied by the server directly to
//! `MoveState::velocity` between steps; clients pick them up through reconciliation.

use engine::physics::{CollisionWorld, MASK_MOVEMENT};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::input::{Buttons, PlayerInput};
use crate::level::JumpPad;
use crate::tuning::MovementTuning;

/// The static world a movement step runs against. Must be identical on client and server.
#[derive(Clone, Copy)]
pub struct MoveEnv<'a> {
    pub world: &'a CollisionWorld,
    pub jump_pads: &'a [JumpPad],
}

/// Everything that affects future movement. Predicted on the client, authoritative on the
/// server, and sent in full to the owning client for reconciliation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoveState {
    /// Feet position (bottom of the capsule).
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    pub prev_buttons: Buttons,
}

/// Things that happened during one movement step (for sounds, effects and gameplay hooks).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveEvents {
    pub jumped: bool,
    /// Landed this tick with the given downward speed.
    pub landed: Option<f32>,
}

impl MoveState {
    pub fn spawn(position: Vec3) -> Self {
        Self {
            position,
            ..Default::default()
        }
    }

    pub fn height(&self, t: &MovementTuning) -> f32 {
        t.standing_height
    }

    /// Capsule centre, inner half-height and radius for the current stance.
    pub fn capsule(&self, t: &MovementTuning) -> (Vec3, f32, f32) {
        let h = self.height(t);
        let half_height = (h * 0.5 - t.radius).max(0.0);
        (self.position + Vec3::Y * h * 0.5, half_height, t.radius)
    }

    pub fn eye_position(&self, t: &MovementTuning) -> Vec3 {
        self.position + Vec3::Y * t.eye_height
    }

    /// Dash invulnerability frames.
    pub fn is_invulnerable(&self) -> bool {
        false
    }
}

const SKIN: f32 = 0.01;

/// Advances one fixed tick.
pub fn step(
    state: &mut MoveState,
    input: &PlayerInput,
    env: &MoveEnv,
    t: &MovementTuning,
    dt: f32,
) -> MoveEvents {
    let world = env.world;
    let mut events = MoveEvents::default();
    let wish = input.wish_dir();

    if state.on_ground {
        let target = wish * t.ground_speed;
        state.velocity.x = target.x;
        state.velocity.z = target.z;
        if input.buttons.contains(Buttons::JUMP) {
            state.velocity.y = t.jump_speed;
            state.on_ground = false;
            events.jumped = true;
        }
    } else {
        state.velocity += wish * t.air_accel * dt;
    }
    state.velocity.y = (state.velocity.y - t.gravity * dt).max(-t.max_fall_speed);

    let fall_speed = -state.velocity.y;
    move_and_slide(state, world, t, state.velocity * dt);

    let (center, hh, r) = state.capsule(t);
    let ground = world.cast_capsule(center, hh, r, Vec3::NEG_Y * 0.05, SKIN, MASK_MOVEMENT);
    let was_on_ground = state.on_ground;
    state.on_ground = state.velocity.y <= 0.0 && ground.is_some_and(|g| g.normal.y > 0.7);
    if state.on_ground {
        state.velocity.y = 0.0;
        if !was_on_ground {
            events.landed = Some(fall_speed);
        }
    }
    state.prev_buttons = input.buttons;
    events
}

fn move_and_slide(state: &mut MoveState, world: &CollisionWorld, t: &MovementTuning, motion: Vec3) {
    let mut remaining = motion;
    for _ in 0..4 {
        if remaining.length_squared() < 1e-10 {
            break;
        }
        let (center, hh, r) = state.capsule(t);
        match world.cast_capsule(center, hh, r, remaining, SKIN, MASK_MOVEMENT) {
            None => {
                state.position += remaining;
                break;
            }
            Some(hit) => {
                state.position += remaining * hit.fraction;
                let left = remaining * (1.0 - hit.fraction);
                remaining = left - hit.normal * left.dot(hit.normal);
                let into = state.velocity.dot(hit.normal);
                if into < 0.0 {
                    state.velocity -= hit.normal * into;
                }
            }
        }
    }
}
