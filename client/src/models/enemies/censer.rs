//! Censer: ranged lobber, a tall robed Mould (~2.6 m to the top of its flue). A floor-length
//! slag-grey robe with brass trim and a ragged hem, pale wax hands, and in place of a head a
//! caged brass brazier full of live coals (the weak point, `Core`). Its right hand swings a
//! perforated brass censer on a long chain. Telegraph (`Glow`): the censer's embers and the
//! flames over the brazier flare while it whirls the censer overhead (Volley) or draws it back
//! for a wide swing.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, ry, rz, seed_phase, smoothstep, telegraph, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const ROBE: usize = 0;
pub const TORSO: usize = 1;
pub const HEAD: usize = 2;
pub const COALS: usize = 3;
pub const FLAMES: usize = 4;
pub const ARM_L: usize = 5;
pub const FOREARM_L: usize = 6;
pub const ARM_R: usize = 7;
pub const FOREARM_R: usize = 8;
pub const CHAIN: usize = 9;
pub const CENSER: usize = 10;
pub const EMBERS: usize = 11;

/// Attack indices (assets/enemies/censer.ron).
pub const SWING: u8 = 0;
pub const VOLLEY: u8 = 1;

const SHOULDER: [f32; 3] = [0.36, 1.84, 0.0];
const ELBOW: [f32; 3] = [0.46, 1.36, -0.06];
const WRIST: [f32; 3] = [0.5, 0.95, -0.18];
/// Censer pot centre at rest (hangs 0.65 m below the hand).
const POT: [f32; 3] = [0.5, 0.3, -0.18];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 12] = [
    joint("robe", ROOT, [0.0, 0.0, 0.0], Shade::Lit),
    joint("torso", ROBE, [0.0, 1.15, 0.0], Shade::Lit),
    joint("head", TORSO, [0.0, 1.95, -0.05], Shade::Lit),
    joint("coals", HEAD, [0.0, 1.95, -0.05], Shade::Core),
    joint("flames", HEAD, [0.0, 1.95, -0.05], Shade::Glow),
    joint("arm_l", TORSO, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("arm_r", TORSO, SHOULDER, Shade::Lit),
    joint("forearm_r", ARM_R, ELBOW, Shade::Lit),
    joint("chain", FOREARM_R, WRIST, Shade::Lit),
    joint("censer", CHAIN, POT, Shade::Lit),
    joint("embers", CENSER, POT, Shade::Glow),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let cloth = rgb(0x4d4641);
    let cloth_dark = rgb(0x2f2a27);
    let wax = rgb(0xd2cbbb);
    let brass = rgb(0xc99a40);
    let iron = rgb(0x4a4642);
    let coal = Vec4::new(1.0, 0.62, 0.2, 1.0);
    let flame = Vec4::new(1.0, 0.5, 0.1, 1.0);
    let ember = Vec4::new(1.0, 0.55, 0.15, 1.0);

    // Robe skirt: a widening cone to the floor, brass hem band, ragged strips.
    let ring = |y: f32, r: f32, z: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, z),
            Vec3::X,
            Vec3::Z,
            Vec2::splat(r),
            10,
            0.0,
        )
    };
    p.add(
        ROBE,
        kit::loft(
            &[
                ring(0.06, 0.6, 0.04),
                ring(0.5, 0.48, 0.02),
                ring(0.95, 0.36, 0.0),
                ring(1.22, 0.3, 0.0),
            ],
            cloth,
            true,
            (true, true),
            kit::Uv::Density(2.0),
        ),
    );
    p.add(
        ROBE,
        kit::band(
            Vec3::new(0.0, 0.2, 0.035),
            Vec3::Y,
            0.575,
            0.08,
            0.03,
            12,
            brass,
        ),
    );
    for k in 0..9 {
        let a = k as f32 / 9.0 * std::f32::consts::TAU + 0.2;
        let (s, c) = a.sin_cos();
        let base = Vec3::new(c * 0.56, 0.12, s * 0.56 + 0.04);
        let len = 0.08 + 0.05 * ((k * 7) % 3) as f32;
        p.add(
            ROBE,
            kit::plate(
                &[
                    base + Vec3::new(-s * 0.09, 0.02, c * 0.09),
                    base + Vec3::new(s * 0.09, 0.02, -c * 0.09),
                    base + Vec3::new(c * 0.05, -len, s * 0.05),
                ],
                0.015,
                cloth_dark,
            ),
        );
    }

    // Upper robe: chest and a heavy mantle over the shoulders, a rope belt, a brass clasp.
    p.add(
        TORSO,
        kit::loft(
            &[
                ring(1.15, 0.3, 0.0),
                ring(1.5, 0.33, 0.0),
                ring(1.78, 0.38, 0.0),
                ring(1.9, 0.26, -0.02),
            ],
            cloth,
            true,
            (true, true),
            kit::Uv::Density(2.0),
        ),
    );
    p.add(
        TORSO,
        ellipsoid(
            Vec3::new(0.0, 1.82, 0.02),
            Vec3::new(0.48, 0.14, 0.34),
            Quat::IDENTITY,
            3,
            10,
            cloth_dark,
            true,
        ),
    );
    p.add(
        TORSO,
        kit::band(
            Vec3::new(0.0, 1.2, 0.0),
            Vec3::Y,
            0.31,
            0.05,
            0.04,
            10,
            rgb(0x6e5a3a),
        ),
    );
    p.add(TORSO, gem(Vec3::new(0.0, 1.62, -0.33), 0.06, 0, brass));
    p.add(
        TORSO,
        spike(
            Vec3::new(0.0, 1.2, -0.3),
            Vec3::new(0.05, 0.85, -0.33),
            0.03,
            4,
            rgb(0x6e5a3a),
        ),
    );

    // The brazier head: a brass cage of bars and rings around a bed of coals, a pointed flue
    // cap with a finial; flames lick out between the bars.
    let c = Vec3::new(0.0, 2.22, -0.1);
    p.add(
        HEAD,
        limb(
            Vec3::new(0.0, 1.9, -0.04),
            Vec3::new(0.0, 2.0, -0.08),
            Vec2::splat(0.1),
            Vec2::splat(0.12),
            iron,
        ),
    );
    p.add(
        HEAD,
        kit::band(
            c + Vec3::new(0.0, -0.2, 0.0),
            Vec3::Y,
            0.33,
            0.06,
            0.04,
            10,
            brass,
        ),
    );
    p.add(
        HEAD,
        kit::band(
            c + Vec3::new(0.0, 0.17, 0.0),
            Vec3::Y,
            0.3,
            0.05,
            0.04,
            10,
            brass,
        ),
    );
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        let (s, co) = a.sin_cos();
        p.add(
            HEAD,
            limb(
                c + Vec3::new(co * 0.33, -0.24, s * 0.33),
                c + Vec3::new(co * 0.27, 0.24, s * 0.27),
                Vec2::splat(0.022),
                Vec2::splat(0.02),
                brass,
            ),
        );
    }
    p.add(
        HEAD,
        kit::loft(
            &[
                kit::ngon_ring(
                    c + Vec3::Y * 0.22,
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.3),
                    8,
                    0.0,
                ),
                kit::ngon_ring(
                    c + Vec3::Y * 0.32,
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.14),
                    8,
                    0.0,
                ),
                vec![c + Vec3::Y * 0.42],
            ],
            iron,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(HEAD, gem(c + Vec3::Y * 0.45, 0.045, 0, brass));
    p.add(
        COALS,
        ellipsoid(
            c,
            Vec3::new(0.27, 0.2, 0.27),
            Quat::IDENTITY,
            4,
            8,
            coal,
            false,
        ),
    );
    for k in 0..5 {
        let a = k as f32 / 5.0 * std::f32::consts::TAU + 0.4;
        let (s, co) = a.sin_cos();
        let base = c + Vec3::new(co * 0.16, 0.12, s * 0.16);
        p.add(
            FLAMES,
            spike(
                base,
                base + Vec3::new(co * 0.08, 0.3, s * 0.08),
                0.07,
                4,
                flame,
            ),
        );
    }

    // Arms: long sleeves flaring at the cuffs, pale wax hands.
    let (s, e, w) = (v(SHOULDER), v(ELBOW), v(WRIST));
    let mut upper = limb(s, e, Vec2::splat(0.11), Vec2::splat(0.1), cloth);
    add(&mut upper, gem(s, 0.13, 0, cloth_dark));
    let mut fore = limb(
        e,
        w + Vec3::Y * 0.12,
        Vec2::splat(0.1),
        Vec2::splat(0.16),
        cloth,
    );
    add(
        &mut fore,
        kit::band(w + Vec3::Y * 0.13, w - e, 0.15, 0.04, 0.03, 8, brass),
    );
    add(
        &mut fore,
        ellipsoid(
            w,
            Vec3::new(0.07, 0.1, 0.06),
            Quat::IDENTITY,
            3,
            6,
            wax,
            true,
        ),
    );
    p.add(FOREARM_L, kit::mirrored_x(&fore));
    p.add(FOREARM_R, fore);
    p.add(ARM_L, kit::mirrored_x(&upper));
    p.add(ARM_R, upper);
    // Left hand: a dangling iron hook on a short chain.
    p.add(
        FOREARM_L,
        spike(
            Vec3::new(-0.5, 0.88, -0.18),
            Vec3::new(-0.5, 0.62, -0.2),
            0.025,
            4,
            iron,
        ),
    );

    // The chain: links from the hand down to the pot.
    let pot = v(POT);
    let links = 7;
    for k in 0..links {
        let f0 = k as f32 / links as f32;
        let f1 = (k as f32 + 0.85) / links as f32;
        let a = w.lerp(pot + Vec3::Y * 0.2, f0);
        let b = w.lerp(pot + Vec3::Y * 0.2, f1);
        let side = if k % 2 == 0 { Vec3::X } else { Vec3::Z };
        p.add(
            CHAIN,
            kit::limb_oriented(
                a,
                b,
                side,
                Vec2::new(0.035, 0.012),
                Vec2::new(0.035, 0.012),
                iron,
            ),
        );
    }
    // The censer: a perforated brass sphere with a lid knob and three hanging chains.
    p.add(
        CENSER,
        ellipsoid(
            pot,
            Vec3::new(0.2, 0.18, 0.2),
            Quat::IDENTITY,
            4,
            8,
            brass,
            false,
        ),
    );
    p.add(
        CENSER,
        kit::band(pot, Vec3::Y, 0.205, 0.04, 0.03, 10, rgb(0x7a5e2a)),
    );
    p.add(CENSER, gem(pot + Vec3::Y * 0.21, 0.05, 0, brass));
    p.add(
        CENSER,
        spike(
            pot - Vec3::Y * 0.17,
            pot - Vec3::Y * 0.3,
            0.05,
            4,
            rgb(0x7a5e2a),
        ),
    );
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU;
        let (s, co) = a.sin_cos();
        p.add(
            CENSER,
            limb(
                pot + Vec3::new(co * 0.17, 0.08, s * 0.17),
                pot + Vec3::Y * 0.2,
                Vec2::splat(0.01),
                Vec2::splat(0.01),
                iron,
            ),
        );
    }
    // Embers glowing through the perforations.
    for k in 0..10 {
        let a = k as f32 / 10.0 * std::f32::consts::TAU;
        let y = if k % 2 == 0 { 0.03 } else { -0.06 };
        let (s, co) = a.sin_cos();
        let at = pot + Vec3::new(co * 0.19, y, s * 0.19);
        p.add(
            EMBERS,
            kit::block(at, Vec3::new(0.035, 0.03, 0.035), ry(-a), ember),
        );
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(47 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, HEAD | CHAIN | CENSER) {
            kit::tex_plate()
        } else {
            kit::tex_grime()
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Chain rotation (relative to the forearm whose world rotation is `parent`) that points the
/// chain out horizontally at yaw `spin` (model space), drooping by `droop` radians.
fn chain_out(parent: Quat, spin: f32, droop: f32) -> Quat {
    // The chain hangs along -Y at rest; rz(pi/2 - droop) turns it out along +X.
    let world = ry(spin) * rz(std::f32::consts::FRAC_PI_2 - droop);
    parent.inverse() * world
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 5);
    let glow = telegraph(a.state, t, 0.7);
    pose.glow = gray(glow);
    pose.core = gray(heat(a.state, glow) * (1.0 + 0.08 * wave(t, 3.1, off)));

    // Glide: no legs, the robe sways and bobs.
    let k = (a.speed / 5.0).clamp(0.0, 1.0);
    let phase = a.cycle * std::f32::consts::TAU;
    let mut robe_rot = rz(0.04 * k * phase.sin() - 0.12 * a.strafe.clamp(-1.0, 1.0))
        * rx(-0.08 * k + 0.02 * wave(t, 0.3, off));
    let mut robe_off = Vec3::new(0.0, 0.03 * k * (2.0 * phase).sin().abs(), 0.0);
    let mut torso = rx(0.03 * wave(t, 0.45, off));
    let mut head = rx(0.05 * wave(t, 0.6, off + 0.3)) * rz(0.04 * wave(t, 0.37, off));
    let mut arm_l = rz(-0.12) * rx(0.15);
    let fore_l = rx(0.3);
    let mut arm_r = rz(0.1) * rx(0.2);
    let mut fore_r = rx(0.25);
    // The censer swings like a pendulum (relative to the forearm).
    let mut chain =
        rx(0.25 * wave(t, 0.55, off) + 0.25 * k * phase.sin()) * rz(0.1 * wave(t, 0.4, off));
    let mut whirl: Option<(f32, f32)> = None;
    let mut squash = 1.0f32;

    let volley = a.attack == VOLLEY;
    match a.state {
        EnemyState::Windup(p) if volley => {
            // Raises the censer overhead and whirls it faster and faster.
            let e = smoothstep(p / 0.35);
            arm_r = rz(0.25 * e) * rx(2.75 * e + 0.2 * (1.0 - e));
            fore_r = rx(0.25 * (1.0 - e));
            torso = rx(-0.12 * e);
            head = rx(-0.1 * e);
            arm_l = rz(-0.5 * e) * rx(0.3 * e);
            let turns = 0.6 * p + 1.6 * p * p;
            whirl = Some((turns * std::f32::consts::TAU * 1.6, 0.25 + 0.2 * (1.0 - e)));
        }
        EnemyState::Attack(p) if volley => {
            // Flings: the whirl keeps going, the body leans into each lob.
            let lob = (p.clamp(0.0, 0.999) * 3.0).fract();
            let kick = (1.0 - lob).powi(2);
            arm_r = rz(0.25) * rx(2.55 - 0.35 * kick);
            torso = rx(-0.12 + 0.18 * kick);
            arm_l = rz(-0.5) * rx(0.3);
            whirl = Some(((2.2 + 3.0 * p) * std::f32::consts::TAU, 0.2));
        }
        EnemyState::Recover(p) if volley => {
            let s = 1.0 - smoothstep(p);
            arm_r = rz(0.1 + 0.15 * s) * rx(0.2 + 2.35 * s);
            fore_r = rx(0.25 * (1.0 - s));
            if s > 0.3 {
                whirl = Some((
                    ((2.2 + 3.0) + 2.0 * p) * std::f32::consts::TAU,
                    0.2 + 0.9 * (1.0 - s),
                ));
            }
        }
        EnemyState::Windup(p) => {
            // Swing: twists right and draws the censer back, chain trailing.
            let e = smoothstep(p / 0.8);
            robe_rot *= ry(-0.7 * e);
            torso = ry(-0.35 * e) * rx(0.06 * e);
            arm_r = rz(1.1 * e) * rx(0.2 - 0.6 * e);
            fore_r = rx(0.25 * (1.0 - e));
            whirl = Some((
                0.9 * e + std::f32::consts::FRAC_PI_2 * (1.0 - e),
                0.35 + 0.8 * (1.0 - e),
            ));
            arm_l = rz(-0.6 * e) * rx(0.4 * e);
        }
        EnemyState::Attack(p) => {
            // A wide 160 degree sweep across the front.
            let s = smoothstep(p / 0.7);
            robe_rot *= ry(-0.7 + 1.3 * s);
            torso = ry(-0.35 + 0.75 * s);
            arm_r = rz(1.1) * rx(-0.4 + 0.9 * s);
            whirl = Some((0.9 - 3.0 * s, 0.25));
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            robe_rot *= ry(0.6 * s);
            torso = ry(0.4 * s);
            arm_r = rz(0.1 + 1.0 * s) * rx(0.2 + 0.3 * s);
        }
        EnemyState::Stunned => {
            let w = wave(t, 1.0, off);
            robe_rot = rz(0.08 * w) * rx(0.12);
            torso = rx(0.35);
            head = rx(0.5) * rz(0.3 * w);
            arm_r = rz(0.2) * rx(-0.1);
            arm_l = rz(-0.2) * rx(-0.1);
            chain = rx(0.1 * w);
        }
        EnemyState::Dying(p) => {
            // The robe crumples into a heap, the brazier tips off and the coals spill, the
            // censer drops.
            let (e, m, sq) = melt(p);
            squash = sq;
            robe_rot = rz(0.15 * e) * rx(0.3 * e);
            robe_off = Vec3::new(0.0, -0.1 * e, 0.0);
            torso = rx(0.6 * e);
            head = rx(1.2 * e) * rz(0.6 * m);
            arm_r = rz(0.6 * e) * rx(0.4 * e);
            arm_l = rz(-0.7 * e) * rx(0.3 * e);
            fore_r = rx(0.25);
            chain = rx(-0.9 * e);
            // The brazier tips off its shoulders.
            pose.rotate_move(HEAD, head, Vec3::new(0.35 * m, -0.4 * m, -0.3 * m));
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    pose.set(
        ROBE,
        Mat4::from_scale_rotation_translation(
            Vec3::new(
                1.0 + 0.4 * (1.0 - squash),
                squash,
                1.0 + 0.4 * (1.0 - squash),
            ),
            robe_rot,
            robe_off,
        ),
    );
    pose.rotate(TORSO, torso);
    if !matches!(a.state, EnemyState::Dying(_)) {
        pose.rotate(HEAD, head);
    }
    pose.rotate(ARM_L, arm_l);
    pose.rotate(FOREARM_L, fore_l);
    pose.rotate(ARM_R, arm_r);
    pose.rotate(FOREARM_R, fore_r);
    if let Some((spin, droop)) = whirl {
        let parent = robe_rot * torso * arm_r * fore_r;
        chain = chain_out(parent, spin, droop);
    }
    pose.rotate(CHAIN, chain);
    // The censer pot spins a little on its chain.
    pose.rotate(CENSER, ry(0.8 * wave(t, 0.2, off)));
    pose
}
