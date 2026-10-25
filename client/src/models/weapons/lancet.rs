//! Lancet: sleek needle-barrelled hitscan sidearm with a dorsal fin and an energy cell along
//! both flanks (glows / shimmers during Overcharge).

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{self, BLACK_STEEL, CHROME, GUNMETAL, add_mirrored, block, limb, tube};
use crate::models::rig::{JointDef, Shade, static_part};

pub const BODY: usize = 0;
pub const CELL: usize = 1;

pub const JOINTS: [JointDef; 2] = [
    static_part("body", Shade::Lit),
    static_part("cell", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, 0.058, -0.43);

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
    // Receiver: long tapered octagon.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(0.075, 0.056, 0.02, 0.026),
                ring(0.03, 0.056, 0.027, 0.042),
                ring(-0.15, 0.058, 0.025, 0.04),
                ring(-0.24, 0.06, 0.017, 0.026),
            ],
            CHROME,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    // Dark top rail and rear dorsal fin.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.099, -0.08),
            Vec3::new(0.008, 0.005, 0.12),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        kit::plate(
            &[
                Vec3::new(0.0, 0.092, 0.0),
                Vec3::new(0.0, 0.122, 0.06),
                Vec3::new(0.0, 0.088, 0.066),
            ],
            0.008,
            kit::BLOOD,
        ),
    );
    // Needle barrel, shroud and muzzle crown.
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, 0.058, -0.22),
            Vec3::new(0.0, 0.058, -0.41),
            0.011,
            0.011,
            6,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, 0.058, -0.235),
            Vec3::new(0.0, 0.058, -0.255),
            0.02,
            0.02,
            8,
            GUNMETAL,
        ),
    );
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, 0.058, -0.405),
            Vec3::new(0.0, 0.058, -0.43),
            0.016,
            0.013,
            8,
            CHROME,
        ),
    );
    // Twin under-prongs.
    let prong = limb(
        Vec3::new(0.012, 0.036, -0.2),
        Vec3::new(0.009, 0.044, -0.37),
        Vec2::new(0.004, 0.005),
        Vec2::new(0.003, 0.003),
        GUNMETAL,
    );
    add_mirrored(p.mesh(BODY), &prong);
    // Raked grip and trigger guard.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.03, 0.03),
            Vec3::new(0.0, -0.105, 0.068),
            Vec2::new(0.019, 0.026),
            Vec2::new(0.017, 0.024),
            GUNMETAL,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, -0.002, -0.035),
            Vec3::new(0.005, 0.004, 0.035),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.012, -0.068),
            Vec3::new(0.005, 0.016, 0.004),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Energy cell strips on both flanks + coil rings around the barrel.
    let strip = block(
        Vec3::new(0.0265, 0.058, -0.075),
        Vec3::new(0.003, 0.011, 0.085),
        Quat::IDENTITY,
        Vec4::ONE,
    );
    add_mirrored(p.mesh(CELL), &strip);
    for z in [-0.28, -0.32, -0.36] {
        p.add(
            CELL,
            kit::band(
                Vec3::new(0.0, 0.058, z),
                Vec3::Z,
                0.015,
                0.008,
                0.006,
                8,
                Vec4::ONE,
            ),
        );
    }

    p.texture(BODY, kit::tex_chrome());
    p.texture(CELL, kit::tex_glow());
    p.finish()
}
