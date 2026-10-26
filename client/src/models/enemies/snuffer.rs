//! Snuffer: the skitterer, a low Mould 2.2 m from tail to cap on six spidery brass legs. A
//! segmented wax body under brass shell plates, and for a head a candle snuffer: a long brass
//! handle of a neck ending in a big bell-shaped cap. Under the cap glows its wick (the weak
//! point, `Core`), shown when it rears the cap up to leap and hidden when it clamps the cap over
//! a victim's chest. Telegraph (`Glow`): the cap's rim, red-hot as it rears to pounce and as it
//! smothers.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, ry, rz, seed_phase, smoothstep, telegraph, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const BODY: usize = 0;
pub const NECK: usize = 1;
pub const CAP: usize = 2;
pub const WICK: usize = 3;
pub const RIM: usize = 4;
pub const LEG_L0: usize = 5;
pub const LEG_L1: usize = 6;
pub const LEG_L2: usize = 7;
pub const LEG_R0: usize = 8;
pub const LEG_R1: usize = 9;
pub const LEG_R2: usize = 10;
pub const TAIL: usize = 11;

/// Attack indices (assets/enemies/snuffer.ron).
pub const LATCH: u8 = 0;
pub const NIP: u8 = 1;

const NECK_AT: [f32; 3] = [0.0, 0.74, -0.52];
/// Where the handle meets the cap (the cap tilts about it).
const CAP_AT: [f32; 3] = [0.0, 1.22, -0.92];
/// Hips along the body (z) of the three leg pairs.
const HIPS: [f32; 3] = [-0.32, 0.08, 0.48];

pub const JOINTS: [JointDef; 12] = [
    joint("body", ROOT, [0.0, 0.66, 0.08], Shade::Lit),
    joint("neck", BODY, NECK_AT, Shade::Lit),
    joint("cap", NECK, CAP_AT, Shade::Lit),
    joint("wick", NECK, NECK_AT, Shade::Core),
    joint("rim", CAP, CAP_AT, Shade::Glow),
    joint("leg_l0", BODY, [-0.36, 0.62, HIPS[0]], Shade::Lit),
    joint("leg_l1", BODY, [-0.4, 0.62, HIPS[1]], Shade::Lit),
    joint("leg_l2", BODY, [-0.34, 0.6, HIPS[2]], Shade::Lit),
    joint("leg_r0", BODY, [0.36, 0.62, HIPS[0]], Shade::Lit),
    joint("leg_r1", BODY, [0.4, 0.62, HIPS[1]], Shade::Lit),
    joint("leg_r2", BODY, [0.34, 0.6, HIPS[2]], Shade::Lit),
    joint("tail", BODY, [0.0, 0.66, 0.82], Shade::Lit),
];

