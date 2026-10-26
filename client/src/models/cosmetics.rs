//! The Shrike's cosmetics (`shared::cosmetics`): every mask, crest, cloak and lantern emblem as
//! one parts kit (`shrike_kit.gltf`), drawn on top of the Shrike rig by [`draw`].
//!
//! Each kit part is built in the Shrike's model space (bind pose) and attached to a Shrike
//! joint ([`Attach`]): masks, eyes and most crests ride the head, emblems the torso, cloaks
//! hang from [`shrike::CLOAK_HANG`] behind the collar and swing with the coat tails
//! ([`shrike::Drape`]), a tassel crest swings from its knot. Parts never touch the hit shapes
//! (purely visual).
//!
//! Paints: masks, crests and trims are `Secondary` (the mask / trim colour), cloak bodies
//! `Primary` (the coat colour), the halo crest and the emblems `Flame` (they burn in the wick's
//! colour), and every mask's eye slits `TeamGlow`: the player's **identity** colour, so every
//! Shrike keeps a readable marker whatever it wears. Fixed-colour details (horns, grille bars,
//! the crown, the lantern cage, the emblem's medallion) are `Lit`.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::gfx::DrawItem;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::cosmetics::Cosmetics;

use super::PartSet;
use super::kit::{self, BRASS, IRON, IVORY, add, add_mirrored, block, limb, plate};
use super::rig::{JointDef, Pose, ROOT, Rig, Shade, Tint, joint};
use super::shrike::{self, CLOAK_HANG, Drape, HEAD_PIVOT, TASSEL_KNOT, TORSO_PIVOT};

/// What a kit part rides on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    Head,
    Torso,
    /// Hangs from [`CLOAK_HANG`] (torso), swinging with `Drape::cloak`.
    Cloak,
    /// Hangs from [`TASSEL_KNOT`] (head), swinging with `Drape::tassel`.
    Tassel,
}

impl Attach {
    /// The kit part's pivot (its mesh is stored relative to it).
    pub const fn pivot(self) -> [f32; 3] {
        match self {
            Attach::Head => HEAD_PIVOT,
            Attach::Torso => TORSO_PIVOT,
            Attach::Cloak => CLOAK_HANG,
            Attach::Tassel => TASSEL_KNOT,
        }
    }
}

const fn part(name: &'static str, attach: Attach, shade: Shade) -> (JointDef, Attach) {
    (joint(name, ROOT, attach.pivot(), shade), attach)
}

use Attach::{Cloak, Head, Tassel, Torso};
use Shade::{Flame, Lit, Primary, Secondary, TeamGlow};

/// First part of each group (indices into [`PARTS`]).
pub const MASK0: usize = 0;
pub const EYES0: usize = 8;
pub const HORNS: usize = 16;
pub const GRILLE_BARS: usize = 17;
pub const CROWN_SPIKES: usize = 18;
pub const LANTERN_CAGE: usize = 19;
/// Crest `c` (1-based: 0 = none) is part `CREST0 + c - 1`.
pub const CREST0: usize = 20;
/// Cloak `c` (1-based) is parts `CLOAK0 + 2 (c - 1)` (body) and `+ 1` (trim).
pub const CLOAK0: usize = 25;
pub const EMBLEM_PLATE: usize = 33;
/// Emblem `e` (1-based) is part `EMBLEM0 + e - 1`.
pub const EMBLEM0: usize = 34;

/// The kit's parts with what they attach to.
pub const PARTS: [(JointDef, Attach); 41] = [
    part("mask_shrike", Head, Secondary),
    part("mask_hookbill", Head, Secondary),
    part("mask_lantern", Head, Secondary),
    part("mask_visor", Head, Secondary),
    part("mask_horned", Head, Secondary),
    part("mask_weeper", Head, Secondary),
    part("mask_grille", Head, Secondary),
    part("mask_crown", Head, Secondary),
    part("eyes_shrike", Head, TeamGlow),
    part("eyes_hookbill", Head, TeamGlow),
    part("eyes_lantern", Head, TeamGlow),
    part("eyes_visor", Head, TeamGlow),
    part("eyes_horned", Head, TeamGlow),
    part("eyes_weeper", Head, TeamGlow),
    part("eyes_grille", Head, TeamGlow),
    part("eyes_crown", Head, TeamGlow),
    part("horns", Head, Lit),
    part("grille_bars", Head, Lit),
    part("crown_spikes", Head, Lit),
    part("lantern_cage", Head, Lit),
    part("crest_quills", Head, Secondary),
    part("crest_fin", Head, Secondary),
    part("crest_antlers", Head, Secondary),
    part("crest_tassel", Tassel, Secondary),
    part("crest_halo", Head, Flame),
    part("cloak_short", Cloak, Primary),
    part("cloak_short_trim", Cloak, Secondary),
    part("cloak_long", Cloak, Primary),
    part("cloak_long_trim", Cloak, Secondary),
    part("cloak_tattered", Cloak, Primary),
    part("cloak_tattered_trim", Cloak, Secondary),
    part("cloak_scarf", Cloak, Primary),
    part("cloak_scarf_trim", Cloak, Secondary),
    part("emblem_plate", Torso, Lit),
    part("emblem_bell", Torso, Flame),
    part("emblem_key", Torso, Flame),
    part("emblem_eye", Torso, Flame),
    part("emblem_spiral", Torso, Flame),
    part("emblem_fang", Torso, Flame),
    part("emblem_star", Torso, Flame),
    part("emblem_wing", Torso, Flame),
];

