//! The held weapon, drawn in the renderer's viewmodel pass (`FrameDesc::viewmodel`, view
//! space, own FOV, never clips into walls), with procedural bob, look sway, landing dip, slide
//! tilt, dash pull-back, weapon-switch raise and a fire kick.
//!
//! Placeholder art: one blocky meshgen gun per weapon plus a chrome forearm. When
//! `client::models` lands, [`Viewmodel::new`] swaps the meshes for the real weapon models; the
//! animation code stays the same. Everything here runs on the *visual* clock (hitstop freezes
//! it).

use engine::assets::meshgen::{BoxDesc, MeshBuilder};
use engine::gfx::{DrawItem, MeshId, TextureId};
use engine::render::Renderer;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::movement::{MoveMode, MoveState};

use crate::controls::WEAPONS;

/// Resting position of the weapon in view space (camera looks down -Z).
const REST: Vec3 = Vec3::new(0.15, -0.14, -0.34);
/// Placeholder meshes are modelled at roughly real size; drawn smaller so they sit in the
/// corner of the screen.
const SCALE: f32 = 0.55;
const SWITCH_TIME: f32 = 0.16;

fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-rate * dt).exp())
}

fn add_box(b: &mut MeshBuilder, center: Vec3, half: Vec3, color: Vec4) {
    b.add_box(&BoxDesc {
        center,
        half_extents: half,
        color,
        ..Default::default()
    });
}

fn add_cyl_z(b: &mut MeshBuilder, center: Vec3, radius: f32, length: f32, color: Vec4) {
    let half = Vec3::Z * length * 0.5;
    b.add_cylinder_between(center - half, center + half, radius, 8, color);
}

/// Placeholder gun meshes (gun origin = grip; barrel points down -Z).
fn gun_mesh(slot: usize) -> MeshBuilder {
    let chrome = Vec4::new(0.56, 0.58, 0.64, 1.0);
    let dark = Vec4::new(0.13, 0.11, 0.12, 1.0);
    let blood = Vec4::new(0.6, 0.06, 0.09, 1.0);
    let bone = Vec4::new(0.72, 0.66, 0.56, 1.0);
    let brass = Vec4::new(0.62, 0.46, 0.22, 1.0);
    let mut b = MeshBuilder::new();
    // Chrome forearm + fist around the grip, the same for every weapon.
    b.add_cylinder_between(
        Vec3::new(0.06, -0.2, 0.42),
        Vec3::new(0.0, -0.06, 0.07),
        0.045,
        6,
        chrome * 0.85,
    );
    add_box(
        &mut b,
        Vec3::new(0.0, -0.05, 0.06),
        Vec3::new(0.045, 0.04, 0.05),
        chrome,
    );
    match slot {
        0 => {
            // LANCET: slim sidearm with a long slide and a red spine.
            add_box(
                &mut b,
                Vec3::new(0.0, 0.03, -0.08),
                Vec3::new(0.03, 0.04, 0.17),
                chrome,
            );
            add_box(
                &mut b,
                Vec3::new(0.0, 0.075, -0.08),
                Vec3::new(0.012, 0.008, 0.15),
                blood,
            );
            add_cyl_z(&mut b, Vec3::new(0.0, 0.03, -0.29), 0.016, 0.08, dark);
            add_box(
                &mut b,
                Vec3::new(0.0, -0.04, 0.04),
                Vec3::new(0.026, 0.07, 0.035),
                dark,
            );
        }
        1 => {
            // THRESHER: chunky double-barrelled scattergun with a bone pump.
            add_box(
                &mut b,
                Vec3::new(0.0, 0.03, -0.02),
                Vec3::new(0.05, 0.05, 0.12),
                dark,
            );
            add_cyl_z(&mut b, Vec3::new(-0.025, 0.05, -0.3), 0.026, 0.42, chrome);
            add_cyl_z(&mut b, Vec3::new(0.025, 0.05, -0.3), 0.026, 0.42, chrome);
            add_box(
                &mut b,
                Vec3::new(0.0, 0.0, -0.3),
                Vec3::new(0.05, 0.03, 0.08),
                bone,
            );
            add_box(
                &mut b,
                Vec3::new(0.0, 0.09, -0.05),
                Vec3::new(0.012, 0.012, 0.06),
                blood,
            );
        }
        2 => {
            // RIVETER: boxy nailer with a brass drum and a short muzzle.
            add_box(
                &mut b,
                Vec3::new(0.0, 0.04, -0.1),
                Vec3::new(0.045, 0.06, 0.19),
                chrome,
            );
            b.add_cylinder_between(
                Vec3::new(-0.06, 0.0, -0.08),
                Vec3::new(0.06, 0.0, -0.08),
                0.07,
                10,
                brass,
            );
            add_cyl_z(&mut b, Vec3::new(0.0, 0.05, -0.33), 0.022, 0.08, dark);
            add_box(
                &mut b,
                Vec3::new(0.0, 0.105, -0.1),
                Vec3::new(0.02, 0.006, 0.16),
                blood,
            );
        }
        _ => {
            // CINDERBOX: fat launcher tube with hazard stripes.
            add_cyl_z(&mut b, Vec3::new(0.0, 0.05, -0.18), 0.058, 0.56, dark);
            add_cyl_z(&mut b, Vec3::new(0.0, 0.05, -0.43), 0.064, 0.05, blood);
            add_cyl_z(&mut b, Vec3::new(0.0, 0.05, 0.04), 0.064, 0.04, brass);
            add_box(
                &mut b,
                Vec3::new(0.0, 0.12, -0.12),
                Vec3::new(0.015, 0.025, 0.05),
                chrome,
            );
            add_box(
                &mut b,
                Vec3::new(0.0, -0.02, 0.0),
                Vec3::new(0.03, 0.06, 0.04),
                dark,
            );
        }
    }
    b.shade_by_normal(1.0, 0.7, 0.45);
    b
}

