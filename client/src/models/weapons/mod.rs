//! Weapons: first-person viewmodels (also held by the third-person Shrike), the right arm that
//! grips them, the punching left fist and the Thorn throw.
//!
//! Weapon model space: the grip (where the hand closes) is at the origin, the barrel points
//! down -Z, top is +Y, units are metres. Glow parts are white; their colour comes from
//! `Pose::glow` (the colour of the selected ability, [`ability_color`], brighter while
//! charging / firing). Core parts are each weapon's own fixed hot colour (the Kiln's firebox,
//! red-hot rivets, the Cinderbox's cinders, the Longwick's arrow flame) and flicker through
//! `Pose::core`.
//!
//! The client draws the shared [`WeaponKind`] (wire ids: `shared::loadout`); everything visual
//! about a weapon (model, muzzle, colours, how it is held) comes from [`WeaponLook`].

pub mod arclash;
pub mod arm;
pub mod bellows;
pub mod cinderbox;
pub mod fist;
pub mod kiln;
pub mod lancet;
pub mod longwick;
/// Melee (docs/refit.md): how the Trimmer and the Tamper are held and swung.
pub mod melee;
pub mod quill;
pub mod riveter;
pub mod tamper;
pub mod thresher;
pub mod trimmer;
pub mod tumbler;

use engine::gfx::DrawItem;
use engine::glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};
pub use shared::loadout::{Ability, WeaponKind};

use super::ModelId;
use super::library::{ModelLibrary, ViewmodelDrawInfo};
use super::rig::{Pose, Tint};
use super::shrike::punch_extension;

/// The weapon of a replicated / input weapon id (unknown ids draw the Quill).
pub fn weapon_kind(id: u8) -> WeaponKind {
    WeaponKind::from_id(id).unwrap_or_default()
}

/// Whether the third-person Shrike holds weapon `id` with both hands.
pub fn two_handed(id: u8) -> bool {
    weapon_kind(id).two_handed()
}

/// What the client draws for a weapon: its model, muzzle, colours and how it is held.
pub trait WeaponLook: Copy {
    fn model(self) -> ModelId;
    /// Muzzle point in weapon model space (the Longwick: the nocked arrow's head).
    fn muzzle(self) -> Vec3;
    /// Held with both hands by the third-person Shrike (the Quill and Lancet are one-handed).
    fn two_handed(self) -> bool;
    /// Glow colour of alt mode 0 / 1 of the weapon's *default build*
    /// ([`WeaponKind::default_build`]); see [`ability_color`] for any build.
    fn glow_color(self, alt_mode: u8) -> Vec4;
    /// Colour of the primary fire (tracers, beams, muzzle flashes): Quill pale gold, Bellows
    /// amber, Arclash blue-white, Tumbler fuse orange, Lancet steel-cyan, Thresher hot amber,
    /// Riveter red-hot, Cinderbox cinder orange, Longwick wick gold, Kiln ember.
    fn primary_color(self) -> Vec4;
}

impl WeaponLook for WeaponKind {
    fn model(self) -> ModelId {
        match self {
            WeaponKind::Quill => ModelId::Quill,
            WeaponKind::Bellows => ModelId::Bellows,
            WeaponKind::Arclash => ModelId::Arclash,
            WeaponKind::Tumbler => ModelId::Tumbler,
            WeaponKind::Lancet => ModelId::Lancet,
            WeaponKind::Thresher => ModelId::Thresher,
            WeaponKind::Riveter => ModelId::Riveter,
            WeaponKind::Cinderbox => ModelId::Cinderbox,
            WeaponKind::Longwick => ModelId::Longwick,
            WeaponKind::Kiln => ModelId::Kiln,
            // Placeholders until the melee models exist (arsenal-client, docs/refit.md).
            WeaponKind::Trimmer => ModelId::Lancet,
            WeaponKind::Tamper => ModelId::Tumbler,
        }
    }

