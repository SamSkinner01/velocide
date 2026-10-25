//! Weapons: first-person viewmodels (also held by the third-person Shrike), the right arm that
//! grips them, the punching left fist and the prism throw.
//!
//! Weapon model space: the grip (where the hand closes) is at the origin, the barrel points
//! down -Z, top is +Y, units are metres. Glow parts are white; their colour comes from
//! `Pose::glow` (weapon colour per alt mode, brighter while charging / firing).

pub mod arm;
pub mod cinderbox;
pub mod fist;
pub mod lancet;
pub mod riveter;
pub mod thresher;

use engine::gfx::DrawItem;
use engine::glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};

use super::ModelId;
use super::library::{ModelLibrary, ViewmodelDrawInfo};
use super::rig::{Pose, Tint};
use super::shrike::punch_extension;

/// The four guns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeaponKind {
    Lancet,
    Thresher,
    Riveter,
    Cinderbox,
}

impl WeaponKind {
    pub const ALL: [WeaponKind; 4] = [
        WeaponKind::Lancet,
        WeaponKind::Thresher,
        WeaponKind::Riveter,
        WeaponKind::Cinderbox,
    ];

    /// Weapon index (0 Lancet, 1 Thresher, 2 Riveter, 3 Cinderbox); out of range = Lancet.
    pub fn from_index(index: u8) -> WeaponKind {
        Self::ALL
            .get(index as usize)
            .copied()
            .unwrap_or(WeaponKind::Lancet)
    }

    pub fn model(self) -> ModelId {
        match self {
            WeaponKind::Lancet => ModelId::Lancet,
            WeaponKind::Thresher => ModelId::Thresher,
            WeaponKind::Riveter => ModelId::Riveter,
            WeaponKind::Cinderbox => ModelId::Cinderbox,
        }
    }

    /// Muzzle point in weapon model space.
    pub fn muzzle(self) -> Vec3 {
        match self {
            WeaponKind::Lancet => lancet::MUZZLE,
            WeaponKind::Thresher => thresher::MUZZLE,
            WeaponKind::Riveter => riveter::MUZZLE,
            WeaponKind::Cinderbox => cinderbox::MUZZLE,
        }
    }

    /// Where the grip sits in view space (lower right of the screen).
    fn view_offset(self) -> Vec3 {
        match self {
            WeaponKind::Lancet => Vec3::new(0.12, -0.112, -0.31),
            WeaponKind::Thresher => Vec3::new(0.125, -0.122, -0.32),
            WeaponKind::Riveter => Vec3::new(0.125, -0.12, -0.32),
            WeaponKind::Cinderbox => Vec3::new(0.13, -0.13, -0.33),
        }
    }

    /// Glow colour for an alt-fire mode (0 = first alt mode, 1 = second).
    pub fn glow_color(self, alt_mode: u8) -> Vec4 {
        let c = match (self, alt_mode % 2) {
            (WeaponKind::Lancet, 0) => Vec3::new(0.35, 1.0, 0.95),
            (WeaponKind::Lancet, _) => Vec3::new(0.75, 0.45, 1.0),
            (WeaponKind::Thresher, 0) => Vec3::new(1.0, 0.62, 0.2),
            (WeaponKind::Thresher, _) => Vec3::new(1.0, 0.3, 0.15),
            (WeaponKind::Riveter, 0) => Vec3::new(1.0, 0.5, 0.12),
            (WeaponKind::Riveter, _) => Vec3::new(0.45, 1.0, 0.35),
            (WeaponKind::Cinderbox, 0) => Vec3::new(1.0, 0.45, 0.08),
            (WeaponKind::Cinderbox, _) => Vec3::new(1.0, 0.85, 0.25),
        };
        c.extend(1.0)
    }
}

/// Everything the first-person viewmodel depends on.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ViewmodelAnim {
    /// Weapon index (see [`WeaponKind::from_index`]).
    pub weapon: u8,
    /// Selected alt-fire mode (0 or 1): changes the weapon's glow colour.
    pub alt_mode: u8,
    /// Recoil 0..1 (set to 1 on fire, decay over ~0.15–0.4 s depending on the weapon).
    pub fire_kick: f32,
    /// Alt-fire charge 0..1 (Lancet overcharge: the cell and coils glow and the gun shakes).
    pub alt_charge: f32,
    /// Weapon raise 0..1 after a switch (0 = just switched, below the screen; 1 = ready).
    pub switch: f32,
    /// Punch progress 0..1 (0 = not punching): the left fist thrusts forward.
    pub punch: f32,
    /// Prism throw progress 0..1 (0 = not throwing): the left hand flicks a spinning prism.
    pub throw: f32,
    /// Walk bob phase in radians (advance by ~speed * 1.2 per second while grounded).
    pub bob_phase: f32,
    /// Bob strength 0..1 (horizontal speed / run speed; 0 in the air).
    pub bob_amount: f32,
    /// Smoothed look velocity (radians per second-ish, x = yaw right, y = pitch up); the
    /// weapon lags behind it.
    pub sway: Vec2,
    /// Riveter barrel rotation in radians (see [`BarrelSpin`]).
    pub spin: f32,
    pub time: f64,
}

