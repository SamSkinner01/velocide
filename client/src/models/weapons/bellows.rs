//! Bellows: twin-barrel blast cannon in brass and leather. Two short, wide side-by-side
//! barrels with flared bell muzzles over a fireside-bellows chamber: a fixed upper board, a
//! lower board hinged at the front and pleated leather between them. Between shots the lower
//! board swings up and the pleats squash (the pump), then it refills. A glowing pressure
//! gauge on the left flank and the barrels' throats show the mode colour.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, GUNMETAL, LEATHER, add_mirrored, block, chamfer_box_z,
    limb, rgb, tube,
};
use crate::models::rig::{JointDef, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const BOARD: usize = 1;
pub const PLEATS: usize = 2;
pub const GLOW: usize = 3;

const BORE_Y: f32 = 0.1;
const BARREL_X: f32 = 0.036;
const BARREL_R: f32 = 0.034;
/// Front hinge of the bellows boards.
pub const HINGE: [f32; 3] = [0.0, 0.036, -0.41];
/// Opening angle of the lower board at rest (radians).
pub const OPEN_ANGLE: f32 = 0.34;
/// Back end of the bellows (z).
const BACK_Z: f32 = -0.1;

pub const JOINTS: [JointDef; 4] = [
    static_part("body", Shade::Lit),
    joint("board", ROOT, HINGE, Shade::Lit),
    joint("pleats", ROOT, HINGE, Shade::Lit),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.52);

fn hinge() -> Vec3 {
    Vec3::new(HINGE[0], HINGE[1], HINGE[2])
}

/// Height of the lower board's top face at `z` (bind pose).
fn board_y(z: f32) -> f32 {
    hinge().y - (z - hinge().z) * OPEN_ANGLE.tan()
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wood = rgb(0x4a2e1e);
    // Breech block and the top rib.
    p.add(
        BODY,
        chamfer_box_z(
            0.092,
            0.05,
            -0.11,
            Vec2::new(0.068, 0.048),
            Vec2::new(0.075, 0.052),
            0.02,
            BRASS,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, BORE_Y + BARREL_R + 0.004, -0.27),
            Vec3::new(0.012, 0.005, 0.16),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    // Barrels: short, wide, banded, with flared bells.
    let mut barrel = tube(
        Vec3::new(BARREL_X, BORE_Y, -0.1),
        Vec3::new(BARREL_X, BORE_Y, -0.43),
        BARREL_R,
        BARREL_R * 0.95,
        8,
        BRASS,
    );
    kit::add(
        &mut barrel,
        tube(
            Vec3::new(BARREL_X, BORE_Y, -0.43),
            Vec3::new(BARREL_X, BORE_Y, -0.515),
            BARREL_R * 0.95,
            BARREL_R * 1.4,
            8,
            BRASS,
        ),
    );
    for z in [-0.14, -0.3] {
        kit::add(
            &mut barrel,
            tube(
                Vec3::new(BARREL_X, BORE_Y, z),
                Vec3::new(BARREL_X, BORE_Y, z - 0.022),
                BARREL_R * 1.08,
                BARREL_R * 1.08,
                8,
                GUNMETAL,
            ),
        );
    }
    // Dark bore.
    kit::add(
        &mut barrel,
        tube(
            Vec3::new(BARREL_X, BORE_Y, -0.505),
            Vec3::new(BARREL_X, BORE_Y, -0.518),
            BARREL_R * 1.05,
            BARREL_R * 1.05,
            8,
            rgb(0x0c0a0a),
        ),
    );
    add_mirrored(p.mesh(BODY), &barrel);
    // Upper bellows board under the barrels.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, hinge().y + 0.012, (BACK_Z + hinge().z) * 0.5),
            Vec3::new(0.062, 0.009, (hinge().z - BACK_Z).abs() * 0.5 + 0.01),
            Quat::IDENTITY,
            wood,
        ),
    );
    // Leather-wrapped grip with a brass pommel, and a short stock strut.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.05, 0.02),
            Vec3::new(0.0, -0.105, 0.065),
            Vec2::new(0.022, 0.028),
            Vec2::new(0.021, 0.027),
            DARK_LEATHER,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, -0.112, 0.067),
            Vec3::new(0.026, 0.01, 0.032),
            Quat::from_rotation_x(-0.3),
            BRASS,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            0.092,
            0.05,
            0.075,
            Vec2::new(0.06, 0.042),
            Vec2::new(0.05, 0.034),
            0.014,
            DARK_LEATHER,
        ),
    );
    // Trigger.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.028, -0.03),
            Vec3::new(0.005, 0.016, 0.005),
            Quat::from_rotation_x(0.3),
            BLACK_STEEL,
        ),
    );
    // Pressure gauge on the left flank.
    p.add(
        BODY,
        tube(
            Vec3::new(-0.06, 0.13, -0.04),
            Vec3::new(-0.082, 0.15, -0.04),
            0.03,
            0.03,
            10,
            BRASS,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Lower board (hinged at the front, opening downwards), with a strap handle underneath.
    let len = (hinge().z - BACK_Z).abs();
    let board_rot = Quat::from_rotation_x(OPEN_ANGLE);
    let mid = hinge() + board_rot * Vec3::new(0.0, -0.01, len * 0.5);
    p.add(
        BOARD,
        block(
            mid,
            Vec3::new(0.064, 0.01, len * 0.5 + 0.012),
            board_rot,
            wood,
        ),
    );
    p.add(
        BOARD,
        block(
            hinge() + board_rot * Vec3::new(0.0, -0.022, len * 0.55),
            Vec3::new(0.016, 0.012, len * 0.3),
            board_rot,
            LEATHER,
        ),
    );
    for z in [0.04, len - 0.03] {
        p.add(
            BOARD,
            block(
                hinge() + board_rot * Vec3::new(0.0, -0.01, z),
                Vec3::new(0.068, 0.013, 0.012),
                board_rot,
                BRASS,
            ),
        );
    }
    kit::finish(p.mesh(BOARD));

    // Pleats: a loft of alternating narrow / wide rectangles between the boards.
    let n = 9;
    let mut rings = Vec::new();
    for k in 0..=n {
        let z = BACK_Z - 0.005 - (k as f32 / n as f32) * (len - 0.035);
        let top = hinge().y + 0.004;
        let bottom = board_y(z) + 0.002;
        let half_w = if k % 2 == 0 { 0.06 } else { 0.088 };
        let half_h = ((top - bottom) * 0.5).max(0.004);
        rings.push(kit::rect_ring(
            Vec3::new(0.0, (top + bottom) * 0.5, z),
            Vec3::X,
            Vec3::Y,
            Vec2::new(half_w, half_h),
        ));
    }
    p.add(
        PLEATS,
        kit::loft(&rings, LEATHER, false, (true, true), kit::Uv::PerFace),
    );
    kit::finish(p.mesh(PLEATS));

    // Glow: the gauge face, a ring in each throat and a vent slit on the breech.
    let mut glow = MeshBuilder::new();
    kit::add(
        &mut glow,
        tube(
            Vec3::new(-0.082, 0.15, -0.04),
            Vec3::new(-0.0839, 0.1517, -0.04),
            0.024,
            0.024,
            10,
            Vec4::ONE,
        ),
    );
    p.add(GLOW, glow);
    let throat = kit::band(
        Vec3::new(BARREL_X, BORE_Y, -0.52),
        Vec3::Z,
        BARREL_R * 0.95,
        0.006,
        0.009,
        8,
        Vec4::ONE,
    );
    add_mirrored(p.mesh(GLOW), &throat);
    for x in [-0.03, 0.03] {
        p.add(
            GLOW,
            block(
                Vec3::new(x, 0.142, -0.03),
                Vec3::new(0.012, 0.003, 0.05),
                Quat::IDENTITY,
                Vec4::ONE,
            ),
        );
    }

    p.texture(BODY, kit::tex_plate());
    p.texture(BOARD, kit::tex_leather());
    p.texture(PLEATS, kit::tex_leather());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}

/// Lower-board rotation and pleat squash for a pump amount 0 (open) ..= 1 (closed).
pub fn pump_transforms(pump: f32) -> (Quat, f32) {
    let close = OPEN_ANGLE * 0.85 * pump.clamp(0.0, 1.0);
    let squash = (OPEN_ANGLE - close).tan() / OPEN_ANGLE.tan();
    (Quat::from_rotation_x(-close), squash.clamp(0.05, 1.0))
}
