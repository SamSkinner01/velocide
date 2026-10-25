//! Camera-feel helpers for the client. Pure functions of the movement state, so the predicted
//! local player and spectated players get the same feel. The client smooths towards these
//! targets; nothing here affects the simulation.

use glam::Vec3;

use super::{MoveMode, MoveState};
use crate::tuning::MovementTuning;

/// Target camera roll in **radians**: positive rolls the view counter-clockwise (the head
/// tilts to the left).
///
/// - Sliding: a slight lean towards the side you are sliding to (left if sliding straight).
/// - Wall-clinging / wall-running in the air: tilt away from the wall.
pub fn view_roll(state: &MoveState, yaw: f32, t: &MovementTuning) -> f32 {
    let (s, c) = yaw.sin_cos();
    let right = Vec3::new(c, 0.0, -s);
    let vh = Vec3::new(state.velocity.x, 0.0, state.velocity.z);

    if !state.on_ground && state.wall_normal != Vec3::ZERO {
        let along = (vh - state.wall_normal * vh.dot(state.wall_normal)).length();
        if state.wall_clinging || along > 6.0 {
            // Wall on the right => normal points left (-right) => roll left (positive).
            let side = -state.wall_normal.dot(right);
            return t.wall_tilt_degrees.to_radians() * side.clamp(-1.0, 1.0);
        }
    }
    if state.mode == MoveMode::Slide {
        let lateral = vh.normalize_or_zero().dot(right);
        let side = if lateral.abs() < 0.2 {
            1.0
        } else {
            -lateral.signum()
        };
        return t.slide_tilt_degrees.to_radians() * side;
    }
    0.0
}

/// Extra field of view in **degrees**: ramps from 0 at `ground_speed` to `fov_kick_degrees` at
/// `fov_kick_speed` (smoothstep on total speed).
pub fn fov_kick(state: &MoveState, t: &MovementTuning) -> f32 {
    let lo = t.ground_speed;
    let hi = t.fov_kick_speed.max(lo + 1e-3);
    let x = ((state.velocity.length() - lo) / (hi - lo)).clamp(0.0, 1.0);
    t.fov_kick_degrees * x * x * (3.0 - 2.0 * x)
}

/// Eye dip in metres for a landing at `landing_speed` (from `MoveEvents::landed`). Soft
/// landings (below ~4 m/s) don't dip; a 30 m/s impact gives the full `landing_dip`.
pub fn landing_dip(landing_speed: f32, t: &MovementTuning) -> f32 {
    t.landing_dip * ((landing_speed - 4.0) / 26.0).clamp(0.0, 1.0)
}
