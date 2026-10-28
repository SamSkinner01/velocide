//! The rework's arrows (docs/rework.md): static models drawn with `ModelLibrary::draw_model`
//! like the other projectiles (centred, flying along -Z, glow parts in their vertex colours).
//!
//! - **Blink arrow** (Blink Arrow): a slim pale arrow whose head is a glass bead holding a cold
//!   mint flame (the light you step to), mint rings down the shaft and pale fletching with a
//!   mint cock vane, ~0.72 m.
//! - **Beacon arrow** (Beacon Arrow): a heavier arrow carrying a little brass lantern behind
//!   its spike: a cage of four bars round a glowing gold bulb, a cap and a ring; gold fletching.
//!   Stuck, the client lights it (a light, a rising flare, the reveal ring).

use engine::assets::gltf::ModelPart;
use engine::glam::{Vec2, Vec3, Vec4};

use super::kit::{self, BRASS, IRON, gem, limb, spike, tube};
use super::rig::{JointDef, Shade, static_part};
use super::{ModelId, PartSet};

const BODY_GLOW: [JointDef; 2] = [
    static_part("body", Shade::Lit),
    static_part("glow", Shade::Glow),
];

/// The Blink arrow's cold flame (its ability colour) and the beacon's gold.
pub const BLINK_MINT: Vec4 = Vec4::new(0.5, 1.0, 0.82, 1.0);
pub const BEACON_GOLD: Vec4 = Vec4::new(1.0, 0.84, 0.36, 1.0);
/// Where the beacon's bulb sits on its arrow (model space): the light and the flare rise from
/// it.
pub const BEACON_BULB: Vec3 = Vec3::new(0.0, 0.0, -0.24);

const PALE_SHAFT: Vec4 = Vec4::new(0.86, 0.84, 0.78, 1.0);
const PARCHMENT: Vec4 = Vec4::new(0.9, 0.84, 0.68, 1.0);
const DARK_WOOD: Vec4 = Vec4::new(0.36, 0.22, 0.13, 1.0);

/// Part table of one of this module's models.
pub(crate) fn joints(_id: ModelId) -> &'static [JointDef] {
    &BODY_GLOW
}

pub(crate) fn build(id: ModelId) -> Vec<ModelPart> {
    match id {
        ModelId::BeaconArrow => beacon_arrow(),
        _ => blink_arrow(),
    }
}

/// Point `z` on the flight axis.
fn on_axis(z: f32) -> Vec3 {
    Vec3::new(0.0, 0.0, z)
}

/// Nock, shaft and three vanes (one coloured `cock`), from `nock` to `front` along -Z.
fn shaft(p: &mut PartSet, nock: f32, front: f32, wood: Vec4, vane: Vec4, cock: Vec4) {
    p.add(
        0,
        tube(on_axis(nock), on_axis(front), 0.009, 0.0075, 6, wood),
    );
    p.add(
        0,
        tube(
            on_axis(nock + 0.012),
            on_axis(nock - 0.02),
            0.011,
            0.011,
            6,
            kit::BLACK_STEEL,
        ),
    );
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        p.add(
            0,
            kit::plate(
                &[
                    on_axis(nock - 0.03) + d * 0.008,
                    on_axis(nock - 0.05) + d * 0.04,
                    on_axis(nock - 0.14) + d * 0.036,
                    on_axis(nock - 0.16) + d * 0.008,
                ],
                0.003,
                if k == 0 { cock } else { vane },
            ),
        );
    }
}

fn finish(mut p: PartSet) -> Vec<ModelPart> {
    kit::finish(p.mesh(0));
    p.texture(0, kit::tex_bone());
    p.texture(1, kit::tex_glow());
    p.finish()
}

fn blink_arrow() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BODY_GLOW);
    let (nock, tip) = (0.36f32, -0.36f32);
    shaft(
        &mut p,
        nock,
        tip + 0.09,
        PALE_SHAFT,
        PARCHMENT,
        Vec4::new(0.35, 0.85, 0.68, 1.0),
    );
    // A short iron point in front of a glass bead; the bead holds the cold flame.
    p.add(0, spike(on_axis(tip + 0.035), on_axis(tip), 0.012, 4, IRON));
    p.add(
        0,
        tube(
            on_axis(tip + 0.095),
            on_axis(tip + 0.082),
            0.014,
            0.016,
            6,
            BRASS,
        ),
    );
    p.add(1, gem(on_axis(tip + 0.06), 0.026, 1, BLINK_MINT));
    p.add(
        1,
        kit::crystal(
            on_axis(tip + 0.06),
            Vec3::NEG_Z,
            (0.018, 0.03),
            0.012,
            5,
            Vec4::new(0.9, 1.0, 0.95, 1.0),
        ),
    );
    // Mint rings down the shaft and a spark at the nock.
    for z in [tip + 0.2, tip + 0.32, nock - 0.2] {
        p.add(
            1,
            kit::band(on_axis(z), Vec3::Z, 0.0105, 0.008, 0.005, 6, BLINK_MINT),
        );
    }
    p.add(1, gem(on_axis(nock + 0.014), 0.011, 0, BLINK_MINT));
    finish(p)
}

fn beacon_arrow() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BODY_GLOW);
    let (nock, tip) = (0.36f32, -0.38f32);
    shaft(
        &mut p,
        nock,
        tip + 0.2,
        DARK_WOOD,
        Vec4::new(0.95, 0.78, 0.36, 1.0),
        Vec4::new(0.85, 0.3, 0.12, 1.0),
    );
    // The spike that drives it home.
    p.add(
        0,
        spike(
            on_axis(tip + 0.08),
            on_axis(tip),
            0.016,
            4,
            kit::MIRROR_STEEL,
        ),
    );
    // The lantern: a base plate, four cage bars round the bulb, a cap and a hanging ring.
    let bulb = BEACON_BULB;
    p.add(
        0,
        tube(
            bulb + Vec3::Z * 0.06,
            bulb + Vec3::Z * 0.045,
            0.03,
            0.03,
            8,
            BRASS,
        ),
    );
    p.add(
        0,
        tube(
            bulb - Vec3::Z * 0.045,
            bulb - Vec3::Z * 0.065,
            0.026,
            0.014,
            8,
            BRASS,
        ),
    );
    for k in 0..4 {
        let a = (k as f32 + 0.5) / 4.0 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        p.add(
            0,
            limb(
                bulb + Vec3::Z * 0.046 + d * 0.027,
                bulb - Vec3::Z * 0.046 + d * 0.024,
                Vec2::splat(0.0045),
                Vec2::splat(0.0045),
                BRASS,
            ),
        );
    }
    p.add(
        0,
        kit::band(
            bulb + Vec3::new(0.0, 0.032, 0.0),
            Vec3::X,
            0.012,
            0.004,
            0.004,
            6,
            IRON,
        ),
    );
    // The bulb and its flame (lit: the client brightens it), gold rings on the shaft.
    p.add(1, gem(bulb, 0.022, 1, BEACON_GOLD));
    p.add(
        1,
        kit::crystal(
            bulb,
            Vec3::Y,
            (0.012, 0.024),
            0.01,
            5,
            Vec4::new(1.0, 0.95, 0.75, 1.0),
        ),
    );
    for z in [bulb.z + 0.16, nock - 0.2] {
        p.add(
            1,
            kit::band(on_axis(z), Vec3::Z, 0.0105, 0.01, 0.005, 6, BEACON_GOLD),
        );
    }
    finish(p)
}
