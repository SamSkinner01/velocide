//! Weapons: first-person viewmodels (also held by the third-person Shrike), the right arm that
//! grips them, the punching left fist and the Thorn throw.
//!
//! Weapon model space: the grip (where the hand closes) is at the origin, the barrel points
//! down -Z, top is +Y, units are metres. Glow parts are white; their colour comes from
//! `Pose::glow` (weapon colour per alt mode, brighter while charging / firing).

pub mod arclash;
pub mod arm;
pub mod bellows;
pub mod fist;
pub mod quill;
pub mod tumbler;

use engine::gfx::DrawItem;
use engine::glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};

use super::ModelId;
use super::library::{ModelLibrary, ViewmodelDrawInfo};
use super::rig::{Pose, Tint};
use super::shrike::punch_extension;

/// The four guns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeaponKind {
    /// Slot 1: precision spike-caster (hitscan).
    Quill,
    /// Slot 2: twin-barrel blast cannon.
    Bellows,
    /// Slot 3: arc caster.
    Arclash,
    /// Slot 4: bouncing-bomb launcher.
    Tumbler,
}

impl WeaponKind {
    pub const ALL: [WeaponKind; 4] = [
        WeaponKind::Quill,
        WeaponKind::Bellows,
        WeaponKind::Arclash,
        WeaponKind::Tumbler,
    ];

    /// Weapon index (0 Quill, 1 Bellows, 2 Arclash, 3 Tumbler); out of range = Quill.
    pub fn from_index(index: u8) -> WeaponKind {
        Self::ALL
            .get(index as usize)
            .copied()
            .unwrap_or(WeaponKind::Quill)
    }

    pub fn model(self) -> ModelId {
        match self {
            WeaponKind::Quill => ModelId::Quill,
            WeaponKind::Bellows => ModelId::Bellows,
            WeaponKind::Arclash => ModelId::Arclash,
            WeaponKind::Tumbler => ModelId::Tumbler,
        }
    }

    /// Muzzle point in weapon model space.
    pub fn muzzle(self) -> Vec3 {
        match self {
            WeaponKind::Quill => quill::MUZZLE,
            WeaponKind::Bellows => bellows::MUZZLE,
            WeaponKind::Arclash => arclash::MUZZLE,
            WeaponKind::Tumbler => tumbler::MUZZLE,
        }
    }

    /// Held with both hands by the third-person Shrike (the Quill is one-handed).
    pub fn two_handed(self) -> bool {
        self != WeaponKind::Quill
    }

    /// Where the grip sits in view space (lower right of the screen).
    fn view_offset(self) -> Vec3 {
        match self {
            WeaponKind::Quill => Vec3::new(0.118, -0.112, -0.3),
            WeaponKind::Bellows => Vec3::new(0.13, -0.13, -0.31),
            WeaponKind::Arclash => Vec3::new(0.13, -0.122, -0.31),
            WeaponKind::Tumbler => Vec3::new(0.14, -0.148, -0.32),
        }
    }

    /// Glow colour for an alt-fire mode (0 = alt A, 1 = alt B): Quill Brand crimson / Splinter
    /// ice; Bellows Gust sky / Both Barrels orange; Arclash Wisp violet / Discharge blue;
    /// Tumbler Breach orange-red / Gravewell green.
    pub fn glow_color(self, alt_mode: u8) -> Vec4 {
        let c = match (self, alt_mode % 2) {
            (WeaponKind::Quill, 0) => Vec3::new(1.0, 0.22, 0.38),
            (WeaponKind::Quill, _) => Vec3::new(0.55, 0.95, 1.0),
            (WeaponKind::Bellows, 0) => Vec3::new(0.72, 0.9, 1.0),
            (WeaponKind::Bellows, _) => Vec3::new(1.0, 0.5, 0.15),
            (WeaponKind::Arclash, 0) => Vec3::new(0.75, 0.5, 1.0),
            (WeaponKind::Arclash, _) => Vec3::new(0.4, 0.8, 1.0),
            (WeaponKind::Tumbler, 0) => Vec3::new(1.0, 0.4, 0.1),
            (WeaponKind::Tumbler, _) => Vec3::new(0.45, 1.0, 0.55),
        };
        c.extend(1.0)
    }

    /// Colour of the primary fire (tracers, beams, muzzle flashes): Quill pale gold, Bellows
    /// amber, Arclash blue-white, Tumbler fuse orange.
    pub fn primary_color(self) -> Vec4 {
        match self {
            WeaponKind::Quill => Vec4::new(1.0, 0.9, 0.6, 1.0),
            WeaponKind::Bellows => Vec4::new(1.0, 0.7, 0.3, 1.0),
            WeaponKind::Arclash => Vec4::new(0.6, 0.85, 1.0, 1.0),
            WeaponKind::Tumbler => Vec4::new(1.0, 0.55, 0.15, 1.0),
        }
    }
}

