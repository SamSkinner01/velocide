//! The Shrike: the players' chrome / blood-red combat automaton (third person, ~1.8 m).
//!
//! Silhouette: bird-skull helm with a swept crest and a glowing V visor, keeled chest with a
//! blood core, two back-swept blades over the shoulders, talon feet. Team colour slot: crest,
//! pauldrons + back blades (`trim`) and the visor.
//!
//! Model space: feet at the origin, facing -Z, right = +X.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4};

use super::PartSet;
use super::kit::{self, BLOOD, CHROME, GUNMETAL, add, add_mirrored, block, gem, limb, spike};
use super::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const TORSO: usize = 1;
pub const CORE: usize = 2;
pub const TRIM: usize = 3;
pub const HEAD: usize = 4;
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

const HIP_Y: f32 = 0.92;
const KNEE_Y: f32 = 0.50;
const ANKLE_Y: f32 = 0.09;
const SHOULDER: [f32; 3] = [0.25, 1.41, 0.0];
const ELBOW: [f32; 3] = [0.27, 1.14, 0.0];
const WRIST: [f32; 3] = [0.28, 0.89, 0.0];

pub const JOINTS: [JointDef; 19] = [
    joint("pelvis", ROOT, [0.0, 0.95, 0.0], Shade::Lit),
    joint("torso", PELVIS, [0.0, 1.02, 0.0], Shade::Lit),
    joint("core", TORSO, [0.0, 1.31, -0.17], Shade::Core),
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
];

/// Metres travelled per full locomotion cycle (two steps) at low speed.
pub const STRIDE: f32 = 2.0;
/// Fastest leg cycle (cycles per second); faster movement lengthens the stride instead.
pub const MAX_CYCLE_RATE: f32 = 2.6;

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

// ---------------------------------------------------------------------------------------------
// Geometry

/// One side's arm / leg pieces built on the right (+X) and mirrored for the left.
fn right_arm() -> [MeshBuilder; 3] {
    let (s, e, w) = (v3(SHOULDER), v3(ELBOW), v3(WRIST));
    let mut upper = limb(
        s + Vec3::new(0.01, -0.02, 0.0),
        e,
        Vec2::new(0.05, 0.055),
        Vec2::new(0.04, 0.045),
        CHROME,
    );
    add(&mut upper, gem(s, 0.068, 0, GUNMETAL));

    let mut fore = gem(e, 0.048, 0, GUNMETAL);
    add(
        &mut fore,
        limb(
            e + Vec3::new(0.0, -0.02, 0.0),
            w + Vec3::new(0.0, 0.02, 0.0),
            Vec2::new(0.042, 0.048),
            Vec2::new(0.056, 0.06),
            CHROME,
        ),
    );
    // Blood-red blade fin along the outside of the forearm.
    add(
        &mut fore,
        kit::plate(
            &[
                Vec3::new(0.325, 1.09, 0.02),
                Vec3::new(0.355, 0.98, 0.05),
                Vec3::new(0.34, 0.93, 0.03),
                Vec3::new(0.325, 0.95, 0.0),
            ],
            0.016,
            BLOOD,
        ),
    );

    let mut hand = block(
        w + Vec3::new(0.0, -0.055, -0.008),
        Vec3::new(0.04, 0.052, 0.05),
        Quat::IDENTITY,
        GUNMETAL,
    );
    add(
        &mut hand,
        block(
            w + Vec3::new(0.0, -0.075, -0.054),
            Vec3::new(0.043, 0.032, 0.012),
            Quat::IDENTITY,
            BLOOD,
        ),
    );
    add(
        &mut hand,
        block(
            w + Vec3::new(-0.045, -0.04, -0.03),
            Vec3::new(0.012, 0.03, 0.018),
            Quat::IDENTITY,
            GUNMETAL,
        ),
    );
    [upper, fore, hand]
}