    fn muzzle(self) -> Vec3 {
        match self {
            WeaponKind::Quill => quill::MUZZLE,
            WeaponKind::Bellows => bellows::MUZZLE,
            WeaponKind::Arclash => arclash::MUZZLE,
            WeaponKind::Tumbler => tumbler::MUZZLE,
            WeaponKind::Lancet => lancet::MUZZLE,
            WeaponKind::Thresher => thresher::MUZZLE,
            WeaponKind::Riveter => riveter::MUZZLE,
            WeaponKind::Cinderbox => cinderbox::MUZZLE,
            WeaponKind::Longwick => longwick::MUZZLE,
            WeaponKind::Kiln => kiln::MUZZLE,
            WeaponKind::Trimmer => lancet::MUZZLE,
            WeaponKind::Tamper => tumbler::MUZZLE,
        }
    }

    fn two_handed(self) -> bool {
        !matches!(self, WeaponKind::Quill | WeaponKind::Lancet)
    }

    fn glow_color(self, alt_mode: u8) -> Vec4 {
        ability_color(self.default_build().ability(alt_mode % 2))
    }

    fn primary_color(self) -> Vec4 {
        match self {
            WeaponKind::Quill => Vec4::new(1.0, 0.9, 0.6, 1.0),
            WeaponKind::Bellows => Vec4::new(1.0, 0.7, 0.3, 1.0),
            WeaponKind::Arclash => Vec4::new(0.6, 0.85, 1.0, 1.0),
            WeaponKind::Tumbler => Vec4::new(1.0, 0.55, 0.15, 1.0),
            WeaponKind::Lancet => Vec4::new(0.75, 0.95, 1.0, 1.0),
            WeaponKind::Thresher => Vec4::new(1.0, 0.62, 0.25, 1.0),
            WeaponKind::Riveter => Vec4::new(1.0, 0.42, 0.18, 1.0),
            WeaponKind::Cinderbox => Vec4::new(1.0, 0.4, 0.12, 1.0),
            WeaponKind::Longwick => Vec4::new(1.0, 0.8, 0.42, 1.0),
            WeaponKind::Kiln => Vec4::new(1.0, 0.5, 0.1, 1.0),
            WeaponKind::Trimmer => Vec4::new(0.9, 0.92, 1.0, 1.0),
            WeaponKind::Tamper => Vec4::new(1.0, 0.66, 0.4, 1.0),
        }
    }
}

/// Glow colour of an ability: the held weapon's glow parts take the selected ability's colour
/// (and its tracers / effects use it). The two abilities of every class pool differ clearly:
///
/// | class | abilities |
/// |---|---|
/// | Precision | Brand crimson, Splinter ice, Skewer white-gold, Flurry mint |
/// | Scatter | Gust sky, Both Barrels orange, Slug brass, Flak rose |
/// | Rapid | Wisp violet, Discharge blue, Volley amber, Flare signal red |
/// | Explosive | Breach orange-red, Gravewell green, Limpet yellow, Mortar lilac |
/// | Draw | Brand crimson, Triple Nock lime, Pinning Arrow steel blue, Ember Arrow ember |
pub fn ability_color(a: Ability) -> Vec4 {
    let c = match a {
        Ability::Brand => Vec3::new(1.0, 0.22, 0.38),
        Ability::Splinter => Vec3::new(0.55, 0.95, 1.0),
        Ability::Gust => Vec3::new(0.72, 0.9, 1.0),
        Ability::BothBarrels => Vec3::new(1.0, 0.5, 0.15),
        Ability::Wisp => Vec3::new(0.75, 0.5, 1.0),
        Ability::Discharge => Vec3::new(0.4, 0.8, 1.0),
        Ability::Breach => Vec3::new(1.0, 0.4, 0.1),
        Ability::Gravewell => Vec3::new(0.45, 1.0, 0.55),
        Ability::Skewer => Vec3::new(1.0, 0.95, 0.55),
        Ability::Flurry => Vec3::new(0.35, 1.0, 0.72),
        Ability::Slug => Vec3::new(1.0, 0.82, 0.36),
        Ability::Flak => Vec3::new(1.0, 0.42, 0.68),
        Ability::Volley => Vec3::new(1.0, 0.76, 0.2),
        Ability::Flare => Vec3::new(1.0, 0.28, 0.2),
        Ability::Limpet => Vec3::new(0.95, 1.0, 0.3),
        Ability::Mortar => Vec3::new(0.82, 0.6, 1.0),
        Ability::TripleNock => Vec3::new(0.7, 1.0, 0.35),
        Ability::PinningArrow => Vec3::new(0.55, 0.75, 1.0),
        Ability::EmberArrow => Vec3::new(1.0, 0.55, 0.16),
        // Melee (placeholders until arsenal-client picks them).
        Ability::Lunge => Vec3::new(1.0, 0.3, 0.3),
        Ability::Whirl => Vec3::new(0.5, 0.9, 1.0),
        Ability::Hurl => Vec3::new(1.0, 0.85, 0.4),
        Ability::Upswing => Vec3::new(0.6, 1.0, 0.5),
    };
    c.extend(1.0)
}

