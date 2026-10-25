//! Enemy projectiles, pickups and level props: simple static models (all parts at the model
//! origin) plus a few pose helpers for props with moving parts (doors, checkpoint, jump pad).
//! The players' projectiles and the Thorn live in `projectiles.rs`.
//!
//! Projectiles are centred on the origin and fly along -Z (orient them with a look rotation
//! towards the velocity). Props stand on the origin (floor) and face -Z.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::kit::{
    self, BLACK_STEEL, CHROME, GUNMETAL, add, add_mirrored, block, gem, limb, rgb, spike, tube,
};
use super::rig::{JointDef, Pose, Shade, static_part};
use super::{ModelId, PartSet};

const BODY_GLOW: [JointDef; 2] = [
    static_part("body", Shade::Lit),
    static_part("glow", Shade::Glow),
];
const SPITTER_ORB: [JointDef; 2] = [
    static_part("core", Shade::Glow),
    static_part("shell", Shade::Glow),
];
const GNAT_BOLT: [JointDef; 2] = [
    static_part("bolt", Shade::Glow),
    static_part("core", Shade::Glow),
];
const BLOOD_CORE: [JointDef; 1] = [static_part("core", Shade::Glow)];
const JUMP_PAD: [JointDef; 3] = [
    static_part("base", Shade::Lit),
    static_part("rim", Shade::Lit),
    static_part("glow", Shade::Glow),
];
const CHECKPOINT: [JointDef; 3] = [
    static_part("base", Shade::Lit),
    static_part("lamp", Shade::Glow),
    static_part("halo", Shade::Glow),
];
const EXIT_GATE: [JointDef; 3] = [
    static_part("frame", Shade::Lit),
    static_part("portal", Shade::Glow),
    static_part("lights", Shade::Core),
];
const DOOR: [JointDef; 4] = [
    static_part("frame", Shade::Lit),
    static_part("panel_l", Shade::Lit),
    static_part("panel_r", Shade::Lit),
    static_part("light", Shade::Glow),
];

/// Part indices of the door model.
pub mod door {
    pub const FRAME: usize = 0;
    pub const PANEL_L: usize = 1;
    pub const PANEL_R: usize = 2;
    pub const LIGHT: usize = 3;
    /// Opening width (m) of the unit door; scale the model to fit a door brush.
    pub const WIDTH: f32 = 3.0;
    pub const HEIGHT: f32 = 3.5;
}

/// Part table of a projectile / prop model.
pub(crate) fn joints(id: ModelId) -> &'static [JointDef] {
    match id {
        ModelId::SpitterOrb => &SPITTER_ORB,
        ModelId::GnatBolt => &GNAT_BOLT,
        ModelId::BloodCore => &BLOOD_CORE,
        ModelId::JumpPad => &JUMP_PAD,
        ModelId::Checkpoint => &CHECKPOINT,
        ModelId::ExitGate => &EXIT_GATE,
        ModelId::Door => &DOOR,
        // Characters, weapons and player projectiles are handled by their own modules.
        _ => &BODY_GLOW,
    }
}