/// The kit's joint table (`ModelId::ShrikeKit`): every part a root part at its pivot.
pub const JOINTS: [JointDef; 41] = {
    let mut out = [PARTS[0].0; 41];
    let mut i = 0;
    while i < 41 {
        out[i] = PARTS[i].0;
        i += 1;
    }
    out
};

/// How far each cloak swings with the drape (a shoulder mantle barely moves).
const CLOAK_SWING: [f32; 4] = [0.25, 1.0, 1.0, 1.2];

/// The kit parts a look draws, in draw order (at most 6).
pub fn parts(c: &Cosmetics) -> Vec<usize> {
    let c = c.sanitized();
    let mut out = Vec::with_capacity(6);
    let mask = c.mask as usize;
    out.push(MASK0 + mask);
    out.push(EYES0 + mask);
    match mask {
        2 => out.push(LANTERN_CAGE),
        4 => out.push(HORNS),
        6 => out.push(GRILLE_BARS),
        7 => out.push(CROWN_SPIKES),
        _ => {}
    }
    if c.crest > 0 {
        out.push(CREST0 + c.crest as usize - 1);
    }
    if c.cloak > 0 {
        let first = CLOAK0 + 2 * (c.cloak as usize - 1);
        out.push(first);
        out.push(first + 1);
    }
    if c.emblem > 0 {
        out.push(EMBLEM_PLATE);
        out.push(EMBLEM0 + c.emblem as usize - 1);
    }
    out
}

/// Draws a look's kit parts on a posed Shrike: `shrike_world` are the Shrike rig's world
/// transforms for `pose` (`Rig::world_transforms`), `tint` already carries the look's paints
/// (`Tint::with_cosmetics`).
pub fn draw(
    kit: &Rig,
    look: &Cosmetics,
    shrike_world: &[Mat4],
    pose: &Pose,
    drape: &Drape,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) {
    let (Some(&head), Some(&torso)) = (
        shrike_world.get(shrike::HEAD),
        shrike_world.get(shrike::TORSO),
    ) else {
        return;
    };
    let offset = |a: [f32; 3], b: [f32; 3]| Mat4::from_translation(Vec3::from(a) - Vec3::from(b));
    let cloak = look.sanitized().cloak as usize;
    let swing = drape.cloak
        * CLOAK_SWING
            .get(cloak.wrapping_sub(1))
            .copied()
            .unwrap_or(1.0);
    for i in parts(look) {
        let world = match PARTS[i].1 {
            Head => head,
            Torso => torso,
            Cloak => {
                torso
                    * offset(CLOAK_HANG, TORSO_PIVOT)
                    * Mat4::from_quat(Quat::from_rotation_x(swing))
            }
            Tassel => {
                head * offset(TASSEL_KNOT, HEAD_PIVOT)
                    * Mat4::from_quat(Quat::from_rotation_x(drape.tassel))
            }
        };
        kit.draw_part(i, world, pose, tint, out);
    }
}

// ---------------------------------------------------------------------------------------------
// Geometry (Shrike model space, bind pose)

/// Painted parts are near white (the paint gives the hue); darker values for engraved lines.
const PAINT: Vec4 = Vec4::ONE;
const PAINT_SHADE: Vec4 = Vec4::new(0.8, 0.8, 0.8, 1.0);
const PAINT_DARK: Vec4 = Vec4::new(0.32, 0.32, 0.32, 1.0);
/// Glow parts: white (the identity / flame colour gives the hue).
const GLOW: Vec4 = Vec4::ONE;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// A face ring for masks: an 8-gon in the XY plane at depth `z`.
fn ring(z: f32, y: f32, w: f32, h: f32) -> Vec<Vec3> {
    kit::ngon_ring(v(0.0, y, z), Vec3::X, Vec3::Y, Vec2::new(w, h), 8, 0.0)
}

/// A mask shell lofted through `rings` (front last) to an `apex`.
fn shell(rings: Vec<Vec<Vec3>>, apex: Vec3) -> MeshBuilder {
    let mut rings = rings;
    rings.push(vec![apex]);
    let mut m = kit::loft(&rings, PAINT, false, (true, false), kit::Uv::PerFace);
    // Porcelain stays bright: a gentler bake than leather.
    m.shade_by_normal(1.0, 0.92, 0.72);
    m
}

/// A thin glowing stroke (eye slits, glyph lines).
fn stroke(a: Vec3, b: Vec3, w: f32, h: f32) -> MeshBuilder {
    limb(a, b, Vec2::new(w, h), Vec2::new(w * 0.85, h * 0.85), GLOW)
}

