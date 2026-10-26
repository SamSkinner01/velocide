//! Lancet: rapid needle pistol (one-handed). A slim blued-steel receiver shaped like a
//! lozenge, a glass vial of glowing needles lying along its top, and a long, thin silver
//! needle of a barrel with three copper fins and a sharpened point. A bright blade spine runs
//! over the needle like the back of a scalpel; two copper finger flanges stand out from the
//! receiver's sides like a syringe's. A copper feed sleeve at the needle's root snaps back on
//! every shot. No cylinder, no hammer, no frame: the grip is a slim swept-back handle with a
//! red lacquer pommel.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, smoothstep, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, COPPER, DARK_LEATHER, MIRROR_STEEL, RED_LACQUER, add_mirrored, block, limb,
    tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const GLASS: usize = 1;
pub const NEEDLES: usize = 2;
pub const SLIDE: usize = 3;
pub const GLOW: usize = 4;

/// Bore height of the needle barrel.
const BORE_Y: f32 = 0.058;
/// The vial's axis height and length.
const VIAL_Y: f32 = 0.104;
const VIAL_Z: (f32, f32) = (0.03, -0.125);
const VIAL_R: f32 = 0.02;
/// The feed sleeve's back face (it slides back from here).
const SLIDE_Z: f32 = -0.172;

pub const JOINTS: [JointDef; 5] = [
    static_part("body", Shade::Lit),
    static_part("glass", Shade::Lit),
    static_part("needles", Shade::Glow),
    joint("slide", ROOT, [0.0, BORE_Y, SLIDE_Z], Shade::Lit),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.67);

