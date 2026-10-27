//! Smelter: the slow heavy, a 3.4 m bow-legged hulk of slag and wax under an iron shoulder
//! yoke, a squat iron head with a furnace-mouth grate (a weak point, `Core`), and a huge iron
//! crucible slung in front of its belly on chains, gripped by both fists. The crucible's riveted
//! pour hatch burns through its seams (the big weak point, `Core`). Telegraph (`Glow`): the
//! molten slag brimming in the crucible and its spout — it brightens as the Smelter tips the
//! crucible to pour, and as it heaves it round.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, ry, rz, seed_phase, smoothstep, telegraph,
    walk_legs, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const BODY: usize = 0;
pub const HEAD: usize = 1;
pub const MOUTH: usize = 2;
pub const CRUCIBLE: usize = 3;
pub const HATCH: usize = 4;
pub const SLAG: usize = 5;
pub const ARM_L: usize = 6;
pub const FORE_L: usize = 7;
pub const ARM_R: usize = 8;
pub const FORE_R: usize = 9;
pub const THIGH_L: usize = 10;
pub const SHIN_L: usize = 11;
pub const THIGH_R: usize = 12;
pub const SHIN_R: usize = 13;

/// Attack indices (assets/enemies/smelter.ron).
pub const HEAVE: u8 = 0;
pub const POUR: u8 = 1;