/// Spin-up / spin-down of the Riveter's barrel cluster (keep one per local player).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BarrelSpin {
    /// Accumulated angle (radians), feed into [`ViewmodelAnim::spin`].
    pub angle: f32,
    /// Current angular speed (radians per second).
    pub speed: f32,
}

impl BarrelSpin {
    pub const MAX_SPEED: f32 = 30.0;

    pub fn update(&mut self, firing: bool, dt: f32) {
        let target = if firing { Self::MAX_SPEED } else { 0.0 };
        let rate = if firing { 90.0 } else { 25.0 };
        self.speed += (target - self.speed).clamp(-rate * dt, rate * dt);
        self.angle = (self.angle + self.speed * dt).rem_euclid(std::f32::consts::TAU * 64.0);
    }
}

fn wave(time: f64, hz: f64) -> f32 {
    ((time * hz).fract() as f32 * std::f32::consts::TAU).sin()
}

/// Pose of a weapon model: glow colour / intensity and moving parts.
/// `kick` = recoil 0..1, `charge` = alt charge 0..1, `spin` = Riveter barrel angle.
pub fn weapon_pose(
    weapon: WeaponKind,
    alt_mode: u8,
    kick: f32,
    charge: f32,
    spin: f32,
    time: f64,
) -> Pose {
    let base = weapon.glow_color(alt_mode);
    let pulse = 0.85 + 0.15 * wave(time, 1.3);
    let mut glow = 0.9 * pulse + kick * 0.8;
    let mut pose = match weapon {
        WeaponKind::Lancet => {
            let mut pose = Pose::rest(lancet::JOINTS.len());
            // Overcharge: brighter and flickering; full charge shimmers white.
            glow += charge * (1.6 + 0.5 * wave(time, 14.0));
            pose.glow = (base.truncate() + Vec3::splat(charge * charge * 0.6)).extend(1.0);
            pose
        }
        WeaponKind::Thresher => {
            let mut pose = Pose::rest(thresher::JOINTS.len());
            // Pump racks back after the shot.
            let rack = (std::f32::consts::PI * (1.0 - kick).clamp(0.0, 1.0)).sin()
                * (kick > 0.0) as u8 as f32;
            pose.set(
                thresher::PUMP,
                Mat4::from_translation(Vec3::new(0.0, 0.0, 0.07 * rack)),
            );
            pose.glow = base;
            pose
        }
        WeaponKind::Riveter => {
            let mut pose = Pose::rest(riveter::JOINTS.len());
            pose.rotate(riveter::BARRELS, Quat::from_rotation_z(spin));
            pose.rotate(riveter::DRUM, Quat::from_rotation_x(-spin * 0.2));
            glow += kick * 0.6;
            pose.glow = base;
            pose
        }
        WeaponKind::Cinderbox => {
            let mut pose = Pose::rest(cinderbox::JOINTS.len());
            glow = 0.8 + 0.35 * wave(time, 0.9) + kick * 1.5;
            pose.glow = base;
            pose
        }
    };
    pose.glow = (pose.glow.truncate() * glow).extend(1.0);
    pose
}

/// Pose of the weapon held by a third-person Shrike (`firing` = recoil 0..1).
pub fn third_person_pose(weapon: WeaponKind, firing: f32, time: f64) -> Pose {
    let spin = if firing > 0.0 {
        (time * 25.0 % 1000.0) as f32
    } else {
        0.0
    };
    weapon_pose(weapon, 0, firing, 0.0, spin, time)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Weapons are modelled at real scale (the third-person Shrike holds them); first person
/// draws them smaller and closer so they keep a sensible share of the screen.
pub const VIEWMODEL_SCALE: f32 = 0.52;

/// Grip transform of the held weapon in view space.
pub fn viewmodel_transform(anim: &ViewmodelAnim) -> Mat4 {
    let w = WeaponKind::from_index(anim.weapon);
    let kick = anim.fire_kick.clamp(0.0, 1.0);
    let (kick_back, kick_pitch) = match w {
        WeaponKind::Lancet => (0.035, 0.18),
        WeaponKind::Thresher => (0.075, 0.42),
        WeaponKind::Riveter => (0.018, 0.05),
        WeaponKind::Cinderbox => (0.09, 0.3),
    };
    let lowered = (1.0 - smoothstep(anim.switch)).powi(2);
    let punch = punch_extension(anim.punch).max(throw_extension(anim.throw) * 0.6);
    let bob = anim.bob_amount.clamp(0.0, 1.0);
    let (s, c) = anim.bob_phase.sin_cos();
    let bob_offset = Vec3::new(s * 0.008, -(c * c) * 0.009 + 0.0045, 0.0) * bob;
    let sway = anim.sway.clamp(Vec2::splat(-3.0), Vec2::splat(3.0));
    let sway_offset = Vec3::new(-sway.x * 0.006, -sway.y * 0.006, 0.0);
    // Overcharge shake.
    let shake = if w == WeaponKind::Lancet && anim.alt_charge > 0.6 {
        let a = (anim.alt_charge - 0.6) / 0.4;
        Vec3::new(wave(anim.time, 31.0), wave(anim.time, 37.0), 0.0) * 0.0025 * a
    } else {
        Vec3::ZERO
    };
    let offset = w.view_offset()
        + bob_offset
        + sway_offset
        + shake
        + Vec3::new(0.0, -0.24 * lowered, 0.04 * lowered)
        + Vec3::new(0.0, -0.008, kick_back * VIEWMODEL_SCALE) * kick
        + Vec3::new(0.025, -0.04, 0.02) * punch;
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        0.045 - sway.x * 0.025 + 0.05 * punch,
        kick_pitch * kick - 0.7 * lowered + sway.y * 0.02 - 0.1 * punch,
        -0.04 * s * bob - 0.2 * lowered,
    );
    Mat4::from_scale_rotation_translation(Vec3::splat(VIEWMODEL_SCALE), rotation, offset)
}

