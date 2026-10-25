//! The held weapon in first person: `client::models` weapon + arm models drawn through
//! `ModelLibrary::draw_viewmodel` into the renderer's viewmodel pass (`FrameDesc::viewmodel`:
//! view space, own FOV, never clips into walls).
//!
//! This module keeps the animation state the models need ([`ViewmodelAnim`]: walk bob, look
//! sway, switch raise, fire kick, punch / prism-throw progress, Riveter barrel spin) and layers
//! the movement feel on top as an extra view-space transform: landing dip, slide tilt, dash
//! pull-back and a counter-roll against the camera tilt. Everything runs on the *visual*
//! clock (hitstop freezes it).
//!
//! Combat hooks: [`Viewmodel::fire`] (recoil kick), [`Viewmodel::punch`],
//! [`Viewmodel::throw_prism`], [`Viewmodel::set_charge`] (Lancet overcharge), `spinning`
//! (Riveter barrels) and [`Viewmodel::last_draw`] (view-space muzzle / fist for tracers and
//! sparks).

use engine::gfx::DrawItem;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::movement::{MoveMode, MoveState};

use crate::models::{BarrelSpin, ModelLibrary, Tint, ViewmodelAnim, ViewmodelDrawInfo};

const SWITCH_TIME: f32 = 0.12;
/// Ground speed giving full bob.
const RUN_SPEED: f32 = 16.0;

fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-rate * dt).exp())
}

/// Procedural weapon animation state.
#[derive(Default)]
pub struct Viewmodel {
    anim: ViewmodelAnim,
    spin: BarrelSpin,
    /// Riveter firing (barrel spin-up), set by combat.
    pub spinning: bool,
    slide: f32,
    dash: f32,
    last_angles: Option<(f32, f32)>,
    time: f64,
    last_draw: Option<ViewmodelDrawInfo>,
}

impl Viewmodel {
    pub fn new() -> Self {
        Self {
            anim: ViewmodelAnim {
                switch: 1.0,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Weapon or mode switched: quick raise from below.
    pub fn on_switch(&mut self) {
        self.anim.switch = 0.0;
    }

    /// A shot: recoil kick (1 = full).
    pub fn fire(&mut self, kick: f32) {
        self.anim.fire_kick = kick.clamp(0.0, 1.0).max(self.anim.fire_kick);
    }

    /// Starts the left-fist punch animation.
    pub fn punch(&mut self) {
        self.anim.punch = 0.01;
    }

    /// Starts the prism throw animation.
    pub fn throw_prism(&mut self) {
        self.anim.throw = 0.01;
    }

    /// Lancet overcharge 0..1.
    pub fn set_charge(&mut self, charge: f32) {
        self.anim.alt_charge = charge.clamp(0.0, 1.0);
    }

    /// View-space muzzle / fist of the last drawn frame.
    pub fn last_draw(&self) -> Option<ViewmodelDrawInfo> {
        self.last_draw
    }

    /// Advances by the visual frame time.
    pub fn update(&mut self, vdt: f32, state: &MoveState, yaw: f32, pitch: f32, view_bob: f32) {
        self.time += vdt as f64;
        let dt = vdt.min(0.1);
        let a = &mut self.anim;
        // Look sway: smoothed look velocity (x = turning right, y = looking up), rad/s.
        if dt > 0.0 {
            let vel = match self.last_angles {
                Some((ly, lp)) => Vec2::new(-(yaw - ly), pitch - lp) / dt,
                None => Vec2::ZERO,
            };
            let vel = vel.clamp(Vec2::splat(-12.0), Vec2::splat(12.0));
            a.sway += (vel - a.sway) * (1.0 - (-12.0 * dt).exp());
            self.last_angles = Some((yaw, pitch));
        }
        let speed = Vec2::new(state.velocity.x, state.velocity.z).length();
        let walking = state.on_ground && state.mode == MoveMode::Walk;
        let target = if walking {
            (speed / RUN_SPEED).min(1.0) * view_bob
        } else {
            0.0
        };
        a.bob_amount = approach(a.bob_amount, target, 8.0, dt);
        if walking {
            a.bob_phase = (a.bob_phase + speed * 1.2 * dt) % (std::f32::consts::TAU * 64.0);
        }
        a.switch = (a.switch + dt / SWITCH_TIME).min(1.0);
        a.fire_kick = (a.fire_kick - dt / 0.3).max(0.0);
        for p in [&mut a.punch, &mut a.throw] {
            if *p > 0.0 {
                *p += dt / 0.4;
                if *p >= 1.0 {
                    *p = 0.0;
                }
            }
        }
        self.spin.update(self.spinning, dt);
        a.spin = self.spin.angle;
        a.time = self.time;
        let sliding = state.mode == MoveMode::Slide;
        self.slide = approach(self.slide, sliding as u8 as f32, 12.0, dt);
        let dashing = state.mode == MoveMode::Dash;
        self.dash = approach(self.dash, dashing as u8 as f32, 18.0, dt);
    }

    /// Appends the weapon + arms. `dip` is the camera's landing dip (the gun lags a little
    /// more), `camera_roll` the view tilt (the gun counter-rolls a bit to stay readable).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        lib: Option<&ModelLibrary>,
        weapon: u8,
        mode: u8,
        team: Vec4,
        dip: f32,
        camera_roll: f32,
        out: &mut Vec<DrawItem>,
    ) {
        let Some(lib) = lib else {
            return;
        };
        self.anim.weapon = weapon;
        self.anim.alt_mode = mode;
        let start = out.len();
        self.last_draw = Some(lib.draw_viewmodel(&self.anim, &Tint::team(team), out));
        // Movement feel on top of the model's own animation.
        let offset = Vec3::new(
            -0.015 * self.slide,
            -0.025 * self.slide + dip * 0.3 - 0.015 * self.dash,
            0.04 * self.dash,
        );
        let rotation = Quat::from_rotation_z(self.slide * 0.12 - camera_roll * 0.4);
        let extra = Mat4::from_rotation_translation(rotation, offset);
        for item in &mut out[start..] {
            item.transform = extra * item.transform;
        }
    }
}
