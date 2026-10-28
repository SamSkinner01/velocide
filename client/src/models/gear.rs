//! The rework's ability gear in the world (docs/rework.md, area **client-b**): what the
//! Bolas, the Bulwark, the Lantern and the Kite put into the level. (The orbiting Wisp is
//! `ModelId::Wisp`; the drunk Kindle Flask is `ModelId::KindleFlask`.)
//!
//! - **Bolas** (`proj_bolas.gltf`): two balls of red sealing wax poured round lead weights,
//!   each with a brass cap where the cord is knotted, on a braided wick cord ~0.65 m long that
//!   smoulders (`Core`). Centred, the balls along ±X, the cord bowed back (+Z) as if trailing;
//!   the client spins it about +Y in flight.
//! - **Bulwark** (`prop_bulwark.gltf`): a pavise of hardened wax 2.2 × 1.8 m and ~0.1 m thick,
//!   bowed towards the front (-Z) and poured in eight staves (translucent honey wax: `slab`,
//!   with drips hanging from its foot), held by brass bands with rivets front and back
//!   (`frame`), a wick laid round its rim that burns (`rim`, `Glow`) and three stages of
//!   cracks glowing hot from inside (`crack_0..2`, `Glow`, on both faces) that
//!   [`bulwark_pose`] shows as it absorbs. Origin at the slab's centre.
//! - **Lantern** (`prop_lantern.gltf`): a brass carriage lantern ~0.6 m tall on three splayed
//!   iron feet: a hexagonal cage with glass panes (`glass`, translucent), a taper burning inside
//!   (`flame`, `Core`, flickers), a pointed roof with a ring, and a lens snout on its front
//!   (`lens`, `Glow`) it fires through. The cage (`head`) turns and tilts towards its target
//!   ([`lantern_pose`]); [`LANTERN_MUZZLE`] is the lens. Origin on the floor, facing -Z.
//! - **Kite** (`proj_kite.gltf`): a diamond kite ~0.9 m long, red oiled paper (`sail`) on a
//!   dark cane frame with brass ferrules (`frame`), its edges and an ember heart at the cross
//!   smouldering (`ember`, `Glow`), a tail of ochre ribbon bows (`tail`). Centred, flying
//!   along -Z with the sail level (a shallow V, its tips raised).

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::kit::{self, BRASS, IRON, block, gem, limb, rgb, tube};
use super::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};
use super::{ModelId, PartSet};

// ---------------------------------------------------------------------------------------------
// Part tables

const BOLAS: [JointDef; 2] = [
    static_part("balls", Shade::Lit),
    static_part("cord", Shade::Core),
];

pub const BULWARK_SLAB: usize = 0;
pub const BULWARK_HEART: usize = 1;
pub const BULWARK_FRAME: usize = 2;
pub const BULWARK_RIM: usize = 3;
/// The first of the three crack stages (`crack_0..2`).
pub const BULWARK_CRACK: usize = 4;
/// How many crack stages the slab shows.
pub const BULWARK_CRACKS: usize = 3;

const BULWARK: [JointDef; 7] = [
    static_part("slab", Shade::Lit),
    static_part("heart", Shade::Core),
    static_part("frame", Shade::Lit),
    static_part("rim", Shade::Glow),
    static_part("crack_0", Shade::Glow),
    static_part("crack_1", Shade::Glow),
    static_part("crack_2", Shade::Glow),
];

/// Width, height and thickness of the Bulwark's slab (docs/rework.md: about 2.2 × 1.8 m).
pub const BULWARK_SIZE: Vec3 = Vec3::new(2.2, 1.8, 0.1);
/// How far the slab's middle bows forward (-Z) of its edges.
const BULWARK_BOW: f32 = 0.14;

pub const LANTERN_BASE: usize = 0;
pub const LANTERN_HEAD: usize = 1;
pub const LANTERN_GLASS: usize = 2;
pub const LANTERN_FLAME: usize = 3;
pub const LANTERN_LENS: usize = 4;