/// Procedural weapon animation state.
pub struct Viewmodel {
    meshes: Vec<MeshId>,
    texture: TextureId,
    bob_phase: f32,
    bob_amount: f32,
    sway: Vec2,
    switch_t: f32,
    kick: f32,
    slide: f32,
    dash: f32,
    last_angles: Option<(f32, f32)>,
    time: f32,
}

impl Viewmodel {
    pub fn new(renderer: &mut Renderer) -> Self {
        let meshes = (0..WEAPONS.len())
            .map(|i| renderer.create_mesh(&gun_mesh(i).build()))
            .collect();
        Self {
            meshes,
            texture: renderer.white_texture(),
            bob_phase: 0.0,
            bob_amount: 0.0,
            sway: Vec2::ZERO,
            switch_t: 1.0,
            kick: 0.0,
            slide: 0.0,
            dash: 0.0,
            last_angles: None,
            time: 0.0,
        }
    }

    /// Weapon or mode switched: quick raise from below.
    pub fn on_switch(&mut self) {
        self.switch_t = 0.0;
    }

    /// Recoil kick (fire): 0.2 light .. 1 heavy. For the combat code.
    pub fn kick(&mut self, amount: f32) {
        self.kick = (self.kick + amount.max(0.0)).min(1.5);
    }

    /// Advances by the visual frame time.
    pub fn update(&mut self, vdt: f32, state: &MoveState, yaw: f32, pitch: f32, view_bob: f32) {
        self.time += vdt;
        let dt = vdt.min(0.1);
        // Look sway: the gun lags behind fast turns.
        if let Some((ly, lp)) = self.last_angles
            && dt > 0.0
        {
            let dy = yaw - ly;
            let dp = pitch - lp;
            self.sway += Vec2::new(dy, dp) * 0.35;
            self.sway = self.sway.clamp(Vec2::splat(-0.08), Vec2::splat(0.08));
        }
        if dt > 0.0 {
            self.last_angles = Some((yaw, pitch));
        }
        self.sway *= (-10.0 * dt).exp();

        let speed = Vec2::new(state.velocity.x, state.velocity.z).length();
        let grounded_run = state.on_ground && state.mode == MoveMode::Walk;
        let target = if grounded_run {
            (speed / 16.0).min(1.0) * view_bob
        } else {
            0.0
        };
        self.bob_amount = approach(self.bob_amount, target, 8.0, dt);
        if grounded_run {
            self.bob_phase += speed * dt * std::f32::consts::PI / 2.6;
        }
        self.slide = approach(
            self.slide,
            if state.mode == MoveMode::Slide {
                1.0
            } else {
                0.0
            },
            12.0,
            dt,
        );
        self.dash = approach(
            self.dash,
            if state.mode == MoveMode::Dash {
                1.0
            } else {
                0.0
            },
            18.0,
            dt,
        );
        self.switch_t = (self.switch_t + dt / SWITCH_TIME).min(1.0);
        self.kick *= (-14.0 * dt).exp();
    }

    /// Appends the weapon. `dip` is the camera's landing dip (the gun lags a little more).
    pub fn draw(&self, weapon: u8, dip: f32, camera_roll: f32, out: &mut Vec<DrawItem>) {
        let Some(&mesh) = self.meshes.get(weapon as usize % self.meshes.len().max(1)) else {
            return;
        };
        let b = self.bob_amount;
        let bob = Vec3::new(
            self.bob_phase.sin() * 0.014 * b,
            -(self.bob_phase * 2.0).cos().abs() * 0.01 * b + 0.004 * b,
            0.0,
        );
        // Idle breathing.
        let idle = Vec3::new(0.0, (self.time * 1.6).sin() * 0.003, 0.0);
        let raise = (1.0 - self.switch_t).powi(2);
        let offset = REST
            + bob
            + idle
            + Vec3::new(-self.sway.x * 0.25, self.sway.y * 0.2, 0.0)
            + Vec3::new(
                -0.02 * self.slide,
                -0.035 * self.slide - raise * 0.22 + dip * 0.35,
                0.0,
            )
            + Vec3::new(0.0, -0.02 * self.dash, 0.06 * self.dash + 0.07 * self.kick);
        let rotation = Quat::from_rotation_y(self.sway.x * 0.9 + 0.03)
            * Quat::from_rotation_x(-self.sway.y * 0.6 + self.kick * 0.25 - raise * 0.6)
            * Quat::from_rotation_z(self.slide * 0.14 - camera_roll * 0.4);
        out.push(DrawItem {
            mesh,
            texture: self.texture,
            transform: Mat4::from_scale_rotation_translation(Vec3::splat(SCALE), rotation, offset),
            tint: Vec4::ONE,
            unlit: false,
        });
    }

    pub fn free(self, renderer: &mut Renderer) {
        for m in self.meshes {
            renderer.free_mesh(m);
        }
    }
}
