//! Heron: the sniper, a 3.4 m stilt-legged wading-bird Mould. Thin brass legs with backward
//! knees and long splayed toes, a narrow wax body with brass wing covers and tail vanes, a long
//! S-neck, and a brass drum of a head with a single great lens (the weak point, `Core`) over a
//! needle-barrel beak. Telegraph (`Glow`): the ring around the lens burns red while it aims
//! (the client draws the aim laser from the lens) and flares once the aim locks; a raised leg
//! for the kick.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, rz, seed_phase, smoothstep, strobe, telegraph,
    wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const BODY: usize = 0;
pub const NECK: usize = 1;
pub const HEAD: usize = 2;
pub const LENS: usize = 3;
pub const RING: usize = 4;
pub const THIGH_L: usize = 5;
pub const SHIN_L: usize = 6;
pub const THIGH_R: usize = 7;
pub const SHIN_R: usize = 8;

/// Attack indices (assets/enemies/heron.ron).
pub const KICK: u8 = 0;
pub const NEEDLE: u8 = 1;

const HIP: [f32; 3] = [0.16, 2.08, 0.1];
const KNEE: [f32; 3] = [0.19, 1.22, 0.36];
const NECK_AT: [f32; 3] = [0.0, 2.42, -0.28];
const HEAD_AT: [f32; 3] = [0.0, 2.98, -0.26];
/// The lens (weak point centre: 0, 3.12, -0.4).
const LENS_AT: [f32; 3] = [0.0, 3.12, -0.5];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 9] = [
    joint("body", ROOT, [0.0, 2.2, 0.05], Shade::Lit),
    joint("neck", BODY, NECK_AT, Shade::Lit),
    joint("head", NECK, HEAD_AT, Shade::Lit),
    joint("lens", HEAD, HEAD_AT, Shade::Core),
    joint("ring", HEAD, HEAD_AT, Shade::Glow),
    joint("thigh_l", BODY, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", BODY, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wax = rgb(0xd6cfc0);
    let brass = rgb(0xc89c45);
    let brass_dark = rgb(0x7d602b);
    let slag = rgb(0x3b3532);
    let lens = Vec4::new(1.0, 0.24, 0.1, 1.0);
    let ring = Vec4::new(1.0, 0.12, 0.06, 1.0);

    // Body: a narrow teardrop of wax, brass wing covers along the flanks, tail vanes.
    let r = |z: f32, y: f32, rr: Vec2| {
        kit::ngon_ring(Vec3::new(0.0, y, z), Vec3::X, Vec3::Y, rr, 8, 0.0)
    };
    p.add(
        BODY,
        kit::loft(
            &[
                r(-0.42, 2.38, Vec2::new(0.12, 0.12)),
                r(-0.22, 2.28, Vec2::new(0.3, 0.28)),
                r(0.15, 2.2, Vec2::new(0.32, 0.3)),
                r(0.5, 2.18, Vec2::new(0.15, 0.14)),
            ],
            wax,
            true,
            (true, true),
            kit::Uv::Density(2.5),
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            BODY,
            kit::plate(
                &[
                    Vec3::new(side * 0.33, 2.4, -0.2),
                    Vec3::new(side * 0.34, 2.42, 0.34),
                    Vec3::new(side * 0.2, 2.2, 0.85),
                    Vec3::new(side * 0.29, 2.02, 0.1),
                ],
                0.04,
                brass,
            ),
        );
    }
    for (k, x) in [-0.08f32, 0.0, 0.08].into_iter().enumerate() {
        let base = Vec3::new(x, 2.2, 0.45);
        let len = 0.5 + 0.12 * (k % 2) as f32;
        p.add(
            BODY,
            kit::plate(
                &[
                    base + Vec3::new(-0.05, 0.0, 0.0),
                    base + Vec3::new(0.05, 0.0, 0.0),
                    base + Vec3::new(x * 1.5, -0.12, len),
                ],
                0.02,
                brass_dark,
            ),
        );
    }
    // Hip joints (brass bosses) and a slag crest along the back.
    for side in [-1.0f32, 1.0] {
        p.add(
            BODY,
            gem(Vec3::new(side * 0.17, 2.08, 0.1), 0.08, 0, brass_dark),
        );
    }
    for k in 0..3 {
        let z = -0.1 + 0.2 * k as f32;
        p.add(
            BODY,
            spike(
                Vec3::new(0.0, 2.4, z),
                Vec3::new(0.0, 2.55, z + 0.12),
                0.04,
                4,
                slag,
            ),
        );
    }

    // Neck: an S of wax over brass vertebra rings.
    let n0 = v(NECK_AT);
    let n1 = Vec3::new(0.0, 2.72, -0.18);
    let n2 = v(HEAD_AT);
    p.add(NECK, kit::smooth_tube(n0, n1, 0.12, 0.09, 6, wax));
    p.add(NECK, kit::smooth_tube(n1, n2, 0.09, 0.08, 6, wax));
    for f in [0.3f32, 0.7] {
        p.add(
            NECK,
            kit::band(n0.lerp(n1, f), n1 - n0, 0.085, 0.035, 0.02, 7, brass),
        );
        p.add(
            NECK,
            kit::band(n1.lerp(n2, f), n2 - n1, 0.072, 0.035, 0.02, 7, brass),
        );
    }

    // Head: a brass drum on its side, the lens on the front, a needle-barrel beak below, a
    // crest of brass vanes behind.
    let lc = v(LENS_AT);
    let back = lc + Vec3::new(0.0, 0.0, 0.34);
    p.add(
        HEAD,
        kit::tube(back, lc + Vec3::new(0.0, 0.0, 0.02), 0.2, 0.22, 10, brass),
    );
    p.add(
        HEAD,
        limb(
            n2,
            Vec3::new(0.0, 3.06, -0.3),
            Vec2::splat(0.07),
            Vec2::splat(0.09),
            brass_dark,
        ),
    );
    p.add(
        HEAD,
        kit::band(
            lc + Vec3::new(0.0, 0.0, 0.08),
            Vec3::Z,
            0.225,
            0.05,
            0.03,
            10,
            brass_dark,
        ),
    );
    for (k, (y, len)) in [(0.14, 0.32), (0.04, 0.38), (-0.06, 0.3)]
        .into_iter()
        .enumerate()
    {
        let base = back + Vec3::new(0.0, y, -0.02);
        p.add(
            HEAD,
            kit::plate(
                &[
                    base + Vec3::new(0.0, 0.05, 0.0),
                    base + Vec3::new(0.0, -0.05, 0.0),
                    base + Vec3::new(0.0, 0.08 + 0.04 * k as f32, len),
                ],
                0.02,
                brass,
            ),
        );
    }
    // The needle barrel: a long thin beak under the lens.
    let beak0 = lc + Vec3::new(0.0, -0.2, 0.06);
    p.add(
        HEAD,
        kit::tube(
            beak0,
            beak0 + Vec3::new(0.0, -0.04, -0.62),
            0.05,
            0.025,
            6,
            brass_dark,
        ),
    );
    p.add(
        HEAD,
        spike(
            beak0 + Vec3::new(0.0, -0.04, -0.62),
            beak0 + Vec3::new(0.0, -0.06, -0.78),
            0.025,
            5,
            slag,
        ),
    );
    // The lens: a domed disc, and the telegraph ring around it.
    p.add(
        LENS,
        ellipsoid(
            lc + Vec3::new(0.0, 0.0, -0.01),
            Vec3::new(0.17, 0.17, 0.07),
            Quat::IDENTITY,
            3,
            10,
            lens,
            false,
        ),
    );
    p.add(
        RING,
        kit::band(
            lc + Vec3::new(0.0, 0.0, -0.03),
            Vec3::Z,
            0.2,
            0.04,
            0.035,
            12,
            ring,
        ),
    );

    // Legs (right built, left mirrored): brass stilts with backward knees, long toes.
    let (hip, knee) = (v(HIP), v(KNEE));
    let foot = Vec3::new(0.2, 0.08, 0.02);
    let mut thigh = limb(hip, knee, Vec2::splat(0.08), Vec2::splat(0.055), brass);
    add(
        &mut thigh,
        kit::band(
            hip.lerp(knee, 0.3),
            knee - hip,
            0.085,
            0.05,
            0.02,
            6,
            brass_dark,
        ),
    );
    let mut shin = limb(knee, foot, Vec2::splat(0.055), Vec2::splat(0.04), brass);
    add(&mut shin, gem(knee, 0.085, 0, brass_dark));
    add(&mut shin, gem(foot, 0.065, 0, brass_dark));
    let mut toes = MeshBuilder::new();
    for (dx, dz) in [(-0.1, -0.32), (0.0, -0.38), (0.1, -0.32), (0.0, 0.2)] {
        add(
            &mut toes,
            limb(
                foot,
                foot + Vec3::new(dx, -0.07, dz),
                Vec2::splat(0.022),
                Vec2::splat(0.012),
                brass_dark,
            ),
        );
    }
    add(&mut shin, toes);
    p.add(THIGH_L, kit::mirrored_x(&thigh));
    p.add(THIGH_R, thigh);
    p.add(SHIN_L, kit::mirrored_x(&shin));
    p.add(SHIN_R, shin);

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(83 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, BODY | NECK) {
            kit::tex_wax()
        } else {
            kit::tex_plate()
        };
        p.texture(i, tex);
    }
    p.finish()
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 13);
    let needle = a.attack == NEEDLE;
    let mut glow = telegraph(a.state, t, 2.0);
    let mut body_off = Vec3::ZERO;
    let mut body_rot = rz(-0.08 * a.strafe.clamp(-1.0, 1.0));
    let mut neck = 0.08 * wave(t, 0.3, off);
    // The head keeps level and aims with the replicated pitch.
    let mut head = -neck + a.aim_pitch.clamp(-0.9, 0.9);
    let mut head_roll = 0.1 * wave(t, 0.25, off + 0.5);
    let mut legs = [(0.0f32, 0.0f32, 0.0f32); 2]; // (swing, splay, knee)
    let mut squash = 1.0f32;

    let k = (a.speed / 7.0).clamp(0.0, 1.0);
    if a.speed > 0.2 {
        // Long, high stilt strides.
        let phase = a.cycle * std::f32::consts::TAU;
        for (i, leg) in legs.iter_mut().enumerate() {
            let ph = phase + i as f32 * std::f32::consts::PI;
            leg.0 = 0.45 * k * ph.sin();
            leg.2 = 0.55 * k * (ph.cos()).max(0.0);
        }
        body_off.y = -0.05 * k * (2.0 * phase).cos().abs();
        neck += 0.12 * k * (2.0 * phase).sin();
        head = -neck + a.aim_pitch.clamp(-0.9, 0.9);
    }

    match a.state {
        EnemyState::Windup(p) if needle => {
            // Braces: legs wide, body low, neck forward and still; the ring burns, brighter
            // and strobing once the aim has locked.
            let e = smoothstep(p / 0.3);
            body_off.y -= 0.18 * e;
            body_rot *= rx(-0.12 * e);
            neck = -0.3 * e;
            head = 0.3 * e + a.aim_pitch.clamp(-0.9, 0.9) * (0.6 + 0.4 * e);
            head_roll *= 1.0 - e;
            for (i, leg) in legs.iter_mut().enumerate() {
                leg.1 = 0.22 * e * if i == 0 { -1.0 } else { 1.0 };
                leg.2 = 0.15 * e;
            }
            glow = 1.2 + 1.3 * p;
            if a.aim_locked {
                glow = 3.0 + 1.2 * strobe(t, 14.0);
            }
        }
        EnemyState::Attack(p) if needle => {
            // Recoil: the head snaps back.
            let kick = 1.0 - smoothstep(p);
            body_off.y -= 0.18;
            neck = -0.3 + 0.25 * kick;
            head = 0.3 - 0.35 * kick + a.aim_pitch.clamp(-0.9, 0.9);
        }
        EnemyState::Windup(p) => {
            // Kick: leans forward and draws the right leg back.
            let e = smoothstep(p / 0.8);
            body_rot *= rx(-0.2 * e);
            neck = 0.2 * e;
            legs[1] = (-0.7 * e, 0.0, 1.0 * e);
            legs[0].2 = 0.2 * e;
        }
        EnemyState::Attack(p) => {
            // Stamps the leg out forward.
            let s = smoothstep(p / 0.5);
            body_rot *= rx(-0.2 + 0.3 * s);
            legs[1] = (-0.7 + 1.9 * s, 0.0, 1.0 - 1.0 * s);
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            if !needle {
                legs[1] = (1.2 * s, 0.0, 0.0);
            }
            body_off.y -= 0.1 * s;
        }
        EnemyState::Stunned => {
            let w = wave(t, 0.9, off);
            body_rot = rz(0.15 * w) * rx(0.2);
            neck = 0.6;
            head = -0.4;
            head_roll = 0.4 * w;
            legs = [(0.2, -0.15, 0.4), (-0.1, 0.15, 0.3)];
        }
        EnemyState::Dying(p) => {
            // The stilts buckle, it folds down and the body melts onto them; the neck droops.
            let (e, m, sq) = melt(p);
            squash = sq;
            body_off.y = -1.6 * e;
            body_rot = rz(0.25 * e) * rx(0.4 * e);
            neck = 1.0 * e;
            head = 0.8 * e;
            head_roll = 0.6 * m;
            legs = [(0.9 * e, -0.5 * e, 2.0 * e), (0.6 * e, 0.6 * e, 1.8 * e)];
        }
        EnemyState::Idle | EnemyState::Walk | EnemyState::Latched => {}
        // (Translated into the states above by `enemies::pose`.)
        EnemyState::Crouch(_)
        | EnemyState::Airborne(_)
        | EnemyState::Melt(_)
        | EnemyState::Reform(_) => {}
    }
    pose.glow = gray(glow);
    // The lens burns brighter while aiming.
    let lens = if needle && matches!(a.state, EnemyState::Windup(_)) {
        heat(a.state, glow).max(2.0)
    } else {
        heat(a.state, glow)
    };
    pose.core = gray(lens);

    pose.set(
        BODY,
        Mat4::from_scale_rotation_translation(Vec3::new(1.0, squash, 1.0), body_rot, body_off),
    );
    pose.rotate(NECK, rx(neck));
    pose.rotate(HEAD, rx(head) * rz(head_roll));
    for (i, (thigh, shin, sgn)) in [(THIGH_L, SHIN_L, -1.0f32), (THIGH_R, SHIN_R, 1.0)]
        .into_iter()
        .enumerate()
    {
        let (swing, splay, knee) = legs[i];
        pose.rotate(thigh, rz(sgn * splay.abs()) * rx(swing));
        // Backward knees: the shin folds forward under the body.
        pose.rotate(shin, rx(knee));
    }
    pose
}