fn right_leg() -> [MeshBuilder; 3] {
    let hip = Vec3::new(0.11, HIP_Y, 0.0);
    let knee = Vec3::new(0.12, KNEE_Y, 0.0);
    let ankle = Vec3::new(0.12, ANKLE_Y, 0.0);
    let mut thigh = limb(
        hip + Vec3::Y * 0.01,
        knee + Vec3::Y * 0.03,
        Vec2::new(0.078, 0.082),
        Vec2::new(0.052, 0.056),
        CHROME,
    );
    add(&mut thigh, gem(hip, 0.07, 0, GUNMETAL));
    // Armour flap on the outside of the thigh.
    add(
        &mut thigh,
        limb(
            hip + Vec3::new(0.07, 0.0, 0.0),
            hip + Vec3::new(0.085, -0.2, 0.0),
            Vec2::new(0.012, 0.07),
            Vec2::new(0.01, 0.05),
            GUNMETAL,
        ),
    );

    let mut shin = gem(knee, 0.052, 0, GUNMETAL);
    add(
        &mut shin,
        limb(
            knee + Vec3::new(0.0, -0.02, 0.005),
            ankle + Vec3::new(0.0, 0.02, 0.01),
            Vec2::new(0.05, 0.062),
            Vec2::new(0.036, 0.042),
            CHROME,
        ),
    );
    // Blood-red knee spike pointing forward-up.
    add(
        &mut shin,
        spike(
            knee + Vec3::new(0.0, -0.02, -0.035),
            knee + Vec3::new(0.0, 0.11, -0.12),
            0.04,
            4,
            BLOOD,
        ),
    );
    // Shin guard.
    add(
        &mut shin,
        limb(
            knee + Vec3::new(0.0, -0.06, -0.05),
            ankle + Vec3::new(0.0, 0.08, -0.035),
            Vec2::new(0.035, 0.012),
            Vec2::new(0.026, 0.01),
            GUNMETAL,
        ),
    );

    let mut foot = gem(ankle, 0.042, 0, GUNMETAL);
    add(
        &mut foot,
        limb(
            Vec3::new(0.12, 0.05, 0.06),
            Vec3::new(0.12, 0.04, -0.12),
            Vec2::new(0.052, 0.045),
            Vec2::new(0.05, 0.03),
            GUNMETAL,
        ),
    );
    // Talons.
    add(
        &mut foot,
        spike(
            Vec3::new(0.095, 0.03, -0.11),
            Vec3::new(0.085, 0.0, -0.21),
            0.022,
            4,
            CHROME,
        ),
    );
    add(
        &mut foot,
        spike(
            Vec3::new(0.145, 0.03, -0.11),
            Vec3::new(0.16, 0.0, -0.2),
            0.022,
            4,
            CHROME,
        ),
    );
    add(
        &mut foot,
        spike(
            Vec3::new(0.12, 0.05, 0.06),
            Vec3::new(0.12, 0.01, 0.15),
            0.02,
            4,
            CHROME,
        ),
    );
    [thigh, shin, foot]
}

