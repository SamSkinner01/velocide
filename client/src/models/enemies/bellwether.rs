//! Bellwether: support, a gaunt 3 m Mould stooped over long knuckling arms, grey wax over a
//! brass-banded ribcage, a hooded head jutting low and forward, and strapped to its back a
//! timber belfry yoke from which a big bronze bell hangs above and behind its head. The bell is
//! the weak point (`Core`: a hot band round its shoulder and the clapper glowing in its mouth).
//! Telegraph (`Glow`): the bell's lip, the eyes in the hood and the brass knuckle studs — for
//! the Toll it rears up, hauls the bell back on its yoke and the lip burns from orange to
//! white-hot before the swing; for the Clapper a fist drawn back.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, ry, rz, seed_phase, smoothstep, strobe, telegraph,
    wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const BODY: usize = 0;
pub const HEAD: usize = 1;
pub const EYES: usize = 2;
pub const YOKE: usize = 3;
pub const BELL: usize = 4;
pub const BELL_CORE: usize = 5;
pub const BELL_LIP: usize = 6;
pub const UPPER_ARM_L: usize = 7;
pub const FOREARM_L: usize = 8;
pub const UPPER_ARM_R: usize = 9;
pub const FOREARM_R: usize = 10;
pub const KNUCKLES: usize = 11;
pub const THIGH_L: usize = 12;
pub const SHIN_L: usize = 13;
pub const THIGH_R: usize = 14;
pub const SHIN_R: usize = 15;

/// Attack indices (assets/enemies/bellwether.ron).
pub const CLAPPER: u8 = 0;
pub const TOLL: u8 = 1;

