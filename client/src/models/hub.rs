//! Props of the warm-up hub (the Vestry, docs/lobby.md), placed by the level as glTF props
//! (`models/prop_ready_board.gltf`, `models/prop_console.gltf`) and found by the client's hub
//! view (`ingame::hub`), which writes on them.
//!
//! Both stand on the origin and face -Z (the hub turns the board round to face the nave).

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::kit::{self, BRASS, IRON, add, add_mirrored, block, limb, rgb, tube};
use super::rig::{JointDef, Shade, static_part};
use super::{ModelId, PartSet};

const READY_BOARD: [JointDef; 3] = [
    static_part("frame", Shade::Lit),
    static_part("slate", Shade::Lit),
    static_part("brass", Shade::Lit),
];
const CONSOLE: [JointDef; 3] = [
    static_part("body", Shade::Lit),
    static_part("brass", Shade::Lit),
    static_part("panel", Shade::Glow),
];

/// The ready board's writing surface in its model space: the slate's front plane `z`, and the
/// rectangle text may fill (x from `-half_w`..`half_w`, y `bottom..top`). The header plate's
/// centre line is `header_y`.
pub mod board {
    pub const FRONT_Z: f32 = -0.085;
    pub const HALF_W: f32 = 2.6;
    pub const BOTTOM: f32 = 0.35;
    pub const TOP: f32 = 2.62;
    pub const HEADER_Y: f32 = 3.04;
    pub const HEADER_HALF_W: f32 = 1.35;
    pub const HEADER_Z: f32 = -0.112;
}

/// The console's sign line above its desk (model space): centre and front plane.
pub mod console {
    pub const SIGN_Y: f32 = 1.72;
    pub const SIGN_Z: f32 = -0.067;
    pub const SIGN_HALF_W: f32 = 0.55;
}

pub(crate) fn joints(id: ModelId) -> &'static [JointDef] {
    match id {
        ModelId::ReadyBoard => &READY_BOARD,
        _ => &CONSOLE,
    }
}

pub(crate) fn build(id: ModelId) -> Vec<ModelPart> {
    match id {
        ModelId::ReadyBoard => ready_board(),
        _ => console(),
    }
}

const WOOD: Vec4 = Vec4::new(0.36, 0.25, 0.2, 1.0);
const SLATE: Vec4 = Vec4::new(0.13, 0.13, 0.15, 1.0);
const STONE: Vec4 = Vec4::new(0.62, 0.58, 0.55, 1.0);
const WAX: Vec4 = Vec4::new(0.95, 0.88, 0.7, 1.0);

/// A 6 × 3.4 m slate in a carved frame with a brass header plate and two candle sconces.
fn ready_board() -> Vec<ModelPart> {
    let mut p = PartSet::new(&READY_BOARD);
    let (w, h, d) = (3.0f32, 2.9f32, 0.1f32);
    let mut frame = MeshBuilder::new();
    for (center, half) in [
        (Vec3::new(0.0, 0.1, -d * 0.5), Vec3::new(w, 0.1, d * 0.5)),
        (
            Vec3::new(0.0, h - 0.1, -d * 0.5),
            Vec3::new(w, 0.1, d * 0.5),
        ),
        (
            Vec3::new(-w + 0.1, h * 0.5, -d * 0.5),
            Vec3::new(0.1, h * 0.5, d * 0.5),
        ),
        (
            Vec3::new(w - 0.1, h * 0.5, -d * 0.5),
            Vec3::new(0.1, h * 0.5, d * 0.5),
        ),
    ] {
        add(&mut frame, block(center, half, Quat::IDENTITY, WOOD));
    }
    // Carved corner blocks and a moulding along the top.
    for x in [-w + 0.1, w - 0.1] {
        for y in [0.1, h - 0.1] {
            add(
                &mut frame,
                block(
                    Vec3::new(x, y, -d - 0.02),
                    Vec3::new(0.14, 0.14, 0.03),
                    Quat::IDENTITY,
                    rgb(0x2c1d18),
                ),
            );
        }
    }
    add(
        &mut frame,
        block(
            Vec3::new(0.0, h + 0.04, -d * 0.6),
            Vec3::new(w + 0.08, 0.05, d * 0.7),
            Quat::IDENTITY,
            WOOD,
        ),
    );
    kit::finish(&mut frame);
    p.add(0, frame);

    p.add(
        1,
        block(
            Vec3::new(0.0, h * 0.5, -0.045),
            Vec3::new(w - 0.2, h * 0.5 - 0.2, 0.03),
            Quat::IDENTITY,
            SLATE,
        ),
    );

    // Brass: the header plate, its rivets and the candle sconces.
    let mut brass = block(
        Vec3::new(0.0, board::HEADER_Y, -0.085),
        Vec3::new(board::HEADER_HALF_W + 0.1, 0.24, 0.02),
        Quat::IDENTITY,
        BRASS,
    );
    for x in [-board::HEADER_HALF_W, board::HEADER_HALF_W] {
        add(
            &mut brass,
            kit::gem(Vec3::new(x, board::HEADER_Y, -0.11), 0.03, 0, rgb(0x6b4d24)),
        );
    }
    let mut sconce = limb(
        Vec3::new(w + 0.05, 1.6, -0.05),
        Vec3::new(w + 0.25, 1.75, -0.3),
        Vec2::splat(0.03),
        Vec2::splat(0.025),
        BRASS,
    );
    add(
        &mut sconce,
        tube(
            Vec3::new(w + 0.25, 1.75, -0.3),
            Vec3::new(w + 0.25, 1.8, -0.3),
            0.09,
            0.11,
            8,
            BRASS,
        ),
    );
    add(
        &mut sconce,
        tube(
            Vec3::new(w + 0.25, 1.8, -0.3),
            Vec3::new(w + 0.25, 2.05, -0.3),
            0.045,
            0.04,
            6,
            WAX,
        ),
    );
    add_mirrored(&mut brass, &sconce);
    kit::finish(&mut brass);
    p.add(2, brass);
    p.texture(0, kit::tex_grime());
    p.texture(1, kit::tex_grime());
    p.texture(2, kit::tex_plate());
    p.finish()
}

