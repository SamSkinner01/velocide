//! Modelling kit: low-poly mesh helpers (lofts, tapered limbs, faceted ellipsoids, spikes,
//! blades), the shared colour palette and the small procedural textures the models use.
//!
//! Every helper returns a [`MeshBuilder`] in *model space* (metres, +Y up, facing -Z) with
//! counter-clockwise front faces and normals that agree with the winding (the tests check it).
//! Textures are near-neutral detail maps (grime, brushed metal, reflection bands); the hue comes
//! from vertex colours so one texture serves many parts.

use engine::assets::meshgen::{self, BoxDesc, MeshBuilder};
use engine::assets::procedural::{Canvas, cellular, fbm, hex, rand01, value_noise2};
use engine::gfx::TextureData;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

// ---------------------------------------------------------------------------------------------
// Palette (sRGB 0..1, shaded in gamma space like everything else)

/// `0xRRGGBB` as an opaque colour.
pub fn rgb(c: u32) -> Vec4 {
    hex(c).extend(1.0)
}

pub fn grey(v: f32) -> Vec4 {
    Vec4::new(v, v, v, 1.0)
}

/// Polished chrome (multiplied with the reflection-band texture).
pub const CHROME: Vec4 = Vec4::new(0.92, 0.94, 0.98, 1.0);
/// Dark gunmetal for joints, grips and inner parts.
pub const GUNMETAL: Vec4 = Vec4::new(0.30, 0.31, 0.35, 1.0);
pub const BLACK_STEEL: Vec4 = Vec4::new(0.16, 0.16, 0.19, 1.0);
/// Blood-red lacquer (Shrike accents, default team colour).
pub const BLOOD: Vec4 = Vec4::new(0.78, 0.06, 0.08, 1.0);
pub const WHITE: Vec4 = Vec4::ONE;

// ---------------------------------------------------------------------------------------------
// Geometry helpers

/// How lofted surfaces get texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Uv {
    /// Each face / ring segment maps the whole texture once (good for reflection bands).
    PerFace,
    /// Texture repeats per metre along the surface.
    Density(f32),
}

/// A perpendicular frame `(side, depth)` for an axis direction: `side` is as close to +X as
/// possible, `depth = side x axis` (so a vertical axis gives side +X, depth +Z).
pub fn frame_for(axis: Vec3) -> (Vec3, Vec3) {
    let axis = axis.normalize_or(Vec3::Y);
    let reference = if axis.x.abs() > 0.9 {
        Vec3::NEG_Z
    } else {
        Vec3::X
    };
    let side = (reference - axis * reference.dot(axis)).normalize_or(Vec3::X);
    let depth = side.cross(axis).normalize_or(Vec3::Z);
    (side, depth)
}

/// Rectangle ring (4 points) around `center` in the plane spanned by `side`/`depth`.
pub fn rect_ring(center: Vec3, side: Vec3, depth: Vec3, half: Vec2) -> Vec<Vec3> {
    vec![
        center - side * half.x - depth * half.y,
        center + side * half.x - depth * half.y,
        center + side * half.x + depth * half.y,
        center - side * half.x + depth * half.y,
    ]
}

/// Regular n-gon ring with radii `r` along `side`/`depth`; `phase` rotates it (radians).
pub fn ngon_ring(
    center: Vec3,
    side: Vec3,
    depth: Vec3,
    r: Vec2,
    n: usize,
    phase: f32,
) -> Vec<Vec3> {
    (0..n)
        .map(|k| {
            let a = phase + k as f32 / n as f32 * std::f32::consts::TAU;
            center + side * (a.cos() * r.x) + depth * (a.sin() * r.y)
        })
        .collect()
}

/// Rectangle with cut corners (8 points): `half` sizes along `side` / `up`, `chamfer` = how
/// much of each corner is cut off.
pub fn chamfer_ring(center: Vec3, side: Vec3, up: Vec3, half: Vec2, chamfer: f32) -> Vec<Vec3> {
    let (w, h, c) = (half.x, half.y, chamfer.min(half.x).min(half.y));
    [
        (-w, -h + c),
        (-w + c, -h),
        (w - c, -h),
        (w, -h + c),
        (w, h - c),
        (w - c, h),
        (-w + c, h),
        (-w, h - c),
    ]
    .iter()
    .map(|&(x, y)| center + side * x + up * y)
    .collect()
}

