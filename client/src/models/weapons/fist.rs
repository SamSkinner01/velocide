//! First-person left fist (punch / parry, prism throw): knuckles at the origin pointing -Z,
//! forearm running back along +Z. Glowing knuckle studs flare on the punch.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{self, BLACK_STEEL, CHROME, GUNMETAL, block, gem, limb};
use crate::models::rig::{JointDef, Shade, static_part};

pub const FOREARM: usize = 0;
pub const FIST: usize = 1;
pub const TRIM: usize = 2;
pub const KNUCKLES: usize = 3;

pub const JOINTS: [JointDef; 4] = [
    static_part("forearm", Shade::Lit),
    static_part("fist", Shade::Lit),
    static_part("trim", Shade::Team),
    static_part("knuckles", Shade::Glow),
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    // Forearm (chrome) with a wrist joint.
    p.add(
        FOREARM,
        gem(Vec3::new(0.0, -0.005, 0.1), 0.034, 0, GUNMETAL),
    );
    p.add(
        FOREARM,
        limb(
            Vec3::new(0.0, -0.008, 0.11),
            Vec3::new(0.0, -0.03, 0.46),
            Vec2::new(0.04, 0.036),
            Vec2::new(0.056, 0.05),
            CHROME,
        ),
    );
    for (z, r) in [(0.2, 0.05), (0.32, 0.056)] {
        p.add(
            FOREARM,
            kit::chamfer_box_z(
                -0.015,
                z,
                z - 0.03,
                Vec2::new(r, r * 0.9),
                Vec2::new(r, r * 0.9),
                0.012,
                BLACK_STEEL,
            ),
        );
    }
    kit::finish(p.mesh(FOREARM));

    // Fist block, curled fingers and the thumb on the inside (+X for the left hand).
    p.add(
        FIST,
        block(
            Vec3::new(0.0, 0.0, 0.045),
            Vec3::new(0.048, 0.04, 0.048),
            Quat::IDENTITY,
            GUNMETAL,
        ),
    );
    for i in 0..4 {
        let x = -0.03 + i as f32 * 0.02;
        p.add(
            FIST,
            block(
                Vec3::new(x, -0.012, 0.004),
                Vec3::new(0.0095, 0.024, 0.012),
                Quat::IDENTITY,
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        FIST,
        block(
            Vec3::new(0.038, -0.022, 0.02),
            Vec3::new(0.012, 0.012, 0.03),
            Quat::from_rotation_y(-0.4),
            GUNMETAL,
        ),
    );
    kit::finish(p.mesh(FIST));

    // Team-coloured knuckle plate and forearm guard.
    p.add(
        TRIM,
        block(
            Vec3::new(0.0, 0.03, 0.02),
            Vec3::new(0.046, 0.012, 0.03),
            Quat::from_rotation_x(0.25),
            kit::WHITE,
        ),
    );
    p.add(
        TRIM,
        limb(
            Vec3::new(0.0, 0.03, 0.17),
            Vec3::new(0.0, 0.026, 0.36),
            Vec2::new(0.016, 0.006),
            Vec2::new(0.022, 0.007),
            kit::WHITE,
        ),
    );
    kit::finish(p.mesh(TRIM));

    // Knuckle studs.
    for i in 0..4 {
        let x = -0.03 + i as f32 * 0.02;
        p.add(
            KNUCKLES,
            block(
                Vec3::new(x, 0.028, -0.006),
                Vec3::new(0.0065, 0.008, 0.006),
                Quat::IDENTITY,
                Vec4::new(1.0, 0.85, 0.7, 1.0),
            ),
        );
    }

    p.texture(FOREARM, kit::tex_chrome());
    p.texture(FIST, kit::tex_plate());
    p.texture(TRIM, kit::tex_chrome());
    p.texture(KNUCKLES, kit::tex_glow());
    p.finish()
}