/// A stone lectern with a slanted brass-rimmed desk, a summoning bell and a lever; its glowing
/// panel faces the player (-Z).
fn console() -> Vec<ModelPart> {
    let mut p = PartSet::new(&CONSOLE);
    let mut body = block(
        Vec3::new(0.0, 0.45, 0.0),
        Vec3::new(0.32, 0.45, 0.28),
        Quat::IDENTITY,
        STONE,
    );
    add(
        &mut body,
        block(
            Vec3::new(0.0, 0.05, 0.0),
            Vec3::new(0.42, 0.05, 0.36),
            Quat::IDENTITY,
            rgb(0x6e6560),
        ),
    );
    // The slanted desk, tilted towards the player.
    let tilt = Quat::from_rotation_x(0.45);
    add(
        &mut body,
        block(
            Vec3::new(0.0, 1.0, -0.02),
            Vec3::new(0.5, 0.06, 0.34),
            tilt,
            STONE,
        ),
    );
    // A sign board on a post behind the desk.
    add(
        &mut body,
        block(
            Vec3::new(0.0, console::SIGN_Y, -0.02),
            Vec3::new(console::SIGN_HALF_W + 0.06, 0.14, 0.04),
            Quat::IDENTITY,
            SLATE,
        ),
    );
    add(
        &mut body,
        limb(
            Vec3::new(0.0, 1.05, 0.12),
            Vec3::new(0.0, console::SIGN_Y - 0.1, 0.02),
            Vec2::splat(0.04),
            Vec2::splat(0.035),
            IRON,
        ),
    );
    kit::finish(&mut body);
    p.add(0, body);

    let mut brass = block(
        Vec3::new(0.0, 1.06, -0.02),
        Vec3::new(0.52, 0.03, 0.36),
        tilt,
        BRASS,
    );
    // Summoning bell on a bracket (right), a lever (left).
    add(
        &mut brass,
        limb(
            Vec3::new(0.38, 1.0, 0.1),
            Vec3::new(0.38, 1.42, 0.1),
            Vec2::splat(0.022),
            Vec2::splat(0.02),
            IRON,
        ),
    );
    add(
        &mut brass,
        kit::loft(
            &[
                kit::ngon_ring(
                    Vec3::new(0.38, 1.18, 0.1),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.11),
                    8,
                    0.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.38, 1.3, 0.1),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.07),
                    8,
                    0.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.38, 1.38, 0.1),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.03),
                    8,
                    0.0,
                ),
            ],
            BRASS,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    add(
        &mut brass,
        limb(
            Vec3::new(-0.36, 1.0, 0.0),
            Vec3::new(-0.42, 1.35, -0.12),
            Vec2::splat(0.025),
            Vec2::splat(0.02),
            IRON,
        ),
    );
    add(
        &mut brass,
        kit::gem(Vec3::new(-0.42, 1.37, -0.13), 0.05, 0, rgb(0x8a1c14)),
    );
    kit::finish(&mut brass);
    p.add(1, brass);

    // The glowing panel on the desk.
    p.add(
        2,
        block(
            Vec3::new(0.0, 1.075, -0.04),
            Vec3::new(0.34, 0.012, 0.2),
            tilt,
            Vec4::new(1.0, 0.62, 0.25, 1.0),
        ),
    );
    p.texture(0, kit::tex_grime());
    p.texture(1, kit::tex_plate());
    p.texture(2, kit::tex_glow());
    p.finish()
}
