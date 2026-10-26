//! Arclash: arc caster. A ribbed, insulated grip under an iron housing wound with copper
//! coils, then a glass capacitor full of crawling light (the `crawl` filaments turn and
//! flicker inside the translucent `glass`), held between copper caps by two tie rods, and
//! three copper prongs that reach forward like a claw around the muzzle. A braided ground
//! strap hangs from the coils with a sparking tip. While Discharge charges, the capacitor
//! brightens towards white and jagged `arcs` jump between the prong tips.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, COPPER, GUNMETAL, IRON, add_mirrored, block, chamfer_box_z, limb, rgb, tube,
};
use crate::models::rig::{JointDef, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const GLASS: usize = 1;
pub const CRAWL: usize = 2;
pub const TIPS: usize = 3;
pub const ARCS: usize = 4;

const BORE_Y: f32 = 0.118;
const GLASS_Z: (f32, f32) = (-0.1, -0.38);
const GLASS_R: f32 = 0.064;
/// Capacitor centre: the crawling filaments turn around the bore axis here.
pub const CAPACITOR: [f32; 3] = [0.0, BORE_Y, (GLASS_Z.0 + GLASS_Z.1) * 0.5];

pub const JOINTS: [JointDef; 5] = [
    static_part("body", Shade::Lit),
    static_part("glass", Shade::Lit),
    joint("crawl", ROOT, CAPACITOR, Shade::Glow),
    static_part("tips", Shade::Glow),
    static_part("arcs", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.6);

/// Prong tip `k` (0 = top, then lower right, lower left), model space.
pub fn prong_tip(k: usize) -> Vec3 {
    let a = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / 3.0;
    Vec3::new(a.cos() * 0.036, BORE_Y + a.sin() * 0.036, -0.6)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let rubber = rgb(0x1e1b1a);
    // Ribbed, insulated grip.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.045, 0.025),
            Vec3::new(0.0, -0.11, 0.06),
            Vec2::new(0.021, 0.027),
            Vec2::new(0.02, 0.026),
            rubber,
        ),
    );
    for i in 0..5 {
        let y = 0.02 - i as f32 * 0.03;
        p.add(
            BODY,
            block(
                Vec3::new(0.0, y, 0.031 + (0.045 - y) * 0.22),
                Vec3::new(0.024, 0.005, 0.029),
                Quat::from_rotation_x(-0.22),
                rgb(0x2e2a28),
            ),
        );
    }
    // Iron housing, trigger, rear cap.
    p.add(
        BODY,
        chamfer_box_z(
            0.062,
            0.085,
            -0.08,
            Vec2::new(0.04, 0.042),
            Vec2::new(0.044, 0.046),
            0.014,
            IRON,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.026, -0.03),
            Vec3::new(0.005, 0.016, 0.005),
            Quat::from_rotation_x(0.3),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, 0.062, 0.084),
            Vec3::new(0.0, 0.062, 0.096),
            0.034,
            0.03,
            8,
            COPPER,
        ),
    );
    // Coil section: dark core wound with copper.
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y, 0.0),
            Vec3::new(0.0, BORE_Y, -0.09),
            0.03,
            0.03,
            10,
            GUNMETAL,
        ),
    );
    for i in 0..6 {
        let z = -0.008 - i as f32 * 0.014;
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, BORE_Y, z),
                Vec3::new(0.0, BORE_Y, z - 0.008),
                0.038,
                0.038,
                10,
                COPPER,
            ),
        );
    }
    // Copper caps at both ends of the capacitor, tie rods between them.
    for (z0, z1) in [
        (GLASS_Z.0 + 0.012, GLASS_Z.0 - 0.012),
        (GLASS_Z.1 + 0.012, GLASS_Z.1 - 0.014),
    ] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, BORE_Y, z0),
                Vec3::new(0.0, BORE_Y, z1),
                GLASS_R + 0.008,
                GLASS_R + 0.008,
                10,
                COPPER,
            ),
        );
    }
    let rod = limb(
        Vec3::new(GLASS_R + 0.012, BORE_Y + 0.01, GLASS_Z.0),
        Vec3::new(GLASS_R + 0.012, BORE_Y + 0.01, GLASS_Z.1),
        Vec2::splat(0.006),
        Vec2::splat(0.006),
        IRON,
    );
    add_mirrored(p.mesh(BODY), &rod);
    // Prong hub and the three prongs: out, then curling in towards the muzzle.
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y, GLASS_Z.1 - 0.014),
            Vec3::new(0.0, BORE_Y, GLASS_Z.1 - 0.05),
            0.048,
            0.034,
            8,
            IRON,
        ),
    );
    for k in 0..3 {
        let a = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / 3.0;
        let radial = Vec3::new(a.cos(), a.sin(), 0.0);
        let c = Vec3::new(0.0, BORE_Y, 0.0);
        let p0 = c + radial * 0.034 + Vec3::Z * (GLASS_Z.1 - 0.04);
        let p1 = c + radial * 0.058 + Vec3::Z * -0.5;
        let p2 = prong_tip(k);
        p.add(
            BODY,
            limb(p0, p1, Vec2::splat(0.009), Vec2::splat(0.008), COPPER),
        );
        p.add(
            BODY,
            limb(p1, p2, Vec2::splat(0.008), Vec2::splat(0.005), COPPER),
        );
        p.add(BODY, kit::gem(p1, 0.011, 0, COPPER));
    }
    // Braided ground strap hanging from the coils.
    let strap = [
        Vec3::new(0.0, 0.03, -0.07),
        Vec3::new(0.012, -0.01, -0.1),
        Vec3::new(0.018, -0.045, -0.14),
        Vec3::new(0.014, -0.07, -0.18),
    ];
    for w in strap.windows(2) {
        p.add(
            BODY,
            limb(
                w[0],
                w[1],
                Vec2::new(0.008, 0.004),
                Vec2::new(0.008, 0.004),
                COPPER,
            ),
        );
    }
    p.add(
        BODY,
        kit::spike(
            strap[3],
            strap[3] + Vec3::new(0.0, -0.03, -0.012),
            0.008,
            4,
            COPPER,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Glass capacitor (translucent).
    p.add(
        GLASS,
        kit::loft(
            &[
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y, GLASS_Z.0),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(GLASS_R * 0.92),
                    10,
                    0.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y, (GLASS_Z.0 + GLASS_Z.1) * 0.5),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(GLASS_R),
                    10,
                    0.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.0, BORE_Y, GLASS_Z.1),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(GLASS_R * 0.92),
                    10,
                    0.0,
                ),
            ],
            Vec4::new(0.75, 0.9, 1.0, 1.0),
            true,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.color(GLASS, Vec4::new(0.8, 0.92, 1.0, 0.38));

    // Crawling light: a central filament and two jagged strands spiralling round it.
    let c = Vec3::from(CAPACITOR);
    let half = (GLASS_Z.0 - GLASS_Z.1).abs() * 0.5 - 0.01;
    p.add(
        CRAWL,
        limb(
            c + Vec3::Z * half,
            c - Vec3::Z * half,
            Vec2::splat(0.005),
            Vec2::splat(0.005),
            Vec4::ONE,
        ),
    );
    for strand in 0..2 {
        let mut points = Vec::new();
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let a = strand as f32 * std::f32::consts::PI + t * std::f32::consts::TAU * 1.25;
            let r = 0.028 + 0.012 * ((i * 7 + strand * 3) % 4) as f32 / 3.0;
            points.push(c + Vec3::new(a.cos() * r, a.sin() * r, half - t * half * 2.0));
        }
        for w in points.windows(2) {
            p.add(
                CRAWL,
                limb(
                    w[0],
                    w[1],
                    Vec2::splat(0.0045),
                    Vec2::splat(0.0035),
                    Vec4::ONE,
                ),
            );
        }
    }
    p.add(CRAWL, kit::gem(c, 0.016, 0, Vec4::ONE));

    // Tips: the prong ends, the strap spark and two status lights on the housing.
    for k in 0..3 {
        p.add(TIPS, kit::gem(prong_tip(k), 0.011, 0, Vec4::ONE));
    }
    p.add(
        TIPS,
        kit::gem(
            strap[3] + Vec3::new(0.0, -0.032, -0.013),
            0.008,
            0,
            Vec4::ONE,
        ),
    );
    let lamp = kit::gem(Vec3::new(0.042, 0.09, 0.03), 0.007, 0, Vec4::ONE);
    add_mirrored(p.mesh(TIPS), &lamp);
    // Charge eye on the back of the housing (faces the wielder).
    p.add(
        TIPS,
        kit::band(
            Vec3::new(0.0, 0.062, 0.098),
            Vec3::Z,
            0.022,
            0.006,
            0.008,
            8,
            Vec4::ONE,
        ),
    );
    p.add(
        TIPS,
        kit::gem(Vec3::new(0.0, 0.062, 0.098), 0.012, 0, Vec4::ONE),
    );

    // Arcs between the prong tips (shown while charging / on a strong shot).
    for k in 0..3 {
        p.add(
            ARCS,
            kit::bolt(
                prong_tip(k),
                prong_tip((k + 1) % 3),
                4,
                0.008,
                k as u32 + 3,
                0.0035,
            ),
        );
        p.add(
            ARCS,
            kit::bolt(
                prong_tip(k),
                Vec3::new(0.0, BORE_Y, -0.64),
                3,
                0.007,
                k as u32 + 11,
                0.003,
            ),
        );
    }

    p.texture(BODY, kit::tex_plate());
    p.texture(GLASS, kit::tex_glow());
    p.texture(CRAWL, kit::tex_glow());
    p.texture(TIPS, kit::tex_glow());
    p.texture(ARCS, kit::tex_glow());
    p.finish()
}