/// Chamfered box running along -Z from `z0` to `z1` (weapon receivers, barrels shrouds).
/// `y` is the centre height, `half` = (half width, half height) at each end.
pub fn chamfer_box_z(
    y: f32,
    z0: f32,
    z1: f32,
    half0: Vec2,
    half1: Vec2,
    chamfer: f32,
    color: Vec4,
) -> MeshBuilder {
    loft(
        &[
            chamfer_ring(Vec3::new(0.0, y, z0), Vec3::X, Vec3::Y, half0, chamfer),
            chamfer_ring(Vec3::new(0.0, y, z1), Vec3::X, Vec3::Y, half1, chamfer),
        ],
        color,
        false,
        (true, true),
        Uv::PerFace,
    )
}

/// Polygon normal by Newell's method (robust with repeated vertices).
fn newell(points: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

fn centroid(points: &[Vec3]) -> Vec3 {
    points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32
}

/// Skins consecutive rings (same point count, or a single apex point) into a closed surface.
///
/// Winding is fixed up per face so normals always point away from the loft's axis, so ring
/// orientation does not matter. `smooth` gives radial vertex normals (organic shapes),
/// otherwise every face is flat (hard-surface look). Caps close the first / last ring.
pub fn loft(
    rings: &[Vec<Vec3>],
    color: Vec4,
    smooth: bool,
    caps: (bool, bool),
    uv: Uv,
) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    let n = rings.iter().map(Vec::len).max().unwrap_or(0);
    if rings.len() < 2 || n < 3 {
        return b;
    }
    let ring_at = |i: usize, k: usize| -> Vec3 {
        let r = &rings[i];
        if r.len() == 1 { r[0] } else { r[k % r.len()] }
    };
    let centers: Vec<Vec3> = rings.iter().map(|r| centroid(r)).collect();
    // Axial distance for UVs.
    let mut along = vec![0.0f32];
    for i in 1..rings.len() {
        along.push(along[i - 1] + centers[i].distance(centers[i - 1]));
    }
    let perimeter = |i: usize| -> f32 {
        (0..n)
            .map(|k| ring_at(i, k).distance(ring_at(i, k + 1)))
            .sum::<f32>()
    };
    let per = (0..rings.len()).map(perimeter).fold(0.0f32, f32::max);
    let uv_of = |i: usize, k: usize, corner: Vec2| -> Vec2 {
        match uv {
            Uv::PerFace => corner,
            Uv::Density(d) => Vec2::new(k as f32 / n as f32 * per * d, along[i] * d),
        }
    };
    // Normal for smooth shading: away from the local axis.
    let radial = |i: usize, p: Vec3| -> Vec3 {
        let c = centers[i];
        let axis = if i + 1 < rings.len() {
            centers[i + 1] - c
        } else {
            c - centers[i - 1]
        }
        .normalize_or(Vec3::Y);
        let d = p - c;
        let r = d - axis * d.dot(axis);
        if r.length_squared() > 1e-10 {
            r.normalize()
        } else {
            // Apex: point along the axis away from the neighbour ring.
            if i == 0 { -axis } else { axis }
        }
    };

    for i in 0..rings.len() - 1 {
        let mid = (centers[i] + centers[i + 1]) * 0.5;
        for k in 0..n {
            let quad = [
                ring_at(i, k),
                ring_at(i, k + 1),
                ring_at(i + 1, k + 1),
                ring_at(i + 1, k),
            ];
            let ids = [(i, k), (i, k + 1), (i + 1, k + 1), (i + 1, k)];
            let corners = [
                Vec2::new(0.0, 0.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ];
            let mut face = newell(&quad);
            let outward = centroid(&quad) - mid;
            let mut order = [0usize, 1, 2, 3];
            if face.dot(outward) < 0.0 {
                order = [0, 3, 2, 1];
                face = -face;
            }
            let face_n = face.normalize_or(outward.normalize_or(Vec3::Y));
            // Drop collapsed corners (apex rings) so we emit a triangle instead.
            let mut idx: Vec<usize> = Vec::with_capacity(4);
            for &o in &order {
                if idx
                    .last()
                    .is_none_or(|&l| quad[l].distance_squared(quad[o]) > 1e-12)
                {
                    idx.push(o);
                }
            }
            if idx.len() > 1
                && quad[idx[0]].distance_squared(quad[*idx.last().unwrap_or(&0)]) <= 1e-12
            {
                idx.pop();
            }
            if idx.len() < 3 {
                continue;
            }
            let base = b.vertices.len() as u32;
            for &o in &idx {
                let (ri, rk) = ids[o];
                let normal = if smooth { radial(ri, quad[o]) } else { face_n };
                b.vertex(quad[o], normal, uv_of(ri, rk, corners[o]), color);
            }
            for t in 1..idx.len() as u32 - 1 {
                b.triangle(base, base + t, base + t + 1);
            }
        }
    }
    let mut cap = |i: usize, away: Vec3| {
        let r = &rings[i];
        if r.len() < 3 {
            return;
        }
        let c = centers[i];
        let normal = newell(r);
        let flip = normal.dot(away) < 0.0;
        let normal = if flip { -normal } else { normal }.normalize_or(away);
        let base = b.vertices.len() as u32;
        b.vertex(c, normal, Vec2::splat(0.5), color);
        for (k, p) in r.iter().enumerate() {
            let a = k as f32 / r.len() as f32 * std::f32::consts::TAU;
            b.vertex(
                *p,
                normal,
                Vec2::new(0.5 + a.cos() * 0.5, 0.5 + a.sin() * 0.5),
                color,
            );
        }
        let m = r.len() as u32;
        for k in 0..m {
            let (p, q) = (base + 1 + k, base + 1 + (k + 1) % m);
            if flip {
                b.triangle(base, q, p);
            } else {
                b.triangle(base, p, q);
            }
        }
    };
    if caps.0 {
        let away = (centers[0] - centers[1]).normalize_or(Vec3::NEG_Y);
        cap(0, away);
    }
    if caps.1 {
        let l = rings.len() - 1;
        let away = (centers[l] - centers[l - 1]).normalize_or(Vec3::Y);
        cap(l, away);
    }
    b
}

/// Tapered box ("limb") from `a` to `b`; `ha`/`hb` are the half sizes (side, depth) at each
/// end. Flat shaded, capped.
pub fn limb(a: Vec3, b: Vec3, ha: Vec2, hb: Vec2, color: Vec4) -> MeshBuilder {
    let (side, depth) = frame_for(b - a);
    loft(
        &[rect_ring(a, side, depth, ha), rect_ring(b, side, depth, hb)],
        color,
        false,
        (true, true),
        Uv::PerFace,
    )
}

/// Like [`limb`] but with an explicit side axis (for twisted / non-vertical parts).
pub fn limb_oriented(
    a: Vec3,
    b: Vec3,
    side_hint: Vec3,
    ha: Vec2,
    hb: Vec2,
    color: Vec4,
) -> MeshBuilder {
    let axis = (b - a).normalize_or(Vec3::Y);
    let side = (side_hint - axis * side_hint.dot(axis)).normalize_or(frame_for(axis).0);
    let depth = side.cross(axis).normalize_or(Vec3::Z);
    loft(
        &[rect_ring(a, side, depth, ha), rect_ring(b, side, depth, hb)],
        color,
        false,
        (true, true),
        Uv::PerFace,
    )
}

/// Faceted n-sided tube from `a` to `b` with radii `ra` / `rb`.
pub fn tube(a: Vec3, b: Vec3, ra: f32, rb: f32, sides: usize, color: Vec4) -> MeshBuilder {
    let (side, depth) = frame_for(b - a);
    let phase = std::f32::consts::PI / sides as f32;
    loft(
        &[
            ngon_ring(a, side, depth, Vec2::splat(ra), sides, phase),
            ngon_ring(b, side, depth, Vec2::splat(rb), sides, phase),
        ],
        color,
        false,
        (true, true),
        Uv::PerFace,
    )
}

/// Smooth-shaded tube (barrels, organic limbs).
pub fn smooth_tube(a: Vec3, b: Vec3, ra: f32, rb: f32, sides: usize, color: Vec4) -> MeshBuilder {
    let (side, depth) = frame_for(b - a);
    loft(
        &[
            ngon_ring(a, side, depth, Vec2::splat(ra), sides, 0.0),
            ngon_ring(b, side, depth, Vec2::splat(rb), sides, 0.0),
        ],
        color,
        true,
        (true, true),
        Uv::Density(4.0),
    )
}

/// Pointed spike from `base` to `tip`.
pub fn spike(base: Vec3, tip: Vec3, radius: f32, sides: usize, color: Vec4) -> MeshBuilder {
    let (side, depth) = frame_for(tip - base);
    loft(
        &[
            ngon_ring(base, side, depth, Vec2::splat(radius), sides, 0.3),
            vec![tip],
        ],
        color,
        false,
        (true, false),
        Uv::PerFace,
    )
}

/// Ellipsoid (radii along X/Y/Z before `rotation`) with `rings` latitude bands.
pub fn ellipsoid(
    center: Vec3,
    radii: Vec3,
    rotation: Quat,
    rings: usize,
    sides: usize,
    color: Vec4,
    smooth: bool,
) -> MeshBuilder {
    let rings = rings.max(2);
    let mut list = Vec::with_capacity(rings + 1);
    for r in 0..=rings {
        let phi = r as f32 / rings as f32 * std::f32::consts::PI;
        let y = -phi.cos();
        let s = phi.sin();
        if r == 0 || r == rings {
            list.push(vec![center + rotation * Vec3::new(0.0, y * radii.y, 0.0)]);
        } else {
            let ring: Vec<Vec3> = (0..sides)
                .map(|k| {
                    let a =
                        (k as f32 + 0.5 * (r % 2) as f32) / sides as f32 * std::f32::consts::TAU;
                    center
                        + rotation
                            * Vec3::new(a.cos() * s * radii.x, y * radii.y, a.sin() * s * radii.z)
                })
                .collect();
            list.push(ring);
        }
    }
    loft(&list, color, smooth, (false, false), Uv::Density(3.0))
}

/// Oriented box.
pub fn block(center: Vec3, half: Vec3, rotation: Quat, color: Vec4) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    b.add_box(&BoxDesc {
        center,
        half_extents: half,
        rotation,
        color,
        ..Default::default()
    });
    b
}

