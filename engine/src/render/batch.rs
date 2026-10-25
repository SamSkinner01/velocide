//! CPU-side frame preparation: GPU data layouts, sprite/decal/line expansion into one dynamic
//! vertex buffer, UI quads, frustum culling. Nothing here touches wgpu, so it is unit-tested.

use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

use crate::gfx::{Camera, DebugLine, Decal, PointLight, Sprite, TextureId, UiDrawList, UiTexture};

/// Must match `Frame` in `assets/shaders/common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuFrame {
    pub view_proj: [[f32; 4]; 4],
    pub viewmodel_view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub fog_color: [f32; 4],
    pub fog_params: [f32; 4],
    pub ambient: [f32; 4],
    pub key_dir: [f32; 4],
    pub key_color: [f32; 4],
    pub snap: [f32; 4],
    pub lighting: [f32; 4],
    pub light_pos: [[f32; 4]; MAX_LIGHTS],
    pub light_color: [[f32; 4]; MAX_LIGHTS],
}

pub(crate) const MAX_LIGHTS: usize = 16;

/// Must match `Draw` in `assets/shaders/common.wgsl` (storage buffer, 144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuDraw {
    pub model: [[f32; 4]; 4],
    pub normal_matrix: [[f32; 4]; 3],
    pub tint: [f32; 4],
    pub params: [f32; 4],
}

impl GpuDraw {
    pub fn new(model: Mat4, tint: Vec4, unlit: bool, viewmodel: bool) -> Self {
        let normal = Mat3::from_mat4(model).inverse().transpose();
        let normal = if normal.is_finite() {
            normal
        } else {
            Mat3::IDENTITY
        };
        Self {
            model: model.to_cols_array_2d(),
            normal_matrix: [
                normal.x_axis.extend(0.0).to_array(),
                normal.y_axis.extend(0.0).to_array(),
                normal.z_axis.extend(0.0).to_array(),
            ],
            tint: tint.to_array(),
            params: [unlit as u32 as f32, viewmodel as u32 as f32, 0.0, 0.0],
        }
    }
}

/// Vertex of sprites, decals and debug lines (`FxIn` in `fx.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FxVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub normal_lit: [f32; 4],
}

/// Vertex of the 2D UI (`UiIn` in `blit.wgsl`), position already in NDC.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct UiVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// `Post` in `post.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PostUniform {
    pub flash: [f32; 4],
    pub params: [f32; 4],
}

/// `Output` in `blit.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct OutputUniform {
    pub sizes: [f32; 4],
    pub flags: [f32; 4],
}

/// A run of consecutive vertices drawn with one texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub texture: TextureId,
    pub start: u32,
    pub count: u32,
}

fn push_run(runs: &mut Vec<Run>, texture: TextureId, start: u32, count: u32) {
    if let Some(last) = runs.last_mut()
        && last.texture == texture
        && last.start + last.count == start
    {
        last.count += count;
        return;
    }
    runs.push(Run {
        texture,
        start,
        count,
    });
}

/// View frustum as 6 inward-facing planes (xyz = normal, w = distance).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Works for any wgpu-style projection with clip-space depth in `[0, w]` (incl. reverse-Z).
    pub fn from_view_proj(m: Mat4) -> Self {
        let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
        let mut planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
        for p in &mut planes {
            let len = p.truncate().length();
            if len > 0.0 {
                *p /= len;
            }
        }
        Self { planes }
    }

    pub fn sphere_visible(&self, center: Vec3, radius: f32) -> bool {
        self.planes
            .iter()
            .all(|p| p.truncate().dot(center) + p.w >= -radius)
    }
}

/// Picks at most [`MAX_LIGHTS`] point lights, preferring the ones closest to the camera.
pub(crate) fn select_lights(lights: &[PointLight], camera_pos: Vec3, out: &mut Vec<PointLight>) {
    out.clear();
    out.extend_from_slice(lights);
    if out.len() > MAX_LIGHTS {
        out.sort_by(|a, b| {
            let da = a.position.distance(camera_pos) - a.radius;
            let db = b.position.distance(camera_pos) - b.radius;
            da.total_cmp(&db)
        });
        out.truncate(MAX_LIGHTS);
    }
}

