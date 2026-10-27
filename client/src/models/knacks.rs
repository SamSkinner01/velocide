//! The knacks' things in the world (docs/refit.md "Knacks", area **arsenal-client**): what a
//! Shrike throws with Q besides the Thorn.
//!
//! - **Kindle flask** (`prop_kindle_flask.gltf`): a squat glass flask in a brass cage with a
//!   leather sling, stoppered with wax; kindling embers glow inside (`Glow`, the heal colour
//!   through `Tint::glow`). Flies along -Z (any orientation: it tumbles), ~0.3 m drawn.
//! - **Snare** (`prop_snare.gltf`): a wax trap laid on the floor, ~0.9 m across: a poured wax
//!   disc with a brass ring, two toothed brass jaws hinged on the ring that lie open flat while
//!   it waits and snap up together when it springs ([`snare_pose`]), and a wick lamp in the
//!   middle (`Glow`): dull while it arms, a slow pulse once armed. Origin on the floor.
//! - **Soot pot** (`prop_soot_pot.gltf`): a round black clay pot with a rag-stuffed neck and a
//!   short fuse spitting sparks (`Glow`), ~0.3 m. Flies along -Z.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::kit::{self, BRASS, DARK_LEATHER, IRON, WRAP, block, gem, limb, rgb, tube};
use super::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};
use super::{ModelId, PartSet};

const FLASK: [JointDef; 3] = [
    static_part("body", Shade::Lit),
    static_part("glass", Shade::Lit),
    static_part("embers", Shade::Glow),
];

/// The snare's jaw hinges (on the brass ring, either side of the middle).
const HINGE_R: [f32; 3] = [0.06, 0.05, 0.0];
const HINGE_L: [f32; 3] = [-0.06, 0.05, 0.0];

pub const SNARE_BASE: usize = 0;
pub const SNARE_JAW_R: usize = 1;
pub const SNARE_JAW_L: usize = 2;
pub const SNARE_LAMP: usize = 3;

const SNARE: [JointDef; 4] = [
    static_part("base", Shade::Lit),
    joint("jaw_r", ROOT, HINGE_R, Shade::Lit),
    joint("jaw_l", ROOT, HINGE_L, Shade::Lit),
    static_part("lamp", Shade::Glow),
];

const POT: [JointDef; 2] = [
    static_part("body", Shade::Lit),
    static_part("fuse", Shade::Glow),
];

/// Radius of the snare's jaws (how far they reach from the hinge).
pub const SNARE_REACH: f32 = 0.36;

/// Part table of one of this module's models.
pub(crate) fn joints(id: ModelId) -> &'static [JointDef] {
    match id {
        ModelId::Snare => &SNARE,
        ModelId::SootPot => &POT,
        _ => &FLASK,
    }
}

pub(crate) fn build(id: ModelId) -> Vec<ModelPart> {
    match id {
        ModelId::Snare => snare(),
        ModelId::SootPot => soot_pot(),
        _ => flask(),
    }
}

fn finish_lit(p: &mut PartSet) {
    let joints = p.joints();
    p.for_each_mesh(|i, m| {
        if !joints[i].shade.is_emissive() {
            kit::finish(m);
        }
    });
}

