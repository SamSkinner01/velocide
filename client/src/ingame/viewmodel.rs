//! The held weapon in first person: `client::models` weapon + arm models drawn through
//! `ModelLibrary::draw_viewmodel` into the renderer's viewmodel pass (`FrameDesc::viewmodel`:
//! view space, own FOV, never clips into walls).
//!
//! This module keeps the animation state the models need ([`ViewmodelAnim`]: walk bob, look
//! sway, switch raise, fire kick, punch / thorn-throw progress, Tumbler drum turn) and layers
//! the movement feel on top as an extra view-space transform: landing dip, slide tilt, dash
//! pull-back and a counter-roll against the camera tilt. Everything runs on the *visual*
//! clock (hitstop freezes it).
//!
//! Combat hooks: [`Viewmodel::fire`] (recoil kick), [`Viewmodel::punch`],
//! [`Viewmodel::throw_thorn`] / [`Viewmodel::throw_item`] (a knack), [`Viewmodel::swing`] (a
//! melee move), [`Viewmodel::set_charge`] (Discharge / Skewer charge),
//! [`Viewmodel::turn_drum`] (a Tumbler or Thresher shot), [`Viewmodel::set_draw`] (Longwick
//! draw), [`Viewmodel::salvo`] (a Cinderbox salvo), [`Viewmodel::spray`] (the Kiln firing)
//! and [`Viewmodel::last_draw`] (view-space muzzle / fist for tracers and sparks).

use engine::gfx::DrawItem;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::movement::{MoveMode, MoveState};

use crate::models::weapons::melee::{self, Swing, SwingKind};
use crate::models::{
    Ability, DrumTurn, ModelId, ModelLibrary, Tint, ViewmodelAnim, ViewmodelDrawInfo, WeaponKind,
};

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
    drum: DrumTurn,
    slide: f32,
    dash: f32,
    last_angles: Option<(f32, f32)>,
    /// Kiln: seconds since the last spray tick (the spray fades out after it).
    since_spray: f32,
    time: f64,
    last_draw: Option<ViewmodelDrawInfo>,
    /// The weapon's heat (Stoke, `combat::stoke`): its glow and body tint.
    heat: Option<(Vec4, Vec4)>,
}

impl Viewmodel {
    pub fn new() -> Self {
        Self {
            anim: ViewmodelAnim {
                switch: 1.0,
                ..Default::default()
            },
            since_spray: f32::INFINITY,
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

    /// Starts the thorn throw animation.
    pub fn throw_thorn(&mut self) {
        self.throw_item(None);
    }

    /// Starts the throw animation with a knack's model in the hand (`None`: a Thorn).
    pub fn throw_item(&mut self, item: Option<ModelId>) {
        self.anim.throw = 0.01;
        self.anim.throw_item = item;
    }

    /// The weapon's heat this frame (Stoke: `combat::stoke::heat_tint`): its glow parts and its
    /// body tinted by these (`Vec4::ONE` both: cold).
    pub fn set_heat(&mut self, glow: Vec4, body: Vec4) {
        self.heat = (glow != Vec4::ONE || body != Vec4::ONE).then_some((glow, body));
    }

    /// Starts drinking the Kindle Flask (the left hand, `models::weapons::drink`).
    pub fn drink(&mut self) {
        self.anim.drink = 1e-3;
    }

    /// Drinking the Kindle Flask right now.
    pub fn drinking(&self) -> bool {
        crate::models::weapons::drink::drinking(self.anim.drink)
    }

    /// The weapon in the hand (last drawn).
    pub fn weapon(&self) -> WeaponKind {
        self.anim.weapon
    }

    /// The last melee move was the Trimmer's left blade (the next slash is the right one's).
    pub fn swing_left(&self) -> bool {
        self.anim.swing.left
    }

    /// A melee move starts (the Trimmer's slashes: `left` = the left blade's turn).
    pub fn swing(&mut self, kind: SwingKind, left: bool) {
        self.anim.swing = Swing {
            kind,
            progress: 1e-3,
            left,
        };
    }

    /// A Tumbler shot: the drum turns one chamber.
    pub fn turn_drum(&mut self) {
        self.drum.kick();
    }

    /// Alt charge 0..1 (Arclash Discharge, Skewer).
    pub fn set_charge(&mut self, charge: f32) {
        self.anim.alt_charge = charge.clamp(0.0, 1.0);
    }

    /// Longwick draw 0..1.
    pub fn set_draw(&mut self, draw: f32) {
        self.anim.draw = if draw.is_finite() {
            draw.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    /// A Cinderbox salvo starts: its tubes fire in turn.
    pub fn salvo(&mut self) {
        self.anim.salvo = Some(0.0);
    }

    /// No weapon at all (gun game's Fist tier): fists only.
    pub fn set_unarmed(&mut self, unarmed: bool) {
        self.anim.unarmed = unarmed;
    }

    /// One Kiln spray tick (every ~0.08 s while the trigger is held).
    pub fn spray(&mut self) {
        self.since_spray = 0.0;
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
        // Recoil snaps back fast, then settles (docs/feel.md): most of it is gone in 0.1 s.
        a.fire_kick = (a.fire_kick * (-7.0 * dt).exp() - 0.6 * dt).max(0.0);
        for p in [&mut a.punch, &mut a.throw] {
            if *p > 0.0 {
                *p += dt / 0.4;
                if *p >= 1.0 {
                    *p = 0.0;
                }
            }
        }
        if a.drink > 0.0 {
            a.drink += dt / crate::models::weapons::drink::DRINK_TIME;
            if a.drink >= 1.0 {
                a.drink = 0.0;
            }
        }
        self.drum.update(dt);
        a.spin = self.drum.angle();
        match melee::of(a.weapon) {
            Some(m) => a.swing.advance(m, dt),
            None => a.swing = Swing::default(),
        }
        if let Some(s) = &mut a.salvo {
            *s += dt;
            if *s > 1.0 {
                a.salvo = None;
            }
        }
        self.since_spray += dt;
        let spraying = self.since_spray < 0.16;
        a.spray = approach(
            a.spray,
            spraying as u8 as f32,
            if spraying { 14.0 } else { 5.0 },
            dt,
        );
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
        weapon: WeaponKind,
        ability: Ability,
        team: Vec4,
        dip: f32,
        camera_roll: f32,
        out: &mut Vec<DrawItem>,
    ) {
        let Some(lib) = lib else {
            return;
        };
        if weapon != self.anim.weapon {
            // Another gun in the hand: no draw, salvo or spray carries over.
            self.anim.draw = 0.0;
            self.anim.salvo = None;
            self.anim.spray = 0.0;
            self.anim.swing = Swing::default();
        }
        self.anim.weapon = weapon;
        self.anim.ability = ability;
        let start = out.len();
        let mut tint = Tint::team(team);
        if let Some((glow, body)) = self.heat {
            tint.glow = glow;
            tint.body = body;
        }
        self.last_draw = Some(lib.draw_viewmodel(&self.anim, &tint, out));
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
