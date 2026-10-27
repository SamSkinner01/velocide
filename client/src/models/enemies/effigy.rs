//! Effigy: the warm-up's training dummy, a 2.2 m wax figure roughly moulded like a Mould,
//! bound with rope to a timber post on an iron tripod foot, arms lashed out along a crossbar. A
//! target sigil burns on its chest and a wick-knot on its round head (both `Core`: the weak
//! points); the rope binds glow faintly (`Glow`, it never winds up). It wobbles on its post
//! when hit (`Recover`) and sags when broken... which it never is.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{EnemyAnim, EnemyState, gray, heat, melt, rx, rz, seed_phase, telegraph, wave};
use crate::models::PartSet;
use crate::models::kit::{self, ellipsoid, gem, limb, rgb};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const POST: usize = 0;
pub const FIGURE: usize = 1;
pub const BINDS: usize = 2;
pub const SIGIL: usize = 3;
pub const HEAD: usize = 4;
pub const WICK: usize = 5;

const NECK: [f32; 3] = [0.0, 1.74, 0.0];

pub const JOINTS: [JointDef; 6] = [
    joint("post", ROOT, [0.0, 0.0, 0.0], Shade::Lit),
    joint("figure", POST, [0.0, 0.5, 0.08], Shade::Lit),
    joint("binds", FIGURE, [0.0, 0.5, 0.08], Shade::Glow),
    joint("sigil", FIGURE, [0.0, 0.5, 0.08], Shade::Core),
    joint("head", FIGURE, NECK, Shade::Lit),
    joint("wick", HEAD, NECK, Shade::Core),
];

