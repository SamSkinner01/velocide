//! Thresher: chunky side-by-side twin-barrel scattergun with a heavy slotted muzzle brake,
//! a racking pump, a shell saddle on the left flank and hot vents on top.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, CHROME, GUNMETAL, block, chamfer_box_z, limb, rgb, tube,
};
use crate::models::rig::{JointDef, Shade, static_part};

pub const BODY: usize = 0;
pub const PUMP: usize = 1;
pub const GLOW: usize = 2;

pub const JOINTS: [JointDef; 3] = [
    static_part("body", Shade::Lit),
    static_part("pump", Shade::Lit),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, 0.085, -0.64);

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let body = rgb(0x5a5650);
    // Receiver.
    p.add(
        BODY,
        chamfer_box_z(
            0.075,
            0.09,
            -0.23,
            Vec2::new(0.048, 0.058),
            Vec2::new(0.05, 0.058),
            0.018,
            body,
        ),
    );
    // Twin barrels with a clamp band.
    for x in [-0.029, 0.029] {
        p.add(
            BODY,
            tube(
                Vec3::new(x, 0.09, -0.2),
                Vec3::new(x, 0.09, -0.58),
                0.027,
                0.027,
                8,
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        BODY,
        chamfer_box_z(
            0.09,
            -0.43,
            -0.47,
            Vec2::new(0.064, 0.035),
            Vec2::new(0.064, 0.035),
            0.012,
            GUNMETAL,
        ),
    );
    // Muzzle brake: wide slotted block.
    p.add(
        BODY,
        chamfer_box_z(
            0.09,
            -0.57,
            -0.64,
            Vec2::new(0.072, 0.04),
            Vec2::new(0.075, 0.042),
            0.014,
            CHROME,
        ),
    );
    for z in [-0.59, -0.62] {
        p.add(
            BODY,
            block(
                Vec3::new(0.0, 0.09, z),
                Vec3::new(0.077, 0.012, 0.007),
                Quat::IDENTITY,
                BLACK_STEEL,
            ),
        );
    }
    // Barrel bores (dark discs on the brake face).
    for x in [-0.029, 0.029] {
        p.add(
            BODY,
            tube(
                Vec3::new(x, 0.09, -0.635),
                Vec3::new(x, 0.09, -0.645),
                0.02,
                0.02,
                8,
                Vec4::new(0.03, 0.03, 0.03, 1.0),
            ),
        );
    }
    // Stock stub and grip.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.06, 0.09),
            Vec3::new(0.0, 0.035, 0.15),
            Vec2::new(0.036, 0.045),
            Vec2::new(0.032, 0.05),
            GUNMETAL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.025, 0.025),
            Vec3::new(0.0, -0.105, 0.065),
            Vec2::new(0.021, 0.028),
            Vec2::new(0.02, 0.026),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.002, -0.04),
            Vec3::new(0.006, 0.005, 0.04),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    // Shell saddle on the left flank (the side the camera sees): four brass shells.
    p.add(
        BODY,
        block(
            Vec3::new(-0.056, 0.07, -0.08),
            Vec3::new(0.006, 0.03, 0.08),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    for i in 0..4 {
        let z = -0.03 - i as f32 * 0.035;
        p.add(
            BODY,
            tube(
                Vec3::new(-0.058, 0.07, z),
                Vec3::new(-0.08, 0.07, z),
                0.013,
                0.013,
                6,
                rgb(0xc8902c),
            ),
        );
        p.add(
            BODY,
            tube(
                Vec3::new(-0.079, 0.07, z),
                Vec3::new(-0.083, 0.07, z),
                0.014,
                0.014,
                6,
                rgb(0xb02018),
            ),
        );
    }
    // Blood-red heat shield on top.
    p.add(
        BODY,
        chamfer_box_z(
            0.142,
            0.06,
            -0.2,
            Vec2::new(0.03, 0.006),
            Vec2::new(0.03, 0.006),
            0.004,
            kit::BLOOD,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Pump fore-end under the barrels.
    p.add(
        PUMP,
        chamfer_box_z(
            0.04,
            -0.27,
            -0.44,
            Vec2::new(0.045, 0.028),
            Vec2::new(0.045, 0.028),
            0.012,
            rgb(0x6a2018),
        ),
    );
    for z in [-0.3, -0.33, -0.36, -0.39] {
        p.add(
            PUMP,
            block(
                Vec3::new(0.0, 0.012, z),
                Vec3::new(0.046, 0.003, 0.006),
                Quat::IDENTITY,
                BLACK_STEEL,
            ),
        );
    }
    kit::finish(p.mesh(PUMP));

    // Hot vents on the receiver top.
    for x in [-0.03, 0.03] {
        p.add(
            GLOW,
            block(
                Vec3::new(x, 0.137, -0.12),
                Vec3::new(0.007, 0.003, 0.06),
                Quat::IDENTITY,
                Vec4::ONE,
            ),
        );
    }
    p.add(
        GLOW,
        block(
            Vec3::new(-0.053, 0.11, -0.19),
            Vec3::new(0.003, 0.008, 0.02),
            Quat::IDENTITY,
            Vec4::ONE,
        ),
    );

    p.texture(BODY, kit::tex_plate());
    p.texture(PUMP, kit::tex_grip());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}