/// A round flat disc facing -Z (lenses, eye holes, medallions).
fn disc(center: Vec3, r: f32, sides: usize, thick: f32, color: Vec4) -> MeshBuilder {
    plate(
        &kit::ngon_ring(center, Vec3::X, Vec3::Y, Vec2::splat(r), sides, 0.0),
        thick,
        color,
    )
}

fn masks() -> [(MeshBuilder, MeshBuilder); 8] {
    [
        mask_shrike(),
        mask_hookbill(),
        mask_lantern(),
        mask_visor(),
        mask_horned(),
        mask_weeper(),
        mask_grille(),
        mask_crown(),
    ]
}

/// SHRIKE: the classic long hooked porcelain beak with a painted ridge.
fn mask_shrike() -> (MeshBuilder, MeshBuilder) {
    let mut m = shell(
        vec![
            ring(-0.03, 1.665, 0.098, 0.106),
            ring(-0.08, 1.668, 0.088, 0.097),
            ring(-0.125, 1.654, 0.058, 0.07),
            ring(-0.165, 1.632, 0.034, 0.046),
            ring(-0.215, 1.604, 0.021, 0.03),
            ring(-0.255, 1.575, 0.012, 0.018),
        ],
        v(0.0, 1.535, -0.272),
    );
    // The ridge down the beak in convex segments.
    let ridge = [
        v(0.0, 1.775, -0.02),
        v(0.0, 1.732, -0.125),
        v(0.0, 1.69, -0.168),
        v(0.0, 1.648, -0.215),
    ];
    for w in ridge.windows(2) {
        let drop = v(0.0, -0.017, 0.0);
        add(
            &mut m,
            plate(&[w[0], w[1], w[1] + drop, w[0] + drop], 0.02, PAINT_SHADE),
        );
    }
    let mut eyes = MeshBuilder::new();
    add_mirrored(
        &mut eyes,
        &stroke(
            v(0.079, 1.703, -0.074),
            v(0.034, 1.687, -0.122),
            0.009,
            0.012,
        ),
    );
    (m, eyes)
}

/// HOOKBILL: a short, heavy raptor beak hooking down under a jutting brow.
fn mask_hookbill() -> (MeshBuilder, MeshBuilder) {
    let mut m = shell(
        vec![
            ring(-0.03, 1.665, 0.1, 0.108),
            ring(-0.075, 1.668, 0.093, 0.1),
            ring(-0.112, 1.652, 0.072, 0.078),
            ring(-0.147, 1.622, 0.05, 0.056),
            ring(-0.172, 1.582, 0.033, 0.04),
            ring(-0.184, 1.542, 0.02, 0.026),
        ],
        v(0.0, 1.497, -0.168),
    );
    let brow = block(
        v(0.0, 1.724, -0.084),
        v(0.088, 0.016, 0.026),
        Quat::from_rotation_x(-0.25),
        PAINT_SHADE,
    );
    add(&mut m, brow);
    // Nostril notches.
    add_mirrored(
        &mut m,
        &block(
            v(0.024, 1.6, -0.165),
            v(0.007, 0.005, 0.01),
            Quat::IDENTITY,
            PAINT_DARK,
        ),
    );
    let mut eyes = MeshBuilder::new();
    add_mirrored(
        &mut eyes,
        &stroke(v(0.078, 1.698, -0.08), v(0.03, 1.684, -0.112), 0.011, 0.013),
    );
    (m, eyes)
}

/// LANTERN: a round face like a lantern's globe, one glowing lens behind a brass cage.
fn mask_lantern() -> (MeshBuilder, MeshBuilder) {
    let mut m = shell(
        vec![
            ring(-0.03, 1.665, 0.106, 0.112),
            ring(-0.085, 1.665, 0.1, 0.106),
            ring(-0.122, 1.665, 0.078, 0.084),
            ring(-0.14, 1.665, 0.046, 0.05),
        ],
        v(0.0, 1.665, -0.146),
    );
    // A sooty rim round the lens window.
    add(
        &mut m,
        disc(v(0.0, 1.672, -0.142), 0.046, 10, 0.006, PAINT_DARK),
    );
    let eyes = disc(v(0.0, 1.672, -0.149), 0.032, 10, 0.008, GLOW);
    (m, eyes)
}

/// The LANTERN mask's brass: ring, cage bars, a little chimney cap on top.
fn lantern_cage() -> MeshBuilder {
    let mut m = kit::band(v(0.0, 1.672, -0.15), Vec3::Z, 0.042, 0.012, 0.01, 10, BRASS);
    for x in [-0.07f32, -0.035, 0.035, 0.07] {
        let z = -0.13 + x.abs() * 0.6;
        add(
            &mut m,
            limb(
                v(x, 1.585, z),
                v(x, 1.755, z),
                Vec2::splat(0.006),
                Vec2::splat(0.006),
                BRASS,
            ),
        );
    }
    add(
        &mut m,
        kit::tube(
            v(0.0, 1.765, -0.02),
            v(0.0, 1.83, -0.02),
            0.03,
            0.022,
            6,
            BRASS,
        ),
    );
    add(
        &mut m,
        kit::tube(
            v(0.0, 1.83, -0.02),
            v(0.0, 1.85, -0.02),
            0.04,
            0.02,
            6,
            BRASS,
        ),
    );
    m
}

