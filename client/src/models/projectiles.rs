//! The players' projectiles and the thrown Thorn: static models drawn with
//! `ModelLibrary::draw_model` (glow parts use their vertex colours as-is).
//!
//! Projectiles are centred on the origin and fly along -Z: orient them with a look rotation
//! towards the velocity. They are drawn noticeably larger than their collision radius so they
//! read at 640×360.
//!
//! - **Bomb** (Tumbler): studded iron ball, ~0.2 m radius with its studs, a glowing orange fuse
//!   band round the middle and a fuse stub with a hot spark on top.
//! - **Gravewell** (Tumbler alt): heavy iron canister ~0.55 m long, three eerie-green core
//!   rings, a glowing lens at the back and three clamp claws at the front (-Z), the end that
//!   bites into the surface it sticks to.
//! - **Wisp** (Arclash alt): a ball of lightning — a white-violet core, jagged violet tendrils
//!   and a faint translucent halo.
//! - **Thorn**: a barbed glass spike ~0.64 m long with a hot glowing core line and a brass
//!   collar with three fletching vanes. The tip points -Z and sits [`THORN_TIP`] in front of
//!   the origin, so a thorn drawn at its planted position with the flight direction sinks its
//!   tip that far into the surface.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::kit::{self, BRASS, IRON, gem, limb, spike, tube};
use super::rig::{JointDef, Shade, static_part};
use super::{ModelId, PartSet};

const BODY_GLOW: [JointDef; 2] = [
    static_part("body", Shade::Lit),
    static_part("glow", Shade::Glow),
];
const WISP: [JointDef; 3] = [
    static_part("core", Shade::Glow),
    static_part("tendrils", Shade::Glow),
    static_part("halo", Shade::Glow),
];
const THORN: [JointDef; 3] = [
    static_part("glass", Shade::Core),
    static_part("core", Shade::Glow),
    static_part("collar", Shade::Lit),
];

/// Distance from the Thorn's origin to its tip along -Z (how deep a planted thorn's tip sinks
/// into the surface when drawn at its planted position).
pub const THORN_TIP: f32 = 0.22;

/// Fuse colour of bombs (also the Tumbler's primary colour).
const FUSE: Vec4 = Vec4::new(1.0, 0.55, 0.15, 1.0);
/// Gravewell green.
const WELL: Vec4 = Vec4::new(0.45, 1.0, 0.55, 1.0);

/// Part table of one of this module's models.
pub(crate) fn joints(id: ModelId) -> &'static [JointDef] {
    match id {
        ModelId::Wisp => &WISP,
        ModelId::Thorn => &THORN,
        _ => &BODY_GLOW,
    }
}

pub(crate) fn build(id: ModelId) -> Vec<ModelPart> {
    match id {
        ModelId::Gravewell => gravewell(),
        ModelId::Wisp => wisp(),
        ModelId::Thorn => thorn(),
        _ => bomb(),
    }
}

fn finish_lit(p: &mut PartSet) {
    let joints = p.joints();
    p.for_each_mesh(|i, m| {
        if !joints[i].shade.is_emissive() {
            kit::finish(m);
        }
    });
}

fn bomb() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BODY_GLOW);
    p.add(
        0,
        kit::ellipsoid(Vec3::ZERO, Vec3::splat(0.16), Quat::IDENTITY, 5, 10, IRON, false),
    );
    // Studs: three staggered rings plus the poles.
    for (ring, (lat, n)) in [(0.55f32, 6), (0.0, 7), (-0.55, 6)].iter().enumerate() {
        for k in 0..*n {
            let a = (k as f32 + ring as f32 * 0.5) / *n as f32 * std::f32::consts::TAU;
            let d = Vec3::new(a.cos() * lat.cos(), lat.sin(), a.sin() * lat.cos());
            // Skip the equator studs where the fuse band runs; put them just off it.
            let d = if ring == 1 {
                (d + Vec3::Y * if k % 2 == 0 { 0.32 } else { -0.32 }).normalize()
            } else {
                d
            };
            p.add(0, spike(d * 0.15, d * 0.205, 0.028, 4, kit::BLACK_STEEL));
        }
    }
    p.add(0, spike(Vec3::NEG_Y * 0.15, Vec3::NEG_Y * 0.2, 0.03, 4, kit::BLACK_STEEL));
    // Fuse stub on top: brass collar and a short cord.
    p.add(
        0,
        tube(Vec3::Y * 0.14, Vec3::Y * 0.185, 0.045, 0.04, 8, BRASS),
    );
    p.add(
        0,
        limb(
            Vec3::Y * 0.18,
            Vec3::new(0.02, 0.24, 0.015),
            Vec2::splat(0.009),
            Vec2::splat(0.007),
            kit::DARK_LEATHER,
        ),
    );
    // Glowing fuse band and spark.
    p.add(1, kit::band(Vec3::ZERO, Vec3::Y, 0.158, 0.035, 0.016, 12, FUSE));
    p.add(1, gem(Vec3::new(0.022, 0.25, 0.017), 0.032, 0, Vec4::new(1.0, 0.85, 0.4, 1.0)));
    finish_lit(&mut p);
    p.texture(0, kit::tex_grime());
    p.texture(1, kit::tex_glow());
    p.finish()
}

