//! Bastion: heavy armoured brute, ~3 m. Massive hunched iron torso with overlapping front
//! armour slabs (yellow-edged), a small helmeted head sunk under the collar with a visor slit,
//! huge pauldrons, gauntlet forearms ending in giant fists, exhaust stacks and an exposed
//! furnace on the back (the unarmoured side, `Core`). Telegraph: the visor, a beacon on the
//! hump and lamps on both fists (`Glow`) burn yellow through the overhead wind-up and strobe
//! white-hot during the parry window (last 35 % of the wind-up).

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{EnemyAnim, EnemyState, rx, ry, rz, seed_phase, smoothstep, strobe, walk_legs, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, GUNMETAL, add, add_mirrored, chamfer_ring, gem, limb, rgb, spike,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const TORSO: usize = 1;
pub const ARMOR: usize = 2;
pub const FURNACE: usize = 3;
pub const HEAD: usize = 4;
pub const VISOR: usize = 5;
pub const BEACON: usize = 6;
pub const UPPER_ARM_L: usize = 7;
pub const FOREARM_L: usize = 8;
pub const FIST_L: usize = 9;
pub const LAMP_L: usize = 10;
pub const UPPER_ARM_R: usize = 11;
pub const FOREARM_R: usize = 12;
pub const FIST_R: usize = 13;
pub const LAMP_R: usize = 14;
pub const THIGH_L: usize = 15;
pub const SHIN_L: usize = 16;
pub const THIGH_R: usize = 17;
pub const SHIN_R: usize = 18;

const SHOULDER: [f32; 3] = [0.74, 2.42, 0.02];
const ELBOW: [f32; 3] = [0.84, 1.8, 0.04];
const WRIST: [f32; 3] = [0.86, 1.16, 0.0];
const HIP: [f32; 3] = [0.34, 1.15, 0.12];
const KNEE: [f32; 3] = [0.38, 0.64, 0.04];

/// Wind-up progress from which the parry window strobe starts (gameplay: last 0.35 s of 1 s).
pub const PARRY_WINDOW_FROM: f32 = 0.65;

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 19] = [
    joint("pelvis", ROOT, [0.0, 1.2, 0.12], Shade::Lit),
    joint("torso", PELVIS, [0.0, 1.38, 0.12], Shade::Lit),
    joint("armor", TORSO, [0.0, 1.38, 0.12], Shade::Lit),
    joint("furnace", TORSO, [0.0, 1.38, 0.12], Shade::Core),
    joint("head", TORSO, [0.0, 2.25, -0.32], Shade::Lit),
    joint("visor", HEAD, [0.0, 2.25, -0.32], Shade::Glow),
    joint("beacon", TORSO, [0.0, 2.78, 0.3], Shade::Glow),
    joint("upper_arm_l", TORSO, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", UPPER_ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("fist_l", FOREARM_L, neg_x(WRIST), Shade::Lit),
    joint("lamp_l", FIST_L, neg_x(WRIST), Shade::Glow),
    joint("upper_arm_r", TORSO, SHOULDER, Shade::Lit),
    joint("forearm_r", UPPER_ARM_R, ELBOW, Shade::Lit),
    joint("fist_r", FOREARM_R, WRIST, Shade::Lit),
    joint("lamp_r", FIST_R, WRIST, Shade::Glow),
    joint("thigh_l", PELVIS, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", PELVIS, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// Chamfered slab facing -Z (front armour), tilted back by `tilt`.
fn slab(center: Vec3, half: Vec3, tilt: f32, color: Vec4) -> engine::assets::meshgen::MeshBuilder {
    let rot = Quat::from_rotation_x(tilt);
    let side = rot * Vec3::X;
    let up = rot * Vec3::Y;
    let fwd = rot * Vec3::NEG_Z;
    kit::loft(
        &[
            chamfer_ring(
                center - fwd * half.z,
                side,
                up,
                Vec2::new(half.x, half.y),
                half.y * 0.35,
            ),
            chamfer_ring(
                center + fwd * half.z,
                side,
                up,
                Vec2::new(half.x * 0.94, half.y * 0.9),
                half.y * 0.35,
            ),
        ],
        color,
        false,
        (true, true),
        kit::Uv::PerFace,
    )
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let iron = rgb(0x4a4440);
    let steel = rgb(0x7a7c78);
    let flesh = rgb(0x6a2420);
    let yellow = rgb(0xe0aa1a);
    let warn = Vec4::new(1.0, 0.82, 0.2, 1.0);

    // Pelvis: belt block with a flesh gap above.
    p.add(
        PELVIS,
        kit::loft(
            &[
                chamfer_ring(
                    Vec3::new(0.0, 1.0, 0.12),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::new(0.38, 0.3),
                    0.1,
                ),
                chamfer_ring(
                    Vec3::new(0.0, 1.32, 0.12),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::new(0.45, 0.34),
                    0.12,
                ),
            ],
            iron,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );

    // Torso: massive barrel rising into a hunched back; flesh showing at the waist.
    let ring = |y: f32, z: f32, half: Vec2| {
        chamfer_ring(
            Vec3::new(0.0, y, z),
            Vec3::X,
            Vec3::Z,
            half,
            half.x.min(half.y) * 0.4,
        )
    };
    p.add(
        TORSO,
        kit::loft(
            &[
                ring(1.3, 0.12, Vec2::new(0.4, 0.3)),
                ring(1.42, 0.1, Vec2::new(0.48, 0.36)),
                ring(1.75, 0.06, Vec2::new(0.62, 0.45)),
                ring(2.2, 0.08, Vec2::new(0.76, 0.52)),
                ring(2.55, 0.16, Vec2::new(0.6, 0.45)),
                ring(2.72, 0.24, Vec2::new(0.32, 0.28)),
            ],
            iron,
            false,
            (true, true),
            kit::Uv::Density(2.0),
        ),
    );
    p.add(
        TORSO,
        kit::loft(
            &[
                ring(1.3, 0.12, Vec2::new(0.43, 0.32)),
                ring(1.44, 0.11, Vec2::new(0.5, 0.37)),
            ],
            flesh,
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    // Exhaust stacks on the back.
    for x in [-0.26, 0.26] {
        p.add(
            TORSO,
            kit::tube(
                Vec3::new(x, 2.35, 0.42),
                Vec3::new(x * 1.15, 3.0, 0.58),
                0.075,
                0.085,
                6,
                BLACK_STEEL,
            ),
        );
        p.add(
            TORSO,
            kit::tube(
                Vec3::new(x * 1.15, 2.96, 0.57),
                Vec3::new(x * 1.16, 3.02, 0.59),
                0.1,
                0.1,
                6,
                GUNMETAL,
            ),
        );
    }
    // Furnace housing bars on the back and the beacon cage.
    for x in [-0.2, -0.07, 0.07, 0.2] {
        p.add(
            TORSO,
            kit::block(
                Vec3::new(x, 1.95, 0.62),
                Vec3::new(0.025, 0.24, 0.03),
                Quat::from_rotation_x(-0.15),
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        TORSO,
        kit::tube(
            Vec3::new(0.0, 2.66, 0.3),
            Vec3::new(0.0, 2.72, 0.3),
            0.13,
            0.12,
            6,
            BLACK_STEEL,
        ),
    );
    for k in 0..4 {
        let a = k as f32 / 4.0 * std::f32::consts::TAU + 0.4;
        let o = Vec3::new(a.cos(), 0.0, a.sin()) * 0.1;
        p.add(
            TORSO,
            limb(
                Vec3::new(0.0, 2.7, 0.3) + o,
                Vec3::new(0.0, 2.88, 0.3) + o * 0.8,
                Vec2::splat(0.012),
                Vec2::splat(0.012),
                BLACK_STEEL,
            ),
        );
    }

    // Armour: front slabs with yellow top edges, huge pauldrons, collar.
    for (c, h, tilt) in [
        (Vec3::new(0.0, 2.08, -0.5), Vec3::new(0.56, 0.3, 0.07), 0.3),
        (
            Vec3::new(0.0, 1.62, -0.44),
            Vec3::new(0.44, 0.2, 0.07),
            0.12,
        ),
        (
            Vec3::new(0.0, 1.22, -0.27),
            Vec3::new(0.3, 0.14, 0.06),
            -0.1,
        ),
    ] {
        p.add(ARMOR, slab(c, h, tilt, steel));
        let top = c + Quat::from_rotation_x(tilt) * Vec3::new(0.0, h.y - 0.02, -h.z - 0.004);
        p.add(
            ARMOR,
            kit::block(
                top,
                Vec3::new(h.x * 0.92, 0.025, 0.012),
                Quat::from_rotation_x(tilt),
                yellow,
            ),
        );
    }
    let pauldron = kit::ellipsoid(
        Vec3::new(0.76, 2.55, 0.02),
        Vec3::new(0.33, 0.24, 0.4),
        Quat::from_rotation_z(-0.35),
        4,
        7,
        steel,
        false,
    );
    let pauldron_rim = kit::block(
        Vec3::new(0.92, 2.38, 0.02),
        Vec3::new(0.17, 0.03, 0.36),
        Quat::from_rotation_z(-0.5),
        yellow,
    );
    let horn = spike(
        Vec3::new(0.82, 2.72, 0.05),
        Vec3::new(1.02, 2.98, 0.2),
        0.07,
        4,
        BLACK_STEEL,
    );
    add_mirrored(p.mesh(ARMOR), &pauldron);
    add_mirrored(p.mesh(ARMOR), &pauldron_rim);
    add_mirrored(p.mesh(ARMOR), &horn);
    // Collar over the head.
    p.add(
        ARMOR,
        slab(
            Vec3::new(0.0, 2.48, -0.38),
            Vec3::new(0.32, 0.1, 0.12),
            -0.5,
            steel,
        ),
    );

    // Furnace glow behind the bars.
    p.add(
        FURNACE,
        kit::block(
            Vec3::new(0.0, 1.95, 0.59),
            Vec3::new(0.26, 0.22, 0.02),
            Quat::from_rotation_x(-0.15),
            Vec4::new(1.0, 0.45, 0.12, 1.0),
        ),
    );

    // Head: small helmet sunk under the collar.
    let h = Vec3::new(0.0, 2.25, -0.45);
    p.add(
        HEAD,
        kit::loft(
            &[
                chamfer_ring(
                    h + Vec3::new(0.0, 0.0, 0.16),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.16, 0.15),
                    0.06,
                ),
                chamfer_ring(
                    h + Vec3::new(0.0, 0.0, -0.12),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.15, 0.14),
                    0.06,
                ),
                chamfer_ring(
                    h + Vec3::new(0.0, -0.03, -0.2),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.1, 0.09),
                    0.04,
                ),
            ],
            GUNMETAL,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        VISOR,
        kit::block(
            h + Vec3::new(0.0, 0.035, -0.168),
            Vec3::new(0.13, 0.034, 0.014),
            Quat::IDENTITY,
            warn,
        ),
    );
    p.add(BEACON, gem(Vec3::new(0.0, 2.82, 0.3), 0.12, 0, warn));

    // Arms (right built, left mirrored).
    let (s, e, w) = (v(SHOULDER), v(ELBOW), v(WRIST));
    let mut upper = limb(s, e, Vec2::new(0.19, 0.2), Vec2::new(0.15, 0.16), iron);
    add(&mut upper, gem(e, 0.17, 0, flesh));
    let mut fore = limb(
        e,
        w + Vec3::new(0.0, 0.05, 0.0),
        Vec2::new(0.17, 0.18),
        Vec2::new(0.25, 0.25),
        steel,
    );
    add(
        &mut fore,
        kit::block(
            w + Vec3::new(0.24, 0.3, 0.0),
            Vec3::new(0.03, 0.24, 0.16),
            Quat::from_rotation_z(0.08),
            yellow,
        ),
    );
    add(
        &mut fore,
        spike(
            e + Vec3::new(0.12, -0.15, 0.1),
            e + Vec3::new(0.32, -0.05, 0.25),
            0.05,
            4,
            BLACK_STEEL,
        ),
    );
    let fist_c = w + Vec3::new(0.0, -0.27, -0.04);
    let mut fist = kit::loft(
        &[
            chamfer_ring(
                fist_c + Vec3::new(0.0, 0.25, 0.0),
                Vec3::X,
                Vec3::Z,
                Vec2::new(0.25, 0.26),
                0.08,
            ),
            chamfer_ring(
                fist_c + Vec3::new(0.0, -0.22, -0.02),
                Vec3::X,
                Vec3::Z,
                Vec2::new(0.28, 0.3),
                0.1,
            ),
        ],
        steel,
        false,
        (true, true),
        kit::Uv::PerFace,
    );
    for dx in [-0.17, -0.06, 0.06, 0.17] {
        add(
            &mut fist,
            kit::block(
                fist_c + Vec3::new(dx, -0.16, -0.31),
                Vec3::new(0.045, 0.05, 0.05),
                Quat::IDENTITY,
                BLACK_STEEL,
            ),
        );
    }
    // Warning lamps: a glowing band around each gauntlet cuff plus a knuckle bar, visible from
    // every side (the fists are what the player has to watch).
    let mut lamps = engine::assets::meshgen::MeshBuilder::new();
    add(
        &mut lamps,
        kit::band(
            fist_c + Vec3::new(0.0, 0.2, -0.01),
            Vec3::Y,
            0.36,
            0.1,
            0.04,
            8,
            warn,
        ),
    );
    add(
        &mut lamps,
        kit::block(
            fist_c + Vec3::new(0.0, 0.0, -0.325),
            Vec3::new(0.23, 0.11, 0.014),
            Quat::IDENTITY,
            warn,
        ),
    );
    add(
        &mut lamps,
        kit::block(
            fist_c + Vec3::new(0.0, -0.225, -0.02),
            Vec3::new(0.2, 0.012, 0.22),
            Quat::IDENTITY,
            warn,
        ),
    );
    let (hip, knee) = (v(HIP), v(KNEE));
    let ankle = Vec3::new(0.38, 0.16, 0.08);
    let mut thigh = limb(hip, knee, Vec2::new(0.2, 0.2), Vec2::new(0.16, 0.17), iron);
    add(&mut thigh, gem(knee, 0.15, 0, flesh));
    let mut shin = limb(
        knee,
        ankle,
        Vec2::new(0.17, 0.18),
        Vec2::new(0.21, 0.21),
        steel,
    );
    add(
        &mut shin,
        slab(
            knee + Vec3::new(0.0, 0.0, -0.17),
            Vec3::new(0.15, 0.13, 0.04),
            -0.2,
            steel,
        ),
    );
    add(
        &mut shin,
        kit::block(
            Vec3::new(0.38, 0.09, -0.05),
            Vec3::new(0.23, 0.09, 0.32),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );

    for (right, left, mesh) in [
        (UPPER_ARM_R, UPPER_ARM_L, upper),
        (FOREARM_R, FOREARM_L, fore),
        (FIST_R, FIST_L, fist),
        (LAMP_R, LAMP_L, lamps),
        (THIGH_R, THIGH_L, thigh),
        (SHIN_R, SHIN_L, shin),
    ] {
        p.add(left, kit::mirrored_x(&mesh));
        p.add(right, mesh);
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(53 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(
            i,
            ARMOR | FOREARM_L | FOREARM_R | FIST_L | FIST_R | SHIN_L | SHIN_R
        ) {
            kit::tex_plate()
        } else {
            kit::tex_grime()
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Telegraph colour multiplier: yellow ramp, white-hot strobe in the parry window.
fn warning_glow(state: EnemyState, t: f64) -> Vec4 {
    let yellow = Vec3::ONE;
    let white_hot = Vec3::new(1.4, 1.9, 4.0);
    let c = match state {
        EnemyState::Idle | EnemyState::Walk => yellow * (0.35 + 0.08 * wave(t, 0.5, 0.0)),
        EnemyState::Windup(p) => {
            let p = p.clamp(0.0, 1.0);
            if p >= PARRY_WINDOW_FROM {
                if strobe(t, 18.0) > 0.5 {
                    white_hot
                } else {
                    yellow * 2.2
                }
            } else {
                yellow * (0.6 + 1.6 * p / PARRY_WINDOW_FROM) * (0.8 + 0.2 * strobe(t, 6.0))
            }
        }
        EnemyState::Attack(p) => yellow * (2.4 - 1.4 * p.clamp(0.0, 1.0)),
        EnemyState::Recover(p) => yellow * (1.0 - 0.6 * p.clamp(0.0, 1.0)),
        EnemyState::Stunned => yellow * (0.1 + 0.3 * strobe(t, 4.0)),
        EnemyState::Dying(p) => yellow * 0.4 * (1.0 - p.clamp(0.0, 1.0)),
    };
    c.extend(1.0)
}

#[derive(Clone, Copy)]
struct Arm {
    upper: f32,
    /// Rotation about Z towards the body (+ = inward).
    inward: f32,
    fore: f32,
    fist: f32,
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 4);
    pose.glow = warning_glow(a.state, t);
    let furnace = 0.9 + 0.2 * wave(t, 0.8, off) + 0.08 * wave(t, 5.3, off);
    pose.core = Vec4::new(furnace, furnace, furnace, 1.0);

    let breathe = wave(t, 0.4, off);
    let mut torso = rx(-0.12 + 0.03 * breathe);
    let mut head = rx(a.aim_pitch.clamp(-0.4, 0.4) * 0.5);
    let mut pelvis_off = Vec3::ZERO;
    let mut pelvis_rot = Quat::IDENTITY;
    let rest = Arm {
        upper: 0.12,
        inward: -0.08,
        fore: 0.25,
        fist: 0.0,
    };
    let mut arms = [rest, rest];
    let mut legs = [(0.05f32, -0.1f32), (-0.05, -0.08)];

    if a.speed > 0.2 {
        let k = (a.speed / 5.0).clamp(0.0, 1.0);
        let amp = 0.2 + 0.25 * k;
        legs = walk_legs(a.cycle, amp, 1.1);
        let phase = a.cycle * std::f32::consts::TAU;
        let (s, _) = phase.sin_cos();
        // Heavy side-to-side sway, slamming steps.
        pelvis_rot = rz(0.07 * s);
        torso *= rz(-0.04 * s) * ry(0.06 * s);
        pelvis_off.y = -0.06 * (2.0 * phase).sin().abs();
        arms[0].upper -= 0.25 * amp * s;
        arms[1].upper += 0.25 * amp * s;
    }
    pelvis_rot *= rz(-0.08 * a.strafe.clamp(-1.0, 1.0));

    let overhead = |arm: &mut Arm, e: f32| {
        arm.upper += (2.95 - arm.upper) * e;
        arm.inward += (0.32 - arm.inward) * e;
        arm.fore += (0.55 - arm.fore) * e;
        arm.fist += 0.3 * e;
    };
    let slammed = |arm: &mut Arm, s: f32| {
        arm.upper += (0.95 - arm.upper) * s;
        arm.inward += (0.18 - arm.inward) * s;
        arm.fore += (0.5 - arm.fore) * s;
        arm.fist += (0.2 - arm.fist) * s;
    };
    match a.state {
        EnemyState::Windup(p) => {
            let e = smoothstep(p / 0.7);
            for arm in &mut arms {
                overhead(arm, e);
            }
            torso = rx(-0.12 + 0.32 * e);
            head = rx(0.25 * e);
            pelvis_off.y -= 0.12 * e;
            legs = [(0.25 * e, -0.45 * e), (-0.1 * e, -0.3 * e)];
            // Trembling with strain in the parry window.
            if p >= PARRY_WINDOW_FROM {
                torso *= rz(0.015 * wave(t, 17.0, 0.0));
            }
        }
        EnemyState::Attack(p) => {
            let s = smoothstep(p / 0.25);
            for arm in &mut arms {
                overhead(arm, 1.0 - s);
                slammed(arm, s);
            }
            torso = rx(-0.12 + 0.32 * (1.0 - s) - 0.62 * s);
            head = rx(0.25 * (1.0 - s) + 0.35 * s);
            pelvis_off.y -= 0.12 + 0.22 * s;
            pelvis_off.z -= 0.15 * s;
            legs = [
                (0.25 + 0.4 * s, -0.45 - 0.5 * s),
                (-0.1 - 0.25 * s, -0.3 - 0.3 * s),
            ];
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            for arm in &mut arms {
                slammed(arm, s);
            }
            torso = rx(-0.12 - 0.62 * s);
            head = rx(0.35 * s);
            pelvis_off.y -= 0.34 * s;
            pelvis_off.z -= 0.15 * s;
            legs = [(0.65 * s, -0.95 * s), (-0.35 * s, -0.6 * s)];
        }
        EnemyState::Stunned => {
            // Dropped to one knee, slumped, arms hanging, swaying groggily.
            let w = wave(t, 0.7, off);
            pelvis_off = Vec3::new(0.0, -0.5, 0.0);
            legs = [(1.4, -1.45), (-0.25, -1.5)];
            torso = rx(-0.6) * rz(0.08 * w);
            head = rx(-0.45) * rz(0.3 * w);
            arms = [
                Arm {
                    upper: 0.55,
                    inward: -0.15,
                    fore: 0.15,
                    fist: 0.0,
                },
                Arm {
                    upper: 0.3,
                    inward: -0.25,
                    fore: 0.05,
                    fist: 0.0,
                },
            ];
            pose.core = Vec4::new(0.5, 0.5, 0.5, 1.0) * (0.6 + 0.5 * strobe(t, 3.0));
            pose.core.w = 1.0;
        }
        EnemyState::Dying(p) => {
            // Drops to its knees, then topples forward onto its fists and face.
            let e = smoothstep(p / 0.5);
            let f = smoothstep((p - 0.4) / 0.6);
            pelvis_off = Vec3::new(0.0, -0.55 * e - 0.25 * f, -0.3 * f);
            pelvis_rot = rx(-0.9 * f);
            legs = [
                (1.5 * e - 0.6 * f, -1.55 * e),
                (1.4 * e - 0.6 * f, -1.5 * e),
            ];
            torso = rx(-0.3 - 0.5 * e);
            head = rx(-0.4 * e) * rz(0.3 * f);
            arms = [
                Arm {
                    upper: 0.12 + 1.1 * f,
                    inward: -0.08 - 0.3 * e,
                    fore: 0.25,
                    fist: 0.0,
                },
                Arm {
                    upper: 0.12 + 0.6 * e,
                    inward: -0.08 - 0.2 * e,
                    fore: 0.25 + 0.3 * f,
                    fist: 0.0,
                },
            ];
            pose.core = Vec4::new(1.0 - e, 1.0 - e, 1.0 - e, 1.0);
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    pose.rotate_move(PELVIS, pelvis_rot, pelvis_off);
    pose.rotate(TORSO, torso);
    pose.rotate(HEAD, head);
    for (i, (upper, fore, fist, side)) in [
        (UPPER_ARM_L, FOREARM_L, FIST_L, -1.0f32),
        (UPPER_ARM_R, FOREARM_R, FIST_R, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let arm = arms[i];
        pose.rotate(upper, rz(arm.inward * side) * rx(arm.upper));
        pose.rotate(fore, rx(arm.fore));
        pose.rotate(fist, rx(arm.fist));
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