/// Offset of decals along their surface normal, on top of the pipeline's depth bias.
pub(crate) const DECAL_OFFSET: f32 = 0.012;

/// Sprites, decals and debug lines of one frame, expanded into world-space triangles/lines.
#[derive(Default)]
pub(crate) struct FxBatch {
    pub vertices: Vec<FxVertex>,
    pub decals: Vec<Run>,
    pub sprites_alpha: Vec<Run>,
    pub sprites_additive: Vec<Run>,
    /// First vertex and vertex count of the line list.
    pub lines: (u32, u32),
    order: Vec<(f32, u32)>,
}

impl FxBatch {
    pub fn build(
        &mut self,
        camera: &Camera,
        frustum: &Frustum,
        sprites: &[Sprite],
        decals: &[Decal],
        lines: &[DebugLine],
    ) {
        self.vertices.clear();
        self.decals.clear();
        self.sprites_alpha.clear();
        self.sprites_additive.clear();

        // Decals: in submission order (newer on top), lit like the surface they sit on.
        for decal in decals {
            let radius = decal.size * 0.71;
            if decal.size <= 0.0 || !frustum.sphere_visible(decal.position, radius) {
                continue;
            }
            let start = self.vertices.len() as u32;
            push_decal(&mut self.vertices, decal);
            push_run(&mut self.decals, decal.texture, start, 6);
        }

        let forward = camera.forward();
        let right = camera.rotation * Vec3::X;
        let up = camera.rotation * Vec3::Y;

        // Alpha-blended sprites: back to front.
        self.order.clear();
        for (i, s) in sprites.iter().enumerate() {
            if s.additive || !sprite_visible(frustum, s) {
                continue;
            }
            let depth = (s.position - camera.position).dot(forward);
            self.order.push((depth, i as u32));
        }
        self.order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        for &(_, i) in &self.order {
            let s = &sprites[i as usize];
            let start = self.vertices.len() as u32;
            push_sprite(&mut self.vertices, s, right, up);
            push_run(&mut self.sprites_alpha, s.texture, start, 6);
        }

        // Additive sprites: order-independent, so group them by texture for fewer binds.
        self.order.clear();
        for (i, s) in sprites.iter().enumerate() {
            if s.additive && sprite_visible(frustum, s) {
                self.order.push((s.texture.0 as f32, i as u32));
            }
        }
        self.order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, i) in &self.order {
            let s = &sprites[i as usize];
            let start = self.vertices.len() as u32;
            push_sprite(&mut self.vertices, s, right, up);
            push_run(&mut self.sprites_additive, s.texture, start, 6);
        }

        let first_line = self.vertices.len() as u32;
        for line in lines {
            for p in [line.a, line.b] {
                self.vertices.push(FxVertex {
                    position: p.to_array(),
                    uv: [0.5, 0.5],
                    color: line.color.to_array(),
                    normal_lit: [0.0; 4],
                });
            }
        }
        self.lines = (first_line, lines.len() as u32 * 2);
    }
}

fn sprite_visible(frustum: &Frustum, s: &Sprite) -> bool {
    let radius = s.size.max_element() * 0.71;
    radius > 0.0 && frustum.sphere_visible(s.position, radius)
}

fn push_quad(
    out: &mut Vec<FxVertex>,
    corners: [Vec3; 4],
    uvs: [Vec2; 4],
    color: Vec4,
    normal_lit: Vec4,
) {
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(FxVertex {
            position: corners[i].to_array(),
            uv: uvs[i].to_array(),
            color: color.to_array(),
            normal_lit: normal_lit.to_array(),
        });
    }
}