/// VISOR: a flat-fronted helm face with one glowing slit across it and a chin guard.
fn mask_visor() -> (MeshBuilder, MeshBuilder) {
    let rect = |z: f32, y: f32, w: f32, h: f32| {
        kit::chamfer_ring(v(0.0, y, z), Vec3::X, Vec3::Y, Vec2::new(w, h), 0.03)
    };
    let mut m = kit::loft(
        &[
            rect(-0.03, 1.665, 0.104, 0.112),
            rect(-0.1, 1.665, 0.098, 0.106),
            rect(-0.128, 1.66, 0.08, 0.09),
        ],
        PAINT,
        false,
        (true, true),
        kit::Uv::PerFace,
    );
    m.shade_by_normal(1.0, 0.92, 0.72);
    // A ridge down the middle and the chin guard.
    add(
        &mut m,
        block(
            v(0.0, 1.62, -0.131),
            v(0.008, 0.05, 0.006),
            Quat::IDENTITY,
            PAINT_SHADE,
        ),
    );
    add(
        &mut m,
        block(
            v(0.0, 1.567, -0.112),
            v(0.075, 0.024, 0.028),
            Quat::from_rotation_x(0.3),
            PAINT_SHADE,
        ),
    );
    let eyes = stroke(
        v(-0.074, 1.69, -0.132),
        v(0.074, 1.69, -0.132),
        0.008,
        0.007,
    );
    (m, eyes)
}

/// HORNED: a short ram-like snout with round eye holes (the horns are a separate part).
fn mask_horned() -> (MeshBuilder, MeshBuilder) {
    let m = shell(
        vec![
            ring(-0.03, 1.665, 0.1, 0.108),
            ring(-0.09, 1.66, 0.086, 0.096),
            ring(-0.13, 1.632, 0.056, 0.062),
            ring(-0.15, 1.603, 0.036, 0.036),
        ],
        v(0.0, 1.588, -0.158),
    );
    let mut eyes = MeshBuilder::new();
    add_mirrored(
        &mut eyes,
        &disc(v(0.046, 1.692, -0.107), 0.017, 6, 0.01, GLOW),
    );
    (m, eyes)
}

/// The HORNED mask's horns: curling back from the temples (bone).
fn horns() -> MeshBuilder {
    let path = [
        v(0.07, 1.725, -0.045),
        v(0.125, 1.79, 0.0),
        v(0.165, 1.81, 0.085),
        v(0.165, 1.765, 0.15),
        v(0.145, 1.715, 0.16),
    ];
    let mut horn = MeshBuilder::new();
    for (k, w) in path.windows(2).enumerate() {
        let r0 = 0.03 - k as f32 * 0.0065;
        let r1 = (r0 - 0.0065).max(0.006);
        add(
            &mut horn,
            limb(w[0], w[1], Vec2::splat(r0), Vec2::splat(r1), IVORY),
        );
    }
    let mut m = MeshBuilder::new();
    add_mirrored(&mut m, &horn);
    m
}

/// WEEPER: a long oval face with downturned eye slits and tear grooves.
fn mask_weeper() -> (MeshBuilder, MeshBuilder) {
    let mut m = shell(
        vec![
            ring(-0.03, 1.65, 0.098, 0.13),
            ring(-0.08, 1.646, 0.09, 0.124),
            ring(-0.11, 1.642, 0.07, 0.1),
            ring(-0.126, 1.64, 0.045, 0.064),
        ],
        v(0.0, 1.64, -0.131),
    );
    let groove = |a: Vec3, b: Vec3| {
        limb(
            a,
            b,
            Vec2::new(0.004, 0.003),
            Vec2::new(0.003, 0.003),
            PAINT_DARK,
        )
    };
    add_mirrored(
        &mut m,
        &groove(v(0.047, 1.672, -0.117), v(0.052, 1.585, -0.108)),
    );
    add_mirrored(
        &mut m,
        &groove(v(0.052, 1.585, -0.108), v(0.045, 1.55, -0.098)),
    );
    add(
        &mut m,
        block(
            v(0.0, 1.565, -0.122),
            v(0.022, 0.003, 0.004),
            Quat::IDENTITY,
            PAINT_DARK,
        ),
    );
    let mut eyes = MeshBuilder::new();
    let mut eye = stroke(
        v(0.078, 1.678, -0.092),
        v(0.052, 1.694, -0.112),
        0.008,
        0.008,
    );
    add(
        &mut eye,
        stroke(
            v(0.052, 1.694, -0.112),
            v(0.022, 1.686, -0.122),
            0.008,
            0.008,
        ),
    );
    add_mirrored(&mut eyes, &eye);
    (m, eyes)
}

