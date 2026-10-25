//! The 3D scene behind the main menu: a level rendered with the full retro pipeline and a slow
//! orbiting camera.

use engine::gfx::Camera;
use engine::render::Renderer;
use glam::Vec3;
use shared::level::LevelDef;

use crate::frame::{FrameBuffers, SceneParams};
use crate::ingame::world_render::WorldRender;
use crate::models::ModelLibrary;

pub struct Backdrop {
    world: WorldRender,
    center: Vec3,
    radius: f32,
    height: f32,
}

/// Preferred backdrop levels, first one found wins.
const LEVELS: [&str; 3] = ["cistern_nine", "movement_test", shared::sim::BUILTIN_LEVEL];

impl Backdrop {
    pub fn new(renderer: &mut Renderer) -> Option<Self> {
        let (name, level) = LEVELS
            .iter()
            .find_map(|n| shared::sim::load_level(n).ok().map(|l| (*n, l)))?;
        let mut world = WorldRender::new();
        world.load(renderer, name, &level);
        let (center, radius, height) = framing(&level);
        Some(Self {
            world,
            center,
            radius,
            height,
        })
    }

    /// Draws the level and returns the scene with the orbit camera at `time`.
    pub fn frame(
        &self,
        time: f64,
        lib: Option<&ModelLibrary>,
        out: &mut FrameBuffers,
    ) -> SceneParams {
        self.world.draw(None, &mut out.draws, &mut out.lights);
        self.world.draw_props(lib, time, &mut out.draws);
        let a = (time * 0.035) as f32 + 0.6;
        let bob = ((time * 0.21).sin() as f32) * 2.0;
        let position = self.center
            + Vec3::new(
                a.sin() * self.radius,
                self.height + bob,
                a.cos() * self.radius,
            );
        let target = self.center + Vec3::new(0.0, 2.0, 0.0);
        let dir = (target - position).normalize_or(Vec3::NEG_Z);
        let yaw = (-dir.x).atan2(-dir.z);
        let pitch = dir.y.clamp(-1.0, 1.0).asin();
        let mut scene = self.world.scene();
        scene.camera = Camera {
            position,
            rotation: Camera::rotation_from_angles(yaw, pitch, 0.04 * (time * 0.3).sin() as f32),
            fov_y_degrees: 62.0,
            near: 0.1,
            far: 600.0,
        };
        scene
    }

    pub fn free(mut self, renderer: &mut Renderer) {
        self.world.free(renderer);
    }
}

/// Orbit centre / radius / height from the level's spawn points (where the action is).
fn framing(level: &LevelDef) -> (Vec3, f32, f32) {
    if level.spawns.is_empty() {
        return (Vec3::ZERO, 40.0, 14.0);
    }
    let sum: Vec3 = level.spawns.iter().map(|s| s.position).sum();
    let center = sum / level.spawns.len() as f32;
    let spread = level
        .spawns
        .iter()
        .map(|s| (s.position - center).length())
        .fold(0.0f32, f32::max);
    let radius = (spread * 0.45).clamp(25.0, 55.0);
    (center, radius, radius * 0.5)
}
