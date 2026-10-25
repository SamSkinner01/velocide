//! First-person camera: sits at the predicted, smoothed eye position and layers the camera
//! feel on top: speed FOV kick, view roll (slides / wall running), landing dip, stair
//! smoothing, slide eye height, the low view while downed and screen shake.
//!
//! Everything here is presentation: it reads the predicted [`MoveState`] and the server's
//! [`MovementTuning`] (via `shared::movement::{view_roll, fov_kick, landing_dip}`) and never
//! feeds back into the simulation.

use engine::gfx::Camera;
use glam::{Quat, Vec3};
use shared::movement::{MoveState, fov_kick, landing_dip, view_roll};
use shared::tuning::MovementTuning;

use super::feedback::ShakeOffset;
use crate::settings::Settings;

/// Exponential smoothing rates (1/s).
const ROLL_RATE: f32 = 9.0;
const FOV_RATE: f32 = 6.0;
const EYE_RATE: f32 = 16.0;
const STEP_RATE: f32 = 14.0;
/// Landing dip spring (rad/s): the dip peaks after 1/ω ≈ 70 ms and recovers in ~0.3 s.
const DIP_OMEGA: f32 = 14.0;
/// Dips smaller than this are ignored (tiny hops).
const MIN_DIP: f32 = 0.01;
/// Eye height while downed in co-op (lying on the floor, waiting for a revive).
pub const DOWNED_EYE_HEIGHT: f32 = 0.6;
/// How fast the view sinks when going down (1/s): slower than crouching, it reads as a fall.
const DOWNED_EYE_RATE: f32 = 5.0;

fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-rate * dt).exp())
}

/// Smoothed camera feel state.
#[derive(Clone, Debug, Default)]
pub struct CameraRig {
    roll: f32,
    fov_kick: f32,
    eye_height: Option<f32>,
    step_offset: f32,
    dip: f32,
    dip_velocity: f32,
}

/// What the camera needs from the current frame.
pub struct CameraFrame<'a> {
    /// Predicted, interpolated feet position (`NetClient::local_render_position`).
    pub feet: Vec3,
    pub state: &'a MoveState,
    pub tuning: &'a MovementTuning,
    pub yaw: f32,
    pub pitch: f32,
    pub settings: &'a Settings,
    pub shake: ShakeOffset,
    /// Downed (co-op): the view drops to [`DOWNED_EYE_HEIGHT`].
    pub downed: bool,
    /// Real frame time.
    pub dt: f32,
}

impl CameraRig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget smoothing (spawn, respawn, teleport). The eye height is kept (it is relative to
    /// the feet), so getting up after a revive or a respawn rises smoothly.
    pub fn reset(&mut self) {
        *self = Self {
            eye_height: self.eye_height,
            ..Self::default()
        };
    }

    /// A landing at `speed` m/s (`MoveEvents::landed`): kick the dip spring.
    pub fn on_landed(&mut self, speed: f32, tuning: &MovementTuning) {
        let dip = landing_dip(speed, tuning);
        if dip >= MIN_DIP {
            // Critically damped spring x(t) = v0·t·e^(-ωt) peaks at v0/(ωe).
            self.dip_velocity -= dip * DIP_OMEGA * std::f32::consts::E;
        }
    }

    /// Climbed a step (`MoveEvents::stepped_up`): start the eye below and let it catch up.
    pub fn on_stepped_up(&mut self, height: f32) {
        if height.is_finite() {
            self.step_offset = (self.step_offset - height).max(-0.6);
        }
    }

    /// Current landing dip (metres, <= 0); the viewmodel follows it.
    pub fn dip(&self) -> f32 {
        self.dip
    }

    pub fn roll(&self) -> f32 {
        self.roll
    }

    pub fn update(&mut self, f: &CameraFrame) -> Camera {
        let dt = f.dt.clamp(0.0, 0.1);
        let t = f.tuning;

        let roll_target = view_roll(f.state, f.yaw, t);
        self.roll = approach(self.roll, roll_target, ROLL_RATE, dt);
        self.fov_kick = approach(self.fov_kick, fov_kick(f.state, t), FOV_RATE, dt);

        let (eye_target, eye_rate) = if f.downed {
            (DOWNED_EYE_HEIGHT, DOWNED_EYE_RATE)
        } else if f.state.crouched {
            (t.slide_eye_height, EYE_RATE)
        } else {
            (t.eye_height, EYE_RATE)
        };
        let eye = match self.eye_height {
            Some(e) => approach(e, eye_target, eye_rate, dt),
            None => eye_target,
        };
        self.eye_height = Some(eye);
        self.step_offset = approach(self.step_offset, 0.0, STEP_RATE, dt);

        // Critically damped spring towards 0, integrated in small steps for stability.
        let steps = (dt / 0.004).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        for _ in 0..steps {
            let accel = -DIP_OMEGA * DIP_OMEGA * self.dip - 2.0 * DIP_OMEGA * self.dip_velocity;
            self.dip_velocity += accel * h;
            self.dip += self.dip_velocity * h;
        }
        self.dip = self.dip.clamp(-0.6, 0.2);

        let rotation = Camera::rotation_from_angles(
            f.yaw + f.shake.angles.x,
            (f.pitch + f.shake.angles.y).clamp(-1.56, 1.56),
            self.roll + f.shake.angles.z,
        );
        let position =
            f.feet + Vec3::Y * (eye + self.step_offset + self.dip) + rotation * f.shake.offset;
        Camera {
            position,
            rotation,
            fov_y_degrees: f.settings.vertical_fov(self.fov_kick),
            near: 0.05,
            far: 600.0,
        }
    }
}

