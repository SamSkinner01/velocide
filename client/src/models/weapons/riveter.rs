//! Riveter: industrial yellow nailer with a rotating five-barrel cluster, a side drum
//! magazine of rivets and a heat coil that glows while firing.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, CHROME, GUNMETAL, block, chamfer_box_z, limb, rgb, tube,
};
use crate::models::rig::{JointDef, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const BARRELS: usize = 1;
pub const DRUM: usize = 2;
pub const GLOW: usize = 3;

const BORE_Y: f32 = 0.078;
const DRUM_CENTER: [f32; 3] = [-0.075, 0.05, -0.02];

pub const JOINTS: [JointDef; 4] = [
    static_part("body", Shade::Lit),
    joint("barrels", ROOT, [0.0, BORE_Y, -0.2], Shade::Lit),
    joint("drum", ROOT, DRUM_CENTER, Shade::Lit),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.53);

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let yellow = rgb(0xd8a21c);
    // Receiver (yellow), black top rail and carry handle.
    p.add(
        BODY,
        chamfer_box_z(
            0.07,
            0.1,
            -0.2,
            Vec2::new(0.046, 0.06),
            Vec2::new(0.044, 0.056),
            0.014,
            yellow,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.135, -0.05),
            Vec3::new(0.012, 0.006, 0.13),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.13, 0.04),
            Vec3::new(0.0, 0.17, 0.0),
            Vec2::new(0.008, 0.01),
            Vec2::new(0.008, 0.01),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.13, -0.13),
            Vec3::new(0.0, 0.17, -0.1),
            Vec2::new(0.008, 0.01),
            Vec2::new(0.008, 0.01),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.17, 0.005),
            Vec3::new(0.0, 0.17, -0.105),
            Vec2::new(0.009, 0.009),
            Vec2::new(0.009, 0.009),
            BLACK_STEEL,
        ),
    );
    // Hazard-striped nose collar holding the barrel cluster.
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y, -0.19),
            Vec3::new(0.0, BORE_Y, -0.26),
            0.05,
            0.047,
            8,
            GUNMETAL,
        ),
    );
    // Rear vent block and grip.
    p.add(
        BODY,
        chamfer_box_z(
            0.07,
            0.13,
            0.1,
            Vec2::new(0.04, 0.05),
            Vec2::new(0.042, 0.054),
            0.01,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.02, 0.03),
            Vec3::new(0.0, -0.105, 0.07),
            Vec2::new(0.02, 0.027),
            Vec2::new(0.019, 0.025),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.0, -0.04),
            Vec3::new(0.006, 0.005, 0.04),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    // Drum axle housing on the left flank.
    let d = Vec3::new(DRUM_CENTER[0], DRUM_CENTER[1], DRUM_CENTER[2]);
    p.add(
        BODY,
        tube(
            d + Vec3::new(0.03, 0.0, 0.0),
            d + Vec3::new(-0.04, 0.0, 0.0),
            0.016,
            0.016,
            6,
            BLACK_STEEL,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Rotating barrel cluster: five barrels between two plates.
    let c = Vec3::new(0.0, BORE_Y, 0.0);
    for k in 0..5 {
        let a = k as f32 / 5.0 * std::f32::consts::TAU;
        let off = Vec3::new(a.cos(), a.sin(), 0.0) * 0.026;
        p.add(
            BARRELS,
            tube(
                c + off + Vec3::new(0.0, 0.0, -0.24),
                c + off + Vec3::new(0.0, 0.0, -0.53),
                0.0105,
                0.0105,
                6,
                BLACK_STEEL,
            ),
        );
        // Rivet tips peeking out of the barrels.
        p.add(
            BARRELS,
            tube(
                c + off + Vec3::new(0.0, 0.0, -0.53),
                c + off + Vec3::new(0.0, 0.0, -0.535),
                0.006,
                0.006,
                4,
                CHROME,
            ),
        );
    }
    p.add(
        BARRELS,
        kit::tube(
            c + Vec3::new(0.0, 0.0, -0.33),
            c + Vec3::new(0.0, 0.0, -0.35),
            0.044,
            0.044,
            5,
            CHROME,
        ),
    );
    p.add(
        BARRELS,
        kit::tube(
            c + Vec3::new(0.0, 0.0, -0.49),
            c + Vec3::new(0.0, 0.0, -0.51),
            0.042,
            0.042,
            5,
            GUNMETAL,
        ),
    );
    p.add(
        BARRELS,
        kit::tube(
            c + Vec3::new(0.0, 0.0, -0.24),
            c + Vec3::new(0.0, 0.0, -0.53),
            0.008,
            0.008,
            5,
            GUNMETAL,
        ),
    );
    kit::finish(p.mesh(BARRELS));

    // Side drum magazine with rivet heads around the rim.
    p.add(
        DRUM,
        tube(
            d + Vec3::new(-0.005, 0.0, 0.0),
            d + Vec3::new(-0.045, 0.0, 0.0),
            0.062,
            0.062,
            10,
            yellow,
        ),
    );
    p.add(
        DRUM,
        tube(
            d + Vec3::new(-0.044, 0.0, 0.0),
            d + Vec3::new(-0.05, 0.0, 0.0),
            0.035,
            0.035,
            8,
            BLACK_STEEL,
        ),
    );
    for k in 0..10 {
        let a = k as f32 / 10.0 * std::f32::consts::TAU;
        let off = Vec3::new(0.0, a.cos(), a.sin()) * 0.05;
        p.add(
            DRUM,
            tube(
                d + off + Vec3::new(-0.044, 0.0, 0.0),
                d + off + Vec3::new(-0.05, 0.0, 0.0),
                0.007,
                0.007,
                4,
                CHROME,
            ),
        );
    }
    kit::finish(p.mesh(DRUM));

    // Heat coil around the nose collar.
    for z in [-0.205, -0.225, -0.245] {
        p.add(
            GLOW,
            kit::band(
                Vec3::new(0.0, BORE_Y, z),
                Vec3::Z,
                0.051,
                0.007,
                0.005,
                8,
                Vec4::ONE,
            ),
        );
    }

    p.texture(BODY, kit::tex_plate());
    p.texture(BARRELS, kit::tex_chrome());
    p.texture(DRUM, kit::tex_plate());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}
