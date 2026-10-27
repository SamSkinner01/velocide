//! The wax puddle a stranded Mould melts into and re-forms out of (`AiState::Melt` /
//! `Reform`, drawn by `entities`): a lumpy pool of overlapping flattened lobes one metre in
//! radius at the origin (scaled per Mould and with the melt's depth), its molten heart
//! glowing through the middle. `pool` is lit (tinted with the Mould's wax colour), `heart` is
//! `Glow` (the Mould's heat; palette-recolourable).

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit;
use crate::models::rig::{JointDef, Shade, static_part};

pub const POOL: usize = 0;
pub const HEART: usize = 1;

pub const JOINTS: [JointDef; 2] = [
    static_part("pool", Shade::Lit),
    static_part("heart", Shade::Glow),
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    // (x, z, radius, height): a broad middle and runs spreading out from it. Ellipsoids
    // centred on the floor: only their upper halves show.
    let lobes = [
        (0.0, 0.0, 0.8, 0.13),
        (0.45, 0.2, 0.48, 0.09),
        (-0.4, 0.32, 0.44, 0.08),
        (0.12, -0.48, 0.5, 0.09),
        (-0.42, -0.3, 0.4, 0.07),
        (0.62, -0.28, 0.3, 0.05),
        (-0.15, 0.68, 0.28, 0.05),
    ];
    for (x, z, r, h) in lobes {
        p.add(
            POOL,
            kit::ellipsoid(
                Vec3::new(x, 0.0, z),
                Vec3::new(r, h, r * 0.9),
                Quat::IDENTITY,
                4,
                12,
                Vec4::new(1.0, 0.97, 0.9, 1.0),
                true,
            ),
        );
    }
    // The heart: molten wax showing through the skin in the middle, and a drip run.
    p.add(
        HEART,
        kit::ellipsoid(
            Vec3::new(0.05, 0.04, 0.0),
            Vec3::new(0.36, 0.11, 0.3),
            Quat::IDENTITY,
            4,
            10,
            Vec4::new(1.0, 0.62, 0.24, 1.0),
            true,
        ),
    );
    p.add(
        HEART,
        kit::ellipsoid(
            Vec3::new(0.4, 0.02, 0.16),
            Vec3::new(0.16, 0.07, 0.12),
            Quat::IDENTITY,
            3,
            8,
            Vec4::new(1.0, 0.5, 0.16, 1.0),
            true,
        ),
    );
    p.texture(POOL, kit::tex_wax());
    p.texture(HEART, kit::tex_glow());
    p.finish()
}