/// The lantern's cage turns about this point (the top of its column).
const LANTERN_PIVOT: [f32; 3] = [0.0, 0.16, 0.0];
/// Base of the taper's flame (it flickers in height about it).
const LANTERN_WICK: [f32; 3] = [0.0, 0.27, 0.0];
/// The lens at the end of the snout, model space at rest: where its shots leave.
pub const LANTERN_MUZZLE: Vec3 = Vec3::new(0.0, 0.31, -0.215);

const LANTERN: [JointDef; 5] = [
    static_part("base", Shade::Lit),
    joint("head", ROOT, LANTERN_PIVOT, Shade::Lit),
    joint("glass", LANTERN_HEAD, LANTERN_PIVOT, Shade::Lit),
    joint("flame", LANTERN_HEAD, LANTERN_WICK, Shade::Core),
    joint("lens", LANTERN_HEAD, LANTERN_PIVOT, Shade::Glow),
];

const KITE: [JointDef; 4] = [
    static_part("frame", Shade::Lit),
    static_part("sail", Shade::Lit),
    static_part("ember", Shade::Glow),
    static_part("tail", Shade::Lit),
];

/// Part table of one of this module's models.
pub(crate) fn joints(id: ModelId) -> &'static [JointDef] {
    match id {
        ModelId::Bulwark => &BULWARK,
        ModelId::Lantern => &LANTERN,
        ModelId::Kite => &KITE,
        _ => &BOLAS,
    }
}

pub(crate) fn build(id: ModelId) -> Vec<ModelPart> {
    match id {
        ModelId::Bulwark => bulwark(),
        ModelId::Lantern => lantern(),
        ModelId::Kite => kite(),
        _ => bolas(),
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

/// Thin segments through `points` (a cord, a wick, an edge).
fn polyline(points: &[Vec3], half: f32, color: Vec4) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    for w in points.windows(2) {
        kit::add(
            &mut m,
            limb(w[0], w[1], Vec2::splat(half), Vec2::splat(half), color),
        );
    }
    m
}

/// A flat strip through `points` facing `face` (-1 = -Z, +1 = +Z): a crack drawn on a
/// surface (two triangles a segment).
fn strip(points: &[Vec3], half: f32, face: f32, color: Vec4) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    let normal = Vec3::Z * face;
    for w in points.windows(2) {
        let side = (w[1] - w[0]).cross(normal).normalize_or(Vec3::X) * half;
        let quad = [w[0] - side, w[0] + side, w[1] + side, w[1] - side];
        let base = m.vertices.len() as u32;
        for (q, uv) in quad
            .iter()
            .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        {
            m.vertex(*q, normal, Vec2::from(uv), color);
        }
        let facing = (quad[1] - quad[0]).cross(quad[2] - quad[0]).dot(normal) > 0.0;
        if facing {
            m.triangle(base, base + 1, base + 2);
            m.triangle(base, base + 2, base + 3);
        } else {
            m.triangle(base, base + 2, base + 1);
            m.triangle(base, base + 3, base + 2);
        }
    }
    m
}

// ---------------------------------------------------------------------------------------------
// Bolas

/// Where the bolas' balls sit (±X).
pub const BOLAS_REACH: f32 = 0.32;

