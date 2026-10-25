//! Procedural mesh building: boxes (level brushes with world-space UVs), cylinders, cones,
//! spheres, quads and wedges, plus helpers to combine meshes and bake vertex colours / fake AO.
//!
//! All primitives use counter-clockwise front faces (the renderer culls back faces) and carry
//! vertex colours, so weapons and enemies can be built from coloured parts without textures.

use glam::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

use crate::gfx::{MeshData, Vertex};

/// Box faces, for [`BoxDesc::faces`].
pub const FACE_POS_X: u8 = 1 << 0;
pub const FACE_NEG_X: u8 = 1 << 1;
pub const FACE_POS_Y: u8 = 1 << 2;
pub const FACE_NEG_Y: u8 = 1 << 3;
pub const FACE_POS_Z: u8 = 1 << 4;
pub const FACE_NEG_Z: u8 = 1 << 5;
pub const FACE_ALL: u8 = 0b11_1111;

/// Default level texel density: a 64 px texture repeats every 2 m.
pub const DEFAULT_TEXELS_PER_METRE: f32 = 32.0;

/// How a box's faces get texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxUv {
    /// Every face shows the whole texture once.
    PerFace,
    /// Planar mapping from world-space position, so adjacent brushes line up.
    /// `tiles_per_metre` = texture repeats per metre.
    World { tiles_per_metre: f32 },
}

impl BoxUv {
    /// World mapping at a texel density (e.g. 32 texels/m for a 64 px texture = 1 repeat / 2 m).
    pub fn texel_density(texels_per_metre: f32, texture_size: f32) -> BoxUv {
        BoxUv::World {
            tiles_per_metre: texels_per_metre / texture_size.max(1.0),
        }
    }
}

/// An oriented box (level brush).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxDesc {
    pub center: Vec3,
    pub half_extents: Vec3,
    pub rotation: Quat,
    pub color: Vec4,
    pub uv: BoxUv,
    /// Split faces into segments no longer than this (metres) so vertex lighting and fog look
    /// right on big surfaces. 0 = no subdivision.
    pub max_segment: f32,
    /// Which faces to emit (`FACE_*` bits).
    pub faces: u8,
}

impl Default for BoxDesc {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            half_extents: Vec3::splat(0.5),
            rotation: Quat::IDENTITY,
            color: Vec4::ONE,
            uv: BoxUv::PerFace,
            max_segment: 0.0,
            faces: FACE_ALL,
        }
    }
}

impl BoxDesc {
    /// Level brush: world UVs at the default texel density for a 64 px texture, faces
    /// subdivided every 2 m.
    pub fn brush(center: Vec3, half_extents: Vec3, rotation: Quat) -> Self {
        Self {
            center,
            half_extents,
            rotation,
            uv: BoxUv::texel_density(DEFAULT_TEXELS_PER_METRE, 64.0),
            max_segment: 2.0,
            ..Default::default()
        }
    }
}

/// Accumulates vertices and triangles.
#[derive(Clone, Debug, Default)]
pub struct MeshBuilder {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl From<MeshData> for MeshBuilder {
    fn from(data: MeshData) -> Self {
        Self {
            vertices: data.vertices,
            indices: data.indices,
        }
    }
}

impl MeshBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(self) -> MeshData {
        MeshData {
            vertices: self.vertices,
            indices: self.indices,
        }
    }

