//! Quill: slender precision spike-caster (one-handed). An ivory receiver with a long tail
//! over the wrist, a long dark-steel barrel ending in a split pen-nib muzzle, a feather vane
//! under the barrel and a glowing channel along both flanks. A crest of five spare quills
//! stands up along the spine; each folds flat (front to back, a ripple) when it fires and
//! rises again. No cylinder, no hammer: the grip is a straight bone handle with a closed
//! finger loop.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, GUNMETAL, IVORY, add_mirrored, block, limb, spike,
};
use crate::models::rig::{JointDef, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const CHANNEL: usize = 1;
/// First of the five crest quills (`QUILL + i`, front to back).
pub const QUILL: usize = 2;
/// First of the five glowing quill tips (`TIP + i`, children of the quills).
pub const TIP: usize = 7;
pub const CREST_COUNT: usize = 5;

/// Bore height of the barrel.
const BORE_Y: f32 = 0.064;
/// Top of the receiver, where the crest quills are socketed.
const SPINE_Y: f32 = 0.1;

/// Socket position (z) of crest quill `i` along the spine.
const fn quill_z(i: usize) -> f32 {
    -0.1 + i as f32 * 0.036
}

pub const JOINTS: [JointDef; 12] = [
    static_part("body", Shade::Lit),
    static_part("channel", Shade::Glow),
    joint("quill_0", ROOT, [0.0, SPINE_Y, quill_z(0)], Shade::Lit),
    joint("quill_1", ROOT, [0.0, SPINE_Y, quill_z(1)], Shade::Lit),
    joint("quill_2", ROOT, [0.0, SPINE_Y, quill_z(2)], Shade::Lit),
    joint("quill_3", ROOT, [0.0, SPINE_Y, quill_z(3)], Shade::Lit),
    joint("quill_4", ROOT, [0.0, SPINE_Y, quill_z(4)], Shade::Lit),
    joint("tip_0", QUILL, [0.0, SPINE_Y, quill_z(0)], Shade::Glow),
    joint("tip_1", QUILL + 1, [0.0, SPINE_Y, quill_z(1)], Shade::Glow),
    joint("tip_2", QUILL + 2, [0.0, SPINE_Y, quill_z(2)], Shade::Glow),
    joint("tip_3", QUILL + 3, [0.0, SPINE_Y, quill_z(3)], Shade::Glow),
    joint("tip_4", QUILL + 4, [0.0, SPINE_Y, quill_z(4)], Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.7);

/// Resting lean of crest quill `i` (radians back from vertical): the front quill stands
/// upright, the others fan backwards.
pub fn crest_rest_angle(i: usize) -> f32 {
    0.08 + i as f32 * 0.17
}

/// Lean of a folded (flattened) quill: lying back along the spine.
pub const CREST_FLAT_ANGLE: f32 = 1.38;

fn quill_len(i: usize) -> f32 {
    [0.118, 0.13, 0.122, 0.108, 0.092][i.min(4)]
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let ring = |z: f32, y: f32, w: f32, h: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, z),
            Vec3::X,
            Vec3::Y,
            Vec2::new(w, h),
            8,
            std::f32::consts::PI / 8.0,
        )
    };
    // Receiver: long tapered ivory octagon.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(0.07, 0.07, 0.018, 0.026),
                ring(0.02, 0.068, 0.029, 0.04),
                ring(-0.1, 0.066, 0.03, 0.042),
                ring(-0.19, 0.064, 0.022, 0.03),
            ],
            IVORY,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    // Tail: the quill's shaft end, running back over the wrist.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.074, 0.06),
            Vec3::new(0.0, 0.09, 0.15),
            Vec2::new(0.01, 0.014),
            Vec2::new(0.003, 0.004),
            IVORY,
        ),
    );
    // Dark spine strip the crest sockets into.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, SPINE_Y - 0.003, -0.03),
            Vec3::new(0.009, 0.006, 0.1),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    // Long diamond-section barrel.
    let diamond = |z: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, BORE_Y, z),
            Vec3::X,
            Vec3::Y,
            Vec2::splat(r),
            4,
            0.0,
        )
    };
    p.add(
        BODY,
        kit::loft(
            &[diamond(-0.17, 0.024), diamond(-0.58, 0.018)],
            BLACK_STEEL,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    // Brass collars.
    for (z, r) in [(-0.2, 0.026), (-0.38, 0.023), (-0.56, 0.021)] {
        p.add(
            BODY,
            kit::tube(
                Vec3::new(0.0, BORE_Y, z),
                Vec3::new(0.0, BORE_Y, z - 0.018),
                r,
                r,
                8,
                BRASS,
            ),
        );
    }
    // Pen-nib muzzle: a flattened point, split by the glowing slit.
    p.add(
        BODY,
        kit::loft(
            &[
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y, -0.578),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.022, 0.016),
                    6,
                    0.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y - 0.002, -0.64),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.014, 0.008),
                    6,
                    0.0,
                ),
                vec![Vec3::new(0.0, BORE_Y - 0.004, -0.705)],
            ],
            GUNMETAL,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    // Feather vane under the barrel.
    p.add(
        BODY,
        kit::plate(
            &[
                Vec3::new(0.0, BORE_Y - 0.02, -0.2),
                Vec3::new(0.0, BORE_Y - 0.02, -0.5),
                Vec3::new(0.0, BORE_Y - 0.04, -0.44),
                Vec3::new(0.0, BORE_Y - 0.055, -0.3),
                Vec3::new(0.0, BORE_Y - 0.045, -0.21),
            ],
            0.006,
            IVORY,
        ),
    );
    // Straight bone handle wrapped in dark leather, ivory pommel.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.04, 0.025),
            Vec3::new(0.0, -0.11, 0.05),
            Vec2::new(0.019, 0.025),
            Vec2::new(0.018, 0.024),
            DARK_LEATHER,
        ),
    );
    for y in [-0.01, -0.05, -0.09] {
        p.add(
            BODY,
            block(
                Vec3::new(0.0, y, 0.031 + (0.04 - y) * 0.16),
                Vec3::new(0.021, 0.004, 0.027),
                Quat::from_rotation_x(-0.16),
                IVORY,
            ),
        );
    }
    p.add(
        BODY,
        kit::ellipsoid(
            Vec3::new(0.0, -0.125, 0.052),
            Vec3::new(0.025, 0.018, 0.03),
            Quat::IDENTITY,
            2,
            6,
            IVORY,
            false,
        ),
    );
    // Closed finger loop in front of the handle.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.03, -0.08),
            Vec3::new(0.0, -0.035, -0.07),
            Vec2::new(0.006, 0.008),
            Vec2::new(0.006, 0.008),
            BRASS,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, -0.035, -0.07),
            Vec3::new(0.0, -0.05, 0.02),
            Vec2::new(0.006, 0.008),
            Vec2::new(0.006, 0.008),
            BRASS,
        ),
    );
    // Trigger lever.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.01, -0.035),
            Vec3::new(0.004, 0.018, 0.004),
            Quat::from_rotation_x(0.3),
            BLACK_STEEL,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Glowing channel along both upper flanks of the barrel and the nib slit.
    let side = block(
        Vec3::new(0.0125, BORE_Y + 0.0125, -0.38),
        Vec3::new(0.0035, 0.0035, 0.17),
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
        Vec4::ONE,
    );
    add_mirrored(p.mesh(CHANNEL), &side);
    p.add(
        CHANNEL,
        block(
            Vec3::new(0.0, BORE_Y + 0.009, -0.645),
            Vec3::new(0.0025, 0.004, 0.045),
            Quat::from_rotation_x(0.15),
            Vec4::ONE,
        ),
    );
    p.add(
        CHANNEL,
        kit::gem(Vec3::new(0.0, BORE_Y + 0.012, -0.598), 0.007, 0, Vec4::ONE),
    );
    // Small eye on each side of the receiver.
    let eye = kit::gem(Vec3::new(0.029, 0.072, -0.06), 0.008, 0, Vec4::ONE);
    add_mirrored(p.mesh(CHANNEL), &eye);

    // Crest quills: shaft, vane and a glowing tip, built upright then leaned back.
    for i in 0..CREST_COUNT {
        let base = Vec3::new(0.0, SPINE_Y, quill_z(i));
        let lean = Quat::from_rotation_x(crest_rest_angle(i));
        let len = quill_len(i);
        let at = |y: f32, z: f32| base + lean * Vec3::new(0.0, y, z);
        let q = QUILL + i;
        p.add(
            q,
            limb(
                at(-0.004, 0.0),
                at(len, 0.0),
                Vec2::new(0.0045, 0.0045),
                Vec2::new(0.0025, 0.0025),
                GUNMETAL,
            ),
        );
        p.add(
            q,
            kit::plate(
                &[
                    at(len * 0.22, 0.0),
                    at(len * 0.95, 0.0),
                    at(len * 0.8, 0.017),
                    at(len * 0.3, 0.013),
                ],
                0.004,
                IVORY,
            ),
        );
        // Dark band across the vane.
        p.add(
            q,
            kit::plate(
                &[
                    at(len * 0.55, 0.0),
                    at(len * 0.68, 0.0),
                    at(len * 0.68, 0.016),
                    at(len * 0.55, 0.016),
                ],
                0.0055,
                DARK_LEATHER,
            ),
        );
        kit::finish(p.mesh(q));
        p.add(
            TIP + i,
            spike(at(len - 0.004, 0.0), at(len + 0.04, 0.0), 0.0075, 4, Vec4::ONE),
        );
    }

    p.texture(BODY, kit::tex_bone());
    p.texture(CHANNEL, kit::tex_glow());
    for i in 0..CREST_COUNT {
        p.texture(QUILL + i, kit::tex_bone());
        p.texture(TIP + i, kit::tex_glow());
    }
    p.finish()
}
