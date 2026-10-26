//! First-person right arm: a cloth-wrapped forearm with a brass bracer and a team-coloured
//! cord tied round it, and a gloved hand closed around the weapon grip (grip at the origin,
//! so it fits every weapon).

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3};

use crate::models::PartSet;
use crate::models::kit::{self, BRASS, DARK_LEATHER, LEATHER, WRAP, block, gem, limb};
use crate::models::rig::{JointDef, Shade, static_part};

pub const FOREARM: usize = 0;
pub const HAND: usize = 1;
pub const TRIM: usize = 2;
pub const BRACER: usize = 3;

pub const JOINTS: [JointDef; 4] = [
    static_part("forearm", Shade::Lit),
    static_part("hand", Shade::Lit),
    static_part("trim", Shade::Team),
    static_part("bracer", Shade::Lit),
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wrist = Vec3::new(0.004, -0.07, 0.05);
    let elbow = Vec3::new(0.07, -0.2, 0.42);
    let dir = (elbow - wrist).normalize();
    // Wrapped forearm.
    p.add(FOREARM, gem(wrist, 0.03, 0, DARK_LEATHER));
    p.add(
        FOREARM,
        limb(
            wrist + Vec3::new(0.0, -0.005, 0.015),
            elbow,
            Vec2::new(0.038, 0.042),
            Vec2::new(0.054, 0.06),
            WRAP,
        ),
    );
    kit::finish(p.mesh(FOREARM));

    // Gloved palm wrapped around the grip, four finger segments in front, thumb over the top,
    // and a flared leather cuff.
    p.add(
        HAND,
        block(
            Vec3::new(0.004, -0.048, 0.03),
            Vec3::new(0.031, 0.05, 0.031),
            Quat::from_rotation_x(-0.3),
            DARK_LEATHER,
        ),
    );
    for (i, y) in [-0.012, -0.036, -0.06, -0.083].iter().enumerate() {
        let z = -0.022 + *y * 0.32;
        p.add(
            HAND,
            block(
                Vec3::new(0.0, *y, z),
                Vec3::new(0.028 - i as f32 * 0.002, 0.011, 0.017),
                Quat::from_rotation_x(-0.3),
                DARK_LEATHER,
            ),
        );
    }
    p.add(
        HAND,
        block(
            Vec3::new(-0.027, 0.008, 0.0),
            Vec3::new(0.01, 0.011, 0.029),
            Quat::from_rotation_y(0.3),
            DARK_LEATHER,
        ),
    );
    p.add(
        HAND,
        limb(
            wrist + dir * 0.01,
            wrist + dir * 0.05,
            Vec2::new(0.042, 0.046),
            Vec2::new(0.05, 0.054),
            LEATHER,
        ),
    );
    kit::finish(p.mesh(HAND));

    // Brass bracer over the middle of the forearm, rivets along its top edge.
    p.add(
        BRACER,
        limb(
            wrist + dir * 0.08,
            wrist + dir * 0.24,
            Vec2::new(0.046, 0.05),
            Vec2::new(0.054, 0.058),
            BRASS,
        ),
    );
    let up = Vec3::new(-0.35, 1.0, 0.0).normalize();
    for k in 0..3 {
        let at = wrist + dir * (0.11 + k as f32 * 0.05) + up * 0.05;
        p.add(BRACER, gem(at, 0.007, 0, DARK_LEATHER));
    }
    kit::finish(p.mesh(BRACER));

    // Team-coloured cord tied round the bracer's ends and a knot on top.
    for d in [0.085, 0.235] {
        p.add(
            TRIM,
            limb(
                wrist + dir * (d - 0.008),
                wrist + dir * (d + 0.008),
                Vec2::new(0.05 + d * 0.04, 0.054 + d * 0.04),
                Vec2::new(0.05 + d * 0.04, 0.054 + d * 0.04),
                kit::WHITE,
            ),
        );
    }
    p.add(
        TRIM,
        gem(wrist + dir * 0.085 + up * 0.055, 0.012, 0, kit::WHITE),
    );
    kit::finish(p.mesh(TRIM));

    p.texture(FOREARM, kit::tex_wrap());
    p.texture(HAND, kit::tex_leather());
    p.texture(TRIM, kit::tex_leather());
    p.texture(BRACER, kit::tex_plate());
    p.finish()
}