/// Thin flat blade (extruded triangle-ish quad outline): `points` is a convex outline in a
/// plane, extruded by `thickness` along the plane normal.
pub fn plate(points: &[Vec3], thickness: f32, color: Vec4) -> MeshBuilder {
    let n = newell(points).normalize_or(Vec3::Z) * (thickness * 0.5);
    let front: Vec<Vec3> = points.iter().map(|p| *p + n).collect();
    let back: Vec<Vec3> = points.iter().map(|p| *p - n).collect();
    loft(&[back, front], color, false, (true, true), Uv::PerFace)
}

/// Mirrors a mesh across the YZ plane (left <-> right) keeping faces front-facing.
pub fn mirrored_x(b: &MeshBuilder) -> MeshBuilder {
    let mut out = MeshBuilder::new();
    out.append(&b.to_mesh(), Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
    out
}

/// Appends `b` and its X mirror into `out`.
pub fn add_mirrored(out: &mut MeshBuilder, b: &MeshBuilder) {
    out.append_builder(b, Mat4::IDENTITY);
    out.append(&b.to_mesh(), Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
}

/// Appends another builder unchanged.
pub fn add(out: &mut MeshBuilder, b: MeshBuilder) {
    out.append(&b.build(), Mat4::IDENTITY);
}

/// Cylinder between two points (meshgen's, smooth sides).
pub fn cyl(a: Vec3, b: Vec3, r: f32, segments: u32, color: Vec4) -> MeshBuilder {
    let mut m = MeshBuilder::new();
    m.add_cylinder_between(a, b, r, segments, color);
    m
}

/// A ring (torus-like band with a square cross-section) around `axis` at `center`.
pub fn band(
    center: Vec3,
    axis: Vec3,
    radius: f32,
    width: f32,
    thickness: f32,
    sides: usize,
    color: Vec4,
) -> MeshBuilder {
    let axis = axis.normalize_or(Vec3::Y);
    let (side, depth) = frame_for(axis);
    let mut b = MeshBuilder::new();
    for k in 0..sides {
        let a0 = k as f32 / sides as f32 * std::f32::consts::TAU;
        let a1 = (k + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let d0 = side * a0.cos() + depth * a0.sin();
        let d1 = side * a1.cos() + depth * a1.sin();
        let p0 = center + d0 * radius;
        let p1 = center + d1 * radius;
        let r = Vec2::new(thickness * 0.5, width * 0.5);
        let ring0 = rect_ring_basis(p0, d0, axis, r);
        let ring1 = rect_ring_basis(p1, d1, axis, r);
        // Each segment is a short box; loft fixes winding relative to the segment axis.
        add(
            &mut b,
            loft(&[ring0, ring1], color, false, (false, false), Uv::PerFace),
        );
    }
    b
}

fn rect_ring_basis(center: Vec3, u: Vec3, v: Vec3, half: Vec2) -> Vec<Vec3> {
    vec![
        center - u * half.x - v * half.y,
        center + u * half.x - v * half.y,
        center + u * half.x + v * half.y,
        center - u * half.x + v * half.y,
    ]
}

/// Bakes the usual "readable at low res" shading into vertex colours: lighter tops, darker
/// undersides.
pub fn finish(b: &mut MeshBuilder) {
    b.shade_by_normal(1.0, 0.86, 0.6);
}

/// Faceted icosphere at a position.
pub fn gem(center: Vec3, radius: f32, subdivisions: u32, color: Vec4) -> MeshBuilder {
    let mut g = meshgen::icosphere(radius, subdivisions, color);
    g.transform(Mat4::from_translation(center));
    g
}

/// Bipyramid crystal (prism shard) along `axis`: hexagonal waist, two pointed tips.
pub fn crystal(
    center: Vec3,
    axis: Vec3,
    half_len: (f32, f32),
    radius: f32,
    sides: usize,
    color: Vec4,
) -> MeshBuilder {
    let axis = axis.normalize_or(Vec3::Y);
    let (side, depth) = frame_for(axis);
    loft(
        &[
            vec![center - axis * half_len.0],
            ngon_ring(
                center,
                side,
                depth,
                Vec2::new(radius, radius * 0.55),
                sides,
                0.0,
            ),
            vec![center + axis * half_len.1],
        ],
        color,
        false,
        (false, false),
        Uv::PerFace,
    )
}

// ---------------------------------------------------------------------------------------------
// Textures (32 px unless noted; neutral detail, the hue comes from vertex colours)

fn opaque(c: Vec3) -> Vec4 {
    c.extend(1.0)
}

/// Mirror chrome: sky / horizon / ground reflection bands with brushed streaks.
pub fn tex_chrome() -> TextureData {
    const SEED: u32 = 701;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let streak = value_noise2(u, v, 2, 16, SEED);
        let warp = (fbm(u, v, 2, 2, SEED + 1) - 0.5) * 0.18;
        let t = v + warp;
        // Bright sky, sharp dark horizon, warm ground bounce.
        let band = if t < 0.42 {
            1.0 - t * 0.3
        } else if t < 0.5 {
            0.5
        } else if t < 0.56 {
            0.62
        } else {
            0.7 + (t - 0.56) * 0.35
        };
        let mut g = band * (0.9 + streak * 0.14);
        if rand01(x, y, SEED + 2) < 0.025 && t < 0.42 {
            g = 1.0;
        }
        opaque(Vec3::new(g, g, g * 1.02))
    });
    c.into_texture()
}

/// Brushed plate with a bevel border and two rivets (armour, weapon bodies).
pub fn tex_plate() -> TextureData {
    const SEED: u32 = 709;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let brushed = value_noise2(u, v, 3, 24, SEED);
        let grime = fbm(u, v, 4, 3, SEED + 1);
        let mut g = 0.74 + brushed * 0.16 - (grime - 0.55).max(0.0) * 0.6;
        if x == 0 || y == 0 {
            g = 0.98;
        } else if x == 31 || y == 31 {
            g = 0.38;
        }
        for (rx, ry) in [(4, 4), (27, 27)] {
            let d = Vec2::new((x - rx) as f32, (y - ry) as f32);
            if d.length() < 1.6 {
                g = if d.x + d.y < 0.0 { 1.0 } else { 0.35 };
            }
        }
        if rand01(x, y, SEED + 2) < 0.03 {
            g *= 0.75;
        }
        opaque(Vec3::splat(g))
    });
    c.into_texture()
}