/// Everything the first-person viewmodel depends on.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ViewmodelAnim {
    /// Weapon index (see [`WeaponKind::from_index`]).
    pub weapon: u8,
    /// Selected alt-fire mode (0 or 1): changes the weapon's glow colour.
    pub alt_mode: u8,
    /// Recoil 0..1 (set to 1 on fire, decay over ~0.15–0.4 s depending on the weapon). Also
    /// folds the Quill's crest flat and pumps the Bellows.
    pub fire_kick: f32,
    /// Alt-fire charge 0..1 (Arclash Discharge: the capacitor brightens towards white, arcs
    /// jump between the prongs and the gun shakes above 0.6).
    pub alt_charge: f32,
    /// Weapon raise 0..1 after a switch (0 = just switched, below the screen; 1 = ready).
    pub switch: f32,
    /// Punch progress 0..1 (0 = not punching): the left fist thrusts forward.
    pub punch: f32,
    /// Thorn throw progress 0..1 (0 = not throwing): the left hand raises a thorn between
    /// its knuckles and flicks it away.
    pub throw: f32,
    /// Walk bob phase in radians (advance by ~speed * 1.2 per second while grounded).
    pub bob_phase: f32,
    /// Bob strength 0..1 (horizontal speed / run speed; 0 in the air).
    pub bob_amount: f32,
    /// Smoothed look velocity (radians per second-ish, x = yaw right, y = pitch up); the
    /// weapon lags behind it.
    pub sway: Vec2,
    /// Tumbler drum angle in radians (see [`DrumTurn`]).
    pub spin: f32,
    pub time: f64,
}

/// The Tumbler's drum: turns one chamber per shot, easing in with a little overshoot (keep
/// one per local player; [`DrumTurn::kick`] on every Tumbler shot, [`DrumTurn::update`] every
/// frame, feed [`DrumTurn::angle`] into [`ViewmodelAnim::spin`]).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct DrumTurn {
    angle: f32,
    target: f32,
    velocity: f32,
}

impl DrumTurn {
    /// One chamber.
    pub const STEP: f32 = std::f32::consts::TAU / tumbler::CHAMBERS as f32;
    /// Spring stiffness (1/s²) and damping ratio: settles in ~0.15 s, ~10 % overshoot.
    const STIFFNESS: f32 = 1400.0;
    const DAMPING: f32 = 0.6;

    /// Advances the drum by one chamber (a shot).
    pub fn kick(&mut self) {
        self.target += Self::STEP;
        // Keep the numbers small; whole turns look the same.
        let turns = std::f32::consts::TAU * 16.0;
        if self.target > turns {
            self.target -= turns;
            self.angle -= turns;
        }
    }

    /// Eases the angle towards the target (damped spring, fixed sub-steps).
    pub fn update(&mut self, dt: f32) {
        let mut left = dt.clamp(0.0, 0.25);
        let c = 2.0 * Self::DAMPING * Self::STIFFNESS.sqrt();
        while left > 0.0 {
            let h = left.min(1.0 / 240.0);
            let acc = Self::STIFFNESS * (self.target - self.angle) - c * self.velocity;
            self.velocity += acc * h;
            self.angle += self.velocity * h;
            left -= h;
        }
        if !self.angle.is_finite() || !self.velocity.is_finite() {
            *self = Self {
                angle: self.target,
                ..Self::default()
            };
        }
    }

    /// Current drum angle (radians).
    pub fn angle(&self) -> f32 {
        self.angle
    }
}

fn wave(time: f64, hz: f64) -> f32 {
    ((time * hz).fract() as f32 * std::f32::consts::TAU).sin()
}

