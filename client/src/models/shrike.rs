//! The Shrike: the players' masked hunter (third person, ~1.85 m).
//!
//! Silhouette: a beaked porcelain mask with glowing eye slits under a leather hood with a
//! swept plume, a high-collared leather coat with two long tails, layered pauldrons, a brass
//! harness holding the chest lantern where the **wick** burns, wrapped forearms with brass
//! bracers, gloves and tall boots. Team colour: the wick flame and eye slits (`TeamGlow`), the
//! mask's ridge and plume (`crest`) and the scarf / pauldron edging (`trim`).
//!
//! Model space: feet at the origin, facing -Z, right = +X.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};

use super::PartSet;
use super::kit::{
    self, BRASS, DARK_LEATHER, FLAME, LEATHER, PORCELAIN, WRAP, add, add_mirrored, block, gem,
    limb, rgb,
};
use super::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const TORSO: usize = 1;
/// The wick flame in the chest lantern (team-tinted, flickers with `Pose::glow`).
pub const WICK: usize = 2;
pub const TRIM: usize = 3;
pub const HEAD: usize = 4;
/// Eye slits of the mask, lit by the wick (`TeamGlow`).
pub const VISOR: usize = 5;
pub const CREST: usize = 6;
pub const UPPER_ARM_L: usize = 7;
pub const FOREARM_L: usize = 8;
pub const HAND_L: usize = 9;
pub const UPPER_ARM_R: usize = 10;
pub const FOREARM_R: usize = 11;
pub const HAND_R: usize = 12;
pub const THIGH_L: usize = 13;
pub const SHIN_L: usize = 14;
pub const FOOT_L: usize = 15;
pub const THIGH_R: usize = 16;
pub const SHIN_R: usize = 17;
pub const FOOT_R: usize = 18;
pub const COAT_L: usize = 19;
pub const COAT_R: usize = 20;
/// The wick's white-hot heart (`Core`, `Pose::core`).
pub const EMBER: usize = 21;

/// Other players' Shrikes are drawn this much larger than the movement capsule (visual only,
/// easier to read at a distance): pass it to `ModelLibrary::draw_player_scaled`.
pub const REMOTE_SCALE: f32 = 1.1;

const HIP_Y: f32 = 0.92;
const KNEE_Y: f32 = 0.50;
const ANKLE_Y: f32 = 0.09;
const SHOULDER: [f32; 3] = [0.25, 1.41, 0.0];
const ELBOW: [f32; 3] = [0.27, 1.14, 0.0];
const WRIST: [f32; 3] = [0.28, 0.89, 0.0];
/// Centre of the chest lantern.
const LANTERN: [f32; 3] = [0.0, 1.29, -0.185];
/// Base of the wick flame (its pivot: the flame flickers in height around it).
const WICK_BASE: [f32; 3] = [LANTERN[0], LANTERN[1] - 0.06, LANTERN[2]];
/// Where the coat tails hang from (right side; the left is mirrored).
const COAT_HANG: [f32; 3] = [0.085, 1.03, 0.1];

pub const JOINTS: [JointDef; 22] = [
    joint("pelvis", ROOT, [0.0, 0.95, 0.0], Shade::Lit),
    joint("torso", PELVIS, [0.0, 1.02, 0.0], Shade::Lit),
    joint("wick", TORSO, WICK_BASE, Shade::TeamGlow),
    joint("trim", TORSO, [0.0, 1.02, 0.0], Shade::Team),
    joint("head", TORSO, [0.0, 1.52, 0.0], Shade::Lit),
    joint("visor", HEAD, [0.0, 1.52, 0.0], Shade::TeamGlow),
    joint("crest", HEAD, [0.0, 1.52, 0.0], Shade::Team),
    joint(
        "upper_arm_l",
        TORSO,
        [-SHOULDER[0], SHOULDER[1], SHOULDER[2]],
        Shade::Lit,
    ),
    joint(
        "forearm_l",
        UPPER_ARM_L,
        [-ELBOW[0], ELBOW[1], ELBOW[2]],
        Shade::Lit,
    ),
    joint(
        "hand_l",
        FOREARM_L,
        [-WRIST[0], WRIST[1], WRIST[2]],
        Shade::Lit,
    ),
    joint("upper_arm_r", TORSO, SHOULDER, Shade::Lit),
    joint("forearm_r", UPPER_ARM_R, ELBOW, Shade::Lit),
    joint("hand_r", FOREARM_R, WRIST, Shade::Lit),
    joint("thigh_l", PELVIS, [-0.11, HIP_Y, 0.0], Shade::Lit),
    joint("shin_l", THIGH_L, [-0.12, KNEE_Y, 0.0], Shade::Lit),
    joint("foot_l", SHIN_L, [-0.12, ANKLE_Y, 0.0], Shade::Lit),
    joint("thigh_r", PELVIS, [0.11, HIP_Y, 0.0], Shade::Lit),
    joint("shin_r", THIGH_R, [0.12, KNEE_Y, 0.0], Shade::Lit),
    joint("foot_r", SHIN_R, [0.12, ANKLE_Y, 0.0], Shade::Lit),
    joint(
        "coat_l",
        PELVIS,
        [-COAT_HANG[0], COAT_HANG[1], COAT_HANG[2]],
        Shade::Lit,
    ),
    joint("coat_r", PELVIS, COAT_HANG, Shade::Lit),
    joint("ember", WICK, WICK_BASE, Shade::Core),
];

/// Metres travelled per full locomotion cycle (two steps) at low speed.
pub const STRIDE: f32 = 2.0;
/// Fastest leg cycle (cycles per second); faster movement lengthens the stride instead.
pub const MAX_CYCLE_RATE: f32 = 2.6;

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// Dark cloth of the trousers and the waist.
const CLOTH: Vec4 = Vec4::new(0.33, 0.3, 0.32, 1.0);

