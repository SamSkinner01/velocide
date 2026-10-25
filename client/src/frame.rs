//! Per-frame plumbing shared by every screen: the reusable draw buffers that become the
//! renderer's [`FrameDesc`], the scene parameters a screen picks, and what a screen tells the
//! app after its frame.

use engine::gfx::{Camera, DebugLine, Decal, DrawItem, FrameDesc, PointLight, Sprite, UiDrawList};
use engine::input::InputState;
use engine::render::Renderer;
use engine::ui::UiInput;
use glam::{Vec2, Vec3, Vec4};

use crate::cli::StartMode;

/// Draw lists filled by a screen each frame. They are cleared (not reallocated) every frame.
#[derive(Default)]
pub struct FrameBuffers {
    pub draws: Vec<DrawItem>,
    pub sprites: Vec<Sprite>,
    pub decals: Vec<Decal>,
    pub viewmodel: Vec<DrawItem>,
    pub lines: Vec<DebugLine>,
    pub lights: Vec<PointLight>,
    pub ui: UiDrawList,
}

impl FrameBuffers {
    pub fn clear(&mut self, canvas: Vec2) {
        self.draws.clear();
        self.sprites.clear();
        self.decals.clear();
        self.viewmodel.clear();
        self.lines.clear();
        self.lights.clear();
        self.ui.clear();
        self.ui.canvas_size = canvas;
    }

    pub fn desc<'a>(&'a self, scene: &SceneParams) -> FrameDesc<'a> {
        FrameDesc {
            camera: scene.camera,
            clear_color: scene.clear_color,
            fog: engine::gfx::Fog {
                color: scene.fog_color,
                start: scene.fog_start,
                end: scene.fog_end,
            },
            ambient: scene.ambient,
            light_dir: scene.light_dir,
            light_color: scene.light_color,
            lights: &self.lights,
            draws: &self.draws,
            sprites: &self.sprites,
            decals: &self.decals,
            viewmodel: &self.viewmodel,
            viewmodel_fov_y_degrees: scene.viewmodel_fov,
            debug_lines: &self.lines,
            screen_flash: scene.screen_flash,
            ui: &self.ui,
        }
    }
}

/// Camera, environment and post settings of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneParams {
    pub camera: Camera,
    pub clear_color: Vec3,
    pub fog_color: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    pub ambient: Vec3,
    pub light_dir: Vec3,
    pub light_color: Vec3,
    pub viewmodel_fov: f32,
    pub screen_flash: Vec4,
}

impl Default for SceneParams {
    fn default() -> Self {
        let env = shared::level::Environment::default();
        Self {
            camera: Camera::default(),
            clear_color: env.sky_color,
            fog_color: env.fog_color,
            fog_start: env.fog_start,
            fog_end: env.fog_end,
            ambient: env.ambient,
            light_dir: env.light_dir.normalize_or(Vec3::NEG_Y),
            light_color: env.light_color,
            viewmodel_fov: 62.0,
            screen_flash: Vec4::ZERO,
        }
    }
}

impl SceneParams {
    /// Environment of a level (sky, fog, lights) with the default camera.
    pub fn for_level(level: &shared::level::LevelDef) -> Self {
        let env = crate::level_mesh::environment(level);
        Self {
            clear_color: env.clear_color,
            fog_color: env.fog.color,
            fog_start: env.fog.start,
            fog_end: env.fog.end,
            ambient: env.ambient,
            light_dir: env.light_dir,
            light_color: env.light_color,
            ..Default::default()
        }
    }
}

/// Engine-side inputs of one frame (no window needed, so screens run headless in tests).
pub struct FrameIo<'a> {
    pub renderer: &'a mut Renderer,
    pub input: &'a InputState,
    pub ui_input: &'a UiInput,
    /// Real seconds since the previous frame.
    pub dt: f32,
    /// Seconds since startup.
    pub time: f64,
    pub window_size: (u32, u32),
}

/// Screen changes requested by a screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Transition {
    /// Start a game (single-player, host or join); the in-game screen shows the connection.
    Start(StartMode),
    /// Back to the main menu, optionally showing a message (e.g. a disconnect reason).
    MainMenu {
        message: Option<String>,
    },
    Quit,
}

/// What a screen reports after its frame.
#[derive(Clone, Debug)]
pub struct ScreenOutput {
    pub scene: SceneParams,
    /// Lock the cursor for mouse look (only while actually playing).
    pub grab_cursor: bool,
    pub transition: Option<Transition>,
}
