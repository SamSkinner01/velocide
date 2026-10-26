//! The Smelter's slag line on the floor (`kind::SLAG_LINE`, drawn by `enemy_fx`): a unit strip
//! one metre wide running from the origin one metre along −Z, scaled per floor segment (width,
//! 1, length). `strip` is the molten surface (brighter runnels, darker crust); `rims` are two
//! thin raised lines along its edges — on their own they are the telegraph line drawn during
//! the Smelter's wind-up. Both `Glow` (palette-recolourable).

use engine::assets::gltf::ModelPart;
use engine::glam::{Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit;
use crate::models::rig::{JointDef, Shade, static_part};

pub const STRIP: usize = 0;
pub const RIMS: usize = 1;

pub const JOINTS: [JointDef; 2] = [
    static_part("strip", Shade::Glow),
    static_part("rims", Shade::Glow),
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    // A 4 x 8 grid, each cell's brightness varied: molten runnels and cooling crust.
    let (nx, nz) = (4, 8);
    let m = p.mesh(STRIP);
    for i in 0..nx {
        for k in 0..nz {
            let x0 = -0.5 + i as f32 / nx as f32;
            let x1 = -0.5 + (i + 1) as f32 / nx as f32;
            let z0 = -(k as f32) / nz as f32;
            let z1 = -((k + 1) as f32) / nz as f32;
            let r = engine::assets::procedural::rand01(i, k, 0x51a6);
            // The middle runs hottest.
            let mid = 1.0 - (x0 + x1).abs();
            let heat = (0.55 + 0.45 * r) * (0.7 + 0.3 * mid);
            let c = Vec4::new(1.0, 0.32 + 0.42 * heat, 0.06 + 0.26 * heat * heat, 1.0) * heat;
            let c = Vec4::new(c.x, c.y, c.z, 1.0);
            m.quad(
                [
                    Vec3::new(x0, 0.0, z0),
                    Vec3::new(x1, 0.0, z0),
                    Vec3::new(x1, 0.0, z1),
                    Vec3::new(x0, 0.0, z1),
                ],
                [
                    Vec2::new(x0 + 0.5, -z0),
                    Vec2::new(x1 + 0.5, -z0),
                    Vec2::new(x1 + 0.5, -z1),
                    Vec2::new(x0 + 0.5, -z1),
                ],
                c,
            );
        }
    }
    // (Corners go +X then −Z: the quads face up.)
    for side in [-1.0f32, 1.0] {
        p.add(
            RIMS,
            kit::block(
                Vec3::new(side * 0.47, 0.03, -0.5),
                Vec3::new(0.03, 0.03, 0.5),
                engine::glam::Quat::IDENTITY,
                Vec4::ONE,
            ),
        );
    }
    p.texture(STRIP, kit::tex_glow());
    p.texture(RIMS, kit::tex_glow());
    p.finish()
}