fn ring(c: Vec3, r: Vec2) -> Vec<Vec3> {
    kit::ngon_ring(c, Vec3::X, Vec3::Z, r, 8, 0.0)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wax = rgb(0xd8d0bf);
    let wax_dark = rgb(0xa49b8a);
    let timber = rgb(0x5a3f2a);
    let iron = rgb(0x3a3735);
    let rope = Vec4::new(1.0, 0.55, 0.18, 1.0);
    let sigil = Vec4::new(1.0, 0.62, 0.2, 1.0);
    let wick = Vec4::new(1.0, 0.75, 0.32, 1.0);

    // The post: an iron tripod foot, a timber post behind the figure, a crossbar at its
    // shoulders.
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + 0.5;
        let tip = Vec3::new(a.cos() * 0.55, 0.04, a.sin() * 0.55 + 0.12);
        p.add(
            POST,
            limb(
                Vec3::new(0.0, 0.35, 0.12),
                tip,
                Vec2::splat(0.05),
                Vec2::splat(0.04),
                iron,
            ),
        );
    }
    p.add(
        POST,
        limb(
            Vec3::new(0.0, 0.0, 0.12),
            Vec3::new(0.0, 2.0, 0.12),
            Vec2::splat(0.07),
            Vec2::splat(0.06),
            timber,
        ),
    );
    p.add(
        POST,
        kit::block(
            Vec3::new(0.0, 1.6, 0.12),
            Vec3::new(0.82, 0.05, 0.05),
            Quat::IDENTITY,
            timber,
        ),
    );

    // The figure: a lumpy wax body hung on the post (feet clear of the floor), arms along
    // the crossbar.
    p.add(
        FIGURE,
        kit::loft(
            &[
                ring(Vec3::new(0.0, 0.5, 0.0), Vec2::new(0.14, 0.12)),
                ring(Vec3::new(0.0, 0.95, -0.02), Vec2::new(0.26, 0.2)),
                ring(Vec3::new(0.0, 1.3, -0.04), Vec2::new(0.32, 0.24)),
                ring(Vec3::new(0.0, 1.58, -0.02), Vec2::new(0.36, 0.22)),
                ring(Vec3::new(0.0, 1.74, 0.0), Vec2::new(0.14, 0.12)),
            ],
            wax,
            true,
            (true, true),
            kit::Uv::Density(2.5),
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            FIGURE,
            limb(
                Vec3::new(side * 0.3, 1.58, 0.0),
                Vec3::new(side * 0.78, 1.62, 0.06),
                Vec2::splat(0.09),
                Vec2::splat(0.07),
                wax_dark,
            ),
        );
        p.add(
            FIGURE,
            ellipsoid(
                Vec3::new(side * 0.84, 1.6, 0.06),
                Vec3::splat(0.09),
                Quat::IDENTITY,
                2,
                6,
                wax_dark,
                false,
            ),
        );
        // Rope binds at the wrists.
        p.add(
            BINDS,
            kit::band(
                Vec3::new(side * 0.7, 1.61, 0.06),
                Vec3::X,
                0.1,
                0.06,
                0.03,
                6,
                rope,
            ),
        );
    }
    for (y, r) in [(0.82, 0.24), (1.18, 0.33)] {
        p.add(
            BINDS,
            kit::band(Vec3::new(0.0, y, 0.0), Vec3::Y, r, 0.05, 0.03, 10, rope),
        );
    }
    // The target sigil: a ring and a dot on the chest.
    let chest = Vec3::new(0.0, 1.35, -0.27);
    p.add(
        SIGIL,
        kit::band(chest, Vec3::Z, 0.17, 0.035, 0.03, 12, sigil),
    );
    p.add(SIGIL, gem(chest + Vec3::Z * -0.01, 0.07, 0, sigil));

    // Head: a round wax knob with a crude thumbed face and a wick-knot on top.
    let head = Vec3::new(0.0, 1.97, -0.02);
    p.add(
        HEAD,
        ellipsoid(
            head,
            Vec3::new(0.22, 0.24, 0.22),
            Quat::IDENTITY,
            4,
            8,
            wax,
            true,
        ),
    );
    p.add(
        HEAD,
        kit::smooth_tube(
            Vec3::from(NECK),
            head - Vec3::Y * 0.15,
            0.1,
            0.12,
            6,
            wax_dark,
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            HEAD,
            gem(head + Vec3::new(side * 0.08, 0.03, -0.2), 0.04, 0, wax_dark),
        );
    }
    p.add(WICK, gem(head + Vec3::new(0.0, 0.24, 0.0), 0.06, 1, wick));

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(191 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if i == POST {
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
    let off = seed_phase(a.seed, 67);
    let glow = telegraph(a.state, t, 2.0);
    let mut sway = 0.015 * wave(t, 0.25, off);
    let mut lean = 0.0f32;
    let mut nod = 0.04 * wave(t, 0.2, off + 0.4);
    let mut drop = 0.0f32;
    let mut squash = 1.0f32;
    match a.state {
        EnemyState::Recover(p) | EnemyState::Attack(p) => {
            // A hit: knocked back on its post, wobbling back upright.
            let d = (1.0 - p.clamp(0.0, 1.0)).powi(2);
            lean = 0.22 * d * (p * 16.0).cos();
            sway = 0.08 * d * (p * 11.0 + off).sin();
            nod = -0.35 * d * (p * 19.0).cos();
        }
        EnemyState::Stunned => {
            lean = 0.15;
            nod = 0.4;
        }
        EnemyState::Dying(p) => {
            let (e, _, sq) = melt(p);
            squash = sq;
            drop = -0.3 * e;
            nod = 0.6 * e;
        }
        EnemyState::Idle | EnemyState::Walk | EnemyState::Windup(_) | EnemyState::Latched => {}
        // (Translated into the states above by `enemies::pose`.)
        EnemyState::Crouch(_)
        | EnemyState::Airborne(_)
        | EnemyState::Melt(_)
        | EnemyState::Reform(_) => {}
    }
    pose.glow = gray(glow);
    pose.core = gray(heat(a.state, glow));
    pose.set(
        FIGURE,
        Mat4::from_scale_rotation_translation(
            Vec3::new(1.0, squash, 1.0),
            rz(sway) * rx(lean),
            Vec3::Y * drop,
        ),
    );
    pose.rotate(HEAD, rx(nod));
    pose
}
