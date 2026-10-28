//! First-person camera: sits at the predicted, smoothed eye position and layers the camera
//! feel on top: speed FOV kick, view roll (slides / wall running), landing dip, stair
//! smoothing, slide eye height, the low view while downed and screen shake, plus the weapon
//! feel (`docs/feel.md`): a short view punch per shot, the bow's zoom at full draw and the
//! Lamplighter's scope (docs/rework.md), which also slows the mouse to match.
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
/// View punch spring (rad/s): a shot's punch peaks after ~1/ω ≈ 25 ms and settles in ~0.15 s.
const PUNCH_OMEGA: f32 = 40.0;
/// Largest view punch (radians, ~1.7°), however many shots stack.
const MAX_PUNCH: f32 = 0.03;
/// Zoom smoothing (1/s).
const ZOOM_RATE: f32 = 10.0;
/// The narrowest view a zoom can ask for (tan(fov/2) × 0.3; the Lamplighter's full focus is
/// × 0.4).
pub const MAX_ZOOM: f32 = 0.3;

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
    /// View punch (pitch up, radians) and its spring velocity: presentation only, the aim
    /// stays where the player put it.
    punch: f32,
    punch_velocity: f32,
    /// How much the field of view is narrowed (0 = none; the bow's full draw zooms in a
    /// little: 0.1 = tan(fov/2) × 0.9; the Lamplighter's full focus 0.6) and its target.
    zoom_in: f32,
    zoom_in_target: f32,
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
    /// Downed (co-op) or dead: the view drops to [`DOWNED_EYE_HEIGHT`].
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

    /// A shot's view punch: `amount` 0..2 (the shot's kick squared; a scattergun's blast more).
    /// The view jumps up a fraction of a degree (1: ~0.7°) and springs back; the aim (and so the
    /// shot) is unaffected.
    pub fn punch(&mut self, amount: f32) {
        if amount.is_finite() && amount > 0.0 {
            self.punch_velocity += amount.min(2.0) * 0.012 * PUNCH_OMEGA * std::f32::consts::E;
        }
    }

    /// Field-of-view scale to ease towards (1 = no zoom; 0.9 = 10 % narrower; the most is
    /// [`MAX_ZOOM`]).
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom_in_target = if zoom.is_finite() {
            1.0 - zoom.clamp(MAX_ZOOM, 1.0)
        } else {
            0.0
        };
    }

    /// The current zoom as a scale of tan(fov/2) (1 = none). Mouse look is multiplied by it, so
    /// the crosshair crosses the zoomed view as fast as the unzoomed one (`Controls::look`).
    pub fn zoom_scale(&self) -> f32 {
        1.0 - self.zoom_in
    }

    /// Current view punch (radians, >= 0).
    pub fn view_punch(&self) -> f32 {
        self.punch
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
        // View punch: the same critically damped spring, stiffer.
        for _ in 0..steps {
            let accel =
                -PUNCH_OMEGA * PUNCH_OMEGA * self.punch - 2.0 * PUNCH_OMEGA * self.punch_velocity;
            self.punch_velocity += accel * h;
            self.punch += self.punch_velocity * h;
        }
        self.punch = self.punch.clamp(-MAX_PUNCH, MAX_PUNCH);
        if !self.punch.is_finite() || !self.punch_velocity.is_finite() {
            (self.punch, self.punch_velocity) = (0.0, 0.0);
        }
        self.zoom_in = approach(self.zoom_in, self.zoom_in_target, ZOOM_RATE, dt);

        let rotation = Camera::rotation_from_angles(
            f.yaw + f.shake.angles.x,
            (f.pitch + f.shake.angles.y + self.punch).clamp(-1.56, 1.56),
            self.roll + f.shake.angles.z,
        );
        let position =
            f.feet + Vec3::Y * (eye + self.step_offset + self.dip) + rotation * f.shake.offset;
        // Zoom narrows the view: tan(fov/2) scales by `zoom`.
        let fov = f.settings.vertical_fov(self.fov_kick);
        let fov_y_degrees = if self.zoom_in > 0.001 {
            let k = 1.0 - self.zoom_in;
            2.0 * ((fov * 0.5).to_radians().tan() * k).atan().to_degrees()
        } else {
            fov
        };
        Camera {
            position,
            rotation,
            fov_y_degrees,
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
    fn shots_punch_the_view_and_it_springs_back() {
        let tuning = MovementTuning::default();
        let settings = Settings::default();
        let state = MoveState::default();
        let mut rig = CameraRig::new();
        let rest = rig.update(&frame(&state, &tuning, &settings)).rotation;
        rig.punch(1.0);
        let mut peak = 0.0f32;
        for _ in 0..8 {
            rig.update(&frame(&state, &tuning, &settings));
            peak = peak.max(rig.view_punch());
        }
        assert!(peak > 0.005 && peak <= MAX_PUNCH, "{peak}");
        for _ in 0..144 {
            rig.update(&frame(&state, &tuning, &settings));
        }
        let back = rig.update(&frame(&state, &tuning, &settings)).rotation;
        assert!(rig.view_punch().abs() < 1e-3 && back.angle_between(rest) < 1e-3);
        // A burst of shots never punches past the cap.
        for _ in 0..50 {
            rig.punch(1.0);
            rig.update(&frame(&state, &tuning, &settings));
            assert!(rig.view_punch() <= MAX_PUNCH);
        }
        // A blast (2) punches about twice as far as a full kick, inside the cap.
        let mut rig = CameraRig::new();
        rig.punch(2.0);
        let mut heavy = 0.0f32;
        for _ in 0..8 {
            rig.update(&frame(&state, &tuning, &settings));
            heavy = heavy.max(rig.view_punch());
        }
        assert!(
            heavy > 1.8 * peak && heavy <= MAX_PUNCH,
            "{heavy} vs {peak}"
        );
    }

    #[test]
    fn zoom_narrows_the_view_smoothly() {
        let tuning = MovementTuning::default();
        let settings = Settings::default();
        let state = MoveState::default();
        let mut rig = CameraRig::new();
        let wide = rig.update(&frame(&state, &tuning, &settings)).fov_y_degrees;
        rig.set_zoom(0.9);
        let first = rig.update(&frame(&state, &tuning, &settings)).fov_y_degrees;
        assert!(first < wide && first > wide * 0.95, "{first} vs {wide}");
        let mut fov = first;
        for _ in 0..300 {
            fov = rig.update(&frame(&state, &tuning, &settings)).fov_y_degrees;
        }
        let expected = 2.0 * ((wide * 0.5).to_radians().tan() * 0.9).atan().to_degrees();
        assert!((fov - expected).abs() < 0.05, "{fov} vs {expected}");
        rig.set_zoom(f32::NAN);
        for _ in 0..300 {
            fov = rig.update(&frame(&state, &tuning, &settings)).fov_y_degrees;
        }
        assert!((fov - wide).abs() < 0.05);
        // The Lamplighter's scope: tan(fov/2) × 0.4, and the mouse scale follows it.
        rig.set_zoom(0.4);
        for _ in 0..300 {
            fov = rig.update(&frame(&state, &tuning, &settings)).fov_y_degrees;
        }
        let scoped = 2.0 * ((wide * 0.5).to_radians().tan() * 0.4).atan().to_degrees();
        assert!((fov - scoped).abs() < 0.05, "{fov} vs {scoped}");
        assert!((rig.zoom_scale() - 0.4).abs() < 1e-3);
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
        assert!(
            first < standing && first > DOWNED_EYE_HEIGHT + 0.5,
            "{first}"
        );
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