/// Where the grip sits in view space (lower right of the screen).
fn view_offset(w: WeaponKind) -> Vec3 {
    match w {
        WeaponKind::Quill => Vec3::new(0.118, -0.112, -0.3),
        WeaponKind::Bellows => Vec3::new(0.13, -0.13, -0.31),
        WeaponKind::Arclash => Vec3::new(0.13, -0.122, -0.31),
        WeaponKind::Tumbler => Vec3::new(0.14, -0.148, -0.32),
        WeaponKind::Lancet => Vec3::new(0.115, -0.108, -0.3),
        WeaponKind::Thresher => Vec3::new(0.135, -0.13, -0.31),
        WeaponKind::Riveter => Vec3::new(0.13, -0.122, -0.31),
        WeaponKind::Cinderbox => Vec3::new(0.14, -0.15, -0.32),
        WeaponKind::Longwick => longwick::VIEW_OFFSET,
        WeaponKind::Kiln => Vec3::new(0.135, -0.135, -0.31),
        WeaponKind::Trimmer => Vec3::new(0.115, -0.108, -0.3),
        WeaponKind::Tamper => Vec3::new(0.14, -0.148, -0.32),
    }
}

/// Extra view rotation: the Longwick is canted so its long limbs cross the screen instead of
/// filling it.
fn view_rotation(w: WeaponKind) -> Quat {
    match w {
        WeaponKind::Longwick => longwick::view_rotation(),
        _ => Quat::IDENTITY,
    }
}

/// Recoil at full kick: (pull back in metres before scaling, muzzle climb in radians).
fn kick_amount(w: WeaponKind) -> (f32, f32) {
    match w {
        WeaponKind::Quill => (0.04, 0.2),
        WeaponKind::Bellows => (0.085, 0.45),
        WeaponKind::Arclash => (0.016, 0.05),
        WeaponKind::Tumbler => (0.09, 0.32),
        WeaponKind::Lancet => (0.03, 0.11),
        WeaponKind::Thresher => (0.07, 0.3),
        WeaponKind::Riveter => (0.022, 0.05),
        WeaponKind::Cinderbox => (0.05, 0.16),
        // The string snaps forward and the bow rocks towards the target.
        WeaponKind::Longwick => (-0.035, -0.06),
        WeaponKind::Kiln => (0.01, 0.025),
        WeaponKind::Trimmer => (0.03, 0.11),
        WeaponKind::Tamper => (0.09, 0.32),
    }
}