/// Camera-facing quad, rotated around the view axis. Corners: TL, TR, BR, BL.
pub(crate) fn push_sprite(out: &mut Vec<FxVertex>, s: &Sprite, right: Vec3, up: Vec3) {
    let (sin, cos) = s.rotation.sin_cos();
    let ax = (right * cos + up * sin) * (s.size.x * 0.5);
    let ay = (up * cos - right * sin) * (s.size.y * 0.5);
    let p = s.position;
    let r = s.uv_rect;
    push_quad(
        out,
        [p - ax + ay, p + ax + ay, p + ax - ay, p - ax - ay],
        [
            Vec2::new(r.x, r.y),
            Vec2::new(r.z, r.y),
            Vec2::new(r.z, r.w),
            Vec2::new(r.x, r.w),
        ],
        s.color,
        Vec4::ZERO,
    );
}

/// Quad lying on the surface, slightly offset along the normal.
pub(crate) fn push_decal(out: &mut Vec<FxVertex>, d: &Decal) {
    let n = d.normal.try_normalize().unwrap_or(Vec3::Y);
    let reference = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::Z };
    let t = reference.cross(n).normalize();
    let b = n.cross(t);
    let (sin, cos) = d.rotation.sin_cos();
    let half = d.size * 0.5;
    let ax = (t * cos + b * sin) * half;
    let ay = (b * cos - t * sin) * half;
    let p = d.position + n * DECAL_OFFSET;
    push_quad(
        out,
        [p - ax + ay, p + ax + ay, p + ax - ay, p - ax - ay],
        [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ],
        d.color,
        n.extend(1.0),
    );
}

/// UI quads converted to NDC triangles, snapped to whole window pixels for crisp text.
#[derive(Default)]
pub(crate) struct UiBatch {
    pub vertices: Vec<UiVertex>,
    pub runs: Vec<Run>,
}

impl UiBatch {
    pub fn build(
        &mut self,
        list: &UiDrawList,
        window: (u32, u32),
        font: TextureId,
        white: TextureId,
    ) {
        self.vertices.clear();
        self.runs.clear();
        let (w, h) = (window.0.max(1) as f32, window.1.max(1) as f32);
        let canvas = if list.canvas_size.x > 0.0 && list.canvas_size.y > 0.0 {
            list.canvas_size
        } else {
            UiDrawList::canvas_for_window(window.0, window.1)
        };
        let scale = Vec2::new(w / canvas.x, h / canvas.y);
        let to_ndc = |p: Vec2| -> [f32; 2] {
            let px = (p * scale).round();
            [px.x / w * 2.0 - 1.0, 1.0 - px.y / h * 2.0]
        };
        for q in &list.quads {
            let texture = match q.texture {
                UiTexture::White => white,
                UiTexture::Font => font,
                UiTexture::Texture(id) => id,
            };
            let (a, b) = (to_ndc(q.min), to_ndc(q.max));
            if a[0] == b[0] || a[1] == b[1] {
                continue;
            }
            let corners = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]];
            let uvs = [
                [q.uv_min.x, q.uv_min.y],
                [q.uv_max.x, q.uv_min.y],
                [q.uv_max.x, q.uv_max.y],
                [q.uv_min.x, q.uv_max.y],
            ];
            let start = self.vertices.len() as u32;
            for i in [0, 1, 2, 0, 2, 3] {
                self.vertices.push(UiVertex {
                    position: corners[i],
                    uv: uvs[i],
                    color: q.color.to_array(),
                });
            }
            push_run(&mut self.runs, texture, start, 6);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn gpu_layouts_match_wgsl() {
        assert_eq!(std::mem::size_of::<GpuFrame>(), 768);
        assert_eq!(std::mem::size_of::<GpuDraw>(), 144);
        assert_eq!(std::mem::size_of::<FxVertex>(), 52);
        assert_eq!(std::mem::size_of::<UiVertex>(), 32);
    }

    fn test_camera() -> (Camera, Frustum) {
        let camera = Camera::default();
        let proj = super::super::reverse_z_perspective(
            70f32.to_radians(),
            16.0 / 9.0,
            camera.near,
            camera.far,
        );
        (camera, Frustum::from_view_proj(proj * camera.view_matrix()))
    }