/// One leg (right side): hip → a high knee → a pointed brass foot.
fn leg(hip: Vec3, splay: f32, brass: Vec4, dark: Vec4) -> engine::assets::meshgen::MeshBuilder {
    let knee = hip + Vec3::new(0.45, 0.36, splay * 0.5);
    let foot = hip + Vec3::new(0.78, -0.6, splay);
    let mut m = limb(hip, knee, Vec2::splat(0.055), Vec2::splat(0.045), brass);
    add(
        &mut m,
        limb(knee, foot, Vec2::splat(0.045), Vec2::splat(0.03), brass),
    );
    add(&mut m, gem(knee, 0.06, 0, dark));
    add(&mut m, spike(foot, foot - Vec3::Y * 0.04, 0.035, 4, dark));
    m
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wax = rgb(0xcdc5b4);
    let wax_dark = rgb(0x9b9283);
    let brass = rgb(0xc89a42);
    let brass_dark = rgb(0x7d602b);
    let soot = rgb(0x2e2a28);
    let wick = Vec4::new(1.0, 0.72, 0.3, 1.0);
    let rim = Vec4::new(1.0, 0.4, 0.08, 1.0);

    // Body: three wax segments, brass shell plates over the back, wax drips down the flanks.
    for (z, r) in [
        (-0.32, Vec3::new(0.4, 0.27, 0.34)),
        (0.12, Vec3::new(0.45, 0.31, 0.36)),
        (0.56, Vec3::new(0.34, 0.25, 0.3)),
    ] {
        p.add(
            BODY,
            ellipsoid(Vec3::new(0.0, 0.64, z), r, Quat::IDENTITY, 4, 9, wax, true),
        );
        p.add(
            BODY,
            ellipsoid(
                Vec3::new(0.0, 0.64 + r.y * 0.62, z + 0.03),
                Vec3::new(r.x * 0.86, 0.1, r.z * 0.9),
                Quat::IDENTITY,
                2,
                8,
                brass,
                false,
            ),
        );
    }
    for (x, z) in [(-0.38, -0.2), (0.42, 0.2), (-0.3, 0.5), (0.3, -0.36)] {
        p.add(
            BODY,
            spike(
                Vec3::new(x, 0.58, z),
                Vec3::new(x * 1.05, 0.36, z),
                0.05,
                4,
                wax_dark,
            ),
        );
    }

    // The neck: the snuffer's long brass handle, with collar rings.
    let neck = Vec3::from(NECK_AT);
    let cap = Vec3::from(CAP_AT);
    p.add(NECK, kit::tube(neck, cap, 0.06, 0.05, 6, brass));
    for f in [0.2f32, 0.55, 0.85] {
        p.add(
            NECK,
            kit::band(
                neck.lerp(cap, f),
                cap - neck,
                0.065,
                0.04,
                0.025,
                6,
                brass_dark,
            ),
        );
    }

    // The cap: a brass bell-cone, mouth down over the wick; a little finial on top.
    let axis = Vec3::new(0.0, 1.0, 0.25).normalize();
    let (side, depth) = kit::frame_for(axis);
    let mouth = Vec3::new(0.0, 0.76, -1.06);
    let cring =
        |h: f32, r: f32| kit::ngon_ring(mouth + axis * h, side, depth, Vec2::splat(r), 10, 0.0);
    p.add(
        CAP,
        kit::loft(
            &[
                cring(0.0, 0.42),
                cring(0.08, 0.38),
                cring(0.3, 0.3),
                cring(0.52, 0.17),
                cring(0.64, 0.07),
            ],
            brass,
            true,
            (false, true),
            kit::Uv::Density(2.5),
        ),
    );
    // Soot inside the mouth (seen from below).
    p.add(
        CAP,
        kit::loft(
            &[cring(0.05, 0.38), vec![mouth + axis * 0.4]],
            soot,
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        CAP,
        spike(
            mouth + axis * 0.62,
            mouth + axis * 0.82,
            0.05,
            5,
            brass_dark,
        ),
    );
    // The wick: a wax stub with a glowing knot, just under the cap.
    p.add(WICK, gem(Vec3::new(0.0, 0.62, -1.02), 0.15, 1, wick));
    p.add(
        WICK,
        kit::tube(
            Vec3::new(0.0, 0.62, -1.02),
            Vec3::new(0.0, 0.8, -1.04),
            0.035,
            0.025,
            5,
            wick,
        ),
    );
    p.add(RIM, kit::band(mouth, axis, 0.42, 0.06, 0.05, 12, rim));

    // Legs: three pairs, splayed forward / out / back.
    for (i, (z, splay)) in HIPS.iter().zip([-0.3f32, 0.0, 0.3]).enumerate() {
        let x = [0.36, 0.4, 0.34][i];
        let y = [0.62, 0.62, 0.6][i];
        let right = leg(Vec3::new(x, y, *z), splay, brass, brass_dark);
        p.add(LEG_L0 + i, kit::mirrored_x(&right));
        p.add(LEG_R0 + i, right);
    }

    // Tail: a short curl of wax ending in a drip.
    let t0 = Vec3::new(0.0, 0.66, 0.82);
    let t1 = Vec3::new(0.0, 0.82, 1.02);
    let t2 = Vec3::new(0.0, 1.0, 0.98);
    p.add(TAIL, kit::smooth_tube(t0, t1, 0.13, 0.09, 6, wax));
    p.add(TAIL, kit::smooth_tube(t1, t2, 0.09, 0.05, 6, wax_dark));

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(151 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, BODY | TAIL) {
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
    let off = seed_phase(a.seed, 41);
    let latch = a.attack == LATCH;
    let mut glow = telegraph(a.state, t, 0.7);
    let mut core = heat(a.state, glow);
    let mut body_off = Vec3::ZERO;
    let mut body_rot = rz(-0.15 * a.strafe.clamp(-1.0, 1.0));
    let mut neck = 0.06 * wave(t, 0.6, off);
    let mut cap = 0.0f32;
    let mut tail = 0.1 * wave(t, 0.8, off);
    // Per leg: (swing forward / back, lift).
    let mut legs = [(0.0f32, 0.0f32); 6];
    let mut squash = 1.0f32;

    if a.speed > 0.2 {
        // A skittering tripod gait: front-left, middle-right, back-left together.
        let k = (a.speed / 10.0).clamp(0.0, 1.0);
        let phase = a.cycle * std::f32::consts::TAU;
        for (i, leg) in legs.iter_mut().enumerate() {
            let group = if matches!(i, 0 | 2 | 4) { 0.0 } else { 0.5 };
            let ph = phase + group * std::f32::consts::TAU;
            leg.0 = 0.35 * k * ph.sin();
            leg.1 = 0.3 * k * ph.cos().max(0.0);
        }
        body_off.y = 0.03 * k * (2.0 * phase).sin();
        neck += 0.08 * k * (2.0 * phase).cos();
        tail += 0.2 * k * phase.sin();
    }

    match a.state {
        EnemyState::Windup(p) if latch => {
            // Rears back, the cap tipped up off the blazing wick, legs bunched; a quiver.
            let e = smoothstep(p / 0.6);
            body_rot *= rx(0.25 * e);
            body_off += Vec3::new(0.0, -0.06 * e, 0.12 * e);
            neck = 0.3 * e;
            cap = 0.45 * e;
            for (i, leg) in legs.iter_mut().enumerate() {
                leg.0 = if i % 3 == 0 { -0.25 * e } else { 0.2 * e };
                leg.1 = -0.25 * e;
            }
            tail = 0.5 * e;
            body_off.x += 0.012 * e * wave(t, 16.0, off);
            core = core.max(1.6 + 1.4 * p);
        }
        EnemyState::Attack(p) if latch => {
            // The leap: stretched out, legs flung forward to grab, the cap open.
            let e = smoothstep(p / 0.3);
            body_rot *= rx(0.25 - 0.45 * e);
            neck = 0.3 - 0.25 * e;
            cap = 0.45 - 0.2 * e;
            for leg in &mut legs {
                leg.0 = 0.6 * e;
                leg.1 = 0.35 * e;
            }
            core = core.max(2.2);
        }
        EnemyState::Latched => {
            // Clamped on: legs wrapped round, the cap pressed down over the victim's wick,
            // heaving as it smothers.
            let h = wave(t, 2.2, off);
            body_rot = rx(-0.2);
            neck = -0.25 + 0.05 * h;
            cap = -0.45;
            for (i, leg) in legs.iter_mut().enumerate() {
                leg.0 = if i % 3 == 0 { 0.7 } else { 0.35 };
                leg.1 = 0.5 + 0.06 * wave(t, 3.0, off + i as f32 * 0.17);
            }
            squash = 1.0 + 0.05 * h;
            glow = 2.0 + 0.6 * h;
            core = 0.9;
        }
        EnemyState::Windup(p) => {
            // Nip: the cap lifts a little, the neck draws back.
            let e = smoothstep(p / 0.8);
            neck = 0.25 * e;
            cap = 0.35 * e;
            body_off.z += 0.08 * e;
        }
        EnemyState::Attack(p) => {
            // Snap: the cap's rim slams down.
            let s = smoothstep(p / 0.4);
            neck = 0.25 - 0.55 * s;
            cap = 0.35 - 0.7 * s;
            body_off.z += 0.08 - 0.2 * s;
        }
        EnemyState::Recover(p) => {
            let d = 1.0 - smoothstep(p);
            neck = -0.2 * d;
            cap = -0.2 * d;
        }
        EnemyState::Stunned => {
            // Knocked onto its back, legs pedalling the air.
            body_rot = rz(std::f32::consts::PI);
            body_off.y = 0.12;
            neck = -0.4;
            cap = 0.6;
            for (i, leg) in legs.iter_mut().enumerate() {
                leg.0 = 0.4 * wave(t, 3.2, off + i as f32 * 0.21);
                leg.1 = 0.3 + 0.2 * wave(t, 2.6, off + i as f32 * 0.37);
            }
            tail = 0.6 * wave(t, 1.5, off);
        }
        EnemyState::Dying(p) => {
            // Legs curl in under it, the cap sags onto the floor, it melts flat.
            let (e, m, sq) = melt(p);
            squash = sq;
            body_off.y = -0.3 * e;
            neck = -0.5 * e;
            cap = -0.4 * e - 0.3 * m;
            for leg in &mut legs {
                leg.0 = 0.0;
                leg.1 = -e;
            }
            tail = -0.6 * e;
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }
    pose.glow = gray(glow);
    pose.core = gray(core);

    let spread = 1.0 + 0.2 * (1.0 - squash.min(1.0));
    pose.set(
        BODY,
        Mat4::from_scale_rotation_translation(
            Vec3::new(spread, squash, spread),
            body_rot,
            body_off,
        ),
    );
    pose.rotate(NECK, rx(neck));
    pose.rotate(CAP, rx(cap));
    pose.rotate(TAIL, rx(tail));
    for (i, part) in [LEG_L0, LEG_L1, LEG_L2, LEG_R0, LEG_R1, LEG_R2]
        .into_iter()
        .enumerate()
    {
        // Left legs (first three) lift with -Z roll, right ones with +Z.
        let sgn = if i < 3 { -1.0f32 } else { 1.0 };
        let (swing, lift) = legs[i];
        pose.rotate(part, ry(sgn * swing) * rz(sgn * lift));
    }
    pose
}