/// Everything the first-person viewmodel depends on.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ViewmodelAnim {
    /// The held weapon.
    pub weapon: WeaponKind,
    /// Selected ability (alt fire): the weapon glows in its colour ([`ability_color`]).
    pub ability: Ability,
    /// Recoil 0..1 (set to 1 on fire, decay over ~0.15–0.4 s depending on the weapon). Also
    /// folds the Quill's crest flat, pumps the Bellows, drives the Riveter's nose and hose,
    /// and snaps the Longwick's string.
    pub fire_kick: f32,
    /// Alt-fire charge 0..1 (Arclash Discharge: the capacitor brightens towards white, arcs
    /// jump between the prongs; Skewer: the barrel glows white-hot). The gun shakes above 0.6.
    pub alt_charge: f32,
    /// Longwick draw 0..1: the string and the nocked arrow pull back, the limbs flex, the
    /// left hand draws, the wick string glows hotter and strobes at full draw.
    pub draw: f32,
    /// Seconds since the Cinderbox's last salvo started (`None`: no salvo): its three tubes
    /// fire in turn, each with its own kick.
    pub salvo: Option<f32>,
    /// Kiln spray 0..1 (smoothed while the trigger is held): the firebox roars brighter, the
    /// damper rattles, the nozzle shakes.
    pub spray: f32,
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
    /// Drum angle in radians (Tumbler, Thresher; see [`DrumTurn`]).
    pub spin: f32,
    /// No weapon at all (gun game's Fist tier): both fists up instead of a gun.
    pub unarmed: bool,
    pub time: f64,
}

impl ViewmodelAnim {
    /// The weapon-pose inputs of this frame.
    pub fn held(&self) -> HeldWeapon {
        HeldWeapon {
            weapon: self.weapon,
            ability: self.ability,
            kick: self.fire_kick,
            charge: self.alt_charge,
            draw: self.draw,
            salvo: self.salvo,
            spray: self.spray,
            spin: self.spin,
            time: self.time,
        }
    }
}

/// What a weapon's pose depends on (first person: [`ViewmodelAnim::held`]; third person:
/// [`third_person_pose`]).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HeldWeapon {
    pub weapon: WeaponKind,
    pub ability: Ability,
    /// Recoil 0..1.
    pub kick: f32,
    /// Alt charge 0..1 (Discharge, Skewer).
    pub charge: f32,
    /// Longwick draw 0..1.
    pub draw: f32,
    /// Seconds since the Cinderbox salvo started.
    pub salvo: Option<f32>,
    /// Kiln spray 0..1.
    pub spray: f32,
    /// Drum angle (Tumbler, Thresher).
    pub spin: f32,
    pub time: f64,
}

/// The Tumbler's drum: turns one chamber per shot, easing in with a little overshoot (keep
/// one per local player; [`DrumTurn::kick`] on every Tumbler / Thresher shot,
/// [`DrumTurn::update`] every frame, feed [`DrumTurn::angle`] into [`ViewmodelAnim::spin`]).
/// The Thresher's drum has the same six-way symmetry, so one step is one shell for it too.
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

pub(crate) fn wave(time: f64, hz: f64) -> f32 {
    ((time * hz).fract() as f32 * std::f32::consts::TAU).sin()
}

