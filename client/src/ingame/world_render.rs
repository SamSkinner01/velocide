//! The level: geometry via [`LevelRender`] (doors hidden while the replicated door state says
//! open), environment (sky, fog, lights), plus debug ghost outlines of the local player's
//! capsule (server-authoritative vs predicted).

use engine::gfx::{DebugLine, DrawItem, PointLight};
use engine::render::Renderer;
use glam::{Vec3, Vec4};
use shared::level::{LevelDef, LevelKind};
use shared::movement::MoveState;
use shared::sim::SimWorld;
use shared::tuning::MovementTuning;

use crate::frame::SceneParams;
use crate::level_mesh::LevelRender;

#[derive(Default)]
pub struct WorldRender {
    level: Option<LevelRender>,
    scene: SceneParams,
    kind: LevelKind,
    name: String,
}

impl WorldRender {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds GPU geometry for a (new) level, freeing the previous one.
    pub fn load(&mut self, renderer: &mut Renderer, name: &str, level: &LevelDef) {
        if let Some(old) = self.level.take() {
            old.free(renderer);
        }
        let started = std::time::Instant::now();
        self.level = Some(LevelRender::new(renderer, level));
        self.scene = SceneParams::for_level(level);
        self.kind = level.kind;
        self.name = if level.name.is_empty() {
            name.to_string()
        } else {
            level.name.clone()
        };
        log::info!(
            "level '{name}' meshes built in {:.1} ms",
            started.elapsed().as_secs_f32() * 1000.0
        );
    }

    pub fn is_loaded(&self) -> bool {
        self.level.is_some()
    }

    /// Environment (camera is filled in by the caller).
    pub fn scene(&self) -> SceneParams {
        self.scene
    }

    pub fn kind(&self) -> LevelKind {
        self.kind
    }

    /// Display name of the level.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Appends level draw items and lights. Doors follow the client's replicated door state.
    pub fn draw(
        &self,
        world: Option<&SimWorld>,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        let Some(level) = &self.level else {
            return;
        };
        match world {
            Some(w) => level.draw(|id| w.collision.is_door_open(id), draws),
            None => level.draw(|_| false, draws),
        }
        lights.extend_from_slice(level.lights());
    }

    pub fn free(&mut self, renderer: &mut Renderer) {
        if let Some(level) = self.level.take() {
            level.free(renderer);
        }
    }
}

/// Ring at the feet plus a velocity arrow: the local predicted body (the camera is inside the
/// full capsule, so drawing all of it would just cover the screen).
pub fn foot_ring_lines(
    state: &MoveState,
    t: &MovementTuning,
    color: Vec4,
    out: &mut Vec<DebugLine>,
) {
    const N: usize = 16;
    let p = state.position + Vec3::Y * 0.02;
    let r = t.radius;
    for i in 0..N {
        let a0 = i as f32 / N as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / N as f32 * std::f32::consts::TAU;
        out.push(DebugLine {
            a: p + Vec3::new(a0.cos(), 0.0, a0.sin()) * r,
            b: p + Vec3::new(a1.cos(), 0.0, a1.sin()) * r,
            color,
        });
    }
    let v = Vec3::new(state.velocity.x, 0.0, state.velocity.z) * 0.1;
    if v.length_squared() > 1e-4 {
        out.push(DebugLine {
            a: p,
            b: p + v,
            color,
        });
    }
}

/// Wireframe capsule (two rings at each end, four vertical lines) around a movement state.
pub fn capsule_lines(state: &MoveState, t: &MovementTuning, color: Vec4, out: &mut Vec<DebugLine>) {
    let (center, half, radius) = state.capsule(t);
    let bottom = center - Vec3::Y * half;
    let top = center + Vec3::Y * half;
    const N: usize = 12;
    let ring = |c: Vec3, i: usize| {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        c + Vec3::new(a.cos(), 0.0, a.sin()) * radius
    };
    for c in [
        bottom,
        top,
        bottom - Vec3::Y * radius * 0.7,
        top + Vec3::Y * radius * 0.7,
    ] {
        let r = if c == bottom || c == top { 1.0 } else { 0.7 };
        for i in 0..N {
            let a = c + (ring(Vec3::ZERO, i)) * r;
            let b = c + (ring(Vec3::ZERO, i + 1)) * r;
            out.push(DebugLine { a, b, color });
        }
    }
    for i in [0, N / 4, N / 2, 3 * N / 4] {
        out.push(DebugLine {
            a: ring(bottom, i),
            b: ring(top, i),
            color,
        });
    }
    // Velocity arrow.
    let v = state.velocity * 0.1;
    if v.length_squared() > 1e-4 {
        out.push(DebugLine {
            a: center,
            b: center + v,
            color,
        });
    }
}