/// Dirty pitted metal, darker and rougher than [`tex_plate`] (Gaoler iron, Censer brass).
pub fn tex_grime() -> TextureData {
    const SEED: u32 = 719;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 4, 4, SEED);
        let streak = value_noise2(u, v, 8, 2, SEED + 3);
        let mut g = 0.6 + n * 0.35 - streak * 0.12;
        if rand01(x, y, SEED + 1) < 0.06 {
            g *= 0.6;
        }
        opaque(Vec3::new(g * 1.03, g, g * 0.94))
    });
    c.into_texture()
}

/// Wet veined skin (neutral; tinted per enemy).
pub fn tex_skin() -> TextureData {
    const SEED: u32 = 727;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 4, 3, SEED);
        let vein = (fbm(u, v, 2, 4, SEED + 5) - 0.5).abs();
        let (f1, _, _) = cellular(u, v, 4, SEED + 9);
        let mut col = Vec3::splat(0.72 + n * 0.3) * (0.9 + f1 * 0.15);
        if vein < 0.03 {
            col = Vec3::new(0.42, 0.22, 0.3);
        }
        if rand01(x, y, SEED + 2) < 0.03 && n > 0.5 {
            col = Vec3::splat(1.0);
        }
        opaque(col)
    });
    c.into_texture()
}

