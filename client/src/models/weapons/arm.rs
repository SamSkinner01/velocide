//! First-person right arm: chrome forearm with a team-coloured guard and a hand closed
//! around the weapon grip (grip at the origin, so it fits every weapon).

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3};

use crate::models::PartSet;
use crate::models::kit::{self, BLACK_STEEL, CHROME, GUNMETAL, block, gem, limb};
use crate::models::rig::{JointDef, Shade, static_part};

pub const FOREARM: usize = 0;
pub const HAND: usize = 1;
pub const TRIM: usize = 2;

pub const JOINTS: [JointDef; 3] = [
    static_part("forearm", Shade::Lit),
    static_part("hand", Shade::Lit),
    static_part("trim", Shade::Team),
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wrist = Vec3::new(0.004, -0.07, 0.05);
    let elbow = Vec3::new(0.07, -0.2, 0.42);
    p.add(FOREARM, gem(wrist, 0.03, 0, GUNMETAL));
    p.add(
        FOREARM,
        limb(
            wrist + Vec3::new(0.0, -0.005, 0.015),
            elbow,
            Vec2::new(0.036, 0.04),
            Vec2::new(0.052, 0.058),
            CHROME,
        ),
    );
    kit::finish(p.mesh(FOREARM));

    // Palm wrapped around the grip, four finger segments in front, thumb over the top.
    p.add(
        HAND,
        block(
            Vec3::new(0.004, -0.048, 0.03),
            Vec3::new(0.03, 0.05, 0.03),
            Quat::from_rotation_x(-0.3),
            GUNMETAL,
        ),
    );
    for (i, y) in [-0.012, -0.036, -0.06, -0.083].iter().enumerate() {
        let z = -0.022 + *y * 0.32;
        p.add(
            HAND,
            block(
                Vec3::new(0.0, *y, z),
                Vec3::new(0.027 - i as f32 * 0.002, 0.0105, 0.016),
                Quat::from_rotation_x(-0.3),
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        HAND,
        block(
            Vec3::new(-0.026, 0.008, 0.0),
            Vec3::new(0.009, 0.01, 0.028),
            Quat::from_rotation_y(0.3),
            GUNMETAL,
        ),
    );
    kit::finish(p.mesh(HAND));

    // Team-coloured guard plate on top of the forearm and a knuckle bar.
    let dir = (elbow - wrist).normalize();
    let up = Vec3::new(-0.35, 1.0, 0.0).normalize();
    p.add(
        TRIM,
        kit::limb_oriented(
            wrist + dir * 0.06 + up * 0.035,
            wrist + dir * 0.3 + up * 0.05,
            Vec3::X,
            Vec2::new(0.03, 0.01),
            Vec2::new(0.042, 0.012),
            kit::WHITE,
        ),
    );
    p.add(
        TRIM,
        block(
            Vec3::new(0.0, -0.048, -0.04),
            Vec3::new(0.031, 0.045, 0.006),
            Quat::from_rotation_x(-0.3),
            kit::WHITE,
        ),
    );
    kit::finish(p.mesh(TRIM));

    p.texture(FOREARM, kit::tex_chrome());
    p.texture(HAND, kit::tex_plate());
    p.texture(TRIM, kit::tex_chrome());
    p.finish()
}