/// GRILLE: a flat face plate with a glowing band behind iron bars (a separate part).
fn mask_grille() -> (MeshBuilder, MeshBuilder) {
    let rect = |z: f32, y: f32, w: f32, h: f32| {
        kit::chamfer_ring(v(0.0, y, z), Vec3::X, Vec3::Y, Vec2::new(w, h), 0.045)
    };
    let mut m = kit::loft(
        &[
            rect(-0.03, 1.665, 0.102, 0.112),
            rect(-0.09, 1.665, 0.096, 0.104),
            rect(-0.118, 1.662, 0.084, 0.09),
        ],
        PAINT,
        false,
        (true, true),
        kit::Uv::PerFace,
    );
    m.shade_by_normal(1.0, 0.92, 0.72);
    // The dark window the bars cross.
    add(
        &mut m,
        block(
            v(0.0, 1.665, -0.118),
            v(0.072, 0.05, 0.004),
            Quat::IDENTITY,
            PAINT_DARK,
        ),
    );
    let eyes = block(
        v(0.0, 1.69, -0.124),
        v(0.064, 0.012, 0.004),
        Quat::IDENTITY,
        GLOW,
    );
    (m, eyes)
}

fn grille_bars() -> MeshBuilder {
    let mut m = MeshBuilder::new();
    for x in [-0.06f32, -0.036, -0.012, 0.012, 0.036, 0.06] {
        add(
            &mut m,
            limb(
                v(x, 1.612, -0.13),
                v(x, 1.722, -0.13),
                Vec2::splat(0.0055),
                Vec2::splat(0.0055),
                IRON,
            ),
        );
    }
    for y in [1.612f32, 1.722] {
        add(
            &mut m,
            limb(
                v(-0.075, y, -0.129),
                v(0.075, y, -0.129),
                Vec2::splat(0.007),
                Vec2::splat(0.007),
                IRON,
            ),
        );
    }
    m
}

/// CROWN: a smooth, serene full face (the crown of spikes is a separate part).
fn mask_crown() -> (MeshBuilder, MeshBuilder) {
    let mut m = shell(
        vec![
            ring(-0.03, 1.66, 0.1, 0.118),
            ring(-0.085, 1.66, 0.092, 0.11),
            ring(-0.118, 1.655, 0.07, 0.088),
            ring(-0.132, 1.65, 0.04, 0.056),
        ],
        v(0.0, 1.65, -0.136),
    );
    // A painted brow line.
    add(
        &mut m,
        block(
            v(0.0, 1.726, -0.1),
            v(0.07, 0.004, 0.006),
            Quat::from_rotation_x(-0.5),
            PAINT_SHADE,
        ),
    );
    let mut eyes = MeshBuilder::new();
    add_mirrored(
        &mut eyes,
        &stroke(
            v(0.072, 1.697, -0.097),
            v(0.026, 1.69, -0.122),
            0.006,
            0.006,
        ),
    );
    (m, eyes)
}

fn crown_spikes() -> MeshBuilder {
    let mut m = kit::band(
        v(0.0, 1.735, 0.025),
        Vec3::Y,
        0.116,
        0.022,
        0.012,
        12,
        BRASS,
    );
    for k in 0..9 {
        let a = std::f32::consts::PI * (0.5 + (k as f32 - 4.0) * 0.2);
        let d = v(a.cos(), 0.0, -a.sin());
        let base = v(0.0, 1.745, 0.025) + d * 0.114;
        let len = if k == 4 {
            0.11
        } else {
            0.07 + 0.01 * (4 - (k - 4i32).abs()) as f32
        };
        add(
            &mut m,
            kit::spike(base, base + v(0.0, len, 0.0) + d * 0.02, 0.014, 4, BRASS),
        );
    }
    m
}