fn bolas() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BOLAS);
    let wax = rgb(0x8e1c16);
    for side in [-1.0f32, 1.0] {
        let c = Vec3::new(side * BOLAS_REACH, 0.0, 0.0);
        p.add(
            0,
            kit::ellipsoid(
                c,
                Vec3::new(0.088, 0.082, 0.085),
                Quat::IDENTITY,
                4,
                8,
                wax,
                false,
            ),
        );
        // The brass cap the cord is knotted into, facing the other ball.
        p.add(
            0,
            tube(
                c - Vec3::X * side * 0.07,
                c - Vec3::X * side * 0.1,
                0.038,
                0.026,
                6,
                BRASS,
            ),
        );
        // Poured drips that set on the way round.
        for (a, l) in [(0.6f32, 0.05), (2.4, 0.04), (4.1, 0.06)] {
            let d = Vec3::new(0.0, a.sin(), a.cos());
            p.add(
                0,
                kit::spike(
                    c + d * 0.07,
                    c + d * (0.08 + l) + Vec3::X * side * 0.02,
                    0.022,
                    4,
                    wax,
                ),
            );
        }
    }
    // The braided wick cord, bowed back as it trails the spin, a few embers caught in it.
    let n = 8;
    let cord: Vec<Vec3> = (0..=n)
        .map(|k| {
            let x = (k as f32 / n as f32 * 2.0 - 1.0) * (BOLAS_REACH - 0.1);
            let u = x / (BOLAS_REACH - 0.1);
            Vec3::new(x, 0.01 * (k % 2) as f32, 0.07 * (1.0 - u * u))
        })
        .collect();
    let warm = Vec4::new(1.0, 0.72, 0.42, 1.0);
    p.add(1, polyline(&cord, 0.011, warm));
    for k in [2usize, 5, 7] {
        p.add(1, gem(cord[k], 0.02, 0, Vec4::new(1.0, 0.9, 0.6, 1.0)));
    }
    finish_lit(&mut p);
    p.texture(0, kit::tex_wax());
    p.texture(1, kit::tex_glow());
    p.finish()
}

// ---------------------------------------------------------------------------------------------
// Bulwark

/// The slab's front face (-Z of it) at `x` across: bowed forward in the middle.
fn bulwark_bow(x: f32) -> f32 {
    let u = (x / (BULWARK_SIZE.x * 0.5)).clamp(-1.0, 1.0);
    -BULWARK_BOW * (1.0 - u * u)
}

/// Half the slab's height at `x` across: the top corners are cut down.
fn bulwark_half_height(x: f32) -> f32 {
    let u = (x / (BULWARK_SIZE.x * 0.5)).abs().clamp(0.0, 1.0);
    BULWARK_SIZE.y * 0.5 - 0.14 * u * u * u
}

/// A point on the slab's surface: `x` across, `y` up, `face` -1 = front, +1 = back.
fn bulwark_point(x: f32, y: f32, face: f32) -> Vec3 {
    Vec3::new(x, y, bulwark_bow(x) + face * (BULWARK_SIZE.z * 0.5 + 0.006))
}