pub(crate) fn build(id: ModelId) -> Vec<ModelPart> {
    match id {
        ModelId::SpitterOrb => spitter_orb(),
        ModelId::GnatBolt => gnat_bolt(),
        ModelId::BloodCore => blood_core(),
        ModelId::JumpPad => jump_pad(),
        ModelId::Checkpoint => checkpoint(),
        ModelId::ExitGate => exit_gate(),
        ModelId::Door => door_model(),
        other => {
            // Characters, weapons and player projectiles are built by their own modules; never
            // reached through `ModelId::build`.
            log::error!("props::build called for non-prop model {other:?}");
            blood_core()
        }
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

// ---- projectiles --------------------------------------------------------------------------

fn spitter_orb() -> Vec<ModelPart> {
    let mut p = PartSet::new(&SPITTER_ORB);
    p.add(0, gem(Vec3::ZERO, 0.12, 1, Vec4::new(0.85, 1.0, 0.45, 1.0)));
    let mut shell = kit::ellipsoid(
        Vec3::new(0.0, 0.0, 0.03),
        Vec3::new(0.19, 0.19, 0.24),
        Quat::IDENTITY,
        5,
        9,
        Vec4::new(0.45, 0.9, 0.12, 1.0),
        true,
    );
    // Trailing drips.
    for (dx, dy) in [(0.07, 0.05), (-0.08, -0.03), (0.0, -0.09)] {
        add(
            &mut shell,
            spike(
                Vec3::new(dx, dy, 0.12),
                Vec3::new(dx * 1.4, dy * 1.4, 0.38),
                0.05,
                4,
                Vec4::new(0.35, 0.75, 0.1, 1.0),
            ),
        );
    }
    p.add(1, shell);
    p.color(1, Vec4::new(1.0, 1.0, 1.0, 0.6));
    p.texture(0, kit::tex_glow());
    p.texture(1, kit::tex_skin());
    p.finish()
}

fn gnat_bolt() -> Vec<ModelPart> {
    let mut p = PartSet::new(&GNAT_BOLT);
    p.add(
        0,
        kit::crystal(
            Vec3::new(0.0, 0.0, 0.05),
            Vec3::NEG_Z,
            (0.22, 0.12),
            0.06,
            6,
            Vec4::new(0.85, 0.35, 1.0, 1.0),
        ),
    );
    p.add(
        1,
        kit::crystal(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::NEG_Z,
            (0.12, 0.09),
            0.025,
            4,
            Vec4::new(1.0, 0.9, 1.0, 1.0),
        ),
    );
    p.color(0, Vec4::new(1.0, 1.0, 1.0, 0.75));
    p.texture(0, kit::tex_glow());
    p.texture(1, kit::tex_glow());
    p.finish()
}

fn blood_core() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BLOOD_CORE);
    let red = Vec4::new(0.95, 0.12, 0.1, 1.0);
    p.add(0, gem(Vec3::ZERO, 0.14, 1, red));
    for k in 0..8 {
        let d = Quat::from_rotation_y(k as f32 * 2.4)
            * Quat::from_rotation_z(0.4 + (k % 3) as f32 * 0.9)
            * Vec3::Y;
        p.add(
            0,
            spike(
                d * 0.1,
                d * (0.22 + (k % 2) as f32 * 0.08),
                0.04,
                3,
                Vec4::new(0.7, 0.05, 0.06, 1.0),
            ),
        );
    }
    p.texture(0, kit::tex_skin());
    p.finish()
}

// ---- props ----------------------------------------------------------------------------------

fn jump_pad() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JUMP_PAD);
    // Octagonal base with a recessed bowl.
    p.add(
        0,
        kit::loft(
            &[
                kit::ngon_ring(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.95),
                    8,
                    0.0,
                ),
                kit::ngon_ring(
                    Vec3::new(0.0, 0.16, 0.0),
                    Vec3::X,
                    Vec3::Z,
                    Vec2::splat(0.85),
                    8,
                    0.0,
                ),
            ],
            GUNMETAL,
            false,
            (false, true),
            kit::Uv::PerFace,
        ),
    );
    for k in 0..4 {
        let a = k as f32 / 4.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_4;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            0,
            block(
                d * 0.8 + Vec3::Y * 0.2,
                Vec3::new(0.08, 0.06, 0.08),
                Quat::from_rotation_y(-a),
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        1,
        kit::band(
            Vec3::new(0.0, 0.17, 0.0),
            Vec3::Y,
            0.72,
            0.03,
            0.14,
            16,
            Vec4::ONE,
        ),
    );
    // Glowing membrane and three chevrons pointing up.
    p.add(
        2,
        tube(
            Vec3::new(0.0, 0.15, 0.0),
            Vec3::new(0.0, 0.17, 0.0),
            0.62,
            0.62,
            16,
            Vec4::new(0.3, 0.95, 1.0, 1.0),
        ),
    );
    for k in 0..3 {
        let y = 0.35 + k as f32 * 0.3;
        let w = 0.32 - k as f32 * 0.06;
        let chevron = |sx: f32| {
            limb(
                Vec3::new(0.0, y + 0.12, 0.0),
                Vec3::new(sx * w, y, 0.0),
                Vec2::new(0.03, 0.03),
                Vec2::new(0.03, 0.03),
                Vec4::new(0.55, 1.0, 1.0, 1.0),
            )
        };
        p.add(2, chevron(1.0));
        p.add(2, chevron(-1.0));
    }
    finish_lit(&mut p);
    p.texture(0, kit::tex_plate());
    p.texture(1, kit::tex_hazard());
    p.texture(2, kit::tex_glow());
    p.finish()
}

