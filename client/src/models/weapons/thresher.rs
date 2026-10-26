//! Thresher: drum-fed auto-scattergun. A boxy iron receiver over a big brass drum that stands
//! upright under the gun like a threshing wheel (its axle runs across, left to right); the
//! drum's faces show six glowing shell windows each and it turns one shell per shot. Forward,
//! a short barrel inside a cage of four iron bars with glowing heat coils between them ends
//! in a flattened brass duckbill muzzle (wide and low, the way the pellets spread). A pistol
//! grip and an upright fore grip. No pump, no slide: it fires as long as the trigger is held.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, IRON, add_mirrored, block, chamfer_box_z, limb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const DRUM: usize = 1;
pub const SHELLS: usize = 2;
pub const GLOW: usize = 3;

/// Drum centre (it turns about +X here).
pub const DRUM_CENTER: [f32; 3] = [0.0, 0.0, -0.125];
const DRUM_R: f32 = 0.094;
const DRUM_HALF: f32 = 0.038;
/// Shell windows per drum face (one turn step of [`super::DrumTurn`] is one window).
pub const WINDOWS: usize = 6;
const BORE_Y: f32 = 0.11;
const BARREL_R: f32 = 0.034;

pub const JOINTS: [JointDef; 4] = [
    static_part("body", Shade::Lit),
    joint("drum", ROOT, DRUM_CENTER, Shade::Lit),
    joint("shells", DRUM, DRUM_CENTER, Shade::Core),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.58);

