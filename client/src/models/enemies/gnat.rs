//! Gnat: flying ~1 m insectoid drone. Rust-and-chrome thorax, twin bolt emitters for
//! mandibles, two pairs of translucent buzzing wings, dangling legs and a caged glowing core
//! in the abdomen (the weak point, `Core`). Telegraph: emitters flare (`Glow`) while the
//! abdomen curls under, presenting the core; each bolt kicks the body back.
//!
//! Root = hover centre (no feet); the pose adds a small bob.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{EnemyAnim, EnemyState, gray, rx, ry, rz, seed_phase, smoothstep, telegraph, wave};
use crate::models::PartSet;
use crate::models::kit::{self, BLACK_STEEL, CHROME, add, add_mirrored, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const THORAX: usize = 0;
pub const HEAD: usize = 1;
pub const EMITTERS: usize = 2;
pub const ABDOMEN: usize = 3;
pub const CORE: usize = 4;
pub const WING_FL: usize = 5;
pub const WING_FR: usize = 6;
pub const WING_BL: usize = 7;
pub const WING_BR: usize = 8;
pub const LEGS: usize = 9;

pub const JOINTS: [JointDef; 10] = [
    joint("thorax", ROOT, [0.0, 0.0, 0.0], Shade::Lit),
    joint("head", THORAX, [0.0, 0.0, -0.13], Shade::Lit),
    joint("emitters", HEAD, [0.0, 0.0, -0.13], Shade::Glow),
    joint("abdomen", THORAX, [0.0, 0.0, 0.1], Shade::Lit),
    joint("core", ABDOMEN, [0.0, -0.01, 0.25], Shade::Core),
    joint("wing_fl", THORAX, [-0.07, 0.075, -0.05], Shade::Lit),
    joint("wing_fr", THORAX, [0.07, 0.075, -0.05], Shade::Lit),
    joint("wing_bl", THORAX, [-0.06, 0.065, 0.04], Shade::Lit),
    joint("wing_br", THORAX, [0.06, 0.065, 0.04], Shade::Lit),
    joint("legs", THORAX, [0.0, -0.06, -0.02], Shade::Lit),
];

/// Wing alpha (translucent pass).
const WING_ALPHA: f32 = 0.55;

fn wing(pivot: Vec3, len: f32, width: f32, sweep: f32) -> MeshBuilder {
    // Outline in the XZ plane extending to +X (right wing); mirrored for the left.
    let pts = [
        pivot + Vec3::new(0.0, 0.0, -0.015),
        pivot + Vec3::new(len * 0.55, 0.02, -0.02 + sweep * 0.4),
        pivot + Vec3::new(len, 0.03, sweep),
        pivot + Vec3::new(len * 0.9, 0.025, sweep + width * 0.7),
        pivot + Vec3::new(len * 0.35, 0.01, width),
        pivot + Vec3::new(0.0, 0.0, 0.03),
    ];
    let mut m = kit::plate(&pts, 0.006, Vec4::new(0.85, 0.95, 1.0, 1.0));
    // A dark vein spar along the leading edge.
    add(
        &mut m,
        limb(
            pivot,
            pivot + Vec3::new(len * 0.95, 0.03, sweep + 0.005),
            Vec2::new(0.006, 0.006),
            Vec2::new(0.003, 0.003),
            BLACK_STEEL,
        ),
    );
    m
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let rust = rgb(0x8a4a2a);
    let hot = Vec4::new(0.95, 0.4, 1.0, 1.0);

    // Thorax: faceted armoured body with a chrome dorsal plate.
    let ring =
        |z: f32, r: Vec2| kit::ngon_ring(Vec3::new(0.0, 0.0, z), Vec3::X, Vec3::Y, r, 6, 0.0);
    p.add(
        THORAX,
        kit::loft(
            &[
                ring(0.11, Vec2::new(0.06, 0.055)),
                ring(0.0, Vec2::new(0.1, 0.085)),
                ring(-0.1, Vec2::new(0.085, 0.07)),
                ring(-0.14, Vec2::new(0.05, 0.045)),
            ],
            rust,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        THORAX,
        kit::block(
            Vec3::new(0.0, 0.082, -0.01),
            Vec3::new(0.055, 0.012, 0.09),
            Quat::from_rotation_x(0.05),
            CHROME,
        ),
    );
    p.add(
        THORAX,
        spike(
            Vec3::new(0.0, 0.09, 0.03),
            Vec3::new(0.0, 0.16, 0.12),
            0.02,
            4,
            CHROME,
        ),
    );

    // Head: a snout; the two big faceted eyes are part of the emitter glow (they flare with
    // the wind-up).
    p.add(
        HEAD,
        kit::loft(
            &[
                ring(-0.12, Vec2::new(0.05, 0.045)),
                ring(-0.2, Vec2::new(0.04, 0.035)),
                vec![Vec3::new(0.0, -0.01, -0.24)],
            ],
            rust,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    // Twin emitter barrels (mandibles).
    let barrel = kit::tube(
        Vec3::new(0.035, -0.03, -0.16),
        Vec3::new(0.028, -0.045, -0.33),
        0.014,
        0.011,
        6,
        BLACK_STEEL,
    );
    add_mirrored(p.mesh(HEAD), &barrel);

    // Emitter tips and eyes.
    let tip = gem(Vec3::new(0.028, -0.045, -0.335), 0.03, 0, hot);
    add_mirrored(p.mesh(EMITTERS), &tip);
    let eye = gem(
        Vec3::new(0.045, 0.02, -0.17),
        0.045,
        0,
        Vec4::new(1.0, 0.3, 0.55, 1.0),
    );
    add_mirrored(p.mesh(EMITTERS), &eye);

    // Abdomen: a cage of armour bands around the core, ending in a stinger.
    let ab = |z: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, -0.01, z),
            Vec3::X,
            Vec3::Y,
            Vec2::splat(r),
            6,
            0.0,
        )
    };
    for (z, r) in [(0.13, 0.075), (0.2, 0.11), (0.27, 0.115), (0.34, 0.095)] {
        p.add(
            ABDOMEN,
            kit::loft(
                &[ab(z, r), ab(z + 0.03, r)],
                rust,
                false,
                (false, false),
                kit::Uv::PerFace,
            ),
        );
    }
    p.add(
        ABDOMEN,
        limb(
            Vec3::new(0.0, 0.06, 0.12),
            Vec3::new(0.0, 0.07, 0.38),
            Vec2::new(0.02, 0.012),
            Vec2::new(0.015, 0.01),
            CHROME,
        ),
    );
    p.add(
        ABDOMEN,
        kit::loft(
            &[ab(0.37, 0.07), vec![Vec3::new(0.0, -0.03, 0.52)]],
            rust,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        ABDOMEN,
        spike(
            Vec3::new(0.0, -0.03, 0.5),
            Vec3::new(0.0, -0.08, 0.6),
            0.015,
            4,
            BLACK_STEEL,
        ),
    );

    // Core (weak point).
    p.add(
        CORE,
        kit::ellipsoid(
            Vec3::new(0.0, -0.01, 0.25),
            Vec3::new(0.085, 0.08, 0.11),
            Quat::IDENTITY,
            4,
            8,
            Vec4::new(1.0, 0.45, 1.0, 1.0),
            false,
        ),
    );

    // Wings (translucent).
    let fl = wing(Vec3::new(0.07, 0.075, -0.05), 0.46, 0.13, 0.1);
    let bl = wing(Vec3::new(0.06, 0.065, 0.04), 0.36, 0.11, 0.14);
    p.add(WING_FR, fl.clone());
    p.add(WING_FL, kit::mirrored_x(&fl));
    p.add(WING_BR, bl.clone());
    p.add(WING_BL, kit::mirrored_x(&bl));

    // Three pairs of thin dangling legs.
    let mut legs = MeshBuilder::new();
    for z in [-0.08, -0.01, 0.06] {
        let hip = Vec3::new(0.05, -0.055, z);
        let knee = Vec3::new(0.13, -0.12, z + 0.02);
        let foot = Vec3::new(0.11, -0.28, z + 0.07);
        add(
            &mut legs,
            limb(
                hip,
                knee,
                Vec2::splat(0.009),
                Vec2::splat(0.007),
                BLACK_STEEL,
            ),
        );
        add(
            &mut legs,
            limb(knee, foot, Vec2::splat(0.007), Vec2::splat(0.004), rust),
        );
    }
    add_mirrored(p.mesh(LEGS), &legs);

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, WING_FL | WING_FR | WING_BL | WING_BR) {
            kit::tex_chrome()
        } else {
            kit::tex_grime()
        };
        p.texture(i, tex);
        if matches!(i, WING_FL | WING_FR | WING_BL | WING_BR) {
            p.color(i, Vec4::new(1.0, 1.0, 1.0, WING_ALPHA));
        }
    }
    p.finish()
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 3);
    pose.glow = gray(telegraph(a.state, t, 0.5) * 1.2);
    let core_pulse = 1.0 + 0.25 * wave(t, 2.2, off);
    pose.core = gray(core_pulse);

    let k = (a.speed / 14.0).clamp(0.0, 1.0);
    let bob = 0.06 * wave(t, 1.3, off);
    let mut body_off = Vec3::new(0.0, bob, 0.0);
    let mut body_rot =
        rx(-0.3 * k) * rz(-0.55 * a.strafe.clamp(-1.0, 1.0)) * rz(0.05 * wave(t, 0.8, off));
    let mut head = a.aim_pitch.clamp(-1.0, 0.8) * 0.6;
    let mut abdomen = 0.12 * wave(t, 1.3, off + 0.25);
    let mut flap_amp = 0.55;
    let mut legs = rx(0.2 * k + 0.08 * wave(t, 1.1, off));

    match a.state {
        EnemyState::Windup(p) => {
            let e = smoothstep(p);
            abdomen += 0.85 * e;
            body_rot *= rx(-0.15 * e);
            head += 0.1 * e;
            flap_amp = 0.55 + 0.2 * e;
            body_off.z += 0.04 * e;
        }
        EnemyState::Attack(p) => {
            // Three bolts: three recoil kicks.
            let shot = (p.clamp(0.0, 0.999) * 3.0).fract();
            let kick = (1.0 - shot).powi(2);
            abdomen += 0.85;
            body_off.z += 0.04 + 0.08 * kick;
            body_rot *= rx(-0.15 + 0.12 * kick);
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            abdomen += 0.85 * s;
            body_off.z += 0.04 * s;
        }
        EnemyState::Stunned => {
            body_rot = rz(0.6 * wave(t, 2.5, off)) * rx(0.4);
            body_off.y = -0.1 + 0.05 * wave(t, 3.0, off);
            flap_amp = 0.3;
        }
        EnemyState::Dying(p) => {
            // Spins out of the air and breaks apart: wings shear off, abdomen drops.
            let e = smoothstep(p);
            // Small drop only: the client lets the corpse fall / removes it.
            let fall = -0.5 * p * p;
            body_off += Vec3::new(0.0, fall, 0.0);
            body_rot = ry(7.0 * p) * rz(2.0 * e) * rx(0.8 * e);
            abdomen = -0.4 * e;
            pose.core = gray(
                core_pulse * (1.0 - e)
                    + 2.5 * (1.0 - smoothstep(p / 0.15)) * (p < 0.15) as u8 as f32,
            );
            for (i, wing) in [WING_FL, WING_FR, WING_BL, WING_BR].into_iter().enumerate() {
                let side = if i % 2 == 0 { -1.0 } else { 1.0 };
                pose.set(
                    wing,
                    Mat4::from_rotation_translation(
                        rz(side * 1.5 * e) * rx(1.2 * e),
                        Vec3::new(
                            side * 0.35 * e,
                            0.25 * e - 0.6 * p * p,
                            0.1 * e * (i as f32 - 1.5),
                        ),
                    ),
                );
            }
            pose.set(
                ABDOMEN,
                Mat4::from_rotation_translation(
                    rx(abdomen - 1.0 * e),
                    Vec3::new(0.0, -0.2 * e, 0.15 * e),
                ),
            );
            pose.rotate_move(THORAX, body_rot, body_off);
            pose.rotate(HEAD, rx(-0.5 * e));
            pose.rotate(LEGS, rx(0.9 * e));
            pose.glow = gray(0.2 * (1.0 - e));
            return pose;
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    // Buzzing wings: very fast flaps (aliasing at 60 fps reads as a blur), front / back
    // pairs out of phase.
    let flap = |phase: f32| flap_amp * wave(t, 23.0, off + phase);
    let (front, back) = (flap(0.0), flap(0.35));
    pose.rotate(WING_FL, rz(-front) * ry(0.15 * front));
    pose.rotate(WING_FR, rz(front) * ry(-0.15 * front));
    pose.rotate(WING_BL, rz(-back));
    pose.rotate(WING_BR, rz(back));
    if !matches!(a.state, EnemyState::Stunned) {
        legs *= rz(0.1 * a.strafe.clamp(-1.0, 1.0));
    }
    pose.rotate_move(THORAX, body_rot, body_off);
    pose.rotate(HEAD, rx(head));
    pose.rotate(ABDOMEN, rx(abdomen));
    pose.rotate(LEGS, legs);
    pose
}