/// Poured wax-flesh (the Moulds): smooth and pale with vertical drip runs, pits and hairline
/// cracks. Neutral; tinted per enemy by vertex colours.
pub fn tex_wax() -> TextureData {
    const SEED: u32 = 739;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 3, 3, SEED);
        // Drip runs: stretched noise along v.
        let drip = value_noise2(u, v, 9, 2, SEED + 4);
        let crack = (fbm(u, v, 2, 5, SEED + 7) - 0.5).abs();
        let mut g = 0.84 + n * 0.14 - (drip - 0.6).max(0.0) * 0.5;
        if crack < 0.018 {
            g *= 0.62;
        }
        if rand01(x, y, SEED + 1) < 0.025 {
            g *= 0.8;
        }
        opaque(Vec3::new(g * 1.02, g, g * 0.95))
    });
    c.into_texture()
}

/// Bone / ivory with cracks.
pub fn tex_bone() -> TextureData {
    const SEED: u32 = 733;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 4, 3, SEED);
        let crack = (fbm(u, v, 2, 4, SEED + 2) - 0.5).abs();
        let mut col = Vec3::new(0.93, 0.88, 0.76) * (0.8 + n * 0.25);
        if crack < 0.02 {
            col *= 0.5;
        }
        if rand01(x, y, SEED + 1) < 0.05 {
            col *= 0.82;
        }
        opaque(col)
    });
    c.into_texture()
}

