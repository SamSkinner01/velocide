//! Husk: fodder melee. A gaunt, hunched ~1.9 m humanoid with grey wet skin, an exposed
//! ribcage, a cracked bone mask with slit eyes, an underbite jaw and long arms ending in three
//! blood-red bone claws. Telegraph: the claws (`Glow`) flare red-hot while both arms rise
//! overhead; the slit eyes (`Core`) always smoulder and flare with them.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, rx, ry, rz, seed_phase, smoothstep, telegraph, walk_legs, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, add_mirrored, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const SPINE: usize = 1;
pub const CHEST: usize = 2;
pub const HEAD: usize = 3;
pub const EYES: usize = 4;
pub const JAW: usize = 5;
pub const UPPER_ARM_L: usize = 6;
pub const FOREARM_L: usize = 7;
pub const HAND_L: usize = 8;
pub const CLAWS_L: usize = 9;
pub const UPPER_ARM_R: usize = 10;
pub const FOREARM_R: usize = 11;
pub const HAND_R: usize = 12;
pub const CLAWS_R: usize = 13;
pub const THIGH_L: usize = 14;
pub const SHIN_L: usize = 15;
pub const THIGH_R: usize = 16;
pub const SHIN_R: usize = 17;

const SHOULDER: [f32; 3] = [0.2, 1.53, 0.02];
const ELBOW: [f32; 3] = [0.22, 1.19, 0.02];
const WRIST: [f32; 3] = [0.23, 0.86, 0.0];
const KNUCKLE: [f32; 3] = [0.235, 0.775, -0.01];
const HIP: [f32; 3] = [0.1, 0.94, 0.0];
const KNEE: [f32; 3] = [0.11, 0.52, 0.0];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 18] = [
    joint("pelvis", ROOT, [0.0, 0.97, 0.0], Shade::Lit),
    joint("spine", PELVIS, [0.0, 1.03, 0.0], Shade::Lit),
    joint("chest", SPINE, [0.0, 1.28, 0.02], Shade::Lit),
    joint("head", CHEST, [0.0, 1.6, -0.02], Shade::Lit),
    joint("eyes", HEAD, [0.0, 1.6, -0.02], Shade::Core),
    joint("jaw", HEAD, [0.0, 1.665, -0.03], Shade::Lit),
    joint("upper_arm_l", CHEST, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", UPPER_ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("hand_l", FOREARM_L, neg_x(WRIST), Shade::Lit),
    joint("claws_l", HAND_L, neg_x(KNUCKLE), Shade::Glow),
    joint("upper_arm_r", CHEST, SHOULDER, Shade::Lit),
    joint("forearm_r", UPPER_ARM_R, ELBOW, Shade::Lit),
    joint("hand_r", FOREARM_R, WRIST, Shade::Lit),
    joint("claws_r", HAND_R, KNUCKLE, Shade::Glow),
    joint("thigh_l", PELVIS, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", PELVIS, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let skin = rgb(0xc4c6b4);
    let dark = rgb(0x7e7a70);
    let bone = rgb(0xe2d6b8);
    let claw = Vec4::new(1.0, 0.3, 0.16, 1.0);

    // Pelvis + hanging rags.
    p.add(
        PELVIS,
        kit::loft(
            &[
                kit::rect_ring(
                    Vec3::new(0.0, 0.88, 0.0),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::new(0.1, 0.07),
                ),
                kit::rect_ring(
                    Vec3::new(0.0, 0.99, 0.0),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::new(0.125, 0.08),
                ),
                kit::rect_ring(
                    Vec3::new(0.0, 1.05, 0.0),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::new(0.08, 0.06),
                ),
            ],
            dark,
            false,
            (true, true),
            kit::Uv::Density(3.0),
        ),
    );
    for (x, len) in [(-0.06, 0.3), (0.05, 0.22)] {
        p.add(
            PELVIS,
            kit::plate(
                &[
                    Vec3::new(x - 0.045, 0.98, -0.085),
                    Vec3::new(x + 0.045, 0.98, -0.085),
                    Vec3::new(x + 0.02, 0.98 - len, -0.1),
                    Vec3::new(x - 0.03, 0.98 - len * 0.8, -0.1),
                ],
                0.012,
                rgb(0x3a2a22),
            ),
        );
    }

    // Emaciated waist with vertebrae.
    p.add(
        SPINE,
        kit::tube(
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 1.3, 0.01),
            0.062,
            0.075,
            6,
            skin,
        ),
    );
    for i in 0..4 {
        p.add(
            SPINE,
            gem(
                Vec3::new(0.0, 1.06 + i as f32 * 0.065, 0.068),
                0.022,
                0,
                bone,
            ),
        );
    }

    // Ribcage chest, rib hoops, shoulder-blade spikes, neck.
    let ell = |y: f32, rx: f32, rz: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, 0.02),
            Vec3::X,
            Vec3::Z,
            Vec2::new(rx, rz),
            8,
            0.0,
        )
    };
    p.add(
        CHEST,
        kit::loft(
            &[
                ell(1.24, 0.11, 0.085),
                ell(1.38, 0.155, 0.11),
                ell(1.52, 0.15, 0.1),
                ell(1.59, 0.08, 0.07),
            ],
            skin,
            false,
            (true, true),
            kit::Uv::Density(3.0),
        ),
    );
    for (i, y) in [1.3, 1.36, 1.42, 1.48].iter().enumerate() {
        let (rx, rz) = (0.14 + i as f32 * 0.006 + 0.012, 0.105 + 0.01);
        p.add(
            CHEST,
            kit::loft(
                &[ell(*y, rx, rz), ell(*y + 0.02, rx, rz)],
                bone,
                false,
                (true, true),
                kit::Uv::PerFace,
            ),
        );
    }
    let blade = spike(
        Vec3::new(0.13, 1.54, 0.06),
        Vec3::new(0.21, 1.74, 0.16),
        0.035,
        4,
        bone,
    );
    add_mirrored(p.mesh(CHEST), &blade);
    p.add(
        CHEST,
        kit::tube(
            Vec3::new(0.0, 1.56, 0.0),
            Vec3::new(0.0, 1.66, -0.035),
            0.04,
            0.036,
            6,
            skin,
        ),
    );

    // Bone mask skull with a heavy brow and two swept horns.
    let ring = |z: f32, y: f32, r: Vec2| {
        kit::ngon_ring(
            Vec3::new(0.0, y, z),
            Vec3::X,
            Vec3::Y,
            r,
            7,
            std::f32::consts::FRAC_PI_2,
        )
    };
    p.add(
        HEAD,
        kit::loft(
            &[
                ring(0.07, 1.72, Vec2::new(0.065, 0.07)),
                ring(-0.02, 1.735, Vec2::new(0.088, 0.1)),
                ring(-0.1, 1.71, Vec2::new(0.072, 0.08)),
                ring(-0.15, 1.68, Vec2::new(0.04, 0.045)),
            ],
            bone,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        HEAD,
        limb(
            Vec3::new(-0.075, 1.75, -0.1),
            Vec3::new(0.075, 1.75, -0.1),
            Vec2::new(0.025, 0.02),
            Vec2::new(0.025, 0.02),
            bone,
        ),
    );
    let horn = spike(
        Vec3::new(0.05, 1.8, 0.0),
        Vec3::new(0.12, 1.86, 0.17),
        0.03,
        4,
        rgb(0xb8a888),
    );
    add_mirrored(p.mesh(HEAD), &horn);

    // Slanted slit eyes.
    let eye = kit::block(
        Vec3::new(0.037, 1.712, -0.118),
        Vec3::new(0.022, 0.008, 0.012),
        Quat::from_rotation_z(-0.35),
        claw,
    );
    add_mirrored(p.mesh(EYES), &eye);

    // Underbite jaw with teeth.
    p.add(
        JAW,
        limb(
            Vec3::new(0.0, 1.66, -0.02),
            Vec3::new(0.0, 1.64, -0.16),
            Vec2::new(0.06, 0.022),
            Vec2::new(0.032, 0.02),
            bone,
        ),
    );
    for x in [-0.024, -0.008, 0.008, 0.024] {
        p.add(
            JAW,
            spike(
                Vec3::new(x, 1.655, -0.15),
                Vec3::new(x, 1.69, -0.155),
                0.008,
                3,
                Vec4::ONE,
            ),
        );
    }

    // Arms (right built, left mirrored).
    let (s, e, w, k) = (v(SHOULDER), v(ELBOW), v(WRIST), v(KNUCKLE));
    let mut upper = limb(s, e, Vec2::new(0.04, 0.04), Vec2::new(0.03, 0.03), skin);
    add(&mut upper, gem(s, 0.05, 0, bone));
    let mut fore = limb(e, w, Vec2::new(0.032, 0.032), Vec2::new(0.027, 0.025), skin);
    add(
        &mut fore,
        spike(
            e + Vec3::new(0.0, 0.0, 0.02),
            e + Vec3::new(0.01, 0.04, 0.12),
            0.022,
            4,
            bone,
        ),
    );
    add(&mut fore, gem(e, 0.035, 0, bone));
    let mut hand = limb(w, k, Vec2::new(0.028, 0.014), Vec2::new(0.036, 0.013), dark);
    add(&mut hand, gem(w, 0.026, 0, dark));
    let mut claws = MeshBuilder::new();
    for (dx, reach) in [(-0.025, 0.24), (0.0, 0.27), (0.025, 0.23)] {
        let base = k + Vec3::new(dx, 0.0, 0.0);
        add(
            &mut claws,
            spike(
                base,
                base + Vec3::new(dx * 0.4, -reach, -0.09),
                0.013,
                4,
                claw,
            ),
        );
    }
    let hip = v(HIP);
    let knee = v(KNEE);
    let ankle = Vec3::new(0.11, 0.07, 0.02);
    let mut thigh = limb(
        hip,
        knee,
        Vec2::new(0.05, 0.05),
        Vec2::new(0.034, 0.034),
        skin,
    );
    add(&mut thigh, gem(knee, 0.04, 0, bone));
    let mut shin = limb(
        knee,
        ankle,
        Vec2::new(0.034, 0.034),
        Vec2::new(0.024, 0.024),
        skin,
    );
    add(
        &mut shin,
        limb(
            Vec3::new(0.11, 0.06, 0.05),
            Vec3::new(0.11, 0.02, -0.16),
            Vec2::new(0.035, 0.03),
            Vec2::new(0.03, 0.014),
            dark,
        ),
    );
    for dx in [-0.02, 0.02] {
        add(
            &mut shin,
            spike(
                Vec3::new(0.11 + dx, 0.025, -0.14),
                Vec3::new(0.11 + dx * 1.5, 0.0, -0.22),
                0.012,
                3,
                bone,
            ),
        );
    }

    for (right, left, mesh) in [
        (UPPER_ARM_R, UPPER_ARM_L, upper),
        (FOREARM_R, FOREARM_L, fore),
        (HAND_R, HAND_L, hand),
        (CLAWS_R, CLAWS_L, claws),
        (THIGH_R, THIGH_L, thigh),
        (SHIN_R, SHIN_L, shin),
    ] {
        p.add(left, kit::mirrored_x(&mesh));
        p.add(right, mesh);
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(17 + i as u32, 0.06);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, HEAD | JAW) {
            kit::tex_bone()
        } else {
            kit::tex_skin()
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Arm swing angles (around X, + = forward/up for a hanging arm) and outward spread.
#[derive(Clone, Copy)]
struct Arm {
    upper: f32,
    spread: f32,
    fore: f32,
    hand: f32,
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 1);
    let glow = telegraph(a.state, t, 0.6);
    pose.glow = gray(glow);
    // Eyes always smoulder (readable at range) and flare with the claws.
    pose.core = gray(glow.max(1.0));

    // Neutral hunch.
    let mut spine = -0.32f32;
    let mut chest = -0.3f32;
    let mut head = 0.5 + a.aim_pitch.clamp(-0.6, 0.6) * 0.6;
    let mut head_yaw = 0.0f32;
    let mut head_roll = 0.15f32;
    let mut jaw = -0.12f32;
    let mut pelvis_off = Vec3::ZERO;
    let mut pelvis_rot = Quat::IDENTITY;
    let rest_arm = Arm {
        upper: 0.45,
        spread: 0.12,
        fore: 0.25,
        hand: 0.15,
    };
    let mut arms = [rest_arm, rest_arm];
    let mut legs = [(0.08f32, -0.18f32), (0.0, -0.1)];

    let k = (a.speed / 6.0).clamp(0.0, 1.0);
    let walking = a.speed > 0.2;
    if walking {
        let amp = 0.25 + 0.4 * k;
        let l = walk_legs(a.cycle, amp, 1.4);
        // Limping: the right leg drags.
        legs = [l[0], (l[1].0 * 0.75, l[1].1 * 0.8)];
        let phase = a.cycle * std::f32::consts::TAU;
        let (s, c) = phase.sin_cos();
        arms[0].upper += -0.35 * amp * s;
        arms[1].upper += 0.25 * amp * s;
        arms[0].fore += 0.15 * c.abs();
        pelvis_off.y = -0.03 * (2.0 * phase).cos().abs();
        pelvis_rot = rz(0.07 * s) * ry(0.1 * s);
        spine -= 0.12 * k;
        head_roll += 0.1 * s;
    } else {
        // Twitchy idle: occasional head jerks, swaying.
        let sway = wave(t, 0.35, off);
        pelvis_rot *= rz(0.04 * sway);
        let slot = (t * 0.6 + off as f64).floor() as i32;
        let jerk = engine::assets::procedural::rand01(slot, a.seed as i32, 77) * 2.0 - 1.0;
        let tween = smoothstep(((t * 0.6 + off as f64).fract() as f32) * 8.0);
        head_yaw = jerk * 0.5 * tween;
        head_roll += jerk * 0.25 * tween;
        jaw -= 0.08 * (wave(t, 0.9, off) * 0.5 + 0.5);
    }

    // Lean into lateral movement.
    pelvis_rot *= rz(-0.15 * a.strafe.clamp(-1.0, 1.0));

    let raise = |arm: &mut Arm, e: f32| {
        arm.upper += (2.75 - arm.upper) * e;
        arm.spread += 0.45 * e;
        arm.fore += (0.75 - arm.fore) * e;
        arm.hand += (-0.4 - arm.hand) * e;
    };
    match a.state {
        EnemyState::Windup(p) => {
            let e = smoothstep(p / 0.8);
            for arm in &mut arms {
                raise(arm, e);
            }
            chest += 0.45 * e;
            spine += 0.15 * e;
            head += 0.25 * e;
            jaw -= 0.55 * e;
            // Shiver at the top of the wind-up.
            let shiver = wave(t, 13.0, 0.0) * 0.04 * e;
            arms[0].upper += shiver;
            arms[1].upper -= shiver;
        }
        EnemyState::Attack(p) => {
            // Both claws rake down and across; the body lunges.
            let s = smoothstep(p / 0.45);
            for arm in &mut arms {
                raise(arm, 1.0 - s);
                arm.upper += -0.15 * s;
                arm.spread -= 0.35 * s;
                arm.fore -= 0.1 * s;
            }
            chest += 0.45 * (1.0 - s) - 0.4 * s;
            spine -= 0.25 * s;
            head += 0.25 * (1.0 - s);
            jaw -= 0.55;
            pelvis_off += Vec3::new(0.0, -0.08 * s, -0.25 * s);
            legs = [(0.75 * s, -0.9 * s), (-0.4 * s, -0.3 * s)];
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            for arm in &mut arms {
                arm.upper -= 0.15 * s;
                arm.spread -= 0.35 * s;
            }
            chest -= 0.4 * s;
            spine -= 0.25 * s;
            jaw -= 0.55 * s;
            pelvis_off += Vec3::new(0.0, -0.08 * s, -0.25 * s);
            legs = [(0.75 * s, -0.9 * s), (-0.4 * s, -0.3 * s)];
        }
        EnemyState::Stunned => {
            let w = wave(t, 1.3, off);
            spine = -0.55;
            chest = -0.35;
            head = -0.3;
            head_roll = 0.5 * w;
            jaw = -0.5;
            pelvis_rot = rz(0.12 * w);
            arms = [Arm {
                upper: 0.6,
                spread: 0.0,
                fore: 0.05,
                hand: 0.0,
            }; 2];
            legs = [(0.35, -0.6), (0.1, -0.35)];
            pelvis_off.y = -0.1;
        }
        EnemyState::Dying(p) => {
            // Knees buckle into a crouch, then it topples forward flat onto the floor.
            let e = smoothstep(p / 0.45);
            let f = smoothstep((p - 0.3) / 0.6);
            legs = [
                (1.5 * e - 1.3 * f, -2.4 * e + 2.1 * f),
                (1.2 * e - 1.1 * f, -2.2 * e + 1.8 * f),
            ];
            pelvis_off = Vec3::new(0.0, -0.6 * e - 0.25 * f, -0.1 * e - 0.25 * f);
            pelvis_rot = rx(-0.25 * e - 1.3 * f) * rz(0.2 * e);
            spine = -0.32 - 0.3 * e + 0.6 * f;
            chest = -0.3 - 0.2 * e + 0.45 * f;
            head = 0.5 - 0.9 * e + 0.3 * f;
            head_roll = 0.15 + 0.9 * e;
            jaw = -0.12 - 0.5 * e;
            arms = [
                Arm {
                    upper: 0.45 + 2.2 * f,
                    spread: 0.12 + 0.5 * e,
                    fore: 0.25,
                    hand: 0.15,
                },
                Arm {
                    upper: 0.45 - 0.3 * e + 0.6 * f,
                    spread: 0.12 + 0.9 * f,
                    fore: 0.6 * e,
                    hand: 0.15,
                },
            ];
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    pose.rotate_move(PELVIS, pelvis_rot, pelvis_off);
    pose.rotate(SPINE, rx(spine));
    pose.rotate(CHEST, rx(chest));
    pose.rotate(HEAD, ry(head_yaw) * rx(head) * rz(head_roll));
    pose.rotate(JAW, rx(jaw));
    for (i, (upper, fore, hand, claws, side)) in [
        (UPPER_ARM_L, FOREARM_L, HAND_L, CLAWS_L, -1.0f32),
        (UPPER_ARM_R, FOREARM_R, HAND_R, CLAWS_R, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let arm = arms[i];
        pose.rotate(upper, rz(arm.spread * side) * rx(arm.upper));
        pose.rotate(fore, rx(arm.fore));
        pose.rotate(hand, rx(arm.hand));
        // Claws flex open during the wind-up.
        let flex = if let EnemyState::Windup(p) = a.state {
            -0.35 * p.clamp(0.0, 1.0)
        } else {
            0.0
        };
        pose.rotate(claws, rx(flex));
    }
    for (i, (thigh, shin)) in [(THIGH_L, SHIN_L), (THIGH_R, SHIN_R)]
        .into_iter()
        .enumerate()
    {
        pose.rotate(thigh, rx(legs[i].0));
        pose.rotate(shin, rx(legs[i].1));
    }
    pose
}