fn checkpoint() -> Vec<ModelPart> {
    let mut p = PartSet::new(&CHECKPOINT);
    // Tripod base, pole and lamp cage.
    p.add(
        0,
        tube(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.12, 0.0),
            0.45,
            0.4,
            6,
            GUNMETAL,
        ),
    );
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            0,
            limb(
                d * 0.35 + Vec3::Y * 0.1,
                Vec3::new(0.0, 1.2, 0.0) + d * 0.06,
                Vec2::splat(0.035),
                Vec2::splat(0.025),
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        0,
        tube(
            Vec3::new(0.0, 0.1, 0.0),
            Vec3::new(0.0, 1.85, 0.0),
            0.05,
            0.04,
            6,
            CHROME,
        ),
    );
    p.add(
        0,
        tube(
            Vec3::new(0.0, 1.8, 0.0),
            Vec3::new(0.0, 1.88, 0.0),
            0.14,
            0.12,
            6,
            GUNMETAL,
        ),
    );
    for k in 0..4 {
        let a = k as f32 / 4.0 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            0,
            limb(
                Vec3::new(0.0, 1.86, 0.0) + d * 0.12,
                Vec3::new(0.0, 2.5, 0.0) + d * 0.1,
                Vec2::splat(0.015),
                Vec2::splat(0.015),
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        0,
        tube(
            Vec3::new(0.0, 2.48, 0.0),
            Vec3::new(0.0, 2.54, 0.0),
            0.14,
            0.08,
            6,
            GUNMETAL,
        ),
    );
    // Lamp crystal and a floating halo.
    p.add(
        1,
        kit::crystal(
            Vec3::new(0.0, 2.14, 0.0),
            Vec3::Y,
            (0.24, 0.3),
            0.13,
            6,
            Vec4::ONE,
        ),
    );
    p.add(
        2,
        kit::band(
            Vec3::new(0.0, 2.14, 0.0),
            Vec3::Y,
            0.36,
            0.05,
            0.03,
            12,
            Vec4::ONE,
        ),
    );
    finish_lit(&mut p);
    p.texture(0, kit::tex_plate());
    p.texture(1, kit::tex_glow());
    p.texture(2, kit::tex_glow());
    p.finish()
}