// ---------------------------------------------------------------------------------------------
// Geometry

/// One side's arm pieces built on the right (+X) and mirrored for the left: leather sleeve,
/// wrapped forearm with a brass bracer, gloved hand.
fn right_arm() -> [MeshBuilder; 3] {
    let (s, e, w) = (v3(SHOULDER), v3(ELBOW), v3(WRIST));
    let mut upper = limb(
        s + Vec3::new(0.01, -0.02, 0.0),
        e,
        Vec2::new(0.055, 0.06),
        Vec2::new(0.045, 0.05),
        LEATHER,
    );
    add(&mut upper, gem(s, 0.066, 0, DARK_LEATHER));
    // Strap round the upper arm.
    let mid = s.lerp(e, 0.55);
    add(
        &mut upper,
        limb(
            mid + Vec3::Y * 0.02,
            mid - Vec3::Y * 0.02,
            Vec2::new(0.052, 0.057),
            Vec2::new(0.052, 0.057),
            DARK_LEATHER,
        ),
    );

    let mut fore = gem(e, 0.046, 0, DARK_LEATHER);
    add(
        &mut fore,
        limb(
            e + Vec3::new(0.0, -0.02, 0.0),
            w + Vec3::new(0.0, 0.02, 0.0),
            Vec2::new(0.042, 0.047),
            Vec2::new(0.044, 0.048),
            WRAP,
        ),
    );
    // Brass bracer with a leather edge.
    add(
        &mut fore,
        limb(
            w + Vec3::new(0.0, 0.03, 0.0),
            w + Vec3::new(-0.002, 0.13, 0.0),
            Vec2::new(0.053, 0.057),
            Vec2::new(0.05, 0.055),
            BRASS,
        ),
    );
    add(
        &mut fore,
        limb(
            w + Vec3::new(-0.002, 0.13, 0.0),
            w + Vec3::new(-0.003, 0.15, 0.0),
            Vec2::new(0.055, 0.059),
            Vec2::new(0.055, 0.059),
            DARK_LEATHER,
        ),
    );

    // Gloved hand with a flared cuff.
    let mut hand = block(
        w + Vec3::new(0.0, -0.058, -0.008),
        Vec3::new(0.042, 0.054, 0.052),
        Quat::IDENTITY,
        DARK_LEATHER,
    );
    add(
        &mut hand,
        limb(
            w,
            w + Vec3::new(0.0, 0.035, 0.0),
            Vec2::new(0.05, 0.054),
            Vec2::new(0.058, 0.062),
            LEATHER,
        ),
    );
    add(
        &mut hand,
        block(
            w + Vec3::new(-0.047, -0.04, -0.03),
            Vec3::new(0.012, 0.03, 0.018),
            Quat::IDENTITY,
            DARK_LEATHER,
        ),
    );
    [upper, fore, hand]
}

/// One side's leg pieces (right; the left is mirrored): trousers with a thigh strap, tall boot
/// with a folded cuff, wraps and buckles.
fn right_leg() -> [MeshBuilder; 3] {
    let hip = Vec3::new(0.11, HIP_Y, 0.0);
    let knee = Vec3::new(0.12, KNEE_Y, 0.0);
    let ankle = Vec3::new(0.12, ANKLE_Y, 0.0);
    let mut thigh = limb(
        hip + Vec3::Y * 0.01,
        knee + Vec3::Y * 0.03,
        Vec2::new(0.08, 0.085),
        Vec2::new(0.058, 0.062),
        CLOTH,
    );
    add(&mut thigh, gem(hip, 0.07, 0, CLOTH));
    let strap = hip.lerp(knee, 0.45);
    add(
        &mut thigh,
        limb(
            strap + Vec3::Y * 0.025,
            strap - Vec3::Y * 0.02,
            Vec2::new(0.074, 0.078),
            Vec2::new(0.072, 0.076),
            DARK_LEATHER,
        ),
    );
    add(
        &mut thigh,
        block(
            strap + Vec3::new(0.072, 0.0, 0.0),
            Vec3::new(0.01, 0.022, 0.02),
            Quat::IDENTITY,
            BRASS,
        ),
    );

    let mut shin = gem(knee, 0.05, 0, DARK_LEATHER);
    add(
        &mut shin,
        limb(
            knee + Vec3::new(0.0, -0.02, 0.005),
            ankle + Vec3::new(0.0, 0.02, 0.01),
            Vec2::new(0.056, 0.064),
            Vec2::new(0.045, 0.052),
            DARK_LEATHER,
        ),
    );
    // Folded boot cuff below the knee.
    add(
        &mut shin,
        limb(
            knee + Vec3::new(0.0, -0.01, 0.0),
            knee + Vec3::new(0.0, -0.09, 0.0),
            Vec2::new(0.068, 0.076),
            Vec2::new(0.064, 0.072),
            LEATHER,
        ),
    );
    // Wraps and two buckles down the outside.
    add(
        &mut shin,
        limb(
            knee + Vec3::new(0.0, -0.2, 0.004),
            knee + Vec3::new(0.0, -0.26, 0.006),
            Vec2::new(0.056, 0.063),
            Vec2::new(0.054, 0.061),
            WRAP,
        ),
    );
    for y in [-0.15, -0.3] {
        add(
            &mut shin,
            block(
                knee + Vec3::new(0.055, y, 0.0),
                Vec3::new(0.008, 0.012, 0.016),
                Quat::IDENTITY,
                BRASS,
            ),
        );
    }

    let mut foot = gem(ankle, 0.044, 0, DARK_LEATHER);
    add(
        &mut foot,
        limb(
            Vec3::new(0.12, 0.06, 0.07),
            Vec3::new(0.12, 0.045, -0.15),
            Vec2::new(0.052, 0.05),
            Vec2::new(0.05, 0.032),
            DARK_LEATHER,
        ),
    );
    // Sole and a brass toe cap.
    add(
        &mut foot,
        limb(
            Vec3::new(0.12, 0.012, 0.08),
            Vec3::new(0.12, 0.012, -0.16),
            Vec2::new(0.055, 0.012),
            Vec2::new(0.052, 0.012),
            rgb(0x1a1414),
        ),
    );
    add(
        &mut foot,
        block(
            Vec3::new(0.12, 0.04, -0.15),
            Vec3::new(0.04, 0.018, 0.016),
            Quat::IDENTITY,
            BRASS,
        ),
    );
    [thigh, shin, foot]
}