fn drum_center() -> Vec3 {
    Vec3::from(DRUM_CENTER)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    // Receiver with a top rib and a notch sight.
    p.add(
        BODY,
        chamfer_box_z(
            0.1,
            0.07,
            -0.22,
            Vec2::new(0.044, 0.048),
            Vec2::new(0.05, 0.054),
            0.016,
            IRON,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.158, -0.08),
            Vec3::new(0.01, 0.006, 0.15),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    for x in [-0.009, 0.009] {
        p.add(
            BODY,
            block(
                Vec3::new(x, 0.17, 0.035),
                Vec3::new(0.004, 0.012, 0.006),
                Quat::IDENTITY,
                BRASS,
            ),
        );
    }
    // Brass bands round the receiver.
    for z in [0.045, -0.17] {
        p.add(
            BODY,
            chamfer_box_z(
                0.1,
                z,
                z - 0.022,
                Vec2::new(0.053, 0.057),
                Vec2::new(0.053, 0.057),
                0.018,
                BRASS,
            ),
        );
    }
    // Drum yoke: two iron cheeks from the receiver down to the axle, and the axle itself.
    let cheek = limb(
        Vec3::new(DRUM_HALF + 0.012, 0.06, -0.125),
        Vec3::new(DRUM_HALF + 0.012, -0.005, -0.125),
        Vec2::new(0.006, 0.03),
        Vec2::new(0.006, 0.022),
        IRON,
    );
    add_mirrored(p.mesh(BODY), &cheek);
    p.add(
        BODY,
        tube(
            drum_center() - Vec3::X * (DRUM_HALF + 0.02),
            drum_center() + Vec3::X * (DRUM_HALF + 0.02),
            0.016,
            0.016,
            8,
            BRASS,
        ),
    );
    // Barrel in a cage of four iron bars held by three rings.
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y, -0.2),
            Vec3::new(0.0, BORE_Y, -0.47),
            BARREL_R,
            BARREL_R,
            8,
            BLACK_STEEL,
        ),
    );
    for k in 0..4 {
        let a = std::f32::consts::FRAC_PI_4 + k as f32 * std::f32::consts::FRAC_PI_2;
        let off = Vec3::new(a.cos(), a.sin(), 0.0) * (BARREL_R + 0.012);
        p.add(
            BODY,
            limb(
                Vec3::new(0.0, BORE_Y, -0.215) + off,
                Vec3::new(0.0, BORE_Y, -0.465) + off,
                Vec2::splat(0.007),
                Vec2::splat(0.007),
                IRON,
            ),
        );
    }
    for z in [-0.22, -0.34, -0.455] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, BORE_Y, z + 0.01),
                Vec3::new(0.0, BORE_Y, z - 0.012),
                BARREL_R + 0.022,
                BARREL_R + 0.022,
                8,
                BRASS,
            ),
        );
    }
    // Duckbill muzzle: round at the barrel, flattened wide and low at the mouth.
    p.add(
        BODY,
        kit::loft(
            &[
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y, -0.465),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(BARREL_R + 0.006),
                    8,
                    std::f32::consts::PI / 8.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y, -0.52),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.06, 0.03),
                    8,
                    std::f32::consts::PI / 8.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y + 0.004, -0.575),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.086, 0.024),
                    8,
                    std::f32::consts::PI / 8.0,
                ),
            ],
            BRASS,
            false,
            (false, true),
            kit::Uv::PerFace,
        ),
    );
    // Pistol grip and upright fore grip.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.05, 0.03),
            Vec3::new(0.0, -0.11, 0.068),
            Vec2::new(0.021, 0.027),
            Vec2::new(0.02, 0.026),
            DARK_LEATHER,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.02, -0.04),
            Vec3::new(0.006, 0.005, 0.028),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, BORE_Y - 0.04, -0.36),
            Vec3::new(0.0, -0.05, -0.33),
            Vec2::new(0.018, 0.022),
            Vec2::new(0.017, 0.021),
            DARK_LEATHER,
        ),
    );
    kit::finish(p.mesh(BODY));

    // The drum: a brass wheel with iron faces and a dark hub.
    let c = drum_center();
    p.add(
        DRUM,
        tube(
            c - Vec3::X * DRUM_HALF,
            c + Vec3::X * DRUM_HALF,
            DRUM_R,
            DRUM_R,
            12,
            BRASS,
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            DRUM,
            tube(
                c + Vec3::X * side * DRUM_HALF,
                c + Vec3::X * side * (DRUM_HALF + 0.005),
                DRUM_R - 0.012,
                DRUM_R - 0.016,
                12,
                IRON,
            ),
        );
        // Ribs between the windows (they show the drum turning).
        for k in 0..WINDOWS {
            let a = (k as f32 + 0.5) / WINDOWS as f32 * std::f32::consts::TAU;
            let d = Vec3::new(0.0, a.sin(), a.cos());
            p.add(
                DRUM,
                limb(
                    c + Vec3::X * side * (DRUM_HALF + 0.006) + d * 0.025,
                    c + Vec3::X * side * (DRUM_HALF + 0.006) + d * (DRUM_R - 0.012),
                    Vec2::new(0.004, 0.006),
                    Vec2::new(0.004, 0.006),
                    BLACK_STEEL,
                ),
            );
        }
    }
    kit::finish(p.mesh(DRUM));
    // Shell windows: a ring of six glowing shell bases on each face.
    for side in [-1.0f32, 1.0] {
        for k in 0..WINDOWS {
            let a = k as f32 / WINDOWS as f32 * std::f32::consts::TAU;
            let d = Vec3::new(0.0, a.sin(), a.cos());
            p.add(
                SHELLS,
                tube(
                    c + Vec3::X * side * (DRUM_HALF + 0.004) + d * 0.058,
                    c + Vec3::X * side * (DRUM_HALF + 0.009) + d * 0.058,
                    0.014,
                    0.012,
                    6,
                    Vec4::new(1.0, 0.7, 0.3, 1.0),
                ),
            );
        }
    }

    // Glow: heat coils round the barrel inside the cage, the duckbill's mouth slit and a
    // small lamp on the receiver's flank.
    for z in [-0.26, -0.3, -0.38, -0.42] {
        p.add(
            GLOW,
            kit::band(
                Vec3::new(0.0, BORE_Y, z),
                Vec3::Z,
                BARREL_R + 0.002,
                0.012,
                0.006,
                8,
                Vec4::ONE,
            ),
        );
    }
    p.add(
        GLOW,
        block(
            Vec3::new(0.0, BORE_Y + 0.004, -0.574),
            Vec3::new(0.07, 0.007, 0.004),
            Quat::IDENTITY,
            Vec4::ONE,
        ),
    );
    let lamp = kit::gem(Vec3::new(0.052, 0.11, -0.05), 0.011, 0, Vec4::ONE);
    add_mirrored(p.mesh(GLOW), &lamp);

    p.texture(BODY, kit::tex_plate());
    p.texture(DRUM, kit::tex_plate());
    p.texture(SHELLS, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}

/// The drum turns with the spin angle (one window per shot); the coils glow hotter while it
/// fires.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    pose.set(DRUM, Mat4::from_rotation_x(-h.spin));
    let kick = unit(h.kick);
    *glow += kick * 1.0;
    let heat = 1.15 + 0.35 * kick + 0.08 * wave(h.time, 2.1);
    pose.core = Vec4::new(heat, heat, heat, 1.0);
    pose
}
