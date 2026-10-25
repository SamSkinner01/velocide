//! Cinderbox: boxy explosive launcher with a glowing core behind grilled side windows, a
//! glowing square maw, a hazard band and a carry handle.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, GUNMETAL, add_mirrored, block, chamfer_box_z, limb, rgb,
};
use crate::models::rig::{JointDef, Shade, static_part};

pub const BODY: usize = 0;
pub const STRIPE: usize = 1;
pub const CORE: usize = 2;

pub const JOINTS: [JointDef; 3] = [
    static_part("body", Shade::Lit),
    static_part("stripe", Shade::Lit),
    static_part("core", Shade::Glow),
];

const Y: f32 = 0.1;
const HALF: f32 = 0.07;

pub const MUZZLE: Vec3 = Vec3::new(0.0, Y, -0.6);

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let olive = rgb(0x4e4a3e);
    // Main box, heavier rear block and the muzzle lip.
    p.add(
        BODY,
        chamfer_box_z(
            Y,
            0.1,
            -0.5,
            Vec2::splat(HALF),
            Vec2::splat(HALF),
            0.018,
            olive,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            Y,
            0.13,
            0.06,
            Vec2::splat(HALF - 0.004),
            Vec2::splat(HALF + 0.008),
            0.024,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            Y,
            -0.5,
            -0.6,
            Vec2::splat(HALF + 0.012),
            Vec2::splat(HALF + 0.016),
            0.02,
            BLACK_STEEL,
        ),
    );
    // Frame ribs.
    for z in [-0.06, -0.3] {
        p.add(
            BODY,
            chamfer_box_z(
                Y,
                z,
                z - 0.025,
                Vec2::splat(HALF + 0.006),
                Vec2::splat(HALF + 0.006),
                0.02,
                GUNMETAL,
            ),
        );
    }
    // Window grille bars over the core panels (both flanks).
    let mut bars = engine::assets::meshgen::MeshBuilder::new();
    for z in [-0.12, -0.165, -0.21, -0.255] {
        kit::add(
            &mut bars,
            block(
                Vec3::new(HALF + 0.006, Y, z),
                Vec3::new(0.004, 0.036, 0.005),
                Quat::IDENTITY,
                BLACK_STEEL,
            ),
        );
    }
    kit::add(
        &mut bars,
        block(
            Vec3::new(HALF + 0.005, Y + 0.04, -0.19),
            Vec3::new(0.005, 0.005, 0.1),
            Quat::IDENTITY,
            GUNMETAL,
        ),
    );
    kit::add(
        &mut bars,
        block(
            Vec3::new(HALF + 0.005, Y - 0.04, -0.19),
            Vec3::new(0.005, 0.005, 0.1),
            Quat::IDENTITY,
            GUNMETAL,
        ),
    );
    add_mirrored(p.mesh(BODY), &bars);
    // Carry handle.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, Y + HALF, 0.03),
            Vec3::new(0.0, Y + HALF + 0.05, 0.02),
            Vec2::new(0.01, 0.012),
            Vec2::new(0.01, 0.012),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, Y + HALF, -0.2),
            Vec3::new(0.0, Y + HALF + 0.05, -0.18),
            Vec2::new(0.01, 0.012),
            Vec2::new(0.01, 0.012),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, Y + HALF + 0.05, 0.03),
            Vec3::new(0.0, Y + HALF + 0.05, -0.19),
            Vec2::new(0.012, 0.011),
            Vec2::new(0.012, 0.011),
            GUNMETAL,
        ),
    );
    // Rear exhaust fins.
    for y in [-0.03, 0.0, 0.03] {
        p.add(
            BODY,
            block(
                Vec3::new(0.0, Y + y, 0.14),
                Vec3::new(0.05, 0.005, 0.012),
                Quat::IDENTITY,
                GUNMETAL,
            ),
        );
    }
    // Grip and trigger guard.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.035, 0.0),
            Vec3::new(0.0, -0.1, 0.045),
            Vec2::new(0.022, 0.028),
            Vec2::new(0.02, 0.026),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.012, -0.045),
            Vec3::new(0.006, 0.005, 0.04),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    // Fore grip under the box.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, Y - HALF, -0.33),
            Vec3::new(0.0, Y - HALF - 0.08, -0.31),
            Vec2::new(0.018, 0.022),
            Vec2::new(0.016, 0.02),
            BLACK_STEEL,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Hazard band near the muzzle.
    p.add(
        STRIPE,
        chamfer_box_z(
            Y,
            -0.4,
            -0.46,
            Vec2::splat(HALF + 0.004),
            Vec2::splat(HALF + 0.004),
            0.02,
            Vec4::ONE,
        ),
    );
    kit::finish(p.mesh(STRIPE));

    // Core: glowing side windows and the inner face of the maw.
    let panel = block(
        Vec3::new(HALF + 0.0015, Y, -0.19),
        Vec3::new(0.002, 0.036, 0.085),
        Quat::IDENTITY,
        Vec4::ONE,
    );
    add_mirrored(p.mesh(CORE), &panel);
    p.add(
        CORE,
        block(
            Vec3::new(0.0, Y, -0.6),
            Vec3::new(HALF - 0.012, HALF - 0.012, 0.004),
            Quat::IDENTITY,
            Vec4::ONE,
        ),
    );

    p.texture(BODY, kit::tex_plate());
    p.texture(STRIPE, kit::tex_hazard());
    p.texture(CORE, kit::tex_glow());
    p.finish()
}