/// Keeled 8-point chest ring at height `y`.
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

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);

    // Pelvis.
    let pelvis = kit::loft(
        &[
            kit::rect_ring(
                Vec3::new(0.0, 0.84, 0.0),
                Vec3::X,
                Vec3::Z,
                Vec2::new(0.12, 0.085),
            ),
            kit::rect_ring(
                Vec3::new(0.0, 0.96, 0.0),
                Vec3::X,
                Vec3::Z,
                Vec2::new(0.16, 0.1),
            ),
            kit::rect_ring(
                Vec3::new(0.0, 1.03, 0.0),
                Vec3::X,
                Vec3::Z,
                Vec2::new(0.1, 0.075),
            ),
        ],
        GUNMETAL,
        false,
        (true, true),
        kit::Uv::PerFace,
    );
    p.add(PELVIS, pelvis);
    p.add(
        PELVIS,
        kit::plate(
            &[
                Vec3::new(-0.085, 0.99, -0.1),
                Vec3::new(0.085, 0.99, -0.1),
                Vec3::new(0.0, 0.82, -0.085),
            ],
            0.025,
            CHROME,
        ),
    );

    // Torso: waist, keeled chest, neck.
    p.add(
        TORSO,
        kit::tube(
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.18, 0.0),
            0.075,
            0.09,
            6,
            GUNMETAL,
        ),
    );
    for y in [1.06, 1.12] {
        p.add(
            TORSO,
            kit::tube(
                Vec3::new(0.0, y, 0.0),
                Vec3::new(0.0, y + 0.025, 0.0),
                0.095,
                0.095,
                6,
                kit::BLACK_STEEL,
            ),
        );
    }
    p.add(
        TORSO,
        kit::loft(
            &[
                chest_ring(1.15, 0.12, -0.075, -0.1, 0.075),
                chest_ring(1.28, 0.2, -0.12, -0.2, 0.11),
                chest_ring(1.4, 0.235, -0.12, -0.16, 0.12),
                chest_ring(1.5, 0.15, -0.075, -0.085, 0.085),
            ],
            CHROME,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        TORSO,
        kit::tube(
            Vec3::new(0.0, 1.47, 0.0),
            Vec3::new(0.0, 1.57, -0.01),
            0.055,
            0.045,
            6,
            GUNMETAL,
        ),
    );
    // Spine ridge / exhaust between the back blades.
    p.add(
        TORSO,
        limb(
            Vec3::new(0.0, 1.2, 0.11),
            Vec3::new(0.0, 1.46, 0.12),
            Vec2::new(0.035, 0.03),
            Vec2::new(0.03, 0.04),
            GUNMETAL,
        ),
    );

    // Blood core in the keel.
    p.add(
        CORE,
        kit::crystal(
            Vec3::new(0.0, 1.3, -0.19),
            Vec3::Y,
            (0.065, 0.06),
            0.04,
            4,
            Vec4::new(1.0, 0.28, 0.16, 1.0),
        ),
    );

    // Team trim: pauldrons and the back blades.
    let mut pauldron = kit::ellipsoid(
        Vec3::new(0.285, 1.46, 0.0),
        Vec3::new(0.105, 0.075, 0.115),
        Quat::from_rotation_z(-0.4),
        3,
        6,
        kit::WHITE,
        false,
    );
    add(
        &mut pauldron,
        kit::plate(
            &[
                Vec3::new(0.24, 1.52, -0.06),
                Vec3::new(0.31, 1.6, 0.02),
                Vec3::new(0.25, 1.52, 0.08),
            ],
            0.02,
            kit::WHITE,
        ),
    );
    let blade = kit::plate(
        &[
            Vec3::new(0.08, 1.44, 0.1),
            Vec3::new(0.08, 1.27, 0.13),
            Vec3::new(0.3, 1.84, 0.44),
        ],
        0.022,
        kit::WHITE,
    );
    let mut trim = MeshBuilder::new();
    add_mirrored(&mut trim, &pauldron);
    add_mirrored(&mut trim, &blade);
    // Lacquered V plate over the keel (around the core).
    let chest_plate = kit::plate(
        &[
            Vec3::new(0.025, 1.43, -0.155),
            Vec3::new(0.16, 1.4, -0.13),
            Vec3::new(0.06, 1.24, -0.17),
            Vec3::new(0.02, 1.2, -0.12),
        ],
        0.02,
        kit::WHITE,
    );
    add_mirrored(&mut trim, &chest_plate);
    p.add(TRIM, trim);

    // Helm: bird skull with a beak.
    let helm_axis_side = Vec3::X;
    let helm_axis_up = Vec3::Y;
    let ring = |z: f32, y: f32, r: Vec2| {
        kit::ngon_ring(
            Vec3::new(0.0, y, z),
            helm_axis_side,
            helm_axis_up,
            r,
            6,
            0.0,
        )
    };
    p.add(
        HEAD,
        kit::loft(
            &[
                ring(0.11, 1.66, Vec2::new(0.055, 0.06)),
                ring(0.03, 1.665, Vec2::new(0.095, 0.1)),
                ring(-0.07, 1.645, Vec2::new(0.088, 0.085)),
                ring(-0.15, 1.61, Vec2::new(0.035, 0.04)),
                vec![Vec3::new(0.0, 1.585, -0.215)],
            ],
            CHROME,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    // Jaw guard.
    p.add(
        HEAD,
        limb(
            Vec3::new(0.0, 1.575, -0.02),
            Vec3::new(0.0, 1.565, -0.14),
            Vec2::new(0.06, 0.03),
            Vec2::new(0.022, 0.02),
            GUNMETAL,
        ),
    );

    // Visor: angry V of two glowing slits.
    let slit = limb(
        Vec3::new(0.083, 1.68, -0.05),
        Vec3::new(0.012, 1.645, -0.128),
        Vec2::new(0.012, 0.016),
        Vec2::new(0.01, 0.012),
        kit::WHITE,
    );
    let mut visor = MeshBuilder::new();
    add_mirrored(&mut visor, &slit);
    p.add(VISOR, visor);

    // Crest blade sweeping back.
    p.add(
        CREST,
        kit::plate(
            &[
                Vec3::new(0.0, 1.735, -0.07),
                Vec3::new(0.0, 1.77, 0.04),
                Vec3::new(0.0, 1.88, 0.26),
                Vec3::new(0.0, 1.7, 0.13),
            ],
            0.028,
            kit::WHITE,
        ),
    );

    // Limbs (right built, left mirrored).
    let [upper, fore, hand] = right_arm();
    let [thigh, shin, foot] = right_leg();
    for (right, left, mesh) in [
        (UPPER_ARM_R, UPPER_ARM_L, upper),
        (FOREARM_R, FOREARM_L, fore),
        (HAND_R, HAND_L, hand),
        (THIGH_R, THIGH_L, thigh),
        (SHIN_R, SHIN_L, shin),
        (FOOT_R, FOOT_L, foot),
    ] {
        p.add(left, kit::mirrored_x(&mesh));
        p.add(right, mesh);
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else {
            kit::tex_chrome()
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
    /// Held weapon index (0 Lancet, 1 Thresher, 2 Riveter, 3 Cinderbox).
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
    // Heartbeat on the blood core.
    let beat = ((t * 1.6).fract() as f32 * std::f32::consts::TAU)
        .sin()
        .max(0.0);
    let core = 0.85 + beat * 0.35;
    pose.core = Vec4::new(core, core, core, 1.0);

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
    let mut two_handed = a.weapon != 0;
    // Ground locomotion keeps the legs world-aligned even though the pelvis leans.
    let mut legs_upright = false;

    if a.sliding || a.slamming || a.dashing || a.wall_cling.is_some() || !a.on_ground {
        leg_axis = Vec3::X;
    }
    if a.sliding {
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
    // Flat on its back, limbs splayed, visor dark.
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
    pose.glow = Vec4::new(0.15, 0.15, 0.15, 1.0);
    pose.core = pose.glow;
    pose
}

fn downed_pose(mut pose: Pose, t: f64) -> Pose {
    // Kneeling on the right knee, slumped, left hand braced on the left knee; the core
    // flickers weakly.
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
    let flicker = if (t * 9.0).fract() < 0.3 { 0.25 } else { 0.6 };
    pose.glow = Vec4::new(flicker, flicker, flicker, 1.0);
    pose.core = pose.glow;
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