const NECK: [f32; 3] = [0.0, 2.84, -0.14];
const SHOULDER: [f32; 3] = [0.95, 2.6, -0.04];
const ELBOW: [f32; 3] = [1.12, 1.98, -0.42];
const FIST: [f32; 3] = [0.72, 1.7, -0.92];
const HIP: [f32; 3] = [0.5, 1.32, 0.24];
const KNEE: [f32; 3] = [0.64, 0.76, 0.02];
/// The crucible tips about its front lip (where it pours from).
const LIP: [f32; 3] = [0.0, 1.98, -1.5];
/// Crucible centre and radius.
const POT: [f32; 3] = [0.0, 1.45, -0.95];
const POT_R: f32 = 0.6;

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 14] = [
    joint("body", ROOT, [0.0, 1.4, 0.24], Shade::Lit),
    joint("head", BODY, NECK, Shade::Lit),
    joint("mouth", HEAD, NECK, Shade::Core),
    joint("crucible", BODY, LIP, Shade::Lit),
    joint("hatch", CRUCIBLE, LIP, Shade::Core),
    joint("slag", CRUCIBLE, LIP, Shade::Glow),
    joint("arm_l", BODY, neg_x(SHOULDER), Shade::Lit),
    joint("fore_l", ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("arm_r", BODY, SHOULDER, Shade::Lit),
    joint("fore_r", ARM_R, ELBOW, Shade::Lit),
    joint("thigh_l", BODY, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", BODY, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

fn ring(c: Vec3, r: Vec2, n: usize) -> Vec<Vec3> {
    kit::ngon_ring(c, Vec3::X, Vec3::Z, r, n, 0.0)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let slag = rgb(0x45403b);
    let slag_dark = rgb(0x2a2624);
    let wax = rgb(0xb8b0a1);
    let iron = rgb(0x4e4a46);
    let iron_dark = rgb(0x2c2a28);
    let brass = rgb(0xb88a3c);
    let molten = Vec4::new(1.0, 0.5, 0.1, 1.0);
    let seam = Vec4::new(1.0, 0.62, 0.18, 1.0);
    let grate = Vec4::new(1.0, 0.45, 0.1, 1.0);

    // Body: a barrel of slag-crusted wax leaning a little forward, wax drips, a crust of slag
    // plates on the shoulders, the iron yoke across them with chain anchors at its ends.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(Vec3::new(0.0, 1.22, 0.3), Vec2::new(0.6, 0.5), 10),
                ring(Vec3::new(0.0, 1.8, 0.22), Vec2::new(0.86, 0.62), 10),
                ring(Vec3::new(0.0, 2.36, 0.08), Vec2::new(0.96, 0.64), 10),
                ring(Vec3::new(0.0, 2.72, -0.04), Vec2::new(0.74, 0.52), 10),
                ring(Vec3::new(0.0, 2.86, -0.1), Vec2::new(0.36, 0.3), 10),
            ],
            wax,
            true,
            (true, true),
            kit::Uv::Density(2.0),
        ),
    );
    for (x, z, r) in [
        (-0.62, 0.0, 0.34),
        (0.6, 0.04, 0.36),
        (-0.2, 0.42, 0.3),
        (0.28, 0.4, 0.28),
    ] {
        p.add(
            BODY,
            ellipsoid(
                Vec3::new(x, 2.6, z),
                Vec3::new(r, r * 0.55, r),
                Quat::from_rotation_z(-x * 0.4),
                2,
                6,
                slag,
                false,
            ),
        );
    }
    for (x, y) in [(-0.7, 1.9), (0.75, 1.7), (0.4, 2.1), (-0.3, 1.5)] {
        p.add(
            BODY,
            spike(
                Vec3::new(x, y, -0.4),
                Vec3::new(x * 1.02, y - 0.3, -0.42),
                0.06,
                4,
                rgb(0x8f877a),
            ),
        );
    }
    p.add(
        BODY,
        kit::block(
            Vec3::new(0.0, 2.8, -0.16),
            Vec3::new(1.12, 0.09, 0.12),
            Quat::IDENTITY,
            iron,
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            BODY,
            kit::band(
                Vec3::new(side * 1.06, 2.8, -0.3),
                Vec3::X,
                0.09,
                0.05,
                0.04,
                6,
                iron_dark,
            ),
        );
    }

    // Head: a squat iron box, the furnace-mouth grate in its face, a stub chimney on top.
    let neck = v(NECK);
    let head = Vec3::new(0.0, 3.04, -0.32);
    p.add(
        HEAD,
        kit::block(
            head,
            Vec3::new(0.3, 0.26, 0.28),
            Quat::from_rotation_x(-0.08),
            iron,
        ),
    );
    p.add(
        HEAD,
        limb(neck, head, Vec2::splat(0.2), Vec2::splat(0.2), slag_dark),
    );
    p.add(
        HEAD,
        kit::tube(
            head + Vec3::new(0.12, 0.2, 0.08),
            head + Vec3::new(0.14, 0.38, 0.12),
            0.07,
            0.06,
            6,
            iron_dark,
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            HEAD,
            gem(head + Vec3::new(side * 0.3, 0.12, -0.2), 0.05, 0, brass),
        );
    }
    for k in 0..4 {
        let x = -0.15 + 0.1 * k as f32;
        p.add(
            MOUTH,
            kit::block(
                head + Vec3::new(x, -0.06, -0.29),
                Vec3::new(0.025, 0.12, 0.02),
                Quat::IDENTITY,
                grate,
            ),
        );
    }
    p.add(
        MOUTH,
        kit::block(
            head + Vec3::new(0.0, 0.08, -0.29),
            Vec3::new(0.18, 0.025, 0.02),
            Quat::IDENTITY,
            grate,
        ),
    );

    // The crucible: a squat iron pot with bands, two lugs gripped by the fists, chains up to
    // the yoke, a spout at the front lip; the riveted hatch on its front; slag brimming on top.
    let pot = v(POT);
    let pring = |y: f32, r: f32| ring(Vec3::new(pot.x, y, pot.z), Vec2::splat(r), 12);
    p.add(
        CRUCIBLE,
        kit::loft(
            &[
                pring(0.92, 0.36),
                pring(1.0, 0.52),
                pring(1.3, POT_R),
                pring(1.75, POT_R),
                pring(1.98, 0.62),
                pring(1.92, 0.54),
            ],
            iron,
            false,
            (true, false),
            kit::Uv::Density(2.0),
        ),
    );
    for y in [1.18, 1.72] {
        p.add(
            CRUCIBLE,
            kit::band(
                Vec3::new(pot.x, y, pot.z),
                Vec3::Y,
                POT_R + 0.02,
                0.08,
                0.04,
                12,
                iron_dark,
            ),
        );
    }
    let lip = v(LIP);
    p.add(
        CRUCIBLE,
        kit::plate(
            &[
                lip + Vec3::new(-0.16, 0.0, 0.1),
                lip + Vec3::new(0.16, 0.0, 0.1),
                lip + Vec3::new(0.1, 0.03, -0.12),
                lip + Vec3::new(-0.1, 0.03, -0.12),
            ],
            0.05,
            iron_dark,
        ),
    );
    for side in [-1.0f32, 1.0] {
        let lug = Vec3::new(side * (POT_R + 0.08), 1.78, pot.z);
        p.add(CRUCIBLE, gem(lug, 0.1, 0, iron_dark));
        // Chains: a few links from the lug up to the yoke's end.
        let top = Vec3::new(side * 1.06, 2.76, -0.3);
        for k in 0..5 {
            let c = lug.lerp(top, (k as f32 + 0.5) / 5.0);
            p.add(
                CRUCIBLE,
                kit::band(c, top - lug, 0.05, 0.03, 0.02, 5, brass),
            );
        }
    }
    // The hatch: a riveted plate on the front face, glowing through its seams.
    let face = Vec3::new(pot.x, 1.45, pot.z - POT_R - 0.02);
    p.add(
        HATCH,
        kit::block(face, Vec3::new(0.26, 0.24, 0.035), Quat::IDENTITY, seam),
    );
    p.add(HATCH, kit::band(face, Vec3::Z, 0.3, 0.05, 0.05, 10, seam));
    // Brimming slag (the telegraph) and the spout's glowing lip.
    p.add(
        SLAG,
        kit::loft(
            &[pring(1.9, 0.53), vec![Vec3::new(pot.x, 1.94, pot.z)]],
            molten,
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(SLAG, gem(lip + Vec3::new(0.0, 0.03, -0.06), 0.1, 0, molten));

    // Arms (right built, left mirrored): massive, the fists clamped on the crucible's lugs.
    let (sh, el, fist) = (v(SHOULDER), v(ELBOW), v(FIST));
    let upper = limb(sh, el, Vec2::splat(0.24), Vec2::splat(0.19), wax);
    let mut fore = limb(el, fist, Vec2::splat(0.19), Vec2::splat(0.15), wax);
    add(
        &mut fore,
        ellipsoid(
            fist,
            Vec3::new(0.18, 0.18, 0.2),
            Quat::IDENTITY,
            3,
            7,
            slag,
            false,
        ),
    );
    add(
        &mut fore,
        kit::band(el.lerp(fist, 0.4), fist - el, 0.2, 0.1, 0.03, 8, iron_dark),
    );
    p.add(ARM_L, kit::mirrored_x(&upper));
    p.add(ARM_R, upper);
    p.add(FORE_L, kit::mirrored_x(&fore));
    p.add(FORE_R, fore);

    // Legs: thick and bowed, flat slag feet.
    let (hip, knee) = (v(HIP), v(KNEE));
    let foot = Vec3::new(0.6, 0.1, 0.16);
    let thigh = limb(hip, knee, Vec2::splat(0.26), Vec2::splat(0.21), wax);
    let mut shin = limb(knee, foot, Vec2::splat(0.21), Vec2::splat(0.17), slag);
    add(
        &mut shin,
        kit::block(
            Vec3::new(0.6, 0.07, 0.02),
            Vec3::new(0.24, 0.07, 0.34),
            Quat::IDENTITY,
            slag_dark,
        ),
    );
    p.add(THIGH_L, kit::mirrored_x(&thigh));
    p.add(THIGH_R, thigh);
    p.add(SHIN_L, kit::mirrored_x(&shin));
    p.add(SHIN_R, shin);

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(171 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, CRUCIBLE | HEAD) {
            kit::tex_plate()
        } else {
            kit::tex_wax()
        };
        p.texture(i, tex);
    }
    p.finish()
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 53);
    let pour = a.attack == POUR;
    let mut glow = telegraph(a.state, t, 0.8);
    let mut body_off = Vec3::ZERO;
    let mut body_rot = rz(-0.06 * a.strafe.clamp(-1.0, 1.0));
    let mut head = 0.05 * wave(t, 0.3, off);
    // Crucible: (tip forward about its lip, roll).
    let mut tip = 0.0f32;
    let mut roll = 0.03 * wave(t, 0.4, off);
    let mut arms = [(0.0f32, 0.0f32); 2]; // (swing, elbow)
    let mut legs = [(0.0f32, 0.0f32); 2];
    let mut squash = 1.0f32;

    let k = (a.speed / 3.6).clamp(0.0, 1.0);
    if a.speed > 0.2 {
        // Heavy, rolling steps; the crucible swings on its chains.
        let l = walk_legs(a.cycle, 0.38 * k, 1.0);
        legs = [l[0], l[1]];
        let phase = a.cycle * std::f32::consts::TAU;
        body_off.y = -0.06 * k * (2.0 * phase).cos().abs();
        body_rot *= rz(0.06 * k * phase.sin());
        roll += 0.08 * k * phase.sin();
        tip += 0.05 * k * (2.0 * phase).sin();
    }

    match a.state {
        EnemyState::Windup(p) if pour => {
            // Plants its feet and leans back, heaving the crucible's rim forward: it tips over
            // its lip, the slag brimming white-hot.
            let e = smoothstep(p / 0.8);
            body_rot *= rx(0.12 * e);
            body_off.y -= 0.1 * e;
            tip = -0.75 * e;
            arms = [(0.25 * e, -0.3 * e); 2];
            legs = [(0.15 * e, -0.25 * e), (-0.15 * e, -0.25 * e)];
            head = -0.15 * e;
        }
        EnemyState::Attack(_) if pour => {
            // Pouring: tipped right over, shaking with the weight.
            body_rot *= rx(0.12);
            body_off.y -= 0.1;
            tip = -1.05 + 0.03 * wave(t, 10.0, off);
            arms = [(0.35, -0.4); 2];
            legs = [(0.15, -0.25), (-0.15, -0.25)];
            head = -0.15;
        }
        EnemyState::Recover(p) if pour => {
            let d = 1.0 - smoothstep(p);
            tip = -1.05 * d;
            arms = [(0.35 * d, -0.4 * d); 2];
        }
        EnemyState::Windup(p) => {
            // Heave: twists away, hauling the crucible round to one side.
            let e = smoothstep(p / 0.8);
            body_rot *= ry(0.6 * e);
            roll = 0.35 * e;
            arms = [(-0.2 * e, -0.3 * e), (0.3 * e, -0.2 * e)];
        }
        EnemyState::Attack(p) => {
            // ...and heaves it across the front.
            let s = smoothstep(p / 0.5);
            body_rot *= ry(0.6 - 1.4 * s);
            roll = 0.35 - 0.8 * s;
            tip = -0.25 * s;
        }
        EnemyState::Recover(p) => {
            let d = 1.0 - smoothstep(p);
            body_rot *= ry(-0.8 * d);
            roll = -0.45 * d;
        }
        EnemyState::Stunned => {
            let w = wave(t, 0.7, off);
            body_rot *= rz(0.08 * w) * rx(-0.1);
            body_off.y -= 0.15;
            head = 0.35;
            roll = 0.2 * w;
            tip = 0.2;
            legs = [(0.2, -0.4); 2];
        }
        EnemyState::Dying(p) => {
            // Drops to its knees, the crucible tips out of its fists and spills; it melts.
            let (e, m, sq) = melt(p);
            squash = sq;
            body_off = Vec3::new(0.0, -0.75 * e, 0.0);
            body_rot = rx(-0.25 * e) * rz(0.12 * m);
            head = 0.5 * e;
            tip = -1.3 * e;
            roll = 0.3 * m;
            arms = [(-0.4 * e, -0.6 * e); 2];
            legs = [(1.2 * e, -1.8 * e); 2];
        }
        EnemyState::Idle | EnemyState::Walk | EnemyState::Latched => {}
        // (Translated into the states above by `enemies::pose`.)
        EnemyState::Crouch(_)
        | EnemyState::Airborne(_)
        | EnemyState::Melt(_)
        | EnemyState::Reform(_) => {}
    }
    // The slag brims brighter as it tips.
    if pour {
        glow = glow.max(0.45 + 2.2 * (-tip).clamp(0.0, 1.05));
    }
    pose.glow = gray(glow);
    pose.core = gray(heat(a.state, glow));

    let spread = 1.0 + 0.12 * (1.0 - squash);
    pose.set(
        BODY,
        Mat4::from_scale_rotation_translation(
            Vec3::new(spread, squash, spread),
            body_rot,
            body_off,
        ),
    );
    pose.rotate(HEAD, rx(head));
    pose.rotate(CRUCIBLE, rz(roll) * rx(tip));
    for (i, (upper, fore, sgn)) in [(ARM_L, FORE_L, -1.0f32), (ARM_R, FORE_R, 1.0)]
        .into_iter()
        .enumerate()
    {
        let (swing, elbow) = arms[i];
        pose.rotate(upper, rz(0.05 * sgn) * rx(swing));
        pose.rotate(fore, rx(elbow));
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