fn flask() -> Vec<ModelPart> {
    let mut p = PartSet::new(&FLASK);
    let up = Vec3::Y;
    let ring = |y: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, 0.0),
            Vec3::X,
            Vec3::Z,
            Vec2::splat(r),
            8,
            0.0,
        )
    };
    // The glass: a squat round belly and a short neck.
    p.add(
        1,
        kit::loft(
            &[
                ring(-0.1, 0.05),
                ring(-0.075, 0.1),
                ring(0.0, 0.115),
                ring(0.06, 0.09),
                ring(0.1, 0.04),
                ring(0.15, 0.035),
            ],
            Vec4::new(0.86, 0.95, 0.9, 1.0),
            true,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.color(1, Vec4::new(0.8, 1.0, 0.86, 0.45));
    // The brass cage: hoops round the belly and four ribs; a wax stopper and its seal.
    for (y, r) in [(-0.05, 0.11), (0.03, 0.112)] {
        p.add(
            0,
            kit::band(Vec3::new(0.0, y, 0.0), up, r, 0.016, 0.008, 10, BRASS),
        );
    }
    for k in 0..4 {
        let a = k as f32 / 4.0 * std::f32::consts::TAU + 0.4;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            0,
            limb(
                d * 0.07 + up * -0.1,
                d * 0.118 + up * 0.0,
                Vec2::splat(0.006),
                Vec2::splat(0.006),
                BRASS,
            ),
        );
        p.add(
            0,
            limb(
                d * 0.118,
                d * 0.05 + up * 0.11,
                Vec2::splat(0.006),
                Vec2::splat(0.006),
                BRASS,
            ),
        );
    }
    p.add(
        0,
        tube(
            Vec3::new(0.0, 0.14, 0.0),
            Vec3::new(0.0, 0.2, 0.0),
            0.04,
            0.046,
            8,
            rgb(0xd9c48e),
        ),
    );
    p.add(0, gem(Vec3::new(0.0, 0.205, 0.0), 0.03, 0, rgb(0xa8322a)));
    // The leather sling looped round the neck.
    p.add(
        0,
        kit::band(
            Vec3::new(0.0, 0.12, 0.0),
            up,
            0.05,
            0.02,
            0.01,
            8,
            DARK_LEATHER,
        ),
    );
    p.add(
        0,
        limb(
            Vec3::new(0.04, 0.12, 0.02),
            Vec3::new(0.1, 0.24, 0.06),
            Vec2::new(0.01, 0.004),
            Vec2::new(0.01, 0.004),
            DARK_LEATHER,
        ),
    );
    // Kindling inside: a clump of embers and sparks floating in the belly.
    p.add(
        2,
        gem(
            Vec3::new(0.0, -0.03, 0.0),
            0.06,
            1,
            Vec4::new(1.0, 0.85, 0.45, 1.0),
        ),
    );
    for (x, y, z) in [
        (0.05, 0.01, 0.02),
        (-0.04, 0.03, -0.03),
        (0.01, 0.05, 0.05),
        (-0.05, -0.05, 0.03),
    ] {
        p.add(
            2,
            gem(Vec3::new(x, y, z), 0.018, 0, Vec4::new(1.0, 0.95, 0.7, 1.0)),
        );
    }
    finish_lit(&mut p);
    p.texture(0, kit::tex_plate());
    p.texture(2, kit::tex_glow());
    p.finish()
}

fn snare() -> Vec<ModelPart> {
    let mut p = PartSet::new(&SNARE);
    let wax = Vec4::new(0.86, 0.8, 0.66, 1.0);
    // The poured wax disc: a low dome with a thick rim, drips round the edge.
    let ring = |y: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, 0.0),
            Vec3::X,
            Vec3::Z,
            Vec2::splat(r),
            12,
            0.0,
        )
    };
    p.add(
        SNARE_BASE,
        kit::loft(
            &[
                ring(0.0, 0.44),
                ring(0.035, 0.43),
                ring(0.05, 0.36),
                ring(0.065, 0.12),
                vec![Vec3::new(0.0, 0.07, 0.0)],
            ],
            wax,
            false,
            (true, false),
            kit::Uv::Density(4.0),
        ),
    );
    for k in 0..7 {
        let a = k as f32 / 7.0 * std::f32::consts::TAU + 0.3;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            SNARE_BASE,
            kit::ellipsoid(
                d * 0.44 + Vec3::Y * 0.015,
                Vec3::new(0.035, 0.02, 0.045),
                Quat::from_rotation_y(-a),
                3,
                6,
                wax,
                false,
            ),
        );
    }
    // The brass ring the jaws are hinged on, and the hinge blocks.
    p.add(
        SNARE_BASE,
        kit::band(
            Vec3::new(0.0, 0.05, 0.0),
            Vec3::Y,
            0.075,
            0.02,
            0.012,
            10,
            BRASS,
        ),
    );
    for x in [HINGE_R[0], HINGE_L[0]] {
        p.add(
            SNARE_BASE,
            block(
                Vec3::new(x, 0.05, 0.0),
                Vec3::new(0.014, 0.014, 0.05),
                Quat::IDENTITY,
                IRON,
            ),
        );
    }
    // Jaws: a toothed half hoop each, lying flat outwards (open). The right one is built on
    // +X, the left mirrored.
    let jaw = |side: f32| {
        let mut m = engine::assets::meshgen::MeshBuilder::new();
        let n = 9;
        let hinge = Vec3::new(side * 0.06, 0.05, 0.0);
        let point = |k: usize| {
            let a = k as f32 / (n - 1) as f32 * std::f32::consts::PI - std::f32::consts::FRAC_PI_2;
            hinge
                + Vec3::new(
                    side * a.cos() * SNARE_REACH * 0.85,
                    0.0,
                    a.sin() * SNARE_REACH,
                )
        };
        for k in 0..n - 1 {
            kit::add(
                &mut m,
                limb(
                    point(k),
                    point(k + 1),
                    Vec2::new(0.012, 0.012),
                    Vec2::new(0.012, 0.012),
                    BRASS,
                ),
            );
        }
        // Teeth along the hoop, pointing up (they face each other once the jaws close).
        for k in 1..n - 1 {
            let at = point(k);
            kit::add(
                &mut m,
                kit::spike(at, at + Vec3::Y * 0.07, 0.012, 4, rgb(0xd8c08a)),
            );
        }
        // Arms from the hinge to the hoop's ends.
        for k in [0, n - 1] {
            kit::add(
                &mut m,
                limb(hinge, point(k), Vec2::splat(0.01), Vec2::splat(0.01), IRON),
            );
        }
        m
    };
    p.add(SNARE_JAW_R, jaw(1.0));
    p.add(SNARE_JAW_L, jaw(-1.0));
    // The wick lamp in the middle: a stubby taper with a flame bead.
    p.add(
        SNARE_LAMP,
        tube(
            Vec3::new(0.0, 0.06, 0.0),
            Vec3::new(0.0, 0.1, 0.0),
            0.025,
            0.02,
            6,
            Vec4::new(1.0, 0.8, 0.45, 1.0),
        ),
    );
    p.add(
        SNARE_LAMP,
        kit::spike(
            Vec3::new(0.0, 0.1, 0.0),
            Vec3::new(0.0, 0.17, 0.0),
            0.02,
            5,
            Vec4::new(1.0, 0.95, 0.75, 1.0),
        ),
    );
    finish_lit(&mut p);
    p.texture(SNARE_BASE, kit::tex_wax());
    p.texture(SNARE_JAW_R, kit::tex_plate());
    p.texture(SNARE_JAW_L, kit::tex_plate());
    p.texture(SNARE_LAMP, kit::tex_glow());
    p.finish()
}

