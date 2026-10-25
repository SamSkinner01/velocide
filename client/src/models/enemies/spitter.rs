//! Spitter: ranged. A hunched, knuckle-walking ~1.6 m brute with a humped, bone-ridged back,
//! an eyeless wedge head and a huge veined throat sac that glows acid green. Telegraph: the
//! sac swells and blazes (plus back pustules and the maw) while the neck rears back; the spit
//! thrusts the head forward and deflates the sac.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{EnemyAnim, EnemyState, gray, rx, ry, rz, seed_phase, smoothstep, telegraph, wave};
use crate::models::PartSet;
use crate::models::kit::{self, add, add_mirrored, frame_for, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const BODY: usize = 1;
pub const PUSTULES: usize = 2;
pub const NECK: usize = 3;
pub const HEAD: usize = 4;
pub const JAW: usize = 5;
pub const MAW: usize = 6;
pub const SAC: usize = 7;
pub const ARM_L: usize = 8;
pub const FOREARM_L: usize = 9;
pub const ARM_R: usize = 10;
pub const FOREARM_R: usize = 11;
pub const THIGH_L: usize = 12;
pub const SHIN_L: usize = 13;
pub const THIGH_R: usize = 14;
pub const SHIN_R: usize = 15;

const SHOULDER: [f32; 3] = [0.34, 1.26, -0.18];
const ELBOW: [f32; 3] = [0.54, 0.8, -0.2];
const HIP: [f32; 3] = [0.2, 0.82, 0.34];
const KNEE: [f32; 3] = [0.26, 0.44, 0.12];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 16] = [
    joint("pelvis", ROOT, [0.0, 0.84, 0.32], Shade::Lit),
    joint("body", PELVIS, [0.0, 0.92, 0.22], Shade::Lit),
    joint("pustules", BODY, [0.0, 0.92, 0.22], Shade::Glow),
    joint("neck", BODY, [0.0, 1.3, -0.3], Shade::Lit),
    joint("head", NECK, [0.0, 1.3, -0.6], Shade::Lit),
    joint("jaw", HEAD, [0.0, 1.25, -0.6], Shade::Lit),
    joint("maw", HEAD, [0.0, 1.25, -0.6], Shade::Glow),
    joint("sac", NECK, [0.0, 1.2, -0.46], Shade::Glow),
    joint("arm_l", BODY, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("arm_r", BODY, SHOULDER, Shade::Lit),
    joint("forearm_r", ARM_R, ELBOW, Shade::Lit),
    joint("thigh_l", PELVIS, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", PELVIS, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// Ring around an arbitrary axis centre with radii along the frame's side / depth.
fn ring(center: Vec3, axis: Vec3, r: Vec2) -> Vec<Vec3> {
    let (side, depth) = frame_for(axis);
    kit::ngon_ring(center, side, depth, r, 8, std::f32::consts::PI / 8.0)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let skin = rgb(0xcfc49a);
    let back = rgb(0x5a3a44);
    let bone = rgb(0xd8ccae);
    let acid = Vec4::new(0.62, 1.0, 0.22, 1.0);

    // Pelvis: a stubby rump.
    p.add(
        PELVIS,
        kit::ellipsoid(
            Vec3::new(0.0, 0.86, 0.36),
            Vec3::new(0.2, 0.17, 0.2),
            Quat::IDENTITY,
            4,
            8,
            skin,
            false,
        ),
    );

    // Hunched body: hips up to a massive hump, then down to the shoulders.
    let pts = [
        (Vec3::new(0.0, 0.88, 0.44), Vec2::new(0.16, 0.14)),
        (Vec3::new(0.0, 1.02, 0.2), Vec2::new(0.25, 0.24)),
        (Vec3::new(0.0, 1.2, -0.05), Vec2::new(0.31, 0.28)),
        (Vec3::new(0.0, 1.3, -0.24), Vec2::new(0.25, 0.2)),
        (Vec3::new(0.0, 1.3, -0.36), Vec2::new(0.13, 0.12)),
    ];
    let rings: Vec<Vec<Vec3>> = pts
        .iter()
        .enumerate()
        .map(|(i, (c, r))| {
            let axis = if i + 1 < pts.len() {
                pts[i + 1].0 - *c
            } else {
                *c - pts[i - 1].0
            };
            ring(*c, axis, *r)
        })
        .collect();
    let mut body = kit::loft(&rings, skin, false, (true, true), kit::Uv::Density(3.0));
    // Pale belly, dark mottled back: blend by height and facing.
    body.map_colors(|vx| {
        let t = ((vx.position[1] - 0.9) / 0.4 + vx.normal[1] * 0.5).clamp(0.0, 1.0);
        skin.lerp(back, t)
    });
    p.add(BODY, body);
    // Bone ridge plates along the spine.
    for (i, (y, z)) in [(1.27, 0.15), (1.41, -0.04), (1.43, -0.18)]
        .iter()
        .enumerate()
    {
        let h = 0.13 - i as f32 * 0.02;
        p.add(
            BODY,
            kit::plate(
                &[
                    Vec3::new(0.0, *y - 0.04, *z + 0.08),
                    Vec3::new(0.0, *y + h, *z + 0.1),
                    Vec3::new(0.0, *y - 0.03, *z - 0.06),
                ],
                0.04,
                bone,
            ),
        );
    }

    // Glowing pustules on the hump.
    for (x, y, z, r) in [
        (0.17, 1.25, 0.06, 0.05),
        (0.21, 1.15, 0.18, 0.04),
        (0.12, 1.36, -0.12, 0.04),
        (0.24, 1.3, -0.06, 0.035),
    ] {
        let g = gem(Vec3::new(x, y, z), r, 0, acid);
        add_mirrored(p.mesh(PUSTULES), &g);
    }

    // Neck.
    p.add(
        NECK,
        kit::tube(
            Vec3::new(0.0, 1.3, -0.3),
            Vec3::new(0.0, 1.31, -0.62),
            0.11,
            0.085,
            7,
            skin,
        ),
    );

    // Eyeless wedge head with a brow ridge.
    let hring =
        |z: f32, y: f32, r: Vec2| kit::ngon_ring(Vec3::new(0.0, y, z), Vec3::X, Vec3::Y, r, 6, 0.0);
    let mut head = kit::loft(
        &[
            hring(-0.52, 1.33, Vec2::new(0.11, 0.1)),
            hring(-0.66, 1.34, Vec2::new(0.15, 0.11)),
            hring(-0.8, 1.31, Vec2::new(0.1, 0.065)),
            vec![Vec3::new(0.0, 1.29, -0.87)],
        ],
        back,
        false,
        (true, false),
        kit::Uv::PerFace,
    );
    head.map_colors(|vx| back.lerp(skin, (-vx.normal[1]).clamp(0.0, 1.0)));
    p.add(HEAD, head);
    p.add(
        HEAD,
        limb(
            Vec3::new(-0.13, 1.41, -0.63),
            Vec3::new(0.13, 1.41, -0.63),
            Vec2::new(0.035, 0.03),
            Vec2::new(0.035, 0.03),
            bone,
        ),
    );
    let mandible = spike(
        Vec3::new(0.09, 1.28, -0.72),
        Vec3::new(0.06, 1.22, -0.93),
        0.03,
        4,
        bone,
    );
    add_mirrored(p.mesh(HEAD), &mandible);

    // Lower jaw.
    p.add(
        JAW,
        limb(
            Vec3::new(0.0, 1.24, -0.56),
            Vec3::new(0.0, 1.21, -0.84),
            Vec2::new(0.11, 0.03),
            Vec2::new(0.06, 0.022),
            skin,
        ),
    );
    // Glowing gullet between the jaws.
    p.add(
        MAW,
        kit::block(
            Vec3::new(0.0, 1.262, -0.72),
            Vec3::new(0.075, 0.014, 0.1),
            Quat::IDENTITY,
            acid,
        ),
    );

    // The throat sac (unlit; veins from the skin texture).
    p.add(
        SAC,
        kit::ellipsoid(
            Vec3::new(0.0, 1.06, -0.5),
            Vec3::new(0.17, 0.17, 0.2),
            Quat::IDENTITY,
            6,
            10,
            acid,
            true,
        ),
    );

    // Thick knuckle-walking arms.
    let (s, e) = (v(SHOULDER), v(ELBOW));
    let knuckle = Vec3::new(0.46, 0.08, -0.42);
    let limb_skin = rgb(0xa89c80);
    let mut upper = kit::tube(s, e, 0.15, 0.1, 6, limb_skin);
    add(&mut upper, gem(s, 0.16, 0, back));
    let mut fore = gem(e, 0.11, 0, limb_skin);
    add(
        &mut fore,
        kit::tube(
            e,
            knuckle + Vec3::new(0.0, 0.06, 0.0),
            0.1,
            0.13,
            6,
            limb_skin,
        ),
    );
    add(
        &mut fore,
        kit::ellipsoid(
            knuckle,
            Vec3::new(0.14, 0.08, 0.15),
            Quat::from_rotation_y(0.2),
            3,
            6,
            back,
            false,
        ),
    );
    for dx in [-0.05, 0.0, 0.05] {
        add(
            &mut fore,
            spike(
                knuckle + Vec3::new(dx, -0.02, -0.08),
                knuckle + Vec3::new(dx * 1.3, -0.07, -0.18),
                0.02,
                3,
                bone,
            ),
        );
    }

    // Short digitigrade hind legs.
    let (hip, knee) = (v(HIP), v(KNEE));
    let foot = Vec3::new(0.26, 0.05, 0.36);
    let mut thigh = kit::tube(hip, knee, 0.15, 0.1, 6, limb_skin);
    add(&mut thigh, gem(knee, 0.095, 0, back));
    let mut shin = kit::tube(
        knee,
        foot + Vec3::new(0.0, 0.03, 0.0),
        0.085,
        0.06,
        6,
        limb_skin,
    );
    add(
        &mut shin,
        kit::ellipsoid(
            foot + Vec3::new(0.0, 0.0, -0.05),
            Vec3::new(0.075, 0.045, 0.13),
            Quat::IDENTITY,
            3,
            6,
            back,
            false,
        ),
    );

    for (right, left, mesh) in [
        (ARM_R, ARM_L, upper),
        (FOREARM_R, FOREARM_L, fore),
        (THIGH_R, THIGH_L, thigh),
        (SHIN_R, SHIN_L, shin),
    ] {
        p.add(left, kit::mirrored_x(&mesh));
        p.add(right, mesh);
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(31 + i as u32, 0.06);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if i == SAC || !j.shade.is_emissive() {
            kit::tex_skin()
        } else {
            kit::tex_glow()
        };
        p.texture(i, tex);
    }
    p.finish()
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 2);
    let glow = telegraph(a.state, t, 0.7);
    pose.glow = gray(glow);

    let breathe = wave(t, 0.6, off);
    let mut sac = 1.0 + 0.05 * breathe;
    let mut neck = 0.05 + a.aim_pitch.clamp(-0.5, 0.6) * 0.5;
    let mut head = a.aim_pitch.clamp(-0.5, 0.6) * 0.3;
    let mut jaw = 0.0f32;
    let mut body_rot = rx(0.02 * breathe);
    let mut pelvis_off = Vec3::ZERO;
    let mut pelvis_rot = Quat::IDENTITY;
    let mut arms = [(0.0f32, 0.0f32); 2];
    let mut legs = [(0.0f32, 0.0f32); 2];
    // Sideways splay of all four limbs (+ = outward).
    let mut splay = 0.0f32;

    if a.speed > 0.2 {
        // Diagonal gait: left arm with the right leg.
        let k = (a.speed / 5.0).clamp(0.0, 1.0);
        let amp = 0.25 + 0.3 * k;
        let phase = a.cycle * std::f32::consts::TAU;
        let (s, c) = phase.sin_cos();
        arms = [
            (-amp * s, 0.4 * amp * c.max(0.0)),
            (amp * s, 0.4 * amp * (-c).max(0.0)),
        ];
        legs = [
            (-amp * s * 0.8, 0.5 * amp * (-c).max(0.0)),
            (amp * s * 0.8, 0.5 * amp * c.max(0.0)),
        ];
        pelvis_off.y = 0.03 * (2.0 * phase).sin();
        pelvis_rot = rz(0.06 * s) * ry(0.05 * c);
        neck += 0.06 * (2.0 * phase).cos();
    }
    pelvis_rot *= rz(-0.12 * a.strafe.clamp(-1.0, 1.0));

    match a.state {
        EnemyState::Windup(p) => {
            let e = smoothstep(p);
            sac = 1.0 + 0.6 * e + 0.06 * e * wave(t, 7.0, 0.0);
            neck += 0.55 * e;
            head += 0.2 * e;
            jaw = 0.25 * e;
            body_rot *= rx(0.12 * e);
            pelvis_off.z += 0.05 * e;
        }
        EnemyState::Attack(p) => {
            let s = smoothstep(p / 0.3);
            sac = 1.6 - 0.85 * s;
            neck += 0.55 - 0.85 * s;
            head += 0.2 - 0.35 * s;
            jaw = 0.25 + 0.5 * s;
            body_rot *= rx(0.12 - 0.08 * s);
            pelvis_off.z += 0.05 + 0.1 * s;
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            sac = 1.0 - 0.25 * s;
            neck -= 0.3 * s;
            head -= 0.15 * s;
            jaw = 0.75 * s;
            pelvis_off.z += 0.15 * s;
        }
        EnemyState::Stunned => {
            let w = wave(t, 1.1, off);
            sac = 0.9;
            neck = -0.35;
            head = -0.2 + 0.15 * w;
            jaw = 0.4;
            pelvis_off.y = -0.12;
            pelvis_rot = rz(0.1 * w);
            arms = [(0.25, 0.35), (-0.15, 0.3)];
        }
        EnemyState::Dying(p) => {
            // The sac balloons and bursts, then the body collapses onto its belly.
            let e = smoothstep(p / 0.8);
            if p < 0.25 {
                sac = 1.0 + 3.0 * p;
            } else {
                pose.hide(SAC);
                pose.hide(MAW);
            }
            pelvis_off = Vec3::new(0.0, -0.62 * e, 0.0);
            pelvis_rot = rz(0.12 * e);
            body_rot = rx(-0.15 * e);
            neck = -0.45 * e;
            head = -0.3 * e;
            jaw = 0.6 * e;
            splay = 0.9 * e;
            arms = [(0.5 * e, -0.6 * e), (-0.3 * e, -0.4 * e)];
            legs = [(-0.5 * e, 0.9 * e), (-0.3 * e, 0.7 * e)];
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    pose.rotate_move(PELVIS, pelvis_rot, pelvis_off);
    pose.rotate(BODY, body_rot);
    pose.rotate(NECK, rx(neck));
    pose.rotate(HEAD, rx(head));
    pose.rotate(JAW, rx(-jaw));
    pose.set(SAC, Mat4::from_scale(Vec3::new(sac, sac * 1.1, sac)));
    for (i, (upper, fore, side)) in [(ARM_L, FOREARM_L, -1.0f32), (ARM_R, FOREARM_R, 1.0)]
        .into_iter()
        .enumerate()
    {
        pose.rotate(upper, rz(splay * side) * rx(arms[i].0));
        pose.rotate(fore, rx(arms[i].1));
    }
    for (i, (thigh, shin, side)) in [(THIGH_L, SHIN_L, -1.0f32), (THIGH_R, SHIN_R, 1.0)]
        .into_iter()
        .enumerate()
    {
        pose.rotate(thigh, rz(splay * side * 0.7) * rx(legs[i].0));
        pose.rotate(shin, rx(legs[i].1));
    }
    pose
}