/// Pose of a weapon model: glow colour / intensity and moving parts.
/// `kick` = recoil 0..1, `charge` = alt charge 0..1 (Arclash Discharge), `spin` = Tumbler drum
/// angle.
pub fn weapon_pose(
    weapon: WeaponKind,
    alt_mode: u8,
    kick: f32,
    charge: f32,
    spin: f32,
    time: f64,
) -> Pose {
    let base = weapon.glow_color(alt_mode).truncate();
    let kick = kick.clamp(0.0, 1.0);
    let charge = charge.clamp(0.0, 1.0);
    let pulse = 0.85 + 0.15 * wave(time, 1.3);
    let mut glow = 0.9 * pulse + kick * 0.8;
    let mut color = base;
    let mut pose = match weapon {
        WeaponKind::Quill => {
            let mut pose = Pose::rest(quill::JOINTS.len());
            // The crest folds flat front to back on the shot, then rises again.
            for i in 0..quill::CREST_COUNT {
                let f = smoothstep(kick * 1.7 - i as f32 * 0.14);
                let rest = quill::crest_rest_angle(i);
                let angle = (quill::CREST_FLAT_ANGLE - rest) * f;
                pose.rotate(quill::QUILL + i, Quat::from_rotation_x(angle));
            }
            glow += kick * 0.6;
            pose
        }
        WeaponKind::Bellows => {
            let mut pose = Pose::rest(bellows::JOINTS.len());
            // Pump between shots: squeeze shut half way through the recoil, refill after.
            let pump = if kick > 0.0 {
                (std::f32::consts::PI * (1.0 - kick)).sin()
            } else {
                0.0
            };
            let (board, squash) = bellows::pump_transforms(pump);
            pose.rotate(bellows::BOARD, board);
            pose.set(
                bellows::PLEATS,
                Mat4::from_scale(Vec3::new(1.0 + 0.04 * pump, squash, 1.0)),
            );
            glow += pump * 0.5;
            pose
        }
        WeaponKind::Arclash => {
            let mut pose = Pose::rest(arclash::JOINTS.len());
            // Light crawls round the capacitor, faster and brighter while charging.
            let crawl = ((time * (2.2 + charge as f64 * 12.0)) % 1000.0) as f32;
            pose.rotate(arclash::CRAWL, Quat::from_rotation_z(crawl));
            let crackle = 0.12 * wave(time, 7.3) * wave(time, 2.9);
            glow += crackle + charge * (1.6 + 0.5 * wave(time, 17.0));
            color = base + Vec3::splat(charge * charge * 0.7);
            let flicker = (time * 23.0).fract() < 0.65;
            let arcing = (charge > 0.08 && flicker) || kick > 0.6;
            if !arcing {
                pose.hide(arclash::ARCS);
            }
            pose
        }
        WeaponKind::Tumbler => {
            let mut pose = Pose::rest(tumbler::JOINTS.len());
            pose.rotate(tumbler::DRUM, Quat::from_rotation_z(-spin));
            glow = 0.8 + 0.3 * wave(time, 1.1) + kick * 1.4;
            pose
        }
    };
    pose.glow = (color * glow).extend(1.0);
    pose
}

/// Pose of the weapon held by a third-person Shrike (`firing` = recoil 0..1).
pub fn third_person_pose(weapon: WeaponKind, firing: f32, time: f64) -> Pose {
    weapon_pose(weapon, 0, firing, 0.0, 0.0, time)
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
        WeaponKind::Quill => (0.04, 0.2),
        WeaponKind::Bellows => (0.085, 0.45),
        WeaponKind::Arclash => (0.016, 0.05),
        WeaponKind::Tumbler => (0.09, 0.32),
    };
    let lowered = (1.0 - smoothstep(anim.switch)).powi(2);
    let punch = punch_extension(anim.punch).max(throw_extension(anim.throw) * 0.6);
    let bob = anim.bob_amount.clamp(0.0, 1.0);
    let (s, c) = anim.bob_phase.sin_cos();
    let bob_offset = Vec3::new(s * 0.008, -(c * c) * 0.009 + 0.0045, 0.0) * bob;
    let sway = anim.sway.clamp(Vec2::splat(-3.0), Vec2::splat(3.0));
    let sway_offset = Vec3::new(-sway.x * 0.006, -sway.y * 0.006, 0.0);
    // Discharge shake near full charge.
    let shake = if w == WeaponKind::Arclash && anim.alt_charge > 0.6 {
        let a = (anim.alt_charge.min(1.0) - 0.6) / 0.4;
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
        // The knuckle embers flare with the wick's light at full extension (parry window).
        let flare = 0.6 + punch * 2.0;
        pose.glow = Vec4::new(flare, flare * 0.85, flare * 0.7, 1.0);
        lib.get(ModelId::Fist).draw(&pose, fist, tint, out);
        fist_point = Some(fist.transform_point3(Vec3::new(0.0, 0.0, -0.02)));
        if anim.throw > 0.0 && anim.throw < 0.55 {
            // A thorn held point-forward between the knuckles until it is flicked away.
            let wobble = Quat::from_rotation_y(0.12 * wave(anim.time, 3.0))
                * Quat::from_rotation_x(-0.2 + 0.05 * wave(anim.time, 4.3));
            let thorn = fist
                * Mat4::from_scale_rotation_translation(
                    Vec3::splat(0.42),
                    wobble,
                    Vec3::new(0.0, 0.055, 0.02),
                );
            lib.get(ModelId::Thorn).draw_static(thorn, tint, out);
            fist_point = Some(thorn.transform_point3(Vec3::new(
                0.0,
                0.0,
                -super::projectiles::THORN_TIP,
            )));
        }
    }
    ViewmodelDrawInfo {
        muzzle: gun.transform_point3(w.muzzle()),
        muzzle_dir: gun.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
        fist: fist_point,
    }
}