fn crests() -> [MeshBuilder; 5] {
    // QUILLS: the classic swept plume.
    let mut quills = MeshBuilder::new();
    for (k, side) in [(0.0f32, 0.0f32), (1.0, 0.035), (1.0, -0.035)] {
        add(
            &mut quills,
            plate(
                &[
                    v(side, 1.775, 0.0),
                    v(side * 1.4, 1.83 - k * 0.03, 0.12),
                    v(side * 1.8, 1.86 - k * 0.05, 0.26 - k * 0.04),
                    v(side * 1.3, 1.79 - k * 0.03, 0.15),
                ],
                0.014,
                PAINT,
            ),
        );
    }
    // FIN: one tall blade from the brow over the crown.
    let fin = plate(
        &[
            v(0.0, 1.74, -0.075),
            v(0.0, 1.84, -0.03),
            v(0.0, 1.915, 0.06),
            v(0.0, 1.905, 0.15),
            v(0.0, 1.8, 0.2),
            v(0.0, 1.765, 0.12),
        ],
        0.018,
        PAINT,
    );
    // ANTLERS: two branching beams.
    let mut antler = MeshBuilder::new();
    let beam = |a: Vec3, b: Vec3, r: f32| limb(a, b, Vec2::splat(r), Vec2::splat(r * 0.7), PAINT);
    add(
        &mut antler,
        beam(v(0.06, 1.75, 0.0), v(0.13, 1.86, 0.03), 0.014),
    );
    add(
        &mut antler,
        beam(v(0.13, 1.86, 0.03), v(0.19, 1.97, 0.09), 0.011),
    );
    add(
        &mut antler,
        beam(v(0.19, 1.97, 0.09), v(0.21, 2.03, 0.16), 0.008),
    );
    add(
        &mut antler,
        beam(v(0.13, 1.86, 0.03), v(0.12, 1.97, -0.02), 0.008),
    );
    add(
        &mut antler,
        beam(v(0.19, 1.97, 0.09), v(0.25, 2.03, 0.06), 0.007),
    );
    let mut antlers = MeshBuilder::new();
    add_mirrored(&mut antlers, &antler);
    // TASSEL: a top knot and a long tassel hanging down the back (swings from the knot).
    let k = Vec3::from(TASSEL_KNOT);
    let mut tassel = kit::gem(k, 0.026, 0, PAINT);
    add(
        &mut tassel,
        limb(
            k,
            k + v(0.0, -0.1, 0.13),
            Vec2::splat(0.008),
            Vec2::splat(0.008),
            PAINT_SHADE,
        ),
    );
    add(
        &mut tassel,
        limb(
            k + v(0.0, -0.1, 0.13),
            k + v(0.0, -0.26, 0.17),
            Vec2::new(0.022, 0.018),
            Vec2::new(0.04, 0.028),
            PAINT,
        ),
    );
    add(
        &mut tassel,
        kit::band(
            k + v(0.0, -0.11, 0.135),
            v(0.0, 0.6, -0.8),
            0.016,
            0.012,
            0.008,
            6,
            PAINT_SHADE,
        ),
    );
    // HALO: a burning ring hovering behind the crown.
    let halo = kit::band(
        v(0.0, 1.87, 0.07),
        v(0.0, 0.93, -0.36),
        0.13,
        0.018,
        0.014,
        16,
        GLOW,
    );
    [quills, fin, antlers, tassel, halo]
}

/// The four cloaks (body, trim), hanging straight down in bind pose from [`CLOAK_HANG`].
fn cloaks() -> [(MeshBuilder, MeshBuilder); 4] {
    let flat = |y: f32, z: f32, w: f32, d: f32| {
        kit::ngon_ring(v(0.0, y, z), Vec3::X, Vec3::Z, Vec2::new(w, d), 10, 0.0)
    };
    let sheet =
        |rings: &[Vec<Vec3>]| kit::loft(rings, PAINT, false, (true, true), kit::Uv::PerFace);

    // SHORT: a mantle round the shoulders, ending at the shoulder blades.
    let mut short = MeshBuilder::new();
    let mut short_trim = MeshBuilder::new();
    let arc = |r: f32, y: f32, a: f32| v(r * a.sin(), y, 0.02 + r * a.cos());
    let segs = 8;
    for k in 0..segs {
        let a0 = -1.95 + 3.9 * k as f32 / segs as f32;
        let a1 = -1.95 + 3.9 * (k + 1) as f32 / segs as f32;
        add(
            &mut short,
            plate(
                &[
                    arc(0.135, 1.53, a0),
                    arc(0.135, 1.53, a1),
                    arc(0.31, 1.3, a1),
                    arc(0.31, 1.3, a0),
                ],
                0.018,
                PAINT,
            ),
        );
        add(
            &mut short_trim,
            limb(
                arc(0.315, 1.295, a0),
                arc(0.315, 1.295, a1),
                Vec2::splat(0.012),
                Vec2::splat(0.012),
                PAINT,
            ),
        );
    }
    add(
        &mut short_trim,
        kit::gem(v(0.0, 1.5, -0.12), 0.02, 0, PAINT_SHADE),
    );

    // LONG: a heavy cloak to the calves, flaring a little.
    let mut long = sheet(&[
        flat(1.47, 0.02, 0.19, 0.03),
        flat(1.2, 0.04, 0.23, 0.034),
        flat(0.8, 0.06, 0.27, 0.04),
        flat(0.4, 0.09, 0.31, 0.045),
    ]);
    long.shade_by_normal(1.0, 0.86, 0.6);
    let mut long_trim = sheet(&[
        flat(0.43, 0.09, 0.315, 0.05),
        flat(0.37, 0.095, 0.32, 0.052),
    ]);
    add(
        &mut long_trim,
        sheet(&[flat(1.49, 0.02, 0.2, 0.036), flat(1.43, 0.025, 0.21, 0.038)]),
    );

    // TATTERED: a long cloak whose lower half hangs in torn strips.
    let mut tattered = sheet(&[
        flat(1.47, 0.02, 0.19, 0.03),
        flat(1.15, 0.04, 0.23, 0.034),
        flat(0.82, 0.055, 0.26, 0.038),
    ]);
    let strips = [
        (-0.22f32, 0.52f32),
        (-0.13, 0.36),
        (-0.04, 0.46),
        (0.05, 0.3),
        (0.14, 0.42),
        (0.22, 0.56),
    ];
    for (x, bottom) in strips {
        add(
            &mut tattered,
            limb(
                v(x, 0.84, 0.058 + 0.03),
                v(x * 1.08, bottom, 0.08 + 0.03),
                Vec2::new(0.042, 0.012),
                Vec2::new(0.022, 0.01),
                PAINT,
            ),
        );
    }
    tattered.shade_by_normal(1.0, 0.86, 0.6);
    let mut tattered_trim = sheet(&[
        flat(1.03, 0.05, 0.245, 0.042),
        flat(0.98, 0.052, 0.25, 0.043),
    ]);
    add(
        &mut tattered_trim,
        sheet(&[flat(1.49, 0.02, 0.2, 0.036), flat(1.43, 0.025, 0.21, 0.038)]),
    );

    // SCARF: two long tails trailing from the back of the neck.
    let mut scarf = MeshBuilder::new();
    let mut scarf_trim = MeshBuilder::new();
    for (x, bottom, lean) in [(-0.045f32, 0.78f32, -0.02f32), (0.05, 0.9, 0.03)] {
        let top = v(x, 1.47, 0.03);
        let end = v(x + lean, bottom, 0.07);
        add(
            &mut scarf,
            limb(
                top,
                end,
                Vec2::new(0.05, 0.009),
                Vec2::new(0.056, 0.008),
                PAINT,
            ),
        );
        // Two stripes near the end.
        for t in [0.8f32, 0.9] {
            let p = top.lerp(end, t);
            add(
                &mut scarf_trim,
                limb(
                    p + v(0.0, 0.015, 0.0),
                    p - v(0.0, 0.015, 0.0),
                    Vec2::new(0.058, 0.011),
                    Vec2::new(0.058, 0.011),
                    PAINT,
                ),
            );
        }
    }
    add(
        &mut scarf,
        kit::band(v(0.0, 1.47, 0.0), Vec3::Y, 0.12, 0.05, 0.03, 8, PAINT_SHADE),
    );
    [
        (short, short_trim),
        (long, long_trim),
        (tattered, tattered_trim),
        (scarf, scarf_trim),
    ]
}