fn exit_gate() -> Vec<ModelPart> {
    // Pointed arch, 4 m wide opening, ~5.4 m tall.
    let mut p = PartSet::new(&EXIT_GATE);
    let stone = rgb(0x5a5450);
    let (w, h) = (2.0f32, 3.6f32);
    for sx in [-1.0f32, 1.0] {
        let mut pillar = MeshBuilder::new();
        add(
            &mut pillar,
            limb(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, h, 0.0),
                Vec2::new(0.32, 0.38),
                Vec2::new(0.28, 0.32),
                stone,
            ),
        );
        add(
            &mut pillar,
            spike(
                Vec3::new(0.0, h, 0.0),
                Vec3::new(sx * -0.2, h + 0.7, 0.0),
                0.12,
                4,
                BLACK_STEEL,
            ),
        );
        pillar.transform(Mat4::from_translation(Vec3::new(sx * (w + 0.3), 0.0, 0.0)));
        p.add(0, pillar);
        // Arch segments leaning in to a point.
        p.add(
            0,
            limb(
                Vec3::new(sx * (w + 0.2), h - 0.1, 0.0),
                Vec3::new(0.0, h + 1.6, 0.0),
                Vec2::new(0.3, 0.3),
                Vec2::new(0.22, 0.26),
                stone,
            ),
        );
    }
    p.add(
        0,
        spike(
            Vec3::new(0.0, h + 1.5, 0.0),
            Vec3::new(0.0, h + 2.3, 0.0),
            0.18,
            4,
            BLACK_STEEL,
        ),
    );
    // Threshold slab.
    p.add(
        0,
        block(
            Vec3::new(0.0, 0.06, 0.0),
            Vec3::new(w + 0.6, 0.06, 0.5),
            Quat::IDENTITY,
            GUNMETAL,
        ),
    );
    // Portal membrane (translucent glow) filling the arch.
    let mut portal = MeshBuilder::new();
    let outline = [
        Vec3::new(-w + 0.05, 0.12, 0.0),
        Vec3::new(w - 0.05, 0.12, 0.0),
        Vec3::new(w - 0.05, h - 0.1, 0.0),
        Vec3::new(0.0, h + 1.45, 0.0),
        Vec3::new(-w + 0.05, h - 0.1, 0.0),
    ];
    add(
        &mut portal,
        kit::plate(&outline, 0.04, Vec4::new(1.0, 0.25, 0.2, 1.0)),
    );
    p.add(1, portal);
    p.color(1, Vec4::new(1.0, 1.0, 1.0, 0.7));
    // Lamps up the pillars.
    for sx in [-1.0f32, 1.0] {
        for k in 0..4 {
            let y = 0.6 + k as f32 * 0.8;
            p.add(
                2,
                block(
                    Vec3::new(sx * (w + 0.3), y, -0.39),
                    Vec3::new(0.08, 0.12, 0.02),
                    Quat::IDENTITY,
                    Vec4::new(1.0, 0.85, 0.5, 1.0),
                ),
            );
        }
    }
    finish_lit(&mut p);
    p.texture(0, kit::tex_grime());
    p.texture(1, kit::tex_skin());
    p.texture(2, kit::tex_glow());
    p.finish()
}

fn door_model() -> Vec<ModelPart> {
    // Unit sliding door: opening door::WIDTH x door::HEIGHT, centred on X, 0.4 m deep frame.
    let mut p = PartSet::new(&DOOR);
    let (w, h) = (door::WIDTH * 0.5, door::HEIGHT);
    for sx in [-1.0f32, 1.0] {
        p.add(
            0,
            block(
                Vec3::new(sx * (w + 0.2), h * 0.5 + 0.15, 0.0),
                Vec3::new(0.2, h * 0.5 + 0.15, 0.25),
                Quat::IDENTITY,
                GUNMETAL,
            ),
        );
    }
    p.add(
        0,
        block(
            Vec3::new(0.0, h + 0.2, 0.0),
            Vec3::new(w + 0.4, 0.2, 0.25),
            Quat::IDENTITY,
            GUNMETAL,
        ),
    );
    // Panels meet in the middle with a jagged seam.
    let panel = |sx: f32| {
        let mut m = MeshBuilder::new();
        add(
            &mut m,
            block(
                Vec3::new(sx * w * 0.5, h * 0.5, 0.0),
                Vec3::new(w * 0.5, h * 0.5, 0.08),
                Quat::IDENTITY,
                rgb(0x6a6e72),
            ),
        );
        add(
            &mut m,
            block(
                Vec3::new(sx * w * 0.5, h * 0.62, -0.09),
                Vec3::new(w * 0.4, 0.12, 0.012),
                Quat::IDENTITY,
                rgb(0xe0aa1a),
            ),
        );
        add(
            &mut m,
            block(
                Vec3::new(sx * w * 0.5, h * 0.62, 0.09),
                Vec3::new(w * 0.4, 0.12, 0.012),
                Quat::IDENTITY,
                rgb(0xe0aa1a),
            ),
        );
        for k in 0..3 {
            let y = 0.5 + k as f32 * 1.1;
            add(
                &mut m,
                block(
                    Vec3::new(sx * 0.06, y, 0.0),
                    Vec3::new(0.06, 0.18, 0.085),
                    Quat::from_rotation_z(0.6 * sx),
                    rgb(0x6a6e72),
                ),
            );
        }
        m
    };
    p.add(door::PANEL_L, panel(-1.0));
    p.add(door::PANEL_R, panel(1.0));
    let light = block(
        Vec3::new(0.0, h + 0.2, -0.26),
        Vec3::new(0.4, 0.06, 0.02),
        Quat::IDENTITY,
        Vec4::ONE,
    );
    p.add(door::LIGHT, light);
    let side = block(
        Vec3::new(w + 0.2, h * 0.5, -0.26),
        Vec3::new(0.05, 0.3, 0.02),
        Quat::IDENTITY,
        Vec4::ONE,
    );
    add_mirrored(p.mesh(door::LIGHT), &side);
    finish_lit(&mut p);
    p.texture(door::FRAME, kit::tex_plate());
    p.texture(door::PANEL_L, kit::tex_plate());
    p.texture(door::PANEL_R, kit::tex_plate());
    p.texture(door::LIGHT, kit::tex_glow());
    p.finish()
}