fn bulwark() -> Vec<ModelPart> {
    let mut p = PartSet::new(&BULWARK);
    let half_w = BULWARK_SIZE.x * 0.5;
    let half_t = BULWARK_SIZE.z * 0.5;
    let wax = Vec4::new(1.0, 0.9, 0.66, 1.0);
    // One poured slab following the bow (lofted across its width), its top corners cut down.
    let staves = 8;
    let stave = BULWARK_SIZE.x / staves as f32;
    let rings: Vec<Vec<Vec3>> = (0..=staves)
        .map(|k| {
            let x = -half_w + stave * k as f32;
            kit::rect_ring(
                Vec3::new(x, 0.0, bulwark_bow(x)),
                Vec3::Y,
                Vec3::Z,
                Vec2::new(bulwark_half_height(x), half_t),
            )
        })
        .collect();
    p.add(
        BULWARK_SLAB,
        kit::loft(&rings, wax, false, (true, true), kit::Uv::Density(1.5)),
    );
    // Drips hanging from the foot (and a few running down the front).
    for k in 0..staves {
        let x = -half_w + stave * (k as f32 + 0.5);
        let h = bulwark_half_height(x);
        let len = 0.07 + 0.09 * ((k * 7 % 4) as f32 / 3.0);
        let foot = Vec3::new(x + 0.05 * ((k % 3) as f32 - 1.0), -h, bulwark_bow(x));
        p.add(
            BULWARK_SLAB,
            kit::spike(foot + Vec3::Y * 0.02, foot - Vec3::Y * len, 0.04, 5, wax),
        );
    }
    for (x, y) in [(-0.62, 0.35), (0.41, 0.62), (0.8, -0.2)] {
        let at = bulwark_point(x, y, -1.0);
        p.add(
            BULWARK_SLAB,
            kit::ellipsoid(
                at - Vec3::Y * 0.12,
                Vec3::new(0.035, 0.15, 0.02),
                Quat::IDENTITY,
                3,
                5,
                wax,
                false,
            ),
        );
    }
    p.color(BULWARK_SLAB, Vec4::new(1.0, 0.95, 0.82, 0.78));
    // The heat still in it: a faint glow through the middle of the wax.
    let heart: Vec<Vec<Vec3>> = rings
        .iter()
        .enumerate()
        .map(|(k, r)| {
            let c = (r[0] + r[2]) * 0.5;
            let shrink = if k == 0 || k == staves { 0.35 } else { 0.86 };
            r.iter()
                .map(|q| c + (*q - c) * Vec3::new(1.0, shrink, 0.4))
                .collect()
        })
        .collect();
    p.add(
        BULWARK_HEART,
        kit::loft(
            &heart,
            Vec4::new(1.0, 0.62, 0.3, 1.0),
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.color(BULWARK_HEART, Vec4::new(1.0, 1.0, 1.0, 0.2));

    // Brass bands across both faces, riveted at every stave join; two iron grips behind.
    for y in [0.56f32, -0.56] {
        for face in [-1.0f32, 1.0] {
            for k in 0..staves {
                let x0 = -half_w + stave * k as f32 + 0.01;
                let x1 = x0 + stave - 0.02;
                p.add(
                    BULWARK_FRAME,
                    limb(
                        bulwark_point(x0, y, face),
                        bulwark_point(x1, y, face),
                        Vec2::new(0.012, 0.045),
                        Vec2::new(0.012, 0.045),
                        BRASS,
                    ),
                );
                if k % 2 == 0 {
                    p.add(
                        BULWARK_FRAME,
                        block(
                            bulwark_point(x0, y, face) + Vec3::Z * face * 0.012,
                            Vec3::splat(0.02),
                            Quat::from_rotation_z(std::f32::consts::FRAC_PI_4),
                            rgb(0xe0b860),
                        ),
                    );
                }
            }
        }
    }
    for x in [-0.3f32, 0.3] {
        let a = bulwark_point(x, 0.25, 1.0);
        let b = bulwark_point(x, -0.25, 1.0);
        p.add(
            BULWARK_FRAME,
            limb(
                a + Vec3::Z * 0.1,
                b + Vec3::Z * 0.1,
                Vec2::splat(0.022),
                Vec2::splat(0.022),
                IRON,
            ),
        );
        for e in [a, b] {
            p.add(
                BULWARK_FRAME,
                limb(
                    e,
                    e + Vec3::Z * 0.1,
                    Vec2::splat(0.018),
                    Vec2::splat(0.018),
                    IRON,
                ),
            );
        }
    }

    // The wick laid round the rim of the front face.
    let n = 12;
    let mut rim = Vec::with_capacity(n * 2 + 3);
    for k in 0..=n {
        let x = -half_w + BULWARK_SIZE.x * k as f32 / n as f32;
        rim.push(bulwark_point(
            x * 0.985,
            bulwark_half_height(x) - 0.03,
            -1.0,
        ));
    }
    for k in (0..=n).rev() {
        let x = -half_w + BULWARK_SIZE.x * k as f32 / n as f32;
        rim.push(bulwark_point(
            x * 0.985,
            -bulwark_half_height(x) + 0.03,
            -1.0,
        ));
    }
    rim.push(rim[0]);
    let hot = Vec4::new(1.0, 0.78, 0.45, 1.0);
    p.add(BULWARK_RIM, polyline(&rim, 0.016, hot));
    // Embers caught in the wick at the corners.
    for i in [0, n, n + 1, 2 * n + 1] {
        p.add(
            BULWARK_RIM,
            gem(rim[i], 0.035, 0, Vec4::new(1.0, 0.92, 0.65, 1.0)),
        );
    }

    // Cracks: jagged runs from three blows, each stage adding more and longer ones.
    let crack = Vec4::new(1.0, 0.62, 0.25, 1.0);
    let stages: [&[(f32, f32, f32, f32)]; BULWARK_CRACKS] = [
        &[
            (0.25, 0.1, 0.7, 0.42),
            (0.25, 0.1, 0.62, -0.32),
            (0.25, 0.1, -0.12, 0.38),
        ],
        &[
            (-0.45, -0.25, -0.95, -0.6),
            (-0.45, -0.25, -0.1, -0.7),
            (-0.45, -0.25, -0.85, 0.3),
            (0.7, 0.42, 1.0, 0.75),
            (0.62, -0.32, 0.95, -0.55),
        ],
        &[
            (-0.12, 0.38, -0.6, 0.82),
            (-0.12, 0.38, 0.3, 0.8),
            (-0.1, -0.7, 0.4, -0.82),
            (0.25, 0.1, -0.45, -0.25),
            (-0.85, 0.3, -1.05, 0.65),
            (0.95, -0.55, 0.6, -0.86),
        ],
    ];
    for (s, runs) in stages.iter().enumerate() {
        for (i, &(x0, y0, x1, y1)) in runs.iter().enumerate() {
            for face in [-1.0f32, 1.0] {
                let seed = (s * 31 + i * 7) as u32 + 3;
                let points: Vec<Vec3> = (0..=5)
                    .map(|k| {
                        let t = k as f32 / 5.0;
                        let wobble = if k == 0 || k == 5 {
                            0.0
                        } else {
                            ((seed as f32 * 1.7 + k as f32 * 2.3).sin()) * 0.06
                        };
                        let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
                        let (nx, ny) = (-(y1 - y0), x1 - x0);
                        let l = (nx * nx + ny * ny).sqrt().max(1e-3);
                        bulwark_point(x + nx / l * wobble, y + ny / l * wobble, face)
                    })
                    .collect();
                p.add(BULWARK_CRACK + s, strip(&points, 0.014, face, crack));
            }
        }
    }

    finish_lit(&mut p);
    p.texture(BULWARK_SLAB, kit::tex_wax());
    p.texture(BULWARK_FRAME, kit::tex_plate());
    for i in BULWARK_RIM..BULWARK.len() {
        p.texture(i, kit::tex_glow());
    }
    p.finish()
}

/// The Bulwark's pose: `absorbed` 0..1 of what it can take (the cracks show as it fills:
/// the first stage from a quarter, all of them from four fifths), `flash` 0..1 a blow just
/// absorbed (the rim and cracks flare), `left` 0..1 of its time up (the rim's wick burns lower
/// towards the end), `time` for the wick's flicker.
pub fn bulwark_pose(absorbed: f32, flash: f32, left: f32, time: f64) -> Pose {
    let mut pose = Pose::rest(BULWARK.len());
    let absorbed = super::weapons::unit(absorbed);
    for (s, from) in [0.25f32, 0.55, 0.8].into_iter().enumerate() {
        if absorbed < from {
            pose.hide(BULWARK_CRACK + s);
        }
    }
    let flicker = 0.9 + 0.1 * super::weapons::wave(time, 5.3) * super::weapons::wave(time, 2.1);
    let burn = 0.55 + 0.45 * super::weapons::unit(left).sqrt();
    let flash = super::weapons::unit(flash);
    let g = flicker * burn * (1.0 + 0.6 * absorbed) + 2.2 * flash;
    pose.glow = Vec4::new(g, g * 0.92, g * 0.85, 1.0);
    // The heat inside rises as it takes blows.
    let h = (0.5 + 0.5 * absorbed) * burn + flash;
    pose.core = Vec4::new(h, h, h, 1.0);
    pose
}

// ---------------------------------------------------------------------------------------------
// Lantern

fn lantern() -> Vec<ModelPart> {
    let mut p = PartSet::new(&LANTERN);
    let hex = |y: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, y, 0.0),
            Vec3::X,
            Vec3::Z,
            Vec2::splat(r),
            6,
            std::f32::consts::FRAC_PI_6,
        )
    };
    let dark_brass = rgb(0x9a6e2c);
    // Base: three splayed iron feet, a brass plate and a short column.
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + 0.5;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        p.add(
            LANTERN_BASE,
            limb(
                Vec3::new(0.0, 0.1, 0.0) + d * 0.05,
                d * 0.19 + Vec3::Y * 0.012,
                Vec2::new(0.014, 0.02),
                Vec2::new(0.012, 0.012),
                IRON,
            ),
        );
        p.add(
            LANTERN_BASE,
            gem(d * 0.19 + Vec3::Y * 0.012, 0.022, 0, kit::BLACK_STEEL),
        );
    }
    p.add(
        LANTERN_BASE,
        tube(
            Vec3::new(0.0, 0.08, 0.0),
            Vec3::new(0.0, 0.115, 0.0),
            0.11,
            0.1,
            8,
            dark_brass,
        ),
    );
    p.add(
        LANTERN_BASE,
        tube(
            Vec3::new(0.0, 0.11, 0.0),
            Vec3::new(0.0, 0.17, 0.0),
            0.035,
            0.03,
            6,
            BRASS,
        ),
    );
    // Head: the hexagonal cage (floor, posts, ceiling), a pointed roof with vents and a ring.
    p.add(
        LANTERN_HEAD,
        kit::loft(
            &[hex(0.16, 0.08), hex(0.19, 0.115), hex(0.2, 0.115)],
            BRASS,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    for k in 0..6 {
        let a = k as f32 / 6.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_6;
        let d = Vec3::new(a.cos(), 0.0, a.sin()) * 0.105;
        p.add(
            LANTERN_HEAD,
            limb(
                d + Vec3::Y * 0.2,
                d + Vec3::Y * 0.42,
                Vec2::splat(0.011),
                Vec2::splat(0.011),
                BRASS,
            ),
        );
    }
    p.add(
        LANTERN_HEAD,
        kit::loft(
            &[
                hex(0.42, 0.115),
                hex(0.44, 0.13),
                hex(0.47, 0.11),
                hex(0.53, 0.05),
                vec![Vec3::new(0.0, 0.575, 0.0)],
            ],
            dark_brass,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        LANTERN_HEAD,
        kit::band(
            Vec3::new(0.0, 0.615, 0.0),
            Vec3::X,
            0.04,
            0.012,
            0.01,
            8,
            BRASS,
        ),
    );
    // The taper on its spike inside.
    p.add(
        LANTERN_HEAD,
        tube(
            Vec3::new(0.0, 0.2, 0.0),
            Vec3::new(0.0, 0.265, 0.0),
            0.026,
            0.022,
            6,
            rgb(0xe8d8b0),
        ),
    );
    // The lens snout on the front (-Z), braced to the cage.
    let snout_y = LANTERN_MUZZLE.y;
    p.add(
        LANTERN_HEAD,
        tube(
            Vec3::new(0.0, snout_y, -0.08),
            Vec3::new(0.0, snout_y, -0.2),
            0.04,
            0.048,
            8,
            dark_brass,
        ),
    );
    p.add(
        LANTERN_HEAD,
        kit::band(
            Vec3::new(0.0, snout_y, -0.2),
            Vec3::Z,
            0.05,
            0.02,
            0.012,
            8,
            BRASS,
        ),
    );
    // Glass panes: a hexagonal prism inside the posts.
    p.add(
        LANTERN_GLASS,
        kit::loft(
            &[hex(0.2, 0.1), hex(0.42, 0.1)],
            Vec4::new(0.95, 0.9, 0.75, 1.0),
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    p.color(LANTERN_GLASS, Vec4::new(1.0, 0.92, 0.75, 0.38));
    // The flame: a teardrop over the wick and a bright bead in its heart.
    p.add(
        LANTERN_FLAME,
        kit::loft(
            &[
                hex(0.268, 0.014),
                hex(0.295, 0.042),
                hex(0.33, 0.034),
                vec![Vec3::new(0.0, 0.395, 0.0)],
            ],
            Vec4::new(1.0, 0.85, 0.55, 1.0),
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        LANTERN_LENS,
        kit::loft(
            &[
                kit::ngon_ring(
                    Vec3::new(0.0, snout_y, -0.2),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(0.038),
                    8,
                    0.0,
                ),
                vec![LANTERN_MUZZLE],
            ],
            Vec4::new(1.0, 0.95, 0.8, 1.0),
            false,
            (false, false),
            kit::Uv::PerFace,
        ),
    );
    finish_lit(&mut p);
    p.texture(LANTERN_BASE, kit::tex_plate());
    p.texture(LANTERN_HEAD, kit::tex_plate());
    p.texture(LANTERN_FLAME, kit::tex_glow());
    p.texture(LANTERN_LENS, kit::tex_glow());
    p.finish()
}

/// The lantern's pose: its cage turned by `yaw` (radians, + = left, like the look) and tilted
/// by `pitch` towards what it shoots at, `kick` 0..1 a shot just fired (the lens flares, the
/// cage jolts back), `color` its flame / lens (the carrier's primary colour), `left` 0..1 of
/// its time (the flame burns low and gutters near the end), `time` for the flicker.
pub fn lantern_pose(yaw: f32, pitch: f32, kick: f32, color: Vec3, left: f32, time: f64) -> Pose {
    use super::weapons::{unit, wave};
    let mut pose = Pose::rest(LANTERN.len());
    let kick = unit(kick);
    let yaw = if yaw.is_finite() { yaw } else { 0.0 };
    let pitch = if pitch.is_finite() {
        pitch.clamp(-0.6, 0.6)
    } else {
        0.0
    };
    pose.set(
        LANTERN_HEAD,
        Mat4::from_rotation_translation(
            Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch - 0.12 * kick),
            Vec3::ZERO,
        ),
    );
    let left = unit(left);
    let gutter = if left < 0.2 {
        // Guttering: the flame drops and flickers out.
        let k = left / 0.2;
        k * (0.6 + 0.4 * (wave(time, 11.0) * 0.5 + 0.5))
    } else {
        1.0
    };
    let flick = (1.0 + 0.12 * wave(time, 7.1) + 0.07 * wave(time + 0.3, 15.3)) * gutter;
    let h = flick.clamp(0.05, 2.0);
    pose.set(
        LANTERN_FLAME,
        Mat4::from_scale(Vec3::new(
            1.0 / h.max(0.4).sqrt(),
            h,
            1.0 / h.max(0.4).sqrt(),
        )),
    );
    // The taper burns warm whatever it fires; the lens takes the carrier's colour.
    let f = (1.3 * flick + 0.8 * kick).max(0.0);
    pose.core = Vec4::new(f, f * 0.9, f * 0.75, 1.0);
    let g = (1.1 * gutter + 2.2 * kick).max(0.0);
    let c = if color.is_finite() {
        color.max(Vec3::ZERO)
    } else {
        Vec3::ONE
    };
    pose.glow = (c * g).extend(1.0);
    pose
}

// ---------------------------------------------------------------------------------------------
// Kite

/// Kite outline (model space): nose, right tip, tail end of the spine, left tip.
const KITE_NOSE: Vec3 = Vec3::new(0.0, 0.0, -0.5);
const KITE_TIP: Vec3 = Vec3::new(0.38, 0.07, -0.14);
const KITE_TAIL: Vec3 = Vec3::new(0.0, 0.0, 0.4);
/// Where the ember heart sits (the spars' cross).
pub const KITE_HEART: Vec3 = Vec3::new(0.0, 0.03, -0.14);

fn kite() -> Vec<ModelPart> {
    let mut p = PartSet::new(&KITE);
    let mirror = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
    let paper = rgb(0xb4281c);
    let dark = rgb(0x5a1410);
    // The sail: four triangles of oiled paper (a shallow V), a darker painted ring between.
    for tip in [KITE_TIP, mirror(KITE_TIP)] {
        p.add(1, kit::plate(&[KITE_NOSE, tip, KITE_HEART], 0.012, paper));
        p.add(1, kit::plate(&[KITE_HEART, tip, KITE_TAIL], 0.012, paper));
    }
    let ring: Vec<Vec3> = (0..8)
        .map(|k| {
            let a = k as f32 / 8.0 * std::f32::consts::TAU;
            KITE_HEART + Vec3::new(a.cos() * 0.15, 0.012, a.sin() * 0.15)
        })
        .collect();
    p.add(1, kit::plate(&ring, 0.008, dark));
    // The frame: cane spine and cross spar, brass ferrules at their ends.
    let cane = rgb(0x4a3020);
    p.add(
        0,
        limb(
            KITE_NOSE - Vec3::Z * 0.03,
            KITE_TAIL + Vec3::Z * 0.03,
            Vec2::splat(0.012),
            Vec2::splat(0.01),
            cane,
        ),
    );
    p.add(
        0,
        limb(
            KITE_TIP + Vec3::X * 0.02,
            mirror(KITE_TIP) - Vec3::X * 0.02,
            Vec2::splat(0.011),
            Vec2::splat(0.011),
            cane,
        ),
    );
    for e in [KITE_NOSE, KITE_TIP, mirror(KITE_TIP), KITE_TAIL] {
        p.add(0, gem(e, 0.022, 0, BRASS));
    }
    // Smouldering edges and the ember heart.
    let ember = Vec4::new(1.0, 0.66, 0.3, 1.0);
    p.add(
        2,
        polyline(
            &[KITE_NOSE, KITE_TIP, KITE_TAIL, mirror(KITE_TIP), KITE_NOSE],
            0.01,
            ember,
        ),
    );
    p.add(
        2,
        gem(
            KITE_HEART + Vec3::Y * 0.02,
            0.055,
            1,
            Vec4::new(1.0, 0.88, 0.55, 1.0),
        ),
    );
    // The tail: ribbon bows down a cord behind.
    let ochre = rgb(0xd89a3a);
    let mut cord = vec![KITE_TAIL];
    for k in 1..=4 {
        let z = KITE_TAIL.z + 0.16 * k as f32;
        let x = 0.04 * if k % 2 == 0 { 1.0 } else { -1.0 };
        cord.push(Vec3::new(x, -0.01 * k as f32, z));
    }
    p.add(3, polyline(&cord, 0.005, rgb(0x6a5030)));
    for c in cord.iter().skip(1) {
        for side in [-1.0f32, 1.0] {
            p.add(
                3,
                kit::plate(
                    &[
                        *c,
                        *c + Vec3::new(side * 0.07, 0.0, -0.03),
                        *c + Vec3::new(side * 0.07, 0.0, 0.03),
                    ],
                    0.006,
                    ochre,
                ),
            );
        }
    }
    finish_lit(&mut p);
    p.texture(1, kit::tex_wrap());
    p.texture(2, kit::tex_glow());
    p.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulwark_cracks_show_in_stages() {
        let shown = |a: f32| {
            (0..BULWARK_CRACKS)
                .filter(|&s| !bulwark_pose(a, 0.0, 1.0, 0.0).is_hidden(BULWARK_CRACK + s))
                .count()
        };
        assert_eq!(shown(0.0), 0);
        assert_eq!(shown(0.3), 1);
        assert_eq!(shown(0.6), 2);
        assert_eq!(shown(1.0), 3);
        assert_eq!(shown(f32::NAN), 0);
        let calm = bulwark_pose(0.0, 0.0, 1.0, 0.0).glow;
        let hit = bulwark_pose(0.0, 1.0, 1.0, 0.0).glow;
        assert!(hit.x > calm.x + 1.0, "a blow flares the rim");
    }

    #[test]
    fn lantern_turns_towards_its_target_and_gutters_out() {
        let lit = lantern_pose(0.0, 0.0, 0.0, Vec3::ONE, 1.0, 0.25);
        let out = lantern_pose(0.0, 0.0, 0.0, Vec3::ONE, 0.0, 0.25);
        assert!(out.glow.x < 0.05 && lit.glow.x > 0.9);
        assert!(out.core.x < 0.05 && lit.core.x > 0.9);
        let turned = lantern_pose(std::f32::consts::FRAC_PI_2, 0.0, 0.0, Vec3::ONE, 1.0, 0.0);
        // The snout (-Z at rest) points along -X turned left by a quarter.
        let dir = turned.locals[LANTERN_HEAD].transform_vector3(Vec3::NEG_Z);
        assert!(dir.x < -0.99, "{dir}");
        let wild = lantern_pose(f32::NAN, f32::INFINITY, 9.0, Vec3::NAN, -1.0, 1e9);
        assert!(wild.is_finite());
    }
}