/// The emblem medallion's centre on the front of the chest lantern.
const MEDALLION: [f32; 3] = [0.0, 1.19, -0.25];

/// The emblems: glyphs on the medallion, burning in the flame colour.
fn emblems() -> (MeshBuilder, [MeshBuilder; 7]) {
    let c = Vec3::from(MEDALLION);
    let z = c.z - 0.008;
    let at = |x: f32, y: f32| v(c.x + x, c.y + y, z);
    let mut medallion = disc(c, 0.05, 10, 0.008, BRASS);
    add(
        &mut medallion,
        kit::band(c, Vec3::Z, 0.05, 0.008, 0.012, 10, BRASS),
    );
    let line = |a: Vec3, b: Vec3| stroke(a, b, 0.004, 0.003);
    let flat = |pts: &[Vec3]| plate(pts, 0.004, GLOW);

    let mut bell = flat(&[
        at(-0.017, -0.012),
        at(0.017, -0.012),
        at(0.011, 0.02),
        at(-0.011, 0.02),
    ]);
    add(
        &mut bell,
        flat(&[
            at(-0.025, -0.019),
            at(0.025, -0.019),
            at(0.025, -0.012),
            at(-0.025, -0.012),
        ]),
    );
    add(&mut bell, disc(at(0.0, -0.026), 0.005, 6, 0.004, GLOW));
    add(
        &mut bell,
        kit::band(at(0.0, 0.026), Vec3::Z, 0.006, 0.003, 0.003, 6, GLOW),
    );

    let mut key = kit::band(at(0.0, 0.018), Vec3::Z, 0.011, 0.004, 0.004, 8, GLOW);
    add(&mut key, line(at(0.0, 0.007), at(0.0, -0.032)));
    add(&mut key, line(at(0.0, -0.022), at(0.012, -0.022)));
    add(&mut key, line(at(0.0, -0.031), at(0.009, -0.031)));

    let mut eye = MeshBuilder::new();
    for (a, b) in [
        (at(-0.03, 0.0), at(0.0, 0.016)),
        (at(0.0, 0.016), at(0.03, 0.0)),
        (at(0.03, 0.0), at(0.0, -0.016)),
        (at(0.0, -0.016), at(-0.03, 0.0)),
    ] {
        add(&mut eye, line(a, b));
    }
    add(&mut eye, disc(at(0.0, 0.0), 0.008, 8, 0.004, GLOW));

    let mut spiral = MeshBuilder::new();
    let point = |t: f32| {
        let r = 0.004 + 0.0028 * t;
        at(r * t.cos(), r * t.sin())
    };
    let steps = 18;
    for k in 0..steps {
        let t0 = k as f32 / steps as f32 * 3.4 * std::f32::consts::PI;
        let t1 = (k + 1) as f32 / steps as f32 * 3.4 * std::f32::consts::PI;
        add(&mut spiral, line(point(t0), point(t1)));
    }

    let mut fang = flat(&[at(-0.022, 0.02), at(-0.004, 0.02), at(-0.012, -0.03)]);
    add(
        &mut fang,
        flat(&[at(0.004, 0.02), at(0.022, 0.02), at(0.012, -0.03)]),
    );
    add(&mut fang, line(at(-0.026, 0.024), at(0.026, 0.024)));

    let mut star = MeshBuilder::new();
    let tip = |k: usize, r: f32| {
        let a = std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::TAU / 5.0;
        at(r * a.cos(), r * a.sin())
    };
    let inner: Vec<Vec3> = (0..5).map(|k| tip(k, 0.011)).collect();
    add(&mut star, flat(&inner));
    // Each point between two neighbouring corners of the inner pentagon.
    for k in 0..5 {
        let outer = {
            let a = std::f32::consts::FRAC_PI_2 + (k as f32 - 0.5) * std::f32::consts::TAU / 5.0;
            at(0.03 * a.cos(), 0.03 * a.sin())
        };
        add(&mut star, flat(&[inner[(k + 4) % 5], outer, inner[k]]));
    }

    let mut wing = MeshBuilder::new();
    for k in 0..3 {
        let a = 0.25 + k as f32 * 0.42;
        let len = 0.036 - k as f32 * 0.005;
        let root = at(-0.02, -0.015);
        let dir = v(a.cos(), a.sin(), 0.0);
        let side = v(-dir.y, dir.x, 0.0);
        add(
            &mut wing,
            flat(&[
                root,
                root + dir * len * 0.5 - side * 0.006,
                root + dir * len,
                root + dir * len * 0.5 + side * 0.006,
            ]),
        );
    }
    (medallion, [bell, key, eye, spiral, fang, star, wing])
}

