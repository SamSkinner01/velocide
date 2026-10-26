//! First-person left fist (punch / parry, Thorn throw): knuckles at the origin pointing -Z,
//! forearm running back along +Z. A gloved fist with brass knuckle studs and a wrapped
//! forearm under a brass bracer; the `knuckles` embers flare in the wick's (team) colour on
//! the punch.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{self, BRASS, DARK_LEATHER, LEATHER, WRAP, block, gem, limb};
use crate::models::rig::{JointDef, Shade, static_part};

pub const FOREARM: usize = 0;
pub const FIST: usize = 1;
pub const TRIM: usize = 2;
pub const KNUCKLES: usize = 3;

pub const JOINTS: [JointDef; 4] = [
    static_part("forearm", Shade::Lit),
    static_part("fist", Shade::Lit),
    static_part("trim", Shade::Team),
    static_part("knuckles", Shade::TeamGlow),
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    // Wrapped forearm with a wrist joint, a brass bracer and a leather cuff.
    p.add(
        FOREARM,
        gem(Vec3::new(0.0, -0.005, 0.1), 0.034, 0, DARK_LEATHER),
    );
    p.add(
        FOREARM,
        limb(
            Vec3::new(0.0, -0.008, 0.11),
            Vec3::new(0.0, -0.03, 0.46),
            Vec2::new(0.04, 0.036),
            Vec2::new(0.056, 0.05),
            WRAP,
        ),
    );
    p.add(
        FOREARM,
        kit::chamfer_box_z(
            -0.015,
            0.33,
            0.17,
            Vec2::new(0.056, 0.05),
            Vec2::new(0.05, 0.045),
            0.014,
            BRASS,
        ),
    );
    for z in [0.2, 0.25, 0.3] {
        p.add(
            FOREARM,
            gem(Vec3::new(0.0, 0.035, z), 0.007, 0, DARK_LEATHER),
        );
    }
    p.add(
        FOREARM,
        kit::chamfer_box_z(
            -0.01,
            0.135,
            0.09,
            Vec2::new(0.046, 0.042),
            Vec2::new(0.054, 0.05),
            0.012,
            LEATHER,
        ),
    );
    kit::finish(p.mesh(FOREARM));

    // Gloved fist, curled fingers and the thumb on the inside (+X for the left hand).
    p.add(
        FIST,
        block(
            Vec3::new(0.0, 0.0, 0.045),
            Vec3::new(0.049, 0.041, 0.049),
            Quat::IDENTITY,
            DARK_LEATHER,
        ),
    );
    for i in 0..4 {
        let x = -0.03 + i as f32 * 0.02;
        p.add(
            FIST,
            block(
                Vec3::new(x, -0.012, 0.004),
                Vec3::new(0.0098, 0.025, 0.013),
                Quat::IDENTITY,
                DARK_LEATHER,
            ),
        );
    }
    p.add(
        FIST,
        block(
            Vec3::new(0.038, -0.022, 0.02),
            Vec3::new(0.013, 0.013, 0.03),
            Quat::from_rotation_y(-0.4),
            DARK_LEATHER,
        ),
    );
    // Brass knuckle plate (the embers sit in it).
    p.add(
        FIST,
        block(
            Vec3::new(0.0, 0.03, 0.006),
            Vec3::new(0.048, 0.01, 0.014),
            Quat::from_rotation_x(0.2),
            BRASS,
        ),
    );
    kit::finish(p.mesh(FIST));

    // Team-coloured cord round the bracer and a knot on the back of the hand.
    for z in [0.17, 0.33] {
        p.add(
            TRIM,
            kit::chamfer_box_z(
                -0.015,
                z + 0.007,
                z - 0.007,
                Vec2::new(0.06, 0.054),
                Vec2::new(0.06, 0.054),
                0.014,
                kit::WHITE,
            ),
        );
    }
    p.add(TRIM, gem(Vec3::new(0.0, 0.045, 0.06), 0.013, 0, kit::WHITE));
    kit::finish(p.mesh(TRIM));

    // Ember studs in the knuckle plate.
    for i in 0..4 {
        let x = -0.03 + i as f32 * 0.02;
        p.add(
            KNUCKLES,
            block(
                Vec3::new(x, 0.04, -0.004),
                Vec3::new(0.0085, 0.011, 0.008),
                Quat::IDENTITY,
                Vec4::new(1.0, 0.86, 0.66, 1.0),
            ),
        );
    }

    p.texture(FOREARM, kit::tex_wrap());
    p.texture(FIST, kit::tex_leather());
    p.texture(TRIM, kit::tex_leather());
    p.texture(KNUCKLES, kit::tex_glow());
    p.finish()
}
