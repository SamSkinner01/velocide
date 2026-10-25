//! The 3D scene behind the main menu: a level rendered with the full retro pipeline and a slow
//! orbiting camera.

use std::time::{SystemTime, UNIX_EPOCH};

use engine::gfx::Camera;
use engine::render::Renderer;
use glam::{Vec2, Vec3};
use shared::level::LevelDef;

use crate::frame::{FrameBuffers, SceneParams};
use crate::ingame::world_render::WorldRender;
use crate::models::ModelLibrary;

pub struct Backdrop {
    world: WorldRender,
    center: Vec3,
    /// Orbit radii along x and z (an ellipse, so long arenas keep the camera inside).
    radius: Vec2,
    height: f32,
}

/// The arenas the backdrop rotates through (one per day, so the main menu and the lobby show
/// the same one), then fallbacks; the first one found wins.
const ARENAS: [&str; 3] = ["cistern_nine", "knell_yard", "ingot_run"];
const FALLBACKS: [&str; 2] = ["movement_test", shared::sim::BUILTIN_LEVEL];

/// Backdrop levels in order of preference for today.
fn candidates() -> impl Iterator<Item = &'static str> {
    let day = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    let first = (day % ARENAS.len() as u64) as usize;
    (0..ARENAS.len())
        .map(move |i| ARENAS[(first + i) % ARENAS.len()])
        .chain(FALLBACKS)
}

impl Backdrop {
    pub fn new(renderer: &mut Renderer) -> Option<Self> {
        let (name, level) =
            candidates().find_map(|n| shared::sim::load_level(n).ok().map(|l| (n, l)))?;
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
                a.sin() * self.radius.x,
                self.height + bob,
                a.cos() * self.radius.y,
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

/// Orbit centre / radii / height from the level's spawn points (where the action is): an
/// ellipse inside the spawns' spread, above the highest spawn.
fn framing(level: &LevelDef) -> (Vec3, Vec2, f32) {
    if level.spawns.is_empty() {
        return (Vec3::ZERO, Vec2::splat(40.0), 14.0);
    }
    let sum: Vec3 = level.spawns.iter().map(|s| s.position).sum();
    let center = sum / level.spawns.len() as f32;
    let spread = level.spawns.iter().fold(Vec2::ZERO, |m, s| {
        let d = s.position - center;
        m.max(Vec2::new(d.x.abs(), d.z.abs()))
    });
    let radius = (spread * 0.75).clamp(Vec2::splat(12.0), Vec2::splat(55.0));
    let top = level.spawns.iter().map(|s| s.position.y).fold(f32::MIN, f32::max);
    (center, radius, (top - center.y + 9.0).max(8.0))
}