/// Blued steel.
const BLUED: Vec4 = Vec4::new(0.3, 0.37, 0.5, 1.0);

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let ring = |z: f32, y: f32, w: f32, h: f32| {
        kit::chamfer_ring(
            Vec3::new(0.0, y, z),
            Vec3::X,
            Vec3::Y,
            Vec2::new(w, h),
            0.008,
        )
    };
    // Lozenge receiver, tapering into the needle at the front.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(0.065, 0.066, 0.011, 0.02),
                ring(0.02, 0.062, 0.019, 0.034),
                ring(-0.1, 0.06, 0.02, 0.032),
                ring(-0.175, BORE_Y, 0.011, 0.016),
            ],
            BLUED,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    // Vial cradle and copper end caps.
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.088, -0.05),
            Vec3::new(0.008, 0.008, 0.07),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    for (z0, z1) in [
        (VIAL_Z.0 + 0.012, VIAL_Z.0 - 0.004),
        (VIAL_Z.1 + 0.004, VIAL_Z.1 - 0.014),
    ] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, VIAL_Y, z0),
                Vec3::new(0.0, VIAL_Y, z1),
                VIAL_R + 0.002,
                VIAL_R + 0.0015,
                8,
                COPPER,
            ),
        );
    }
    // Blade spine over the needle: a thin bright sliver, high at the vial, low at the point.
    p.add(
        BODY,
        kit::plate(
            &[
                Vec3::new(0.0, 0.096, -0.13),
                Vec3::new(0.0, 0.076, -0.5),
                Vec3::new(0.0, 0.068, -0.48),
                Vec3::new(0.0, 0.072, -0.13),
            ],
            0.005,
            MIRROR_STEEL,
        ),
    );
    // The needle: long thin silver barrel, three copper fins and a sharpened point.
    p.add(
        BODY,
        kit::smooth_tube(
            Vec3::new(0.0, BORE_Y, -0.17),
            Vec3::new(0.0, BORE_Y, -0.6),
            0.0085,
            0.0075,
            6,
            MIRROR_STEEL,
        ),
    );
    p.add(
        BODY,
        kit::spike(
            Vec3::new(0.0, BORE_Y, -0.598),
            Vec3::new(0.0, BORE_Y, -0.675),
            0.0078,
            6,
            MIRROR_STEEL,
        ),
    );
    for (z, r) in [(-0.27, 0.017), (-0.37, 0.015), (-0.47, 0.013)] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, BORE_Y, z),
                Vec3::new(0.0, BORE_Y, z - 0.012),
                r,
                r * 0.8,
                6,
                COPPER,
            ),
        );
    }
    // Finger flanges: copper loops standing out from both sides of the receiver.
    let flange = kit::band(
        Vec3::new(0.036, 0.05, -0.03),
        Vec3::Z,
        0.014,
        0.008,
        0.005,
        8,
        COPPER,
    );
    add_mirrored(p.mesh(BODY), &flange);
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.05, -0.03),
            Vec3::new(0.026, 0.004, 0.004),
            Quat::IDENTITY,
            COPPER,
        ),
    );
    // Slim swept-back grip, leather wrapped, red lacquer pommel; a blade trigger.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.04, 0.022),
            Vec3::new(0.0, -0.105, 0.066),
            Vec2::new(0.016, 0.022),
            Vec2::new(0.015, 0.02),
            DARK_LEATHER,
        ),
    );
    p.add(
        BODY,
        kit::ellipsoid(
            Vec3::new(0.0, -0.112, 0.07),
            Vec3::new(0.02, 0.016, 0.026),
            Quat::from_rotation_x(-0.3),
            2,
            6,
            RED_LACQUER,
            false,
        ),
    );
    p.add(
        BODY,
        kit::plate(
            &[
                Vec3::new(0.0, 0.03, -0.035),
                Vec3::new(0.0, -0.012, -0.05),
                Vec3::new(0.0, -0.02, -0.042),
                Vec3::new(0.0, 0.025, -0.022),
            ],
            0.005,
            RED_LACQUER,
        ),
    );
    kit::finish(p.mesh(BODY));

    // The vial's glass and the needles inside it.
    p.add(
        GLASS,
        tube(
            Vec3::new(0.0, VIAL_Y, VIAL_Z.0),
            Vec3::new(0.0, VIAL_Y, VIAL_Z.1),
            VIAL_R,
            VIAL_R,
            8,
            Vec4::new(0.8, 0.95, 1.0, 1.0),
        ),
    );
    p.color(GLASS, Vec4::new(0.85, 0.95, 1.0, 0.36));
    for k in 0..4 {
        let a = k as f32 / 4.0 * std::f32::consts::TAU + 0.4;
        let off = Vec3::new(a.cos(), a.sin(), 0.0) * 0.009;
        let base = Vec3::new(0.0, VIAL_Y, VIAL_Z.0 - 0.01 - k as f32 * 0.006) + off;
        let tip = Vec3::new(0.0, VIAL_Y, VIAL_Z.1 + 0.012) + off;
        p.add(
            NEEDLES,
            limb(
                base,
                tip,
                Vec2::splat(0.0028),
                Vec2::splat(0.0018),
                Vec4::ONE,
            ),
        );
    }

    // Feed sleeve with two small fins: it snaps back on the shot.
    p.add(
        SLIDE,
        tube(
            Vec3::new(0.0, BORE_Y, SLIDE_Z),
            Vec3::new(0.0, BORE_Y, SLIDE_Z - 0.05),
            0.017,
            0.014,
            8,
            COPPER,
        ),
    );
    let fin = kit::plate(
        &[
            Vec3::new(0.016, BORE_Y, SLIDE_Z - 0.004),
            Vec3::new(0.034, BORE_Y + 0.006, SLIDE_Z - 0.012),
            Vec3::new(0.03, BORE_Y + 0.004, SLIDE_Z - 0.04),
            Vec3::new(0.014, BORE_Y, SLIDE_Z - 0.046),
        ],
        0.004,
        BLACK_STEEL,
    );
    add_mirrored(p.mesh(SLIDE), &fin);
    kit::finish(p.mesh(SLIDE));

    // Glow: a thin channel under the needle, the ring at its point and the receiver's eyes.
    p.add(
        GLOW,
        limb(
            Vec3::new(0.0, BORE_Y - 0.009, -0.2),
            Vec3::new(0.0, BORE_Y - 0.008, -0.58),
            Vec2::new(0.0022, 0.0022),
            Vec2::new(0.0015, 0.0015),
            Vec4::ONE,
        ),
    );
    p.add(
        GLOW,
        kit::band(
            Vec3::new(0.0, BORE_Y, -0.592),
            Vec3::Z,
            0.0095,
            0.006,
            0.004,
            6,
            Vec4::ONE,
        ),
    );
    let eye = kit::gem(Vec3::new(0.021, 0.072, -0.075), 0.007, 0, Vec4::ONE);
    add_mirrored(p.mesh(GLOW), &eye);

    p.texture(BODY, kit::tex_plate());
    p.texture(GLASS, kit::tex_glow());
    p.texture(NEEDLES, kit::tex_glow());
    p.texture(SLIDE, kit::tex_plate());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}

/// The feed sleeve snaps back on the shot and the needles flare; at rest they shimmer.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let kick = unit(h.kick);
    let snap = smoothstep(kick * 1.6);
    pose.set(
        SLIDE,
        engine::glam::Mat4::from_translation(Vec3::new(0.0, 0.0, 0.028 * snap)),
    );
    *glow += kick * 0.9 + 0.08 * wave(h.time, 5.3);
    pose
}