    #[test]
    fn frustum_culls_behind_and_far() {
        let (_, f) = test_camera();
        assert!(f.sphere_visible(Vec3::new(0.0, 0.0, -10.0), 0.5));
        assert!(!f.sphere_visible(Vec3::new(0.0, 0.0, 10.0), 0.5));
        assert!(!f.sphere_visible(Vec3::new(0.0, 0.0, -1000.0), 0.5));
        assert!(!f.sphere_visible(Vec3::new(100.0, 0.0, -10.0), 0.5));
        // Large sphere straddling the near plane is visible.
        assert!(f.sphere_visible(Vec3::new(0.0, 0.0, 2.0), 5.0));
    }

    fn sprite(z: f32, additive: bool, texture: u32) -> Sprite {
        Sprite {
            position: Vec3::new(0.0, 0.0, z),
            size: Vec2::ONE,
            color: Vec4::ONE,
            texture: TextureId(texture),
            uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
            rotation: 0.0,
            additive,
        }
    }

    #[test]
    fn sprites_sorted_back_to_front_and_batched() {
        let (camera, frustum) = test_camera();
        let sprites = [
            sprite(-5.0, false, 1),
            sprite(-20.0, false, 1),
            sprite(-10.0, true, 2),
            sprite(-3.0, true, 2),
            sprite(5.0, false, 1), // behind the camera: culled
        ];
        let mut batch = FxBatch::default();
        batch.build(&camera, &frustum, &sprites, &[], &[]);
        assert_eq!(
            batch.sprites_alpha,
            vec![Run {
                texture: TextureId(1),
                start: 0,
                count: 12
            }]
        );
        assert_eq!(
            batch.sprites_additive,
            vec![Run {
                texture: TextureId(2),
                start: 12,
                count: 12
            }]
        );
        // The farthest alpha sprite comes first.
        assert_eq!(batch.vertices[0].position[2], -20.0);
    }

    #[test]
    fn sprite_quad_faces_camera() {
        let mut out = Vec::new();
        let rot = Quat::from_rotation_y(0.7);
        let s = sprite(0.0, false, 0);
        push_sprite(&mut out, &s, rot * Vec3::X, rot * Vec3::Y);
        let fwd = rot * -Vec3::Z;
        for v in &out {
            assert!(Vec3::from(v.position).dot(fwd).abs() < 1e-5);
        }
    }

    #[test]
    fn decal_lies_on_surface() {
        let mut out = Vec::new();
        let d = Decal {
            position: Vec3::new(1.0, 2.0, 3.0),
            normal: Vec3::X,
            size: 2.0,
            rotation: 0.3,
            color: Vec4::ONE,
            texture: TextureId(0),
        };
        push_decal(&mut out, &d);
        assert_eq!(out.len(), 6);
        for v in &out {
            assert!((v.position[0] - (1.0 + DECAL_OFFSET)).abs() < 1e-5);
            assert_eq!(v.normal_lit, [1.0, 0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn ui_quads_map_to_ndc_and_batch_by_texture() {
        let mut list = UiDrawList::new(Vec2::new(640.0, 360.0));
        list.rect(Vec2::ZERO, Vec2::new(320.0, 180.0), Vec4::ONE);
        list.rect(Vec2::new(320.0, 180.0), Vec2::new(640.0, 360.0), Vec4::ONE);
        let mut batch = UiBatch::default();
        batch.build(&list, (1280, 720), TextureId(1), TextureId(0));
        assert_eq!(
            batch.runs,
            vec![Run {
                texture: TextureId(0),
                start: 0,
                count: 12
            }]
        );
        assert_eq!(batch.vertices[0].position, [-1.0, 1.0]);
        assert_eq!(batch.vertices[2].position, [0.0, 0.0]);
        assert_eq!(batch.vertices[8].position, [1.0, -1.0]);
    }

    #[test]
    fn light_selection_keeps_nearest() {
        let lights: Vec<PointLight> = (0..40)
            .map(|i| PointLight {
                position: Vec3::new(i as f32, 0.0, 0.0),
                color: Vec3::ONE,
                radius: 1.0,
            })
            .collect();
        let mut out = Vec::new();
        select_lights(&lights, Vec3::ZERO, &mut out);
        assert_eq!(out.len(), MAX_LIGHTS);
        assert!(out.iter().all(|l| l.position.x < MAX_LIGHTS as f32));
    }
}