/// Pose of a weapon model: glow colour / intensity and moving parts.
pub fn weapon_pose(h: &HeldWeapon) -> Pose {
    let base = ability_color(h.ability).truncate();
    let kick = unit(h.kick);
    let charge = unit(h.charge);
    let time = h.time;
    let pulse = 0.85 + 0.15 * wave(time, 1.3);
    let mut glow = 0.9 * pulse + kick * 0.8;
    let mut color = base;
    let mut pose = match h.weapon {
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
            glow += crackle;
            let flicker = (time * 23.0).fract() < 0.65;
            let arcing = (charge > 0.08 && flicker) || kick > 0.6;
            if !arcing {
                pose.hide(arclash::ARCS);
            }
            pose
        }
        // The melee weapons wear the Lancet / Tumbler models until theirs exist.
        WeaponKind::Tumbler | WeaponKind::Tamper => {
            let mut pose = Pose::rest(tumbler::JOINTS.len());
            pose.rotate(tumbler::DRUM, Quat::from_rotation_z(-h.spin));
            glow = 0.8 + 0.3 * wave(time, 1.1) + kick * 1.4;
            pose
        }
        WeaponKind::Lancet | WeaponKind::Trimmer => lancet::pose(h, &mut glow),
        WeaponKind::Thresher => thresher::pose(h, &mut glow),
        WeaponKind::Riveter => riveter::pose(h, &mut glow),
        WeaponKind::Cinderbox => cinderbox::pose(h, &mut glow),
        WeaponKind::Longwick => longwick::pose(h, &mut glow, &mut color),
        WeaponKind::Kiln => kiln::pose(h, &mut glow),
    };
    // Charging an alt (Discharge, Skewer): brighter and whiter towards full.
    if charge > 0.0 {
        glow += charge * (1.6 + 0.5 * wave(time, 17.0));
        color += Vec3::splat(charge * charge * 0.7);
    }
    pose.glow = (color * glow).extend(1.0);
    pose
}

/// Pose of the weapon held by a third-person Shrike (`firing` = recoil 0..1) with its
/// default build's first ability.
pub fn third_person_pose(weapon: WeaponKind, firing: f32, time: f64) -> Pose {
    weapon_pose(&HeldWeapon {
        weapon,
        ability: weapon.default_build().abilities[0],
        kick: firing,
        time,
        ..HeldWeapon::default()
    })
}

pub(crate) fn smoothstep(t: f32) -> f32 {
    let t = unit(t);
    t * t * (3.0 - 2.0 * t)
}

/// `x` clamped to 0..1, NaN = 0 (animation inputs come from outside).
pub(crate) fn unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// Weapons are modelled at real scale (the third-person Shrike holds them); first person
/// draws them smaller and closer so they keep a sensible share of the screen.
pub const VIEWMODEL_SCALE: f32 = 0.52;

/// Grip transform of the held weapon in view space.
pub fn viewmodel_transform(anim: &ViewmodelAnim) -> Mat4 {
    let w = anim.weapon;
    let kick = anim
        .fire_kick
        .clamp(0.0, 1.0)
        .max(cinderbox::salvo_kick(anim.salvo));
    let (kick_back, kick_pitch) = kick_amount(w);
    let lowered = (1.0 - smoothstep(anim.switch)).powi(2);
    let punch = punch_extension(anim.punch).max(throw_extension(anim.throw) * 0.6);
    let bob = anim.bob_amount.clamp(0.0, 1.0);
    let (s, c) = anim.bob_phase.sin_cos();
    let bob_offset = Vec3::new(s * 0.008, -(c * c) * 0.009 + 0.0045, 0.0) * bob;
    let sway = anim.sway.clamp(Vec2::splat(-3.0), Vec2::splat(3.0));
    let sway_offset = Vec3::new(-sway.x * 0.006, -sway.y * 0.006, 0.0);
    // Shake near a full alt charge, at full draw and while the Kiln roars.
    let tremble = if anim.alt_charge > 0.6 {
        (anim.alt_charge.min(1.0) - 0.6) / 0.4
    } else {
        0.0
    } + if w == WeaponKind::Longwick {
        0.35 * smoothstep((anim.draw - 0.85) / 0.15)
    } else {
        0.0
    } + if w == WeaponKind::Kiln {
        0.5 * unit(anim.spray)
    } else {
        0.0
    };
    let shake = Vec3::new(wave(anim.time, 31.0), wave(anim.time, 37.0), 0.0) * 0.0025 * tremble;
    // The bow comes up and in towards the eye as it is drawn.
    let draw = if w == WeaponKind::Longwick {
        smoothstep(anim.draw)
    } else {
        0.0
    };
    let offset = view_offset(w)
        + bob_offset * (1.0 - 0.6 * draw)
        + sway_offset
        + shake
        + longwick::DRAW_SHIFT * draw
        + Vec3::new(0.0, -0.24 * lowered, 0.04 * lowered)
        + Vec3::new(0.0, -0.008, kick_back * VIEWMODEL_SCALE) * kick
        + Vec3::new(0.025, -0.04, 0.02) * punch;
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        0.045 - sway.x * 0.025 + 0.05 * punch,
        kick_pitch * kick - 0.7 * lowered + sway.y * 0.02 - 0.1 * punch,
        -0.04 * s * bob - 0.2 * lowered,
    ) * view_rotation(w);
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