/// Knurled rubber grip (16 px).
pub fn tex_grip() -> TextureData {
    let mut c = Canvas::new(16, 16);
    c.fill(|x, y, _, _| {
        let d = ((x + y) % 4 == 0) as i32 + ((x - y).rem_euclid(4) == 0) as i32;
        let g = match d {
            0 => 0.85,
            1 => 0.6,
            _ => 0.45,
        };
        opaque(Vec3::splat(g))
    });
    c.into_texture()
}

/// Yellow/black warning stripes (16 px, not neutral: use with white vertex colour).
pub fn tex_hazard() -> TextureData {
    const SEED: u32 = 743;
    let mut c = Canvas::new(16, 16);
    c.fill(|x, y, u, v| {
        let stripe = ((x + y) / 4) % 2 == 0;
        let grime = fbm(u, v, 2, 2, SEED);
        let base = if stripe { hex(0xe8b41c) } else { hex(0x1c1a18) };
        opaque(base * (0.8 + grime * 0.3))
    });
    c.into_texture()
}

/// Emissive core: hot centre lines (unlit parts). Neutral white-ish so vertex colour sets hue.
pub fn tex_glow() -> TextureData {
    const SEED: u32 = 751;
    let mut c = Canvas::new(16, 16);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 2, 3, SEED);
        let g = 0.7 + n * 0.3 + if rand01(x, y, SEED) < 0.08 { 0.2 } else { 0.0 };
        opaque(Vec3::splat(g.min(1.0)))
    });
    c.into_texture()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every triangle's winding agrees with its vertex normals, indices are valid and all
    /// values are finite.
    pub fn assert_valid(b: &engine::gfx::MeshData, name: &str) {
        assert!(!b.indices.is_empty(), "{name}: empty mesh");
        assert!(b.indices.len().is_multiple_of(3), "{name}: index count");
        assert!(
            b.indices.iter().all(|&i| (i as usize) < b.vertices.len()),
            "{name}: index out of range"
        );
        for v in &b.vertices {
            let ok = v
                .position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .chain(&v.color)
                .all(|x| x.is_finite());
            assert!(ok, "{name}: non-finite vertex");
            let n = Vec3::from(v.normal).length();
            assert!((n - 1.0).abs() < 1e-3, "{name}: normal not unit ({n})");
        }
        let mut bad = 0;
        let mut total = 0;
        for tri in b.indices.chunks_exact(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from(b.vertices[i as usize].position))
                .collect();
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            if face.length() < 1e-10 {
                continue;
            }
            total += 1;
            let n: Vec3 = tri
                .iter()
                .map(|&i| Vec3::from(b.vertices[i as usize].normal))
                .sum();
            if face.dot(n) <= 0.0 {
                bad += 1;
            }
        }
        assert!(
            bad == 0,
            "{name}: {bad}/{total} triangles wound against their normals"
        );
    }

    #[test]
    fn helpers_face_outwards() {
        let c = Vec4::ONE;
        for (name, m) in [
            (
                "limb",
                limb(
                    Vec3::ZERO,
                    Vec3::new(0.1, -0.5, 0.05),
                    Vec2::new(0.05, 0.04),
                    Vec2::new(0.03, 0.02),
                    c,
                ),
            ),
            (
                "limb_x",
                limb(Vec3::ZERO, Vec3::X, Vec2::splat(0.1), Vec2::splat(0.05), c),
            ),
            (
                "tube",
                tube(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), 0.1, 0.05, 6, c),
            ),
            (
                "smooth_tube",
                smooth_tube(Vec3::ZERO, Vec3::Y, 0.1, 0.1, 8, c),
            ),
            (
                "spike",
                spike(Vec3::ZERO, Vec3::new(0.2, 0.5, 0.0), 0.1, 5, c),
            ),
            (
                "ellipsoid",
                ellipsoid(
                    Vec3::ONE,
                    Vec3::new(0.3, 0.2, 0.4),
                    Quat::from_rotation_z(0.4),
                    4,
                    7,
                    c,
                    false,
                ),
            ),
            (
                "ellipsoid_smooth",
                ellipsoid(Vec3::ZERO, Vec3::ONE, Quat::IDENTITY, 5, 8, c, true),
            ),
            (
                "plate",
                plate(&[Vec3::ZERO, Vec3::X, Vec3::new(0.5, 1.0, 0.0)], 0.02, c),
            ),
            (
                "crystal",
                crystal(Vec3::ZERO, Vec3::Y, (0.3, 0.2), 0.1, 6, c),
            ),
            ("band", band(Vec3::ZERO, Vec3::Z, 0.3, 0.05, 0.02, 10, c)),
            (
                "mirrored",
                mirrored_x(&limb(
                    Vec3::X,
                    Vec3::new(1.0, 1.0, 0.0),
                    Vec2::splat(0.1),
                    Vec2::splat(0.1),
                    c,
                )),
            ),
        ] {
            assert_valid(&m.build(), name);
        }
    }

    #[test]
    fn textures_have_expected_sizes() {
        for t in [
            tex_chrome(),
            tex_plate(),
            tex_grime(),
            tex_skin(),
            tex_bone(),
            tex_grip(),
            tex_hazard(),
            tex_glow(),
        ] {
            assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize);
            assert!(t.width <= 64 && t.height <= 64);
        }
    }
}