/// Camera rotation without roll or shake (for listeners and look-direction math).
pub fn look_rotation(yaw: f32, pitch: f32) -> Quat {
    Camera::rotation_from_angles(yaw, pitch, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame<'a>(
        state: &'a MoveState,
        tuning: &'a MovementTuning,
        s: &'a Settings,
    ) -> CameraFrame<'a> {
        CameraFrame {
            feet: Vec3::ZERO,
            state,
            tuning,
            yaw: 0.0,
            pitch: 0.0,
            settings: s,
            shake: ShakeOffset::default(),
            downed: false,
            dt: 1.0 / 144.0,
        }
    }

    #[test]
    fn hard_landing_dips_then_recovers() {
        let tuning = MovementTuning::default();
        let settings = Settings::default();
        let state = MoveState {
            on_ground: true,
            ..Default::default()
        };
        let mut rig = CameraRig::new();
        let rest = rig.update(&frame(&state, &tuning, &settings)).position.y;
        assert!((rest - tuning.eye_height).abs() < 1e-4);
        rig.on_landed(30.0, &tuning);
        let mut lowest = f32::MAX;
        for _ in 0..30 {
            lowest = lowest.min(rig.update(&frame(&state, &tuning, &settings)).position.y);
        }
        let dip = rest - lowest;
        assert!(
            (dip - tuning.landing_dip).abs() < 0.05,
            "peak dip {dip} ~ {}",
            tuning.landing_dip
        );
        for _ in 0..300 {
            rig.update(&frame(&state, &tuning, &settings));
        }
        assert!(rig.dip().abs() < 0.005);
    }

    #[test]
    fn tiny_landings_do_not_dip() {
        let tuning = MovementTuning::default();
        let mut rig = CameraRig::new();
        rig.on_landed(3.0, &tuning);
        assert_eq!(rig.dip_velocity, 0.0);
    }

    #[test]
    fn slide_lowers_the_eye_smoothly() {
        let tuning = MovementTuning::default();
        let settings = Settings::default();
        let standing = MoveState::default();
        let crouched = MoveState {
            crouched: true,
            ..Default::default()
        };
        let mut rig = CameraRig::new();
        rig.update(&frame(&standing, &tuning, &settings));
        let first = rig.update(&frame(&crouched, &tuning, &settings)).position.y;
        assert!(first < tuning.eye_height && first > tuning.slide_eye_height);
        for _ in 0..200 {
            rig.update(&frame(&crouched, &tuning, &settings));
        }
        let settled = rig.update(&frame(&crouched, &tuning, &settings)).position.y;
        assert!((settled - tuning.slide_eye_height).abs() < 1e-3);
    }

    #[test]
    fn downed_lowers_the_eye_and_getting_up_restores_it() {
        let tuning = MovementTuning::default();
        let settings = Settings::default();
        let state = MoveState::default();
        let mut rig = CameraRig::new();
        let standing = rig.update(&frame(&state, &tuning, &settings)).position.y;
        assert!((standing - tuning.eye_height).abs() < 1e-4);
        let downed = |rig: &mut CameraRig| {
            let mut f = frame(&state, &tuning, &settings);
            f.downed = true;
            rig.update(&f).position.y
        };
        // Sinks smoothly, not in one frame.
        let first = downed(&mut rig);
        assert!(first < standing && first > DOWNED_EYE_HEIGHT + 0.5, "{first}");
        let mut y = first;
        for _ in 0..300 {
            y = downed(&mut rig);
        }
        assert!((y - DOWNED_EYE_HEIGHT).abs() < 1e-3, "{y}");
        // Revived / respawned: the Spawned event resets the rig; the view rises smoothly back
        // to standing height.
        rig.reset();
        let first = rig.update(&frame(&state, &tuning, &settings)).position.y;
        assert!(first > DOWNED_EYE_HEIGHT && first < standing, "{first}");
        for _ in 0..300 {
            y = rig.update(&frame(&state, &tuning, &settings)).position.y;
        }
        assert!((y - tuning.eye_height).abs() < 1e-3, "{y}");
    }
}