fn soot_pot() -> Vec<ModelPart> {
    let mut p = PartSet::new(&POT);
    let clay = rgb(0x2a2420);
    let ring = |y: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, 0.0),
            Vec3::X,
            Vec3::Z,
            Vec2::splat(r),
            9,
            0.0,
        )
    };
    p.add(
        0,
        kit::loft(
            &[
                ring(-0.1, 0.05),
                ring(-0.07, 0.1),
                ring(0.0, 0.12),
                ring(0.06, 0.1),
                ring(0.09, 0.05),
                ring(0.12, 0.045),
            ],
            clay,
            false,
            (true, true),
            kit::Uv::Density(5.0),
        ),
    );
    // A cord net round the belly, the rag stuffed in the neck.
    p.add(
        0,
        kit::band(Vec3::ZERO, Vec3::Y, 0.122, 0.014, 0.008, 9, WRAP),
    );
    p.add(
        0,
        kit::ellipsoid(
            Vec3::new(0.0, 0.135, 0.0),
            Vec3::new(0.05, 0.035, 0.05),
            Quat::from_rotation_z(0.2),
            3,
            6,
            rgb(0x8c7a5c),
            false,
        ),
    );
    p.add(
        0,
        limb(
            Vec3::new(0.0, 0.15, 0.0),
            Vec3::new(0.03, 0.22, 0.01),
            Vec2::splat(0.006),
            Vec2::splat(0.005),
            rgb(0x3a3026),
        ),
    );
    // The fuse's spitting end.
    p.add(
        1,
        gem(
            Vec3::new(0.032, 0.225, 0.01),
            0.022,
            0,
            Vec4::new(1.0, 0.7, 0.3, 1.0),
        ),
    );
    p.add(
        1,
        gem(
            Vec3::new(0.05, 0.25, 0.0),
            0.01,
            0,
            Vec4::new(1.0, 0.9, 0.6, 1.0),
        ),
    );
    finish_lit(&mut p);
    p.texture(0, kit::tex_grime());
    p.texture(1, kit::tex_glow());
    p.finish()
}

/// The snare's pose: `sprung` 0 = jaws lying open, 1 = snapped shut (upright, meeting over
/// the middle); `armed` 0..1 lights the lamp (dull while arming, a slow pulse once armed, a
/// flash on the spring).
pub fn snare_pose(armed: f32, sprung: f32, time: f64) -> Pose {
    let mut pose = Pose::rest(SNARE.len());
    let s = sprung.clamp(0.0, 1.0);
    // Closed is a little past upright so the teeth interlock.
    let close = s * 1.72;
    pose.rotate(SNARE_JAW_R, Quat::from_rotation_z(close));
    pose.rotate(SNARE_JAW_L, Quat::from_rotation_z(-close));
    let pulse = 0.5 + 0.5 * ((time * 3.0).fract() as f32 * std::f32::consts::TAU).sin();
    let glow = if s > 0.0 {
        2.2 * (1.0 - s) + 0.4
    } else if armed >= 1.0 {
        1.0 + 0.6 * pulse
    } else {
        0.25 + 0.35 * armed.clamp(0.0, 1.0)
    };
    pose.glow = Vec4::new(glow, glow, glow, 1.0);
    pose
}