/// Builds the kit's parts (`ModelId::ShrikeKit`).
pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    for (k, (mask, eyes)) in masks().into_iter().enumerate() {
        p.add(MASK0 + k, mask);
        p.add(EYES0 + k, eyes);
    }
    p.add(HORNS, horns());
    p.add(GRILLE_BARS, grille_bars());
    p.add(CROWN_SPIKES, crown_spikes());
    p.add(LANTERN_CAGE, lantern_cage());
    for (k, crest) in crests().into_iter().enumerate() {
        p.add(CREST0 + k, crest);
    }
    for (k, (body, trim)) in cloaks().into_iter().enumerate() {
        p.add(CLOAK0 + 2 * k, body);
        p.add(CLOAK0 + 2 * k + 1, trim);
    }
    let (medallion, glyphs) = emblems();
    p.add(EMBLEM_PLATE, medallion);
    for (k, glyph) in glyphs.into_iter().enumerate() {
        p.add(EMBLEM0 + k, glyph);
    }
    p.for_each_mesh(|i, m| {
        let shade = JOINTS[i].shade;
        // Masks carry their own porcelain bake.
        if !shade.is_emissive() && !(MASK0..EYES0).contains(&i) {
            kit::finish(m);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = match (j.shade, i) {
            (s, _) if s.is_emissive() => kit::tex_glow(),
            (_, i) if (MASK0..EYES0).contains(&i) => kit::tex_porcelain(),
            (_, HORNS) => kit::tex_bone(),
            (_, i) if (CLOAK0..EMBLEM_PLATE).contains(&i) => kit::tex_wrap(),
            (_, GRILLE_BARS | CROWN_SPIKES | LANTERN_CAGE | EMBLEM_PLATE) => kit::tex_plate(),
            _ => kit::tex_leather(),
        };
        p.texture(i, tex);
    }
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::cosmetics::{CLOAKS, CRESTS, EMBLEMS, MASKS};

    #[test]
    fn every_option_has_its_parts() {
        assert_eq!(MASK0 + MASKS.len(), EYES0);
        assert_eq!(CREST0 + CRESTS.len() - 1, CLOAK0);
        assert_eq!(CLOAK0 + 2 * (CLOAKS.len() - 1), EMBLEM_PLATE);
        assert_eq!(EMBLEM0 + EMBLEMS.len() - 1, PARTS.len());
        for (i, (j, a)) in PARTS.iter().enumerate() {
            assert_eq!(JOINTS[i].name, j.name);
            assert_eq!(j.pivot, Vec3::from(a.pivot()), "{}", j.name);
        }
        // Every option draws something distinct, nothing out of range.
        let mut seen = std::collections::BTreeSet::new();
        for m in 0..MASKS.len() as u8 {
            for c in 0..CRESTS.len() as u8 {
                for cl in 0..CLOAKS.len() as u8 {
                    for e in 0..EMBLEMS.len() as u8 {
                        let look = Cosmetics {
                            mask: m,
                            crest: c,
                            cloak: cl,
                            emblem: e,
                            ..Default::default()
                        };
                        let p = parts(&look);
                        assert!(p.iter().all(|i| *i < PARTS.len()));
                        assert!(
                            seen.insert(p),
                            "{look:?} draws the same parts as another look"
                        );
                    }
                }
            }
        }
        // Identity stays readable: every mask has glowing eyes in the team colour.
        for m in 0..MASKS.len() {
            assert_eq!(PARTS[EYES0 + m].0.shade, TeamGlow);
        }
    }
}