/// 8-point chest ring at height `y` with a soft front keel.
fn chest_ring(y: f32, hw: f32, front: f32, keel: f32, back: f32) -> Vec<Vec3> {
    vec![
        Vec3::new(-hw, y, 0.0),
        Vec3::new(-hw * 0.72, y, front),
        Vec3::new(0.0, y, keel),
        Vec3::new(hw * 0.72, y, front),
        Vec3::new(hw, y, 0.0),
        Vec3::new(hw * 0.7, y, back),
        Vec3::new(0.0, y, back * 1.06),
        Vec3::new(-hw * 0.7, y, back),
    ]
}

/// A coat tail hanging from [`COAT_HANG`] (right side).
fn coat_tail() -> MeshBuilder {
    let top = v3(COAT_HANG);
    let ring = |offset: Vec3, half: Vec2| kit::rect_ring(top + offset, Vec3::X, Vec3::Z, half);
    let mut m = kit::loft(
        &[
            ring(Vec3::ZERO, Vec2::new(0.085, 0.012)),
            ring(Vec3::new(0.012, -0.3, 0.02), Vec2::new(0.095, 0.011)),
            ring(Vec3::new(0.03, -0.62, 0.035), Vec2::new(0.11, 0.01)),
        ],
        LEATHER,
        false,
        (true, true),
        kit::Uv::PerFace,
    );
    // Dark hem.
    add(
        &mut m,
        limb(
            top + Vec3::new(0.03, -0.6, 0.036),
            top + Vec3::new(0.03, -0.66, 0.036),
            Vec2::new(0.112, 0.012),
            Vec2::new(0.112, 0.012),
            DARK_LEATHER,
        ),
    );
    m
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let rect = |y: f32, half: Vec2| kit::rect_ring(Vec3::new(0.0, y, 0.0), Vec3::X, Vec3::Z, half);

    // Pelvis: trousers, belt with a brass buckle and two pouches.
    p.add(
        PELVIS,
        kit::loft(
            &[
                rect(0.84, Vec2::new(0.125, 0.09)),
                rect(0.96, Vec2::new(0.165, 0.105)),
                rect(1.03, Vec2::new(0.115, 0.085)),
            ],
            CLOTH,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        PELVIS,
        kit::loft(
            &[
                rect(0.94, Vec2::new(0.172, 0.112)),
                rect(1.0, Vec2::new(0.15, 0.1)),
            ],
            DARK_LEATHER,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        PELVIS,
        block(
            Vec3::new(0.0, 0.97, -0.112),
            Vec3::new(0.035, 0.028, 0.008),
            Quat::IDENTITY,
            BRASS,
        ),
    );
    let pouch = block(
        Vec3::new(0.17, 0.92, -0.02),
        Vec3::new(0.025, 0.045, 0.05),
        Quat::IDENTITY,
        LEATHER,
    );
    add_mirrored(p.mesh(PELVIS), &pouch);

    // Torso: waist, layered leather jerkin, coat lapels and back, high collar.
    p.add(
        TORSO,
        kit::tube(
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.18, 0.0),
            0.085,
            0.1,
            6,
            CLOTH,
        ),
    );
    p.add(
        TORSO,
        kit::loft(
            &[
                chest_ring(1.12, 0.13, -0.08, -0.1, 0.085),
                chest_ring(1.27, 0.2, -0.12, -0.15, 0.115),
                chest_ring(1.4, 0.23, -0.12, -0.14, 0.125),
                chest_ring(1.49, 0.15, -0.08, -0.085, 0.09),
            ],
            LEATHER,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    let lapel = kit::plate(
        &[
            Vec3::new(0.06, 1.48, -0.1),
            Vec3::new(0.2, 1.4, -0.105),
            Vec3::new(0.19, 1.1, -0.09),
            Vec3::new(0.09, 1.12, -0.115),
        ],
        0.02,
        DARK_LEATHER,
    );
    add_mirrored(p.mesh(TORSO), &lapel);
    p.add(
        TORSO,
        limb(
            Vec3::new(0.0, 1.46, 0.12),
            Vec3::new(0.0, 1.03, 0.115),
            Vec2::new(0.21, 0.025),
            Vec2::new(0.16, 0.025),
            LEATHER,
        ),
    );
    let collar_ring = |y: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, 0.015),
            Vec3::X,
            Vec3::Z,
            Vec2::new(r, r * 0.95),
            8,
            0.0,
        )
    };
    p.add(
        TORSO,
        kit::loft(
            &[collar_ring(1.46, 0.105), collar_ring(1.62, 0.128)],
            DARK_LEATHER,
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    // Layered leather pauldrons with brass studs.
    let mut pauldron = kit::ellipsoid(
        Vec3::new(0.28, 1.46, 0.0),
        Vec3::new(0.11, 0.07, 0.12),
        Quat::from_rotation_z(-0.45),
        3,
        6,
        DARK_LEATHER,
        false,
    );
    add(
        &mut pauldron,
        kit::ellipsoid(
            Vec3::new(0.31, 1.41, 0.0),
            Vec3::new(0.1, 0.055, 0.115),
            Quat::from_rotation_z(-0.6),
            3,
            6,
            LEATHER,
            false,
        ),
    );
    add(&mut pauldron, gem(Vec3::new(0.27, 1.53, -0.05), 0.014, 0, BRASS));
    add(&mut pauldron, gem(Vec3::new(0.27, 1.53, 0.05), 0.014, 0, BRASS));
    add_mirrored(p.mesh(TORSO), &pauldron);
    // Harness: straps from the shoulders to the lantern and on to the belt, brass buckles.
    let lantern = v3(LANTERN);
    let strap_top = limb(
        Vec3::new(0.15, 1.47, -0.105),
        lantern + Vec3::new(0.055, 0.05, 0.03),
        Vec2::new(0.022, 0.008),
        Vec2::new(0.02, 0.008),
        DARK_LEATHER,
    );
    let strap_low = limb(
        lantern + Vec3::new(0.05, -0.07, 0.035),
        Vec3::new(0.1, 1.04, -0.118),
        Vec2::new(0.02, 0.008),
        Vec2::new(0.022, 0.008),
        DARK_LEATHER,
    );
    let buckle = block(
        Vec3::new(0.12, 1.42, -0.125),
        Vec3::new(0.02, 0.02, 0.008),
        Quat::from_rotation_z(0.4),
        BRASS,
    );
    for m in [&strap_top, &strap_low, &buckle] {
        add_mirrored(p.mesh(TORSO), m);
    }
    // Lantern cage: brass caps and bars round the wick, a sooty back plate.
    p.add(
        TORSO,
        kit::tube(
            lantern + Vec3::Y * 0.075,
            lantern + Vec3::Y * 0.1,
            0.062,
            0.045,
            8,
            BRASS,
        ),
    );
    p.add(
        TORSO,
        kit::tube(
            lantern - Vec3::Y * 0.1,
            lantern - Vec3::Y * 0.075,
            0.05,
            0.064,
            8,
            BRASS,
        ),
    );
    for k in 0..5 {
        let a = std::f32::consts::PI + k as f32 / 4.0 * std::f32::consts::PI;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            TORSO,
            limb(
                lantern + d * 0.056 - Vec3::Y * 0.08,
                lantern + d * 0.056 + Vec3::Y * 0.08,
                Vec2::splat(0.007),
                Vec2::splat(0.007),
                BRASS,
            ),
        );
    }
    p.add(
        TORSO,
        block(
            lantern + Vec3::new(0.0, 0.0, 0.035),
            Vec3::new(0.05, 0.075, 0.012),
            Quat::IDENTITY,
            rgb(0x141010),
        ),
    );

    // The wick: a tall flame with a forked tip, and its hot heart.
    let base = v3(WICK_BASE);
    let flame_ring = |y: f32, r: f32, phase: f32| {
        kit::ngon_ring(base + Vec3::Y * y, Vec3::X, Vec3::Z, Vec2::splat(r), 6, phase)
    };
    p.add(
        WICK,
        kit::loft(
            &[
                vec![base],
                flame_ring(0.03, 0.03, 0.0),
                flame_ring(0.07, 0.034, 0.3),
                flame_ring(0.105, 0.018, 0.6),
                vec![base + Vec3::new(0.008, 0.145, 0.0)],
            ],
            FLAME,
            true,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        WICK,
        kit::spike(
            base + Vec3::new(-0.01, 0.085, 0.0),
            base + Vec3::new(-0.03, 0.13, 0.0),
            0.012,
            4,
            FLAME,
        ),
    );
    p.add(
        EMBER,
        kit::ellipsoid(
            base + Vec3::Y * 0.04,
            Vec3::new(0.016, 0.03, 0.016),
            Quat::IDENTITY,
            2,
            6,
            Vec4::new(1.0, 0.95, 0.8, 1.0),
            false,
        ),
    );

    // Team trim: a scarf round the collar with a trailing end, and pauldron edging.
    let mut trim = kit::band(
        Vec3::new(0.0, 1.475, 0.01),
        Vec3::Y,
        0.11,
        0.045,
        0.03,
        8,
        kit::WHITE,
    );
    add(
        &mut trim,
        kit::plate(
            &[
                Vec3::new(-0.06, 1.47, 0.12),
                Vec3::new(-0.11, 1.47, 0.11),
                Vec3::new(-0.15, 1.18, 0.16),
                Vec3::new(-0.1, 1.16, 0.17),
            ],
            0.016,
            kit::WHITE,
        ),
    );
    let edge = kit::ellipsoid(
        Vec3::new(0.335, 1.385, 0.0),
        Vec3::new(0.022, 0.018, 0.118),
        Quat::from_rotation_z(-0.6),
        2,
        6,
        kit::WHITE,
        false,
    );
    add_mirrored(&mut trim, &edge);
    p.add(TRIM, trim);

    // Head: leather hood with a peak, the porcelain mask with its long hooked beak.
    p.add(
        HEAD,
        kit::ellipsoid(
            Vec3::new(0.0, 1.665, 0.025),
            Vec3::new(0.108, 0.125, 0.118),
            Quat::IDENTITY,
            4,
            8,
            DARK_LEATHER,
            false,
        ),
    );
    p.add(
        HEAD,
        kit::spike(
            Vec3::new(0.0, 1.73, 0.1),
            Vec3::new(0.0, 1.76, 0.2),
            0.045,
            4,
            DARK_LEATHER,
        ),
    );
    kit::finish(p.mesh(HEAD));
    let mask_ring = |z: f32, y: f32, w: f32, h: f32| {
        kit::ngon_ring(Vec3::new(0.0, y, z), Vec3::X, Vec3::Y, Vec2::new(w, h), 8, 0.0)
    };
    let mut mask = kit::loft(
        &[
            mask_ring(-0.03, 1.665, 0.098, 0.106),
            mask_ring(-0.08, 1.668, 0.088, 0.097),
            mask_ring(-0.125, 1.654, 0.058, 0.07),
            mask_ring(-0.165, 1.632, 0.034, 0.046),
            mask_ring(-0.215, 1.604, 0.021, 0.03),
            mask_ring(-0.255, 1.575, 0.012, 0.018),
            vec![Vec3::new(0.0, 1.535, -0.272)],
        ],
        PORCELAIN,
        false,
        (true, false),
        kit::Uv::PerFace,
    );
    // Porcelain stays bright: a gentler bake than leather.
    mask.shade_by_normal(1.0, 0.92, 0.72);
    kit::add(p.mesh(HEAD), mask);

    // Eye slits on the mask (lit from inside by the wick).
    let slit = limb(
        Vec3::new(0.079, 1.703, -0.074),
        Vec3::new(0.034, 1.687, -0.122),
        Vec2::new(0.009, 0.012),
        Vec2::new(0.007, 0.01),
        kit::WHITE,
    );
    let mut visor = MeshBuilder::new();
    add_mirrored(&mut visor, &slit);
    p.add(VISOR, visor);

    // Crest: the painted ridge down the beak and a swept plume from the hood.
    // The ridge follows the beak in convex segments (top edge, then 1.7 cm below it).
    let ridge = [
        Vec3::new(0.0, 1.775, -0.02),
        Vec3::new(0.0, 1.732, -0.125),
        Vec3::new(0.0, 1.69, -0.168),
        Vec3::new(0.0, 1.648, -0.215),
    ];
    let mut crest = MeshBuilder::new();
    for w in ridge.windows(2) {
        let drop = Vec3::new(0.0, -0.017, 0.0);
        add(
            &mut crest,
            kit::plate(&[w[0], w[1], w[1] + drop, w[0] + drop], 0.02, kit::WHITE),
        );
    }
    for (k, side) in [(0.0f32, 0.0f32), (1.0, 0.035), (1.0, -0.035)] {
        add(
            &mut crest,
            kit::plate(
                &[
                    Vec3::new(side, 1.775, 0.0),
                    Vec3::new(side * 1.4, 1.83 - k * 0.03, 0.12),
                    Vec3::new(side * 1.8, 1.86 - k * 0.05, 0.26 - k * 0.04),
                    Vec3::new(side * 1.3, 1.79 - k * 0.03, 0.15),
                ],
                0.014,
                kit::WHITE,
            ),
        );
    }
    p.add(CREST, crest);

    // Limbs (right built, left mirrored) and the coat tails.
    let [upper, fore, hand] = right_arm();
    let [thigh, shin, foot] = right_leg();
    let tail = coat_tail();
    for (right, left, mesh) in [
        (UPPER_ARM_R, UPPER_ARM_L, upper),
        (FOREARM_R, FOREARM_L, fore),
        (HAND_R, HAND_L, hand),
        (THIGH_R, THIGH_L, thigh),
        (SHIN_R, SHIN_L, shin),
        (FOOT_R, FOOT_L, foot),
        (COAT_R, COAT_L, tail),
    ] {
        p.add(left, kit::mirrored_x(&mesh));
        p.add(right, mesh);
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() && i != HEAD {
            kit::finish(m);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else {
            match i {
                HEAD => kit::tex_porcelain(),
                FOREARM_L | FOREARM_R => kit::tex_wrap(),
                _ => kit::tex_leather(),
            }
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Transform from the right hand's joint frame to a held weapon's grip frame (weapon models
/// have their grip at the origin, barrel along -Z, top +Y).
pub fn grip_transform() -> Mat4 {
    Mat4::from_rotation_translation(
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        Vec3::new(0.0, -0.06, -0.012),
    )
}

// ---------------------------------------------------------------------------------------------
// Animation

/// Everything the Shrike's pose depends on. All fields are plain replicated / predicted state.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PlayerAnim {
    /// World-space velocity (m/s).
    pub velocity: Vec3,
    /// Look yaw (radians, 0 = -Z, + = left) — the body faces this way.
    pub yaw: f32,
    /// Look pitch (radians, + = up).
    pub pitch: f32,
    pub on_ground: bool,
    pub sliding: bool,
    pub dashing: bool,
    pub slamming: bool,
    /// World-space normal of the wall being clung to.
    pub wall_cling: Option<Vec3>,
    /// Weapon recoil 0..1 (1 = just fired, decaying).
    pub firing: f32,
    /// Punch progress 0..1 (0 = not punching).
    pub punching: f32,
    pub downed: bool,
    pub dead: bool,
    /// Seconds (any monotonically increasing clock).
    pub time: f64,
    /// Held weapon index (0 Quill (one-handed), 1 Bellows, 2 Arclash, 3 Tumbler).
    pub weapon: u8,
    /// Locomotion phase in cycles (keep it per player with [`PlayerAnim::advance_cycle`]).
    pub cycle: f32,
}

impl PlayerAnim {
    /// Advances a locomotion phase by `dt` at the given world velocity. Keeping the phase
    /// (instead of deriving it from time x speed) keeps the legs continuous when speed changes.
    pub fn advance_cycle(cycle: f32, velocity: Vec3, dt: f32) -> f32 {
        let speed = Vec2::new(velocity.x, velocity.z).length();
        let rate = (speed / STRIDE).min(MAX_CYCLE_RATE);
        (cycle + rate * dt).rem_euclid(1.0)
    }
}

fn rx(a: f32) -> Quat {
    Quat::from_rotation_x(a)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn wave(time: f64, hz: f64) -> f32 {
    ((time * hz).fract() as f32 * std::f32::consts::TAU).sin()
}

/// The wick flame's local transform for a flicker level (~0.75..1.25): taller and thinner
/// when it flares.
fn flame_scale(flick: f32) -> Mat4 {
    let h = flick.clamp(0.1, 2.0);
    Mat4::from_scale(Vec3::new(1.0 / h.sqrt(), h, 1.0 / h.sqrt()))
}

/// Punch extension curve: fast out (first 25 %), slower return.
pub fn punch_extension(p: f32) -> f32 {
    if p <= 0.0 || p >= 1.0 {
        0.0
    } else if p < 0.25 {
        smoothstep(p / 0.25)
    } else {
        1.0 - smoothstep((p - 0.25) / 0.75)
    }
}

/// Root transform for a Shrike standing at `feet`, facing `anim.yaw`.
pub fn root_transform(anim: &PlayerAnim, feet: Vec3) -> Mat4 {
    Mat4::from_rotation_translation(Quat::from_rotation_y(anim.yaw), feet)
}

/// Swing angles (around X, + = forward for hanging limbs) of one side.
#[derive(Clone, Copy, Default)]
struct Limb {
    upper: f32,
    lower: f32,
    end: f32,
}

/// Computes the Shrike's pose.
pub fn pose(a: &PlayerAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    // The wick flickers (a few incommensurate wobbles); the eye slits flicker with it.
    let flick = 1.0 + 0.12 * wave(t, 6.3) + 0.08 * wave(t + 0.37, 13.7) + 0.05 * wave(t, 23.1);
    pose.glow = Vec4::new(flick, flick, flick, 1.0);
    let heat = 1.1 * flick;
    pose.core = Vec4::new(heat, heat * 0.95, heat * 0.85, 1.0);
    pose.set(WICK, flame_scale(flick));

    if a.dead {
        return dead_pose(pose);
    }
    if a.downed {
        return downed_pose(pose, t);
    }

    let local_v = Quat::from_rotation_y(-a.yaw) * a.velocity;
    let flat = Vec2::new(local_v.x, local_v.z);
    let speed = flat.length();
    // Movement direction relative to facing; + = moving to the left.
    let mut move_angle = if speed > 0.5 {
        (-flat.x).atan2(-flat.y)
    } else {
        0.0
    };
    let mut backwards = false;
    if move_angle.abs() > 1.75 {
        move_angle -= std::f32::consts::PI * move_angle.signum();
        backwards = true;
    }
    // The hips turn part of the way towards the movement direction; the rest of the angle
    // tilts the legs' swing plane (side steps instead of sideways kicks).
    let hip_yaw = move_angle.clamp(-0.6, 0.6);
    let residual = move_angle - hip_yaw;
    let mut leg_axis = Vec3::new(-residual.sin(), 0.0, -residual.cos()).cross(Vec3::Y);

    let mut pelvis_offset = Vec3::ZERO;
    let mut lean = 0.0f32; // + = lean back (rotation about X on upright parts)
    let mut roll = 0.0f32;
    let mut torso_pitch = 0.0f32;
    let mut leg_l = Limb::default();
    let mut leg_r = Limb::default();
    let mut arm_l = Limb {
        upper: 0.1,
        lower: 0.35,
        end: 0.0,
    };
    let mut arm_l_out = 0.12f32; // sideways raise of the left arm (+ = outward)
    let mut two_handed = super::weapons::WeaponKind::from_index(a.weapon).two_handed();
    // World swing of the coat tails (radians about X, - = streaming back).
    let coat: f32;
    let mut flutter = 0.0f32;
    // Ground locomotion keeps the legs world-aligned even though the pelvis leans.
    let mut legs_upright = false;

    if a.sliding || a.slamming || a.dashing || a.wall_cling.is_some() || !a.on_ground {
        leg_axis = Vec3::X;
    }
    if a.sliding {
        coat = -1.3;
        flutter = 0.5;
        // Leaning back on the folded left leg, right leg stretched out in front. Leg angles
        // are relative to the leaning pelvis (world angle = lean + local).
        pelvis_offset.y = -0.6;
        lean = 0.5;
        leg_r = Limb {
            upper: 1.4 - lean,
            lower: -0.1,
            end: -0.5,
        };
        leg_l = Limb {
            upper: 0.9 - lean,
            lower: -2.4,
            end: 0.6,
        };
        arm_l = Limb {
            upper: -0.75,
            lower: 0.15,
            end: 0.0,
        };
        arm_l_out = 0.55;
        two_handed = false;
    } else if a.slamming {
        coat = -2.3;
        flutter = 1.0;
        pelvis_offset.y = 0.1;
        lean = -0.3;
        leg_l = Limb {
            upper: 1.7,
            lower: -2.1,
            end: 0.3,
        };
        leg_r = Limb {
            upper: 1.5,
            lower: -2.0,
            end: 0.3,
        };
        arm_l = Limb {
            upper: 0.6,
            lower: 0.9,
            end: 0.0,
        };
        two_handed = false;
    } else if a.dashing {
        coat = -1.15;
        flutter = 1.0;
        lean = -0.55;
        leg_l = Limb {
            upper: -0.75,
            lower: -0.7,
            end: 0.2,
        };
        leg_r = Limb {
            upper: 0.35,
            lower: -1.3,
            end: 0.3,
        };
        arm_l = Limb {
            upper: -1.0,
            lower: 0.2,
            end: 0.0,
        };
        arm_l_out = 0.3;
        two_handed = false;
    } else if let Some(normal) = a.wall_cling {
        coat = -0.3;
        let n = Quat::from_rotation_y(-a.yaw) * normal;
        let side = if n.x >= 0.0 { 1.0 } else { -1.0 }; // + = wall on the left
        roll = -0.3 * side;
        pelvis_offset.y = -0.12;
        leg_l = Limb {
            upper: 0.9,
            lower: -1.3,
            end: 0.2,
        };
        leg_r = Limb {
            upper: 0.4,
            lower: -0.9,
            end: 0.2,
        };
        if side > 0.0 {
            arm_l = Limb {
                upper: 0.2,
                lower: 0.25,
                end: 0.0,
            };
            arm_l_out = 1.35;
            two_handed = false;
        }
    } else if !a.on_ground {
        // Rising: one knee tucked. Falling: legs reach down, arms rise for balance.
        let rise = smoothstep(local_v.y / 6.0 * 0.5 + 0.5);
        // Rising: the tails trail below; falling: they lift and flap.
        coat = -0.25 - (1.0 - rise) * 0.95;
        flutter = 0.6;
        leg_l = Limb {
            upper: 0.4 + rise * 0.9,
            lower: -0.5 - rise * 1.2,
            end: 0.2,
        };
        leg_r = Limb {
            upper: 0.15 + rise * 0.1,
            lower: -0.3 - rise * 0.3,
            end: 0.1,
        };
        lean = -0.1;
        arm_l = Limb {
            upper: 0.4 - rise * 0.2,
            lower: 0.5,
            end: 0.0,
        };
        arm_l_out = 0.5 - rise * 0.25;
    } else {
        // Ground locomotion.
        let k = (speed / 9.0).clamp(0.0, 1.0);
        let phase = a.cycle * std::f32::consts::TAU * if backwards { -1.0 } else { 1.0 };
        let (s, c) = phase.sin_cos();
        let swing = 0.25 + 0.75 * k;
        let amp = if speed > 0.3 { swing } else { 0.0 };
        leg_l.upper = amp * 0.95 * s;
        leg_r.upper = -amp * 0.95 * s;
        leg_l.lower = -amp * (0.25 + 1.25 * c.max(0.0));
        leg_r.lower = -amp * (0.25 + 1.25 * (-c).max(0.0));
        leg_l.end = amp * 0.3 * c.max(0.0);
        leg_r.end = amp * 0.3 * (-c).max(0.0);
        arm_l.upper = -amp * 0.8 * s;
        arm_l.lower = 0.35 + amp * 0.6;
        pelvis_offset.y = -0.04 * amp - 0.05 * amp * (2.0 * phase).cos().abs() + 0.03 * amp;
        lean = -0.32 * k;
        roll = 0.12 * residual.sin() * k;
        legs_upright = true;
        coat = -(0.08 + 0.75 * k) + 0.07 * amp * s;
        flutter = k * 0.5;
        // Idle breathing.
        if speed <= 0.3 {
            let breathe = ((t * 0.5).fract() as f32 * std::f32::consts::TAU).sin();
            torso_pitch += 0.02 * breathe;
            leg_l = Limb {
                upper: 0.06,
                lower: -0.1,
                end: 0.04,
            };
            leg_r = Limb {
                upper: -0.05,
                lower: -0.06,
                end: 0.1,
            };
        }
    }

    // Coat tails: world swing minus the pelvis lean (they hang from it), a flutter at speed,
    // splayed a little apart; the two tails are slightly out of phase.
    for (part, side, phase) in [(COAT_L, -1.0f32, 0.0f64), (COAT_R, 1.0, 0.31)] {
        let flap = flutter * 0.1 * wave(t + phase, 11.0);
        pose.rotate(
            part,
            rx(coat + flap - lean) * Quat::from_rotation_z(side * (0.06 + 0.05 * flutter)),
        );
    }

    // Pelvis: lower / lean / roll, legs turn towards the movement direction.
    let pelvis_rot =
        Quat::from_rotation_y(hip_yaw) * Quat::from_euler(EulerRot::XYZ, lean, 0.0, roll);
    pose.rotate_move(PELVIS, pelvis_rot, pelvis_offset);
    // Torso counter-rotates so the upper body keeps facing the aim; pitch follows the aim a bit.
    let aim_torso = a.pitch.clamp(-1.2, 1.2) * 0.35;
    let punch = punch_extension(a.punching);
    let torso_yaw = -hip_yaw - 0.4 * punch;
    let torso_rot = Quat::from_rotation_y(torso_yaw) * rx(torso_pitch + aim_torso - lean * 0.35);
    pose.rotate(TORSO, torso_rot);
    pose.rotate(
        HEAD,
        Quat::from_rotation_y(0.4 * punch) * rx(a.pitch.clamp(-1.2, 1.2) * 0.5 + lean * 0.4),
    );

    for (thigh, shin, foot, leg) in [
        (THIGH_L, SHIN_L, FOOT_L, leg_l),
        (THIGH_R, SHIN_R, FOOT_R, leg_r),
    ] {
        let comp = if legs_upright {
            rx(-lean)
        } else {
            Quat::IDENTITY
        };
        pose.rotate(thigh, comp * Quat::from_axis_angle(leg_axis, leg.upper));
        pose.rotate(shin, rx(leg.lower));
        pose.rotate(foot, rx(leg.end));
    }

    // Right arm: hold the weapon on the aim line. Sum of X swings along the chain
    // (pelvis lean + torso pitch + arm) must equal 90 degrees + pitch for the gun to point at
    // the aim; the hand absorbs the remainder.
    let recoil = a.firing.clamp(0.0, 1.0);
    let chain = lean + torso_pitch + aim_torso - lean * 0.35;
    let target = std::f32::consts::FRAC_PI_2 + a.pitch.clamp(-1.3, 1.3);
    let (upper_r, fore_r) = if two_handed {
        (0.95, 0.45)
    } else {
        (1.15, 0.3)
    };
    let upper_r = upper_r + 0.45 * a.pitch.clamp(-1.3, 1.3) - 0.25 * recoil;
    let fore_r = fore_r + 0.45 * recoil;
    let hand_r = target - chain - upper_r - fore_r + 0.25 * recoil;
    pose.rotate(UPPER_ARM_R, Quat::from_rotation_y(0.32) * rx(upper_r));
    pose.rotate(FOREARM_R, rx(fore_r));
    pose.rotate(HAND_R, rx(hand_r));

    // Left arm: punch, support the weapon, or swing.
    if punch > 0.0 {
        let target = std::f32::consts::FRAC_PI_2 + a.pitch.clamp(-1.0, 1.0) * 0.6 - chain;
        let up = arm_l.upper + (target - 0.05 - arm_l.upper) * punch;
        let lo = arm_l.lower + (0.05 - arm_l.lower) * punch;
        pose.rotate(UPPER_ARM_L, Quat::from_rotation_y(-0.15 * punch) * rx(up));
        pose.rotate(FOREARM_L, rx(lo));
        pose.rotate(HAND_L, rx(0.0));
    } else if two_handed {
        pose.rotate(
            UPPER_ARM_L,
            Quat::from_rotation_y(-0.75) * rx(0.85 + 0.4 * a.pitch.clamp(-1.3, 1.3)),
        );
        pose.rotate(
            FOREARM_L,
            Quat::from_rotation_y(-0.3) * rx(0.85 + 0.3 * recoil),
        );
        pose.rotate(HAND_L, rx(0.2));
    } else {
        pose.rotate(
            UPPER_ARM_L,
            Quat::from_rotation_z(-arm_l_out) * rx(arm_l.upper),
        );
        pose.rotate(FOREARM_L, rx(arm_l.lower));
        pose.rotate(HAND_L, rx(arm_l.end));
    }
    pose
}

fn dead_pose(mut pose: Pose) -> Pose {
    // Flat on its back, limbs splayed.
    pose.rotate_move(PELVIS, rx(1.45), Vec3::new(0.0, -0.8, 0.0));
    pose.rotate(TORSO, rx(0.1) * Quat::from_rotation_z(0.15));
    pose.rotate(HEAD, Quat::from_rotation_y(0.7) * rx(0.3));
    pose.rotate(UPPER_ARM_L, Quat::from_rotation_z(-1.3) * rx(0.4));
    pose.rotate(FOREARM_L, rx(0.6));
    pose.rotate(UPPER_ARM_R, Quat::from_rotation_z(1.0) * rx(-0.2));
    pose.rotate(FOREARM_R, rx(0.9));
    pose.rotate(THIGH_L, Quat::from_rotation_z(-0.25) * rx(0.3));
    pose.rotate(SHIN_L, rx(-0.5));
    pose.rotate(THIGH_R, Quat::from_rotation_z(0.15));
    pose.rotate(FOOT_R, rx(0.6));
    // The wick is snuffed: no flame, dark eye slits, a dull smoulder in the lantern.
    pose.hide(WICK);
    pose.glow = Vec4::new(0.05, 0.05, 0.05, 1.0);
    pose.core = Vec4::new(0.35, 0.12, 0.06, 1.0);
    pose
}

fn downed_pose(mut pose: Pose, t: f64) -> Pose {
    // Kneeling on the right knee, slumped, left hand braced on the left knee.
    let sway = ((t * 0.7).fract() as f32 * std::f32::consts::TAU).sin() * 0.05;
    pose.rotate_move(PELVIS, rx(-0.1), Vec3::new(0.0, -0.4, 0.0));
    pose.rotate(THIGH_R, rx(-0.15));
    pose.rotate(SHIN_R, rx(-1.45));
    pose.rotate(FOOT_R, rx(0.9));
    pose.rotate(THIGH_L, rx(1.45));
    pose.rotate(SHIN_L, rx(-1.35));
    pose.rotate(FOOT_L, rx(0.0));
    pose.rotate(TORSO, rx(-0.55 + sway));
    pose.rotate(HEAD, rx(-0.45) * Quat::from_rotation_z(0.2));
    pose.rotate(UPPER_ARM_L, Quat::from_rotation_y(-0.3) * rx(0.75));
    pose.rotate(FOREARM_L, rx(0.55));
    pose.rotate(UPPER_ARM_R, Quat::from_rotation_z(0.2) * rx(0.35));
    pose.rotate(FOREARM_R, rx(0.4));
    // The wick gutters: a small, sputtering flame.
    let flicker = if (t * 9.0).fract() < 0.3 { 0.25 } else { 0.6 };
    pose.glow = Vec4::new(flicker, flicker, flicker, 1.0);
    pose.core = Vec4::new(flicker, flicker * 0.8, flicker * 0.6, 1.0);
    pose.set(WICK, flame_scale(0.35 + flicker * 0.3));
    pose.rotate(COAT_L, rx(-0.5) * Quat::from_rotation_z(-0.1));
    pose.rotate(COAT_R, rx(-0.45) * Quat::from_rotation_z(0.1));
    pose
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_is_on_the_right() {
        assert!(JOINTS[HAND_R].pivot.x > 0.0 && JOINTS[HAND_L].pivot.x < 0.0);
    }
}
