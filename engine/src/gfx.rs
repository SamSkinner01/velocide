//! Plain-data types shared between the game and the renderer.
//!
//! The game builds a [`FrameDesc`] every frame and hands it to `render::Renderer::render`.
//! Nothing in here touches wgpu, so these types are cheap to construct and easy to test.

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

use crate::color_vision::ColorVision;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct MeshId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct TextureId(pub u32);

/// Vertex format used by every 3D mesh (world, enemies, viewmodels).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Vertex colour, multiplied with the texture. Also used for baked lighting/AO.
    pub color: [f32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// RGBA8 (sRGB) pixels, row-major, top row first.
#[derive(Clone, Debug, Default)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// User-facing retro rendering options. Every effect can be toggled from the settings menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetroSettings {
    /// Internal render height in pixels; width follows the window aspect ratio.
    /// 0 means "native" (no downscale).
    pub internal_height: u32,
    /// Snap clip-space vertices to the low-res pixel grid (PS1 wobble).
    pub vertex_snap: bool,
    /// Grid divisor for vertex snapping; 1.0 = snap to internal pixels, higher = coarser.
    pub vertex_snap_strength: f32,
    /// Affine (non perspective-correct) texture mapping.
    pub affine_textures: bool,
    /// Ordered (Bayer 4x4) dithering in the post-process pass.
    pub dithering: bool,
    /// Bits per colour channel after quantisation (e.g. 5 = 32 levels). 8 = off.
    pub color_bits: u32,
    /// Distance fog.
    pub fog: bool,
    /// Simple per-vertex point lights (otherwise ambient + directional only).
    pub vertex_lighting: bool,
    pub vsync: bool,
    /// Game-side effects; the renderer ignores these but they live here so all visual
    /// toggles are in one settings struct.
    pub screen_shake: bool,
    pub screen_flashes: bool,
    pub hitstop: bool,
    /// Colour-vision correction applied in the post pass, before colour quantisation (see
    /// `color_vision`). Not saved with the retro options: the game keeps it with its
    /// accessibility settings and copies it in.
    #[serde(skip)]
    pub color_vision: ColorVision,
}

impl Default for RetroSettings {
    fn default() -> Self {
        Self {
            internal_height: 360,
            vertex_snap: true,
            vertex_snap_strength: 1.0,
            affine_textures: false,
            dithering: true,
            color_bits: 5,
            fog: true,
            vertex_lighting: true,
            vsync: true,
            screen_shake: true,
            screen_flashes: true,
            hitstop: true,
            color_vision: ColorVision::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    /// World-from-camera rotation. The camera looks down its local -Z with +Y up.
    pub rotation: Quat,
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            fov_y_degrees: 70.0,
            near: 0.05,
            far: 400.0,
        }
    }
}

impl Camera {
    /// Rotation from yaw (around +Y, radians, 0 = looking down -Z) and pitch (radians, + = up)
    /// plus a roll used for view tilt.
    pub fn rotation_from_angles(yaw: f32, pitch: f32, roll: f32) -> Quat {
        Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll)
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.position).inverse()
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * -Vec3::Z
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
}

impl Default for Fog {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.25, 0.08, 0.06),
            start: 20.0,
            end: 120.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    pub radius: f32,
}

/// One mesh instance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawItem {
    pub mesh: MeshId,
    pub texture: TextureId,
    /// World transform (or view-space transform for viewmodel items).
    pub transform: Mat4,
    /// Multiplied with texture * vertex colour. Alpha < 1 draws translucent.
    pub tint: Vec4,
    /// Fullbright (emissive) ignores lighting but still gets fog.
    pub unlit: bool,
}

/// Camera-facing billboard (muzzle flash, embers, wax lumps, projectiles, particles).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub position: Vec3,
    pub size: Vec2,
    pub color: Vec4,
    pub texture: TextureId,
    /// (u0, v0, u1, v1) in the texture, for atlases / animation frames.
    pub uv_rect: Vec4,
    /// Rotation around the view axis, radians.
    pub rotation: f32,
    /// Additive blending (fire, flashes) instead of alpha blending.
    pub additive: bool,
}

/// A flat quad stuck to a surface (wax splats, scorch marks).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decal {
    pub position: Vec3,
    /// Surface normal; the quad is placed slightly in front of the surface.
    pub normal: Vec3,
    pub size: f32,
    pub rotation: f32,
    pub color: Vec4,
    pub texture: TextureId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugLine {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec4,
}

/// Which texture a UI quad samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiTexture {
    /// Solid colour.
    #[default]
    White,
    /// The built-in bitmap font atlas (see `ui::font`).
    Font,
    Texture(TextureId),
}

/// A 2D quad in UI canvas coordinates (origin top-left, +y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiQuad {
    pub min: Vec2,
    pub max: Vec2,
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    /// sRGB-encoded colour in `0..1` (as picked in a paint program), multiplied with the
    /// texture. The renderer blends UI in gamma space and only linearises when the output view
    /// is an sRGB format.
    pub color: Vec4,
    pub texture: UiTexture,
}

/// 2D draw commands for HUD and menus, drawn on top of the upscaled 3D image.
///
/// The canvas is `canvas_size` units big (height is always [`UI_CANVAS_HEIGHT`], width follows
/// the window aspect). The renderer scales it to the window. Quads are drawn in order.
#[derive(Clone, Debug, Default)]
pub struct UiDrawList {
    pub canvas_size: Vec2,
    pub quads: Vec<UiQuad>,
}

/// Virtual UI canvas height. Text and widgets are laid out in these units.
pub const UI_CANVAS_HEIGHT: f32 = 360.0;

impl UiDrawList {
    pub fn new(canvas_size: Vec2) -> Self {
        Self {
            canvas_size,
            quads: Vec::new(),
        }
    }

    /// Canvas size for a window of the given pixel size.
    pub fn canvas_for_window(width: u32, height: u32) -> Vec2 {
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        Vec2::new(UI_CANVAS_HEIGHT * aspect, UI_CANVAS_HEIGHT)
    }

    pub fn clear(&mut self) {
        self.quads.clear();
    }

    pub fn rect(&mut self, min: Vec2, max: Vec2, color: Vec4) {
        self.quads.push(UiQuad {
            min,
            max,
            uv_min: Vec2::ZERO,
            uv_max: Vec2::ONE,
            color,
            texture: UiTexture::White,
        });
    }
}

/// Everything needed to draw one frame.
pub struct FrameDesc<'a> {
    pub camera: Camera,
    pub clear_color: Vec3,
    pub fog: Fog,
    /// Ambient light colour (added to every lit surface).
    pub ambient: Vec3,
    /// Directional "key" light, pointing *from* the light *towards* the scene.
    pub light_dir: Vec3,
    pub light_color: Vec3,
    pub lights: &'a [PointLight],
    pub draws: &'a [DrawItem],
    pub sprites: &'a [Sprite],
    pub decals: &'a [Decal],
    /// Held weapon, drawn in a separate pass with its own FOV and a cleared depth buffer so it
    /// never clips into walls. Transforms are in *view space* (camera at origin looking -Z).
    pub viewmodel: &'a [DrawItem],
    pub viewmodel_fov_y_degrees: f32,
    /// Lines drawn on top of the 3D scene (hitboxes, server ghost outlines).
    pub debug_lines: &'a [DebugLine],
    /// Full-screen colour flash blended over the 3D image (alpha = strength).
    pub screen_flash: Vec4,
    pub ui: &'a UiDrawList,
}