/// Unarmed (gun game's Fist tier): the right hand (the arm's closed glove) raised where a grip
/// would be, the left fist up in a guard until it punches or throws.
fn draw_fists(
    lib: &ModelLibrary,
    anim: &ViewmodelAnim,
    gun: Mat4,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) -> ViewmodelDrawInfo {
    let raise = Mat4::from_translation(Vec3::new(-0.02, 0.05, 0.0)) * Mat4::from_rotation_x(0.35);
    let right = gun * Mat4::from_quat(view_rotation(anim.weapon).inverse()) * raise;
    lib.get(ModelId::ArmRight).draw_static(right, tint, out);
    let left = fist_transform(anim).unwrap_or_else(|| {
        let bob = gun.w_axis.truncate() - view_offset(anim.weapon);
        Mat4::from_scale_rotation_translation(
            Vec3::splat(VIEWMODEL_SCALE),
            Quat::from_euler(EulerRot::YXZ, -0.12, 0.25, 0.35),
            Vec3::new(-0.11, -0.135, -0.3) + bob,
        )
    });
    let mut pose = Pose::rest(fist::JOINTS.len());
    let flare = 0.6 + punch_extension(anim.punch) * 2.0;
    pose.glow = Vec4::new(flare, flare * 0.85, flare * 0.7, 1.0);
    lib.get(ModelId::Fist).draw(&pose, left, tint, out);
    let knuckles = left.transform_point3(Vec3::new(0.0, 0.0, -0.02));
    ViewmodelDrawInfo {
        muzzle: right.transform_point3(Vec3::new(0.0, 0.0, -0.05)),
        muzzle_dir: Vec3::NEG_Z,
        fist: Some(knuckles),
    }
}

/// Draws the viewmodel; see [`ModelLibrary::draw_viewmodel`].
pub(crate) fn draw_viewmodel(
    lib: &ModelLibrary,
    anim: &ViewmodelAnim,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) -> ViewmodelDrawInfo {
    let w = anim.weapon;
    let gun = viewmodel_transform(anim);
    if anim.unarmed {
        return draw_fists(lib, anim, gun, tint, out);
    }
    let held = anim.held();
    let pose = weapon_pose(&held);
    lib.get(w.model()).draw(&pose, gun, tint, out);
    // The arm doesn't follow a weapon's cant (the bow's): it still runs down off the screen.
    let arm = gun * Mat4::from_quat(view_rotation(w).inverse());
    lib.get(ModelId::ArmRight).draw_static(arm, tint, out);

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
            fist_point =
                Some(thorn.transform_point3(Vec3::new(0.0, 0.0, -super::projectiles::THORN_TIP)));
        }
    } else if w == WeaponKind::Longwick && anim.draw > 0.01 {
        // The left hand pinches the nock and draws the string back.
        let hand = longwick::draw_hand(gun, anim.draw, anim.time);
        let mut pose = Pose::rest(fist::JOINTS.len());
        let ember = 0.5 + 0.5 * anim.draw;
        pose.glow = Vec4::new(ember, ember * 0.85, ember * 0.7, 1.0);
        lib.get(ModelId::Fist).draw(&pose, hand, tint, out);
    }
    ViewmodelDrawInfo {
        muzzle: gun.transform_point3(w.muzzle()),
        muzzle_dir: gun.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z),
        fist: fist_point,
    }
}