const NECK: [f32; 3] = [0.0, 2.02, -0.42];
const SHOULDER: [f32; 3] = [0.42, 1.96, -0.28];
const ELBOW: [f32; 3] = [0.62, 1.22, -0.52];
const WRIST: [f32; 3] = [0.58, 0.36, -0.7];
const HIP: [f32; 3] = [0.26, 1.18, 0.3];
const KNEE: [f32; 3] = [0.32, 0.64, 0.12];
/// Where the bell hangs from the yoke's beam (it swings about this point).
const HANG: [f32; 3] = [0.0, 3.02, 0.3];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 16] = [
    joint("body", ROOT, [0.0, 1.25, 0.2], Shade::Lit),
    joint("head", BODY, NECK, Shade::Lit),
    joint("eyes", HEAD, NECK, Shade::Glow),
    joint("yoke", BODY, [0.0, 2.0, 0.28], Shade::Lit),
    joint("bell", YOKE, HANG, Shade::Lit),
    joint("bell_core", BELL, HANG, Shade::Core),
    joint("bell_lip", BELL, HANG, Shade::Glow),
    joint("upper_arm_l", BODY, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", UPPER_ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("upper_arm_r", BODY, SHOULDER, Shade::Lit),
    joint("forearm_r", UPPER_ARM_R, ELBOW, Shade::Lit),
    joint("knuckles", FOREARM_R, ELBOW, Shade::Glow),
    joint("thigh_l", BODY, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", BODY, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// Horizontal ring (for lofting a body along a slanted spine).
fn ring(c: Vec3, r: Vec2, n: usize) -> Vec<Vec3> {
    kit::ngon_ring(c, Vec3::X, Vec3::Z, r, n, 0.0)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wax = rgb(0xcbc3b2);
    let wax_dark = rgb(0x968d7e);
    let brass = rgb(0xc89a42);
    let bronze = rgb(0xc08a3e);
    let bronze_dark = rgb(0x6e4a22);
    let timber = rgb(0x5a3f2a);
    let slag = rgb(0x3b3532);
    let hot = Vec4::new(1.0, 0.62, 0.18, 1.0);
    let lip = Vec4::new(1.0, 0.5, 0.1, 1.0);
    let eye = Vec4::new(1.0, 0.45, 0.12, 1.0);

    // Body: a stooped wax trunk from the hips up and forward to the shoulders, brass bands
    // round the ribs, a slag spine.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(Vec3::new(0.0, 1.05, 0.28), Vec2::new(0.3, 0.26), 8),
                ring(Vec3::new(0.0, 1.4, 0.2), Vec2::new(0.36, 0.3), 8),
                ring(Vec3::new(0.0, 1.72, 0.02), Vec2::new(0.42, 0.34), 8),
                ring(Vec3::new(0.0, 1.96, -0.24), Vec2::new(0.4, 0.3), 8),
                ring(Vec3::new(0.0, 2.08, -0.4), Vec2::new(0.2, 0.16), 8),
            ],
            wax,
            true,
            (true, true),
            kit::Uv::Density(2.5),
        ),
    );
    for (y, z, r) in [(1.48, 0.17, 0.36), (1.66, 0.06, 0.4), (1.84, -0.1, 0.41)] {
        p.add(
            BODY,
            kit::band(
                Vec3::new(0.0, y, z),
                Vec3::new(0.0, 1.0, -0.6),
                r + 0.02,
                0.06,
                0.03,
                10,
                brass,
            ),
        );
    }
    for k in 0..4 {
        let t = k as f32 / 3.0;
        let base = Vec3::new(0.0, 1.35, 0.42).lerp(Vec3::new(0.0, 2.0, 0.08), t);
        p.add(
            BODY,
            spike(base, base + Vec3::new(0.0, 0.12, 0.14), 0.05, 4, slag),
        );
    }

    // Head: a low hooded skull on a short neck, two ember eyes in the hollow of the hood.
    let neck = v(NECK);
    let skull = Vec3::new(0.0, 1.96, -0.74);
    p.add(HEAD, kit::smooth_tube(neck, skull, 0.14, 0.12, 6, wax_dark));
    p.add(
        HEAD,
        ellipsoid(
            skull + Vec3::new(0.0, 0.06, 0.02),
            Vec3::new(0.2, 0.22, 0.24),
            Quat::IDENTITY,
            4,
            8,
            wax,
            true,
        ),
    );
    // The hood: a drooping cowl over the skull.
    p.add(
        HEAD,
        kit::loft(
            &[
                ring(skull + Vec3::new(0.0, 0.0, 0.18), Vec2::new(0.26, 0.2), 8),
                ring(skull + Vec3::new(0.0, 0.14, 0.0), Vec2::new(0.27, 0.24), 8),
                vec![skull + Vec3::new(0.0, 0.36, 0.1)],
            ],
            wax_dark,
            true,
            (false, false),
            kit::Uv::Density(2.0),
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            EYES,
            gem(skull + Vec3::new(side * 0.08, 0.04, -0.2), 0.045, 0, eye),
        );
    }

    // The belfry yoke: two timber posts from its back to a beam above its head, braces, brass
    // straps round its chest.
    let hang = v(HANG);
    for side in [-1.0f32, 1.0] {
        let foot = Vec3::new(side * 0.28, 1.92, 0.3);
        let top = Vec3::new(side * 0.4, 3.12, 0.3);
        p.add(
            YOKE,
            limb(foot, top, Vec2::splat(0.07), Vec2::splat(0.06), timber),
        );
        p.add(
            YOKE,
            limb(
                Vec3::new(side * 0.3, 2.3, 0.3),
                Vec3::new(side * 0.08, 3.08, 0.3),
                Vec2::splat(0.035),
                Vec2::splat(0.035),
                timber,
            ),
        );
        p.add(
            YOKE,
            kit::band(foot + Vec3::Y * 0.12, Vec3::Y, 0.085, 0.05, 0.02, 6, brass),
        );
        p.add(YOKE, gem(top + Vec3::Y * 0.06, 0.05, 0, brass));
    }
    p.add(
        YOKE,
        kit::block(
            Vec3::new(0.0, 3.15, 0.3),
            Vec3::new(0.5, 0.06, 0.07),
            Quat::IDENTITY,
            timber,
        ),
    );
    p.add(
        YOKE,
        kit::block(
            Vec3::new(0.0, 2.0, 0.3),
            Vec3::new(0.34, 0.07, 0.06),
            Quat::IDENTITY,
            timber,
        ),
    );

    // The bell: a bronze shell hanging mouth-down from a crown loop, a hot band round its
    // shoulder and the glowing clapper in its mouth (the weak point), the lip that flares.
    let bring = |y: f32, r: f32| ring(Vec3::new(hang.x, y, hang.z), Vec2::splat(r), 12);
    p.add(
        BELL,
        kit::loft(
            &[
                bring(2.98, 0.12),
                bring(2.9, 0.3),
                bring(2.78, 0.37),
                bring(2.58, 0.41),
                bring(2.38, 0.48),
                bring(2.25, 0.6),
                bring(2.27, 0.52),
            ],
            bronze,
            true,
            (true, false),
            kit::Uv::Density(2.0),
        ),
    );
    p.add(
        BELL,
        kit::band(
            hang - Vec3::Y * 0.02,
            Vec3::X,
            0.06,
            0.04,
            0.03,
            6,
            bronze_dark,
        ),
    );
    p.add(
        BELL_CORE,
        kit::band(
            Vec3::new(hang.x, 2.66, hang.z),
            Vec3::Y,
            0.41,
            0.11,
            0.04,
            12,
            hot,
        ),
    );
    // Inside the mouth: a glowing disc and the clapper hanging below the lip.
    p.add(
        BELL_CORE,
        kit::loft(
            &[bring(2.3, 0.47), vec![Vec3::new(hang.x, 2.42, hang.z)]],
            hot,
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        BELL_CORE,
        gem(Vec3::new(hang.x, 2.12, hang.z), 0.15, 1, hot),
    );
    p.add(
        BELL_LIP,
        kit::band(
            Vec3::new(hang.x, 2.26, hang.z),
            Vec3::Y,
            0.6,
            0.05,
            0.05,
            12,
            lip,
        ),
    );

    // Arms (right built, left mirrored): long and gaunt, down to big knuckled fists.
    let (sh, el, wr) = (v(SHOULDER), v(ELBOW), v(WRIST));
    let upper = limb(sh, el, Vec2::splat(0.12), Vec2::splat(0.09), wax);
    let mut fore = limb(el, wr, Vec2::splat(0.09), Vec2::splat(0.08), wax);
    add(
        &mut fore,
        ellipsoid(
            wr + Vec3::new(0.0, -0.14, -0.06),
            Vec3::new(0.15, 0.13, 0.18),
            Quat::IDENTITY,
            3,
            7,
            wax_dark,
            false,
        ),
    );
    add(
        &mut fore,
        kit::band(el.lerp(wr, 0.7), wr - el, 0.095, 0.07, 0.02, 6, brass),
    );
    p.add(UPPER_ARM_L, kit::mirrored_x(&upper));
    p.add(UPPER_ARM_R, upper);
    p.add(FOREARM_L, kit::mirrored_x(&fore));
    p.add(FOREARM_R, fore);
    for k in 0..3 {
        let x = wr.x - 0.08 + 0.08 * k as f32;
        p.add(
            KNUCKLES,
            gem(Vec3::new(x, wr.y - 0.06, wr.z - 0.22), 0.04, 0, lip),
        );
    }

    // Legs: short, bowed, splayed feet.
    let (hip, knee) = (v(HIP), v(KNEE));
    let foot = Vec3::new(0.3, 0.07, 0.3);
    let thigh = limb(hip, knee, Vec2::splat(0.12), Vec2::splat(0.1), wax_dark);
    let mut shin = limb(knee, foot, Vec2::splat(0.1), Vec2::splat(0.08), wax_dark);
    add(
        &mut shin,
        ellipsoid(
            foot + Vec3::new(0.0, -0.02, -0.12),
            Vec3::new(0.12, 0.07, 0.22),
            Quat::IDENTITY,
            2,
            6,
            wax_dark,
            false,
        ),
    );
    p.add(THIGH_L, kit::mirrored_x(&thigh));
    p.add(THIGH_R, thigh);
    p.add(SHIN_L, kit::mirrored_x(&shin));
    p.add(SHIN_R, shin);

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(131 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, BELL | YOKE) {
            kit::tex_plate()
        } else {
            kit::tex_wax()
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Arm angles of one side: (swing forward, spread out, elbow).
#[derive(Clone, Copy, Default)]
struct Arm {
    swing: f32,
    spread: f32,
    elbow: f32,
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 29);
    let toll = a.attack == TOLL;
    let mut glow = telegraph(a.state, t, 0.75);
    let mut body_off = Vec3::ZERO;
    let mut body_rot = rz(-0.1 * a.strafe.clamp(-1.0, 1.0));
    let mut head = 0.08 * wave(t, 0.35, off);
    let mut head_yaw = 0.0f32;
    let mut bell = 0.06 * wave(t, 0.5, off);
    let mut bell_roll = 0.0f32;
    let mut arms = [Arm::default(); 2];
    let mut legs = [(0.0f32, 0.0f32); 2];
    let mut squash = 1.0f32;

    let k = (a.speed / 6.0).clamp(0.0, 1.0);
    if a.speed > 0.2 {
        // Knuckle-walking: each arm swings with the opposite leg, the bell sways behind.
        let phase = a.cycle * std::f32::consts::TAU;
        let (s, c) = phase.sin_cos();
        let amp = 0.25 + 0.3 * k;
        arms[0].swing = amp * s;
        arms[1].swing = -amp * s;
        arms[0].elbow = -0.2 * amp * (1.0 + c).max(0.0);
        arms[1].elbow = -0.2 * amp * (1.0 - c).max(0.0);
        legs[0] = (-amp * s, 0.6 * amp * (1.0 - c).max(0.0));
        legs[1] = (amp * s, 0.6 * amp * (1.0 + c).max(0.0));
        body_off.y = -0.05 * k * (2.0 * phase).sin().abs();
        bell += 0.18 * k * (phase + 0.6).sin();
        bell_roll = 0.08 * k * s;
    }

    match a.state {
        EnemyState::Windup(p) if toll => {
            // Rears up and hauls the bell back on its yoke (still showing over its head: it
            // is what you shoot); it trembles at the top.
            let e = smoothstep(p / 0.55);
            body_rot *= rx(0.28 * e);
            body_off += Vec3::new(0.0, 0.1 * e, 0.06 * e);
            head = -0.2 * e;
            for arm in &mut arms {
                arm.swing = 2.2 * e;
                arm.spread = 0.3 * e;
                arm.elbow = -0.6 * e;
            }
            legs = [(0.2 * e, 0.35 * e); 2];
            bell = -0.6 * e + 0.05 * e * wave(t, 9.0, off);
            glow = 0.6 + 2.6 * p;
            if p > 0.75 {
                glow *= 0.75 + 0.6 * strobe(t, 14.0);
            }
        }
        EnemyState::Attack(p) if toll => {
            // The bell swings through and tolls; it heaves forward over its arms.
            let s = smoothstep(p / 0.4);
            body_rot *= rx(0.28 - 0.5 * s);
            body_off.y += 0.1 - 0.13 * s;
            head = -0.2 + 0.3 * s;
            for arm in &mut arms {
                arm.swing = 2.2 - 2.0 * s;
                arm.spread = 0.3;
                arm.elbow = -0.6 + 0.4 * s;
            }
            bell = -0.6 + 1.5 * s;
            glow = 3.4 - 1.0 * p;
        }
        EnemyState::Recover(p) if toll => {
            // The bell rocks to rest.
            let d = 1.0 - smoothstep(p);
            bell = 1.0 * d * (p * 18.0).cos();
            body_rot *= rx(-0.2 * d);
        }
        EnemyState::Windup(p) => {
            // Clapper: the right fist drawn back and up, the body twisting away.
            let e = smoothstep(p / 0.8);
            body_rot *= ry(0.45 * e);
            arms[1] = Arm {
                swing: -0.9 * e,
                spread: 0.6 * e,
                elbow: -0.6 * e,
            };
            head_yaw = -0.3 * e;
        }
        EnemyState::Attack(p) => {
            // Backhand sweep across the front.
            let s = smoothstep(p / 0.5);
            body_rot *= ry(0.45 - 0.95 * s);
            arms[1] = Arm {
                swing: -0.9 + 2.0 * s,
                spread: 0.6 - 0.9 * s,
                elbow: -0.6 + 0.4 * s,
            };
            bell = 0.3 * s;
        }
        EnemyState::Recover(p) => {
            let d = 1.0 - smoothstep(p);
            body_rot *= ry(-0.5 * d);
            arms[1].swing = 1.1 * d;
        }
        EnemyState::Stunned => {
            // Cracked mid-swing: it reels, the bell swinging wildly on its yoke.
            let w = wave(t, 0.9, off);
            body_rot *= rz(0.18 * w) * rx(-0.15);
            body_off.y -= 0.12;
            head = 0.4;
            head_yaw = 0.3 * w;
            bell = 0.8 * wave(t, 1.6, off);
            bell_roll = 0.35 * wave(t, 1.1, off + 0.3);
            for arm in &mut arms {
                arm.spread = 0.4;
                arm.elbow = -0.6;
            }
            legs = [(0.3, 0.5), (-0.2, 0.3)];
        }
        EnemyState::Dying(p) => {
            // Topples forward under the yoke; the bell drags it down and it melts.
            let (e, m, sq) = melt(p);
            squash = sq;
            body_off = Vec3::new(0.0, -0.9 * e, -0.3 * e);
            body_rot = rx(-0.7 * e) * rz(0.2 * m);
            head = 0.6 * e;
            bell = 1.2 * e;
            bell_roll = 0.4 * m;
            for arm in &mut arms {
                arm.swing = 0.9 * e;
                arm.spread = 0.5 * e;
                arm.elbow = -e;
            }
            legs = [(0.9 * e, 1.6 * e), (0.7 * e, 1.4 * e)];
        }
        EnemyState::Idle | EnemyState::Walk | EnemyState::Latched => {}
    }
    pose.glow = gray(glow);
    // The bell's band burns brighter while it tolls.
    let core = if toll && matches!(a.state, EnemyState::Windup(_) | EnemyState::Attack(_)) {
        heat(a.state, glow).max(1.6)
    } else {
        heat(a.state, glow)
    };
    pose.core = gray(core);

    let spread = 1.0 + 0.12 * (1.0 - squash);
    pose.set(
        BODY,
        Mat4::from_scale_rotation_translation(
            Vec3::new(spread, squash, spread),
            body_rot,
            body_off,
        ),
    );
    pose.rotate(HEAD, ry(head_yaw) * rx(head));
    pose.rotate(BELL, rz(bell_roll) * rx(bell));
    for (i, (upper, fore, sgn)) in [
        (UPPER_ARM_L, FOREARM_L, -1.0f32),
        (UPPER_ARM_R, FOREARM_R, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let arm = arms[i];
        pose.rotate(upper, rz(arm.spread * sgn) * rx(arm.swing));
        pose.rotate(fore, rx(arm.elbow));
    }
    for (i, (thigh, shin)) in [(THIGH_L, SHIN_L), (THIGH_R, SHIN_R)]
        .into_iter()
        .enumerate()
    {
        pose.rotate(thigh, rx(legs[i].0));
        pose.rotate(shin, rx(-legs[i].1));
    }
    pose
}