/// Throw curve: wind up (raise) for the first half, flick forward, then drop away.
fn throw_extension(p: f32) -> f32 {
    if p <= 0.0 || p >= 1.0 {
        0.0
    } else if p < 0.6 {
        smoothstep(p / 0.25)
    } else {
        1.0 - smoothstep((p - 0.6) / 0.4)
    }
}

/// View-space transform of the left fist for a punch (`ext` 0..1) or throw.
fn fist_transform(anim: &ViewmodelAnim) -> Option<Mat4> {
    let punch = punch_extension(anim.punch);
    if punch > 0.0 {
        let hidden = Vec3::new(-0.16, -0.32, -0.1);
        let out = Vec3::new(-0.045, -0.05, -0.37);
        let p = hidden.lerp(out, punch);
        let rot = Quat::from_euler(
            EulerRot::YXZ,
            -0.14 + 0.06 * punch,
            0.3 - 0.05 * punch,
            0.5 * (1.0 - punch),
        );
        return Some(Mat4::from_scale_rotation_translation(
            Vec3::splat(VIEWMODEL_SCALE),
            rot,
            p,
        ));
    }
    let throw = throw_extension(anim.throw);
    if throw > 0.0 {
        let p = anim.throw;
        let hidden = Vec3::new(-0.17, -0.33, -0.14);
        let raised = Vec3::new(-0.1, -0.12, -0.25);
        let flick = smoothstep((p - 0.4) / 0.2);
        let pos = hidden.lerp(raised, throw) + Vec3::new(0.04, 0.02, -0.09) * flick * throw;
        let rot = Quat::from_euler(EulerRot::YXZ, -0.25, 0.55 - 0.9 * flick, 0.25);
        return Some(Mat4::from_scale_rotation_translation(
            Vec3::splat(VIEWMODEL_SCALE),
            rot,
            pos,
        ));
    }
    None
}

/// Draws the viewmodel; see [`ModelLibrary::draw_viewmodel`].
pub(crate) fn draw_viewmodel(
    lib: &ModelLibrary,
    anim: &ViewmodelAnim,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) -> ViewmodelDrawInfo {
    let w = WeaponKind::from_index(anim.weapon);
    let gun = viewmodel_transform(anim);
    let pose = weapon_pose(
        w,
        anim.alt_mode,
        anim.fire_kick.clamp(0.0, 1.0),
        anim.alt_charge.clamp(0.0, 1.0),
        anim.spin,
        anim.time,
    );
    lib.get(w.model()).draw(&pose, gun, tint, out);
    lib.get(ModelId::ArmRight).draw_static(gun, tint, out);

    let mut fist_point = None;
    if let Some(fist) = fist_transform(anim) {
        let mut pose = Pose::rest(fist::JOINTS.len());
        let punch = punch_extension(anim.punch);
        // Knuckle studs flare at full extension (parry window feedback).
        let flare = 0.6 + punch * 2.0;
        pose.glow = Vec4::new(flare, flare, flare, 1.0);
        lib.get(ModelId::Fist).draw(&pose, fist, tint, out);
        fist_point = Some(fist.transform_point3(Vec3::new(0.0, 0.0, -0.02)));
        if anim.throw > 0.0 && anim.throw < 0.55 {
            // The prism hovers spinning above the knuckles until it is flicked away.
            let spin = Quat::from_rotation_y((anim.time * 9.0 % 1000.0) as f32)
                * Quat::from_rotation_z(0.35);
            let prism = fist
                * Mat4::from_scale_rotation_translation(
                    Vec3::splat(0.32),
                    spin,
                    Vec3::new(0.0, 0.11, 0.02),
                );
            lib.get(ModelId::Prism).draw_static(prism, tint, out);
            fist_point = Some(prism.transform_point3(Vec3::ZERO));
        }
    }
    ViewmodelDrawInfo {
        muzzle: gun.transform_point3(w.muzzle()),
        muzzle_dir: gun.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
        fist: fist_point,
    }
}