    /// A copy of the current contents as mesh data.
    pub fn to_mesh(&self) -> MeshData {
        MeshData {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex(&mut self, position: Vec3, normal: Vec3, uv: Vec2, color: Vec4) -> u32 {
        self.vertices.push(Vertex {
            position: position.to_array(),
            normal: normal.to_array(),
            uv: uv.to_array(),
            color: color.to_array(),
        });
        self.vertices.len() as u32 - 1
    }

    pub fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Flat quad; `corners` counter-clockwise as seen from the front.
    pub fn quad(&mut self, corners: [Vec3; 4], uvs: [Vec2; 4], color: Vec4) {
        let normal = (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .try_normalize()
            .unwrap_or(Vec3::Y);
        let base = self.vertices.len() as u32;
        for (p, uv) in corners.iter().zip(uvs) {
            self.vertex(*p, normal, uv, color);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Flat triangle with a computed face normal (counter-clockwise from the front).
    pub fn flat_triangle(&mut self, p: [Vec3; 3], uvs: [Vec2; 3], color: Vec4) {
        let normal = (p[1] - p[0])
            .cross(p[2] - p[0])
            .try_normalize()
            .unwrap_or(Vec3::Y);
        let base = self.vertices.len() as u32;
        for (pos, uv) in p.iter().zip(uvs) {
            self.vertex(*pos, normal, uv, color);
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Adds an oriented, optionally subdivided box.
    pub fn add_box(&mut self, desc: &BoxDesc) {
        // (normal, right, up) per face; right x up = normal, so corners wind CCW.
        const FACES: [(u8, Vec3, Vec3, Vec3); 6] = [
            (FACE_POS_X, Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (FACE_NEG_X, Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (FACE_POS_Y, Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (FACE_NEG_Y, Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (FACE_POS_Z, Vec3::Z, Vec3::X, Vec3::Y),
            (FACE_NEG_Z, Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        let h = desc.half_extents.abs();
        for (bit, n, r, t) in FACES {
            if desc.faces & bit == 0 {
                continue;
            }
            let (hr, ht) = ((r * h).abs().max_element(), (t * h).abs().max_element());
            let face_center = n * (n * h).abs().max_element();
            let segments = |len: f32| {
                if desc.max_segment > 0.0 {
                    ((len / desc.max_segment).ceil() as u32).clamp(1, 256)
                } else {
                    1
                }
            };
            let (nr, nt) = (segments(hr * 2.0), segments(ht * 2.0));
            let (rw, tw, nw) = (desc.rotation * r, desc.rotation * t, desc.rotation * n);
            let base = self.vertices.len() as u32;
            for j in 0..=nt {
                for i in 0..=nr {
                    let (fu, fv) = (i as f32 / nr as f32, j as f32 / nt as f32);
                    let local =
                        face_center + r * (hr * (fu * 2.0 - 1.0)) + t * (ht * (fv * 2.0 - 1.0));
                    let world = desc.center + desc.rotation * local;
                    let uv = match desc.uv {
                        BoxUv::PerFace => Vec2::new(fu, 1.0 - fv),
                        BoxUv::World { tiles_per_metre } => {
                            Vec2::new(world.dot(rw), -world.dot(tw)) * tiles_per_metre
                        }
                    };
                    self.vertex(world, nw, uv, desc.color);
                }
            }
            let row = nr + 1;
            for j in 0..nt {
                for i in 0..nr {
                    let a = base + j * row + i;
                    let (b, c, d) = (a + 1, a + row + 1, a + row);
                    self.indices.extend_from_slice(&[a, b, c, a, c, d]);
                }
            }
        }
    }

    /// Appends another mesh transformed by `transform` (normals use the inverse transpose;
    /// mirroring transforms flip the winding so faces stay front-facing).
    pub fn append(&mut self, other: &MeshData, transform: Mat4) {
        let normal_matrix = Mat3::from_mat4(transform).inverse().transpose();
        let flip = transform.determinant() < 0.0;
        let base = self.vertices.len() as u32;
        for v in &other.vertices {
            let mut v = *v;
            v.position = transform
                .transform_point3(Vec3::from(v.position))
                .to_array();
            v.normal = (normal_matrix * Vec3::from(v.normal))
                .try_normalize()
                .unwrap_or(Vec3::Y)
                .to_array();
            self.vertices.push(v);
        }
        for tri in other.indices.chunks_exact(3) {
            if flip {
                self.indices
                    .extend_from_slice(&[base + tri[0], base + tri[2], base + tri[1]]);
            } else {
                self.indices
                    .extend_from_slice(&[base + tri[0], base + tri[1], base + tri[2]]);
            }
        }
    }

    pub fn append_builder(&mut self, other: &MeshBuilder, transform: Mat4) {
        self.append(&other.to_mesh(), transform);
    }

    /// Transforms every vertex in place.
    pub fn transform(&mut self, transform: Mat4) {
        let data = MeshData {
            vertices: std::mem::take(&mut self.vertices),
            indices: std::mem::take(&mut self.indices),
        };
        self.append(&data, transform);
    }

    /// Adds a cylinder whose axis runs from `a` to `b`.
    pub fn add_cylinder_between(
        &mut self,
        a: Vec3,
        b: Vec3,
        radius: f32,
        segments: u32,
        color: Vec4,
    ) {
        let axis = b - a;
        let len = axis.length();
        if len <= 1e-6 {
            return;
        }
        let rotation = Quat::from_rotation_arc(Vec3::Y, axis / len);
        let transform = Mat4::from_rotation_translation(rotation, (a + b) * 0.5);
        self.append(&cylinder(radius, len, segments, color).build(), transform);
    }

    // ---- colour helpers -------------------------------------------------------------------

    pub fn set_color(&mut self, color: Vec4) {
        for v in &mut self.vertices {
            v.color = color.to_array();
        }
    }

    pub fn multiply_color(&mut self, color: Vec4) {
        for v in &mut self.vertices {
            v.color = (Vec4::from(v.color) * color).to_array();
        }
    }

    /// Replaces every vertex colour with `f(vertex)`.
    pub fn map_colors(&mut self, f: impl Fn(&Vertex) -> Vec4) {
        for v in &mut self.vertices {
            v.color = f(v).to_array();
        }
    }

    /// Multiplies colours by a bottom-to-top gradient over the mesh's height.
    pub fn vertical_gradient(&mut self, bottom: Vec4, top: Vec4) {
        let (lo, hi) = self.bounds();
        let height = (hi.y - lo.y).max(1e-6);
        for v in &mut self.vertices {
            let t = (v.position[1] - lo.y) / height;
            v.color = (Vec4::from(v.color) * bottom.lerp(top, t)).to_array();
        }
    }

    /// Bakes simple directional shading into the colours: faces pointing up get `top`, sideways
    /// `side`, down `bottom` (blended by the normal's Y).
    pub fn shade_by_normal(&mut self, top: f32, side: f32, bottom: f32) {
        for v in &mut self.vertices {
            let ny = v.normal[1];
            let f = if ny >= 0.0 {
                side + (top - side) * ny
            } else {
                side + (bottom - side) * -ny
            };
            let c = Vec4::from(v.color);
            v.color = (c.truncate() * f).extend(c.w).to_array();
        }
    }

    /// Fake ambient occlusion: darkens vertices close above `floor_y` (fading out over
    /// `height` metres) by up to `strength` (0..1).
    pub fn ao_near_floor(&mut self, floor_y: f32, height: f32, strength: f32) {
        for v in &mut self.vertices {
            let t = ((v.position[1] - floor_y) / height.max(1e-6)).clamp(0.0, 1.0);
            let f = 1.0 - strength * (1.0 - t) * (1.0 - t);
            let c = Vec4::from(v.color);
            v.color = (c.truncate() * f).extend(c.w).to_array();
        }
    }

    /// Fake ambient occlusion from occluder spheres (centre, radius): vertices inside a sphere
    /// are darkened by up to `strength`, more towards its centre.
    pub fn ao_spheres(&mut self, occluders: &[(Vec3, f32)], strength: f32) {
        for v in &mut self.vertices {
            let p = Vec3::from(v.position);
            let mut occlusion = 0.0f32;
            for &(center, radius) in occluders {
                let t = 1.0 - (p.distance(center) / radius.max(1e-6)).min(1.0);
                occlusion = occlusion.max(t);
            }
            let f = 1.0 - strength * occlusion;
            let c = Vec4::from(v.color);
            v.color = (c.truncate() * f).extend(c.w).to_array();
        }
    }

    /// Deterministic per-vertex brightness jitter (`amount` = max change, e.g. 0.1), keyed by
    /// the vertex position so shared corners get the same value.
    pub fn jitter_colors(&mut self, seed: u32, amount: f32) {
        for v in &mut self.vertices {
            let key = Vec3::from(v.position) * 64.0;
            let r = super::procedural::rand01(
                key.x as i32 ^ (key.z as i32).rotate_left(11),
                key.y as i32,
                seed,
            );
            let f = 1.0 + (r * 2.0 - 1.0) * amount;
            let c = Vec4::from(v.color);
            v.color = (c.truncate() * f).extend(c.w).to_array();
        }
    }

    /// Axis-aligned bounds `(min, max)`; zero for an empty mesh.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        bounds(&self.vertices)
    }

    /// Reverses the winding of every triangle (and the normals), turning the mesh inside out.
    pub fn flip(&mut self) {
        for tri in self.indices.chunks_exact_mut(3) {
            tri.swap(1, 2);
        }
        for v in &mut self.vertices {
            v.normal = (-Vec3::from(v.normal)).to_array();
        }
    }
}

/// Axis-aligned bounds of some vertices `(min, max)`; zero when empty.
pub fn bounds(vertices: &[Vertex]) -> (Vec3, Vec3) {
    if vertices.is_empty() {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    vertices.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), v| {
            let p = Vec3::from(v.position);
            (lo.min(p), hi.max(p))
        },
    )
}

// ---- primitives ---------------------------------------------------------------------------

/// Axis-aligned box centred at the origin, each face mapping the full texture.
pub fn cuboid(half_extents: Vec3, color: Vec4) -> MeshBuilder {
    let mut b = MeshBuilder::new();
    b.add_box(&BoxDesc {
        half_extents,
        color,
        ..Default::default()
    });
    b
}

/// Quad in the XY plane facing +Z, centred at the origin.
pub fn quad(size: Vec2, color: Vec4) -> MeshBuilder {
    let (x, y) = (size.x * 0.5, size.y * 0.5);
    let mut b = MeshBuilder::new();
    b.quad(
        [
            Vec3::new(-x, -y, 0.0),
            Vec3::new(x, -y, 0.0),
            Vec3::new(x, y, 0.0),
            Vec3::new(-x, y, 0.0),
        ],
        [
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 0.0),
        ],
        color,
    );
    b
}

/// Cylinder along Y centred at the origin (`y` in `[-height/2, height/2]`), smooth sides,
/// flat caps.
pub fn cylinder(radius: f32, height: f32, segments: u32, color: Vec4) -> MeshBuilder {
    let segments = segments.max(3);
    let mut b = MeshBuilder::new();
    let (y0, y1) = (-height * 0.5, height * 0.5);
    let base = b.vertices.len() as u32;
    for i in 0..=segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let n = Vec3::new(a.cos(), 0.0, -a.sin());
        let u = i as f32 / segments as f32;
        b.vertex(n * radius + Vec3::Y * y0, n, Vec2::new(u, 1.0), color);
        b.vertex(n * radius + Vec3::Y * y1, n, Vec2::new(u, 0.0), color);
    }
    for i in 0..segments {
        let (a0, a1, b0, b1) = (
            base + i * 2,
            base + i * 2 + 1,
            base + i * 2 + 2,
            base + i * 2 + 3,
        );
        b.indices.extend_from_slice(&[a0, b0, b1, a0, b1, a1]);
    }
    disc(&mut b, y1, radius, segments, Vec3::Y, color);
    disc(&mut b, y0, radius, segments, Vec3::NEG_Y, color);
    b
}

/// Flat circular cap at height `y` facing `normal` (+Y or -Y).
fn disc(b: &mut MeshBuilder, y: f32, radius: f32, segments: u32, normal: Vec3, color: Vec4) {
    let center = b.vertex(Vec3::Y * y, normal, Vec2::splat(0.5), color);
    let first = b.vertices.len() as u32;
    for i in 0..segments {
        let a = i as f32 / segments as f32 * std::f32::consts::TAU;
        let (c, s) = (a.cos(), -a.sin());
        b.vertex(
            Vec3::new(c * radius, y, s * radius),
            normal,
            Vec2::new(0.5 + c * 0.5, 0.5 + s * 0.5),
            color,
        );
    }
    for i in 0..segments {
        let (p, q) = (first + i, first + (i + 1) % segments);
        if normal.y > 0.0 {
            b.triangle(center, p, q);
        } else {
            b.triangle(center, q, p);
        }
    }
}

/// Cone along Y: base disc at `y = -height/2`, tip at `y = height/2`.
pub fn cone(radius: f32, height: f32, segments: u32, color: Vec4) -> MeshBuilder {
    let segments = segments.max(3);
    let mut b = MeshBuilder::new();
    let (y0, y1) = (-height * 0.5, height * 0.5);
    let slope = radius / height.max(1e-6);
    for i in 0..segments {
        let (a0, a1) = (
            i as f32 / segments as f32 * std::f32::consts::TAU,
            (i + 1) as f32 / segments as f32 * std::f32::consts::TAU,
        );
        let dir = |a: f32| Vec3::new(a.cos(), 0.0, -a.sin());
        let normal = |a: f32| (dir(a) + Vec3::Y * slope).normalize();
        let am = (a0 + a1) * 0.5;
        let p0 = b.vertex(
            dir(a0) * radius + Vec3::Y * y0,
            normal(a0),
            Vec2::new(i as f32 / segments as f32, 1.0),
            color,
        );
        let p1 = b.vertex(
            dir(a1) * radius + Vec3::Y * y0,
            normal(a1),
            Vec2::new((i + 1) as f32 / segments as f32, 1.0),
            color,
        );
        let tip = b.vertex(
            Vec3::Y * y1,
            normal(am),
            Vec2::new((i as f32 + 0.5) / segments as f32, 0.0),
            color,
        );
        b.triangle(p0, p1, tip);
    }
    disc(&mut b, y0, radius, segments, Vec3::NEG_Y, color);
    b
}

/// Flat-shaded (faceted) icosphere centred at the origin.
pub fn icosphere(radius: f32, subdivisions: u32, color: Vec4) -> MeshBuilder {
    let t = (1.0 + 5f32.sqrt()) * 0.5;
    let mut points: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut tris: Vec<[usize; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions.min(5) {
        let mut cache = std::collections::HashMap::new();
        let mut midpoint = |a: usize, b: usize, points: &mut Vec<Vec3>| -> usize {
            let key = (a.min(b), a.max(b));
            *cache.entry(key).or_insert_with(|| {
                points.push(((points[a] + points[b]) * 0.5).normalize());
                points.len() - 1
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let ab = midpoint(a, b, &mut points);
            let bc = midpoint(b, c, &mut points);
            let ca = midpoint(c, a, &mut points);
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    let mut b = MeshBuilder::new();
    let sphere_uv = |p: Vec3| {
        Vec2::new(
            0.5 + p.z.atan2(p.x) / std::f32::consts::TAU,
            0.5 - p.y.asin() / std::f32::consts::PI,
        )
    };
    for [i0, i1, i2] in tris {
        let p = [points[i0], points[i1], points[i2]];
        b.flat_triangle(p.map(|q| q * radius), p.map(sphere_uv), color);
    }
    b
}

/// Smooth UV sphere centred at the origin.
pub fn uv_sphere(radius: f32, rings: u32, segments: u32, color: Vec4) -> MeshBuilder {
    let (rings, segments) = (rings.max(2), segments.max(3));
    let mut b = MeshBuilder::new();
    for r in 0..=rings {
        let v = r as f32 / rings as f32;
        let phi = v * std::f32::consts::PI;
        for s in 0..=segments {
            let u = s as f32 / segments as f32;
            let theta = u * std::f32::consts::TAU;
            let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), -phi.sin() * theta.sin());
            b.vertex(n * radius, n, Vec2::new(u, v), color);
        }
    }
    let row = segments + 1;
    for r in 0..rings {
        for s in 0..segments {
            let a = r * row + s;
            let (b1, c, d) = (a + 1, a + row + 1, a + row);
            // The pole rows collapse to a point; skip the degenerate half of their quads.
            if r + 1 < rings {
                b.indices.extend_from_slice(&[a, d, c]);
            }
            if r > 0 {
                b.indices.extend_from_slice(&[a, c, b1]);
            }
        }
    }
    b
}

/// Ramp / wedge: a triangular prism filling the box `[-h, h]` whose slope rises from the
/// front-bottom edge (`+Z`, `-Y`) to the back-top edge (`-Z`, `+Y`).
pub fn wedge(half_extents: Vec3, color: Vec4) -> MeshBuilder {
    let h = half_extents;
    let mut b = MeshBuilder::new();
    let (fbl, fbr) = (Vec3::new(-h.x, -h.y, h.z), Vec3::new(h.x, -h.y, h.z));
    let (bbl, bbr) = (Vec3::new(-h.x, -h.y, -h.z), Vec3::new(h.x, -h.y, -h.z));
    let (btl, btr) = (Vec3::new(-h.x, h.y, -h.z), Vec3::new(h.x, h.y, -h.z));
    let uv = [
        Vec2::new(0.0, 1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, 0.0),
    ];
    b.quad([fbl, fbr, btr, btl], uv, color); // slope
    b.quad([bbr, bbl, btl, btr], uv, color); // back
    b.quad([bbl, bbr, fbr, fbl], uv, color); // bottom
    b.flat_triangle(
        [fbr, bbr, btr],
        [
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 0.0),
        ],
        color,
    );
    b.flat_triangle(
        [bbl, fbl, btl],
        [
            Vec2::new(0.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 0.0),
        ],
        color,
    );
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle's winding must agree with its vertex normals (front faces CCW).
    fn assert_outward(b: &MeshBuilder) {
        assert_outward_named(b, "mesh");
    }

    fn assert_outward_named(b: &MeshBuilder, name: &str) {
        for tri in b.indices.chunks_exact(3) {
            let p: Vec<Vec3> = tri
                .iter()
                .map(|&i| Vec3::from(b.vertices[i as usize].position))
                .collect();
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            if face.length() < 1e-9 {
                continue;
            }
            let n: Vec3 = tri
                .iter()
                .map(|&i| Vec3::from(b.vertices[i as usize].normal))
                .sum();
            assert!(
                face.dot(n) > 0.0,
                "{name}: triangle winding disagrees with normals"
            );
        }
    }

    fn assert_closed_and_centered(b: &MeshBuilder) {
        // Outward normals: (position - centre) . normal >= 0 for convex shapes centred at 0.
        for v in &b.vertices {
            assert!(Vec3::from(v.position).dot(Vec3::from(v.normal)) >= -1e-4);
        }
    }

    #[test]
    fn primitives_face_outwards() {
        for (name, b) in [
            ("cuboid", cuboid(Vec3::new(1.0, 2.0, 3.0), Vec4::ONE)),
            ("cylinder", cylinder(0.5, 2.0, 8, Vec4::ONE)),
            ("cone", cone(0.5, 1.0, 7, Vec4::ONE)),
            ("icosphere", icosphere(1.0, 1, Vec4::ONE)),
            ("uv_sphere", uv_sphere(1.0, 6, 8, Vec4::ONE)),
            ("wedge", wedge(Vec3::new(1.0, 0.5, 2.0), Vec4::ONE)),
        ] {
            assert!(!b.is_empty());
            assert_outward_named(&b, name);
            assert_closed_and_centered(&b);
            assert!(b.indices.iter().all(|&i| (i as usize) < b.vertices.len()));
        }
    }

    #[test]
    fn rotated_subdivided_brush() {
        let mut b = MeshBuilder::new();
        b.add_box(&BoxDesc::brush(
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(4.0, 0.5, 1.0),
            Quat::from_rotation_y(0.6),
        ));
        assert_outward(&b);
        // 8 m long faces split into 2 m segments.
        assert!(b.triangle_count() > 12);
        let (lo, hi) = b.bounds();
        assert!(((lo + hi).x / 2.0 - 10.0).abs() < 1e-3);
    }

    #[test]
    fn world_uvs_line_up_between_brushes() {
        // Two adjacent floor brushes: the shared edge must get the same UVs from both.
        let uv = BoxUv::texel_density(32.0, 64.0);
        let mut a = MeshBuilder::new();
        a.add_box(&BoxDesc {
            center: Vec3::new(-1.0, 0.0, 0.0),
            half_extents: Vec3::ONE,
            uv,
            faces: FACE_POS_Y,
            ..Default::default()
        });
        let mut b = MeshBuilder::new();
        b.add_box(&BoxDesc {
            center: Vec3::new(1.0, 0.0, 0.0),
            half_extents: Vec3::ONE,
            uv,
            faces: FACE_POS_Y,
            ..Default::default()
        });
        let edge_uv = |m: &MeshBuilder| {
            let mut uvs: Vec<[f32; 2]> = m
                .vertices
                .iter()
                .filter(|v| v.position[0].abs() < 1e-5)
                .map(|v| v.uv)
                .collect();
            uvs.sort_by(|p, q| p[1].total_cmp(&q[1]));
            uvs
        };
        assert_eq!(edge_uv(&a), edge_uv(&b));
        // 32 texels/m on a 64 px texture = half a repeat per metre.
        let top = &a.vertices;
        let span = top.iter().map(|v| v.uv[0]).fold(f32::MIN, f32::max)
            - top.iter().map(|v| v.uv[0]).fold(f32::MAX, f32::min);
        assert!((span - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mirrored_append_keeps_front_faces() {
        let mut b = MeshBuilder::new();
        b.append(
            &cuboid(Vec3::ONE, Vec4::ONE).build(),
            Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
        );
        assert_outward(&b);
    }

    #[test]
    fn color_helpers() {
        let mut b = cuboid(Vec3::ONE, Vec4::ONE);
        b.ao_near_floor(-1.0, 2.0, 0.5);
        let bottom = b.vertices.iter().find(|v| v.position[1] < 0.0).unwrap();
        let top = b.vertices.iter().find(|v| v.position[1] > 0.0).unwrap();
        assert!(bottom.color[0] < top.color[0]);
        b.shade_by_normal(1.0, 0.8, 0.5);
        b.jitter_colors(3, 0.1);
        assert!(b.vertices.iter().all(|v| v.color[3] == 1.0));
    }

    #[test]
    fn cylinder_between_points() {
        let mut b = MeshBuilder::new();
        b.add_cylinder_between(Vec3::ZERO, Vec3::new(0.0, 0.0, -2.0), 0.1, 6, Vec4::ONE);
        let (lo, hi) = b.bounds();
        assert!((lo.z + 2.0).abs() < 1e-4 && hi.z.abs() < 1e-4);
        assert_outward(&b);
    }
}