// ---- prop poses -----------------------------------------------------------------------------

fn wave(time: f64, hz: f64) -> f32 {
    ((time * hz).fract() as f32 * std::f32::consts::TAU).sin()
}

/// Door pose: `open` 0 (closed) .. 1 (panels fully retracted into the frame). The status light
/// is red while closed / locked and green when open.
pub fn door_pose(open: f32, locked: bool) -> Pose {
    let mut pose = Pose::rest(DOOR.len());
    let o = open.clamp(0.0, 1.0) * door::WIDTH * 0.5;
    pose.set(
        door::PANEL_L,
        Mat4::from_translation(Vec3::new(-o, 0.0, 0.0)),
    );
    pose.set(
        door::PANEL_R,
        Mat4::from_translation(Vec3::new(o, 0.0, 0.0)),
    );
    pose.glow = if locked {
        Vec4::new(1.0, 0.12, 0.08, 1.0)
    } else if open > 0.5 {
        Vec4::new(0.3, 1.0, 0.35, 1.0)
    } else {
        Vec4::new(1.0, 0.7, 0.2, 1.0)
    };
    pose
}

/// Checkpoint beacon: dim red until reached, then bright cyan with a bobbing halo.
pub fn checkpoint_pose(active: bool, time: f64) -> Pose {
    let mut pose = Pose::rest(CHECKPOINT.len());
    let bob = 0.08 * wave(time, 0.5);
    let halo = Mat4::from_translation(Vec3::new(0.0, 2.14, 0.0))
        * Mat4::from_rotation_translation(
            Quat::from_rotation_y((time * 1.5 % 1000.0) as f32) * Quat::from_rotation_x(0.25),
            Vec3::new(0.0, bob, 0.0),
        )
        * Mat4::from_translation(Vec3::new(0.0, -2.14, 0.0));
    pose.set(2, halo);
    pose.glow = if active {
        Vec4::new(0.4, 1.0, 1.0, 1.0) * (1.1 + 0.2 * wave(time, 1.2))
    } else {
        Vec4::new(0.8, 0.12, 0.1, 1.0) * 0.6
    };
    pose.glow.w = 1.0;
    pose
}

/// Jump pad: chevrons pulse upwards.
pub fn jump_pad_pose(time: f64) -> Pose {
    let mut pose = Pose::rest(JUMP_PAD.len());
    let g = 1.0 + 0.3 * wave(time, 2.0);
    pose.glow = Vec4::new(g, g, g, 1.0);
    pose
}

/// Exit gate: the portal membrane shimmers.
pub fn exit_gate_pose(time: f64) -> Pose {
    let mut pose = Pose::rest(EXIT_GATE.len());
    let g = 1.0 + 0.25 * wave(time, 0.7) + 0.1 * wave(time, 3.1);
    pose.glow = Vec4::new(g, g, g, 1.0);
    pose
}