fn gravewell() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BODY_GLOW);
    let z = |z: f32| Vec3::new(0.0, 0.0, z);
    // Canister body with heavy end caps.
    p.add(0, tube(z(0.2), z(-0.16), 0.095, 0.095, 10, IRON));
    p.add(0, tube(z(0.2), z(0.26), 0.11, 0.08, 10, BRASS));
    p.add(0, tube(z(-0.16), z(-0.2), 0.11, 0.1, 10, kit::BLACK_STEEL));
    // Rings the core bands sit between.
    for zz in [0.13, 0.03, -0.07] {
        p.add(0, tube(z(zz + 0.012), z(zz - 0.012), 0.104, 0.104, 10, kit::BLACK_STEEL));
    }
    // Clamp claws: out from the front cap, then hooking in.
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + 0.5;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        let p0 = d * 0.08 + z(-0.19);
        let p1 = d * 0.13 + z(-0.27);
        let p2 = d * 0.06 + z(-0.34);
        p.add(0, limb(p0, p1, Vec2::splat(0.018), Vec2::splat(0.015), BRASS));
        p.add(0, limb(p1, p2, Vec2::splat(0.015), Vec2::splat(0.008), BRASS));
        p.add(0, gem(p1, 0.02, 0, kit::BLACK_STEEL));
    }
    // Eerie green core bands, a lens at the back and a spike of light at the front.
    for zz in [0.08, -0.02] {
        p.add(1, kit::band(z(zz), Vec3::Z, 0.097, 0.05, 0.01, 10, WELL));
    }
    p.add(1, tube(z(0.26), z(0.27), 0.05, 0.05, 10, WELL));
    p.add(1, gem(z(-0.24), 0.04, 0, Vec4::new(0.75, 1.0, 0.8, 1.0)));
    finish_lit(&mut p);
    p.texture(0, kit::tex_grime());
    p.texture(1, kit::tex_glow());
    p.finish()
}

fn wisp() -> Vec<ModelPart> {
    let mut p = PartSet::new(&WISP);
    p.add(0, gem(Vec3::ZERO, 0.12, 1, Vec4::new(0.95, 0.88, 1.0, 1.0)));
    // Jagged tendrils in every direction (a fixed scatter; the client spins the model).
    let violet = Vec4::new(0.72, 0.5, 1.0, 1.0);
    for k in 0..10u32 {
        let d = Quat::from_rotation_y(k as f32 * 2.399)
            * Quat::from_rotation_x(0.6 + (k % 4) as f32 * 0.7)
            * Vec3::Y;
        let len = 0.24 + (k % 3) as f32 * 0.05;
        let mut bolt = kit::bolt(d * 0.08, d * len, 3, 0.035, k + 1, 0.009);
        bolt.set_color(violet);
        p.add(1, bolt);
    }
    p.add(2, gem(Vec3::ZERO, 0.24, 1, Vec4::new(0.6, 0.45, 1.0, 1.0)));
    p.color(2, Vec4::new(1.0, 1.0, 1.0, 0.3));
    p.texture(0, kit::tex_glow());
    p.texture(1, kit::tex_glow());
    p.texture(2, kit::tex_glow());
    p.finish()
}

fn thorn() -> Vec<ModelPart> {
    let mut p = PartSet::new(&THORN);
    let glass = Vec4::new(0.74, 0.96, 1.0, 1.0);
    let hot = Vec4::new(1.0, 0.8, 0.4, 1.0);
    let z = |z: f32| Vec3::new(0.0, 0.0, z);
    let ring = |zz: f32, r: f32| kit::ngon_ring(z(zz), Vec3::X, Vec3::Y, Vec2::splat(r), 6, 0.0);
    // Glass spike: needle tip, widest just behind it, a long tapering shaft.
    p.add(
        0,
        kit::loft(
            &[
                vec![z(-THORN_TIP)],
                ring(-0.07, 0.05),
                ring(0.07, 0.036),
                ring(0.36, 0.022),
                vec![z(0.42)],
            ],
            glass,
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    // Barbs: two staggered rows pointing back and out.
    for (row, (zz, r)) in [(-0.05f32, 0.048f32), (0.09, 0.035)].iter().enumerate() {
        for k in 0..3 {
            let a = (k as f32 + row as f32 * 0.5) / 3.0 * std::f32::consts::TAU;
            let d = Vec3::new(a.cos(), a.sin(), 0.0);
            p.add(
                0,
                kit::crystal(
                    z(*zz) + d * (*r * 0.8) + z(0.04),
                    (d * 0.45 + Vec3::Z).normalize(),
                    (0.035, 0.06),
                    0.015,
                    3,
                    glass,
                ),
            );
        }
    }
    p.mesh(0).shade_by_normal(1.0, 0.75, 0.5);
    p.color(0, Vec4::new(0.8, 0.95, 1.0, 0.42));
    // Hot core line, the heart behind the tip and a ring at the collar: the lit relay.
    p.add(1, limb(z(-0.17), z(0.36), Vec2::splat(0.02), Vec2::splat(0.011), hot));
    p.add(1, gem(z(-0.07), 0.03, 0, Vec4::new(1.0, 0.95, 0.78, 1.0)));
    p.add(1, kit::band(z(0.27), Vec3::Z, 0.031, 0.012, 0.01, 6, hot));
    // Brass collar and fletching at the tail.
    p.add(2, tube(z(0.24), z(0.3), 0.03, 0.027, 6, BRASS));
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + 0.5;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        p.add(
            2,
            kit::plate(
                &[d * 0.024 + z(0.26), d * 0.08 + z(0.36), d * 0.024 + z(0.4)],
                0.007,
                BRASS,
            ),
        );
    }
    kit::finish(p.mesh(2));
    p.texture(0, kit::tex_glow());
    p.texture(1, kit::tex_glow());
    p.texture(2, kit::tex_plate());
    p.finish()
}
