//! Tumbler: squat drum-fed bomb launcher in black iron and brass. A big six-chamber drum
//! turns one chamber per shot (`drum`, the glowing fuse bands of the loaded bombs turn with
//! it), a fat round barrel ends in a wide flared muzzle with a glowing throat, an iron bridge
//! with a sight runs over the drum, and a vertical fore grip hangs under the barrel.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, IRON, add_mirrored, block, chamfer_box_z, limb, rgb,
    tube,
};
use crate::models::rig::{JointDef, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const DRUM: usize = 1;
pub const DRUM_GLOW: usize = 2;
pub const GLOW: usize = 3;

/// Drum axis centre (the drum turns about +Z here).
pub const DRUM_CENTER: [f32; 3] = [0.0, 0.05, -0.13];
const DRUM_R: f32 = 0.12;
const DRUM_HALF: f32 = 0.09;
/// Radius of the chamber circle; the top chamber lines up with the barrel.
const CHAMBER_R: f32 = 0.072;
pub const CHAMBERS: usize = 6;
const BORE_Y: f32 = DRUM_CENTER[1] + CHAMBER_R;
const BARREL_R: f32 = 0.088;

pub const JOINTS: [JointDef; 4] = [
    static_part("body", Shade::Lit),
    joint("drum", ROOT, DRUM_CENTER, Shade::Lit),
    joint("drum_glow", DRUM, DRUM_CENTER, Shade::Glow),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.56);

fn drum_center() -> Vec3 {
    Vec3::from(DRUM_CENTER)
}

/// Centre of chamber `k` in the drum's bind pose (`k = 0` is the top one, in line with the
/// barrel).
fn chamber(k: usize) -> Vec3 {
    let a = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / CHAMBERS as f32;
    drum_center() + Vec3::new(a.cos(), a.sin(), 0.0) * CHAMBER_R
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let dark = rgb(0x0c0a09);
    // Low rear housing (the drum's back face with its glowing chambers shows above it) and
    // the grip.
    p.add(
        BODY,
        chamfer_box_z(
            0.035,
            0.095,
            -0.03,
            Vec2::new(0.046, 0.05),
            Vec2::new(0.052, 0.056),
            0.018,
            IRON,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            0.035,
            0.105,
            0.093,
            Vec2::new(0.04, 0.044),
            Vec2::new(0.046, 0.05),
            0.014,
            BRASS,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.03, 0.03),
            Vec3::new(0.0, -0.11, 0.07),
            Vec2::new(0.023, 0.029),
            Vec2::new(0.022, 0.028),
            DARK_LEATHER,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.012, -0.045),
            Vec3::new(0.006, 0.005, 0.03),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    // Drum axle and the lower frame bar to the barrel.
    p.add(
        BODY,
        tube(
            drum_center() + Vec3::Z * (DRUM_HALF + 0.02),
            drum_center() - Vec3::Z * (DRUM_HALF + 0.03),
            0.022,
            0.022,
            8,
            BRASS,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, drum_center().y - 0.035, -0.29),
            Vec3::new(0.026, 0.02, 0.07),
            Quat::IDENTITY,
            IRON,
        ),
    );
    // Iron bridge over the drum with a sight blade.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, drum_center().y + DRUM_R + 0.016, -0.17),
            Vec3::new(0.022, 0.011, 0.105),
            Quat::IDENTITY,
            IRON,
        ),
    );
    p.add(
        BODY,
        kit::plate(
            &[
                Vec3::new(0.0, drum_center().y + DRUM_R + 0.026, -0.08),
                Vec3::new(0.0, drum_center().y + DRUM_R + 0.026, -0.15),
                Vec3::new(0.0, drum_center().y + DRUM_R + 0.06, -0.105),
            ],
            0.01,
            BRASS,
        ),
    );
    // Fat barrel with brass bands and a wide flared muzzle.
    let bore = |z: f32| Vec3::new(0.0, BORE_Y, z);
    p.add(
        BODY,
        tube(
            bore(-0.215),
            bore(-0.47),
            BARREL_R,
            BARREL_R * 0.94,
            10,
            IRON,
        ),
    );
    for z in [-0.25, -0.38] {
        p.add(
            BODY,
            tube(
                bore(z),
                bore(z - 0.03),
                BARREL_R + 0.008,
                BARREL_R + 0.006,
                10,
                BRASS,
            ),
        );
    }
    p.add(
        BODY,
        tube(
            bore(-0.47),
            bore(-0.56),
            BARREL_R * 0.94,
            BARREL_R * 1.2,
            10,
            IRON,
        ),
    );
    p.add(
        BODY,
        tube(
            bore(-0.548),
            bore(-0.562),
            BARREL_R * 1.12,
            BARREL_R * 1.12,
            10,
            dark,
        ),
    );
    // Studs around the muzzle lip.
    for k in 0..6 {
        let a = k as f32 / 6.0 * std::f32::consts::TAU + 0.3;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        p.add(
            BODY,
            kit::gem(bore(-0.52) + d * (BARREL_R * 1.1), 0.012, 0, BRASS),
        );
    }
    // Vertical fore grip.
    p.add(
        BODY,
        limb(
            bore(-0.36) - Vec3::Y * (BARREL_R - 0.01),
            bore(-0.35) - Vec3::Y * (BARREL_R + 0.11),
            Vec2::new(0.02, 0.024),
            Vec2::new(0.019, 0.023),
            DARK_LEATHER,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Drum: fluted iron cylinder, dark chamber mouths on both faces, brass flutes between.
    let c = drum_center();
    p.add(
        DRUM,
        tube(
            c + Vec3::Z * DRUM_HALF,
            c - Vec3::Z * DRUM_HALF,
            DRUM_R,
            DRUM_R,
            12,
            IRON,
        ),
    );
    for k in 0..CHAMBERS {
        let at = chamber(k);
        for z in [DRUM_HALF, -DRUM_HALF] {
            let face = Vec3::new(at.x, at.y, c.z + z);
            let out = face + Vec3::Z * z.signum() * 0.004;
            p.add(DRUM, tube(face, out, 0.031, 0.031, 8, dark));
        }
        // Brass flute between this chamber and the next.
        let a = std::f32::consts::FRAC_PI_2
            + (k as f32 + 0.5) * std::f32::consts::TAU / CHAMBERS as f32;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        p.add(
            DRUM,
            block(
                c + d * (DRUM_R + 0.004),
                Vec3::new(0.012, 0.006, DRUM_HALF * 0.85),
                Quat::from_rotation_z(a - std::f32::consts::FRAC_PI_2),
                BRASS,
            ),
        );
    }
    kit::finish(p.mesh(DRUM));

    // Fuse bands of the loaded bombs, glowing in each chamber mouth.
    for k in 0..CHAMBERS {
        let at = chamber(k);
        for z in [DRUM_HALF, -DRUM_HALF] {
            p.add(
                DRUM_GLOW,
                kit::band(
                    Vec3::new(at.x, at.y, c.z + z + z.signum() * 0.006),
                    Vec3::Z,
                    0.022,
                    0.006,
                    0.008,
                    6,
                    Vec4::ONE,
                ),
            );
        }
    }

    // Throat ring inside the muzzle and a fuse window on each side of the housing.
    p.add(
        GLOW,
        kit::band(
            bore(-0.566),
            Vec3::Z,
            BARREL_R * 0.86,
            0.006,
            0.014,
            10,
            Vec4::ONE,
        ),
    );
    let window = block(
        Vec3::new(0.0535, 0.045, 0.035),
        Vec3::new(0.002, 0.016, 0.03),
        Quat::IDENTITY,
        Vec4::ONE,
    );
    add_mirrored(p.mesh(GLOW), &window);

    p.texture(BODY, kit::tex_grime());
    p.texture(DRUM, kit::tex_plate());
    p.texture(DRUM_GLOW, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}
