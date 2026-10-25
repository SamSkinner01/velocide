//! Drawing other players (interpolated `NetClient::remote_players()`).
//!
//! This is the seam for player models: [`RemotePlayers::draw`] gets every interpolated player
//! entity and appends draw items. The placeholder is a blocky chrome "Shrike" made of tinted
//! cubes (legs and arms swing with speed, the body squashes while sliding, the head follows the
//! pitch, a visor glows in the player's colour). When `client::models` provides the Shrike rig,
//! replace the body of `draw` with the model + its procedural animation; per-player animation
//! state already lives in [`PlayerAnim`].

use std::collections::HashMap;

use engine::assets::meshgen::cuboid;
use engine::gfx::{DrawItem, MeshId, TextureId};
use engine::render::Renderer;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::protocol::{EntityState, NetId, player_flags};
use shared::sim::PlayerId;

/// Distinct colours per player slot (sRGB).
pub const PLAYER_COLORS: [Vec4; 8] = [
    Vec4::new(0.93, 0.15, 0.17, 1.0),
    Vec4::new(0.38, 0.78, 1.0, 1.0),
    Vec4::new(1.0, 0.72, 0.2, 1.0),
    Vec4::new(0.55, 1.0, 0.45, 1.0),
    Vec4::new(0.85, 0.4, 1.0, 1.0),
    Vec4::new(1.0, 0.5, 0.75, 1.0),
    Vec4::new(0.35, 1.0, 0.9, 1.0),
    Vec4::new(1.0, 1.0, 0.55, 1.0),
];

pub fn player_color(id: PlayerId) -> Vec4 {
    PLAYER_COLORS[id.0 as usize % PLAYER_COLORS.len()]
}

/// Where a name tag hangs (above the head).
pub fn name_tag_anchor(e: &EntityState) -> Vec3 {
    let low = e.flags & player_flags::LOW_PROFILE != 0;
    e.position + Vec3::Y * if low { 1.35 } else { 2.15 }
}

/// Per-player animation state.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerAnim {
    pub phase: f32,
    /// 0 = standing still .. 1 = full run.
    pub run: f32,
    /// Smoothed crouch (slide) amount.
    pub crouch: f32,
    pub seen: bool,
}

pub struct RemotePlayers {
    cube: MeshId,
    texture: TextureId,
    anims: HashMap<NetId, PlayerAnim>,
}

const CHROME: Vec4 = Vec4::new(0.72, 0.74, 0.8, 1.0);
const DARK: Vec4 = Vec4::new(0.2, 0.18, 0.2, 1.0);

impl RemotePlayers {
    pub fn new(renderer: &mut Renderer) -> Self {
        let mut cube = cuboid(Vec3::splat(0.5), Vec4::ONE);
        cube.shade_by_normal(1.0, 0.8, 0.55);
        Self {
            cube: renderer.create_mesh(&cube.build()),
            texture: renderer.white_texture(),
            anims: HashMap::new(),
        }
    }

    pub fn anim(&self, id: NetId) -> Option<&PlayerAnim> {
        self.anims.get(&id)
    }

    fn part(
        &self,
        base: Mat4,
        center: Vec3,
        half: Vec3,
        rot: Quat,
        tint: Vec4,
        unlit: bool,
    ) -> DrawItem {
        DrawItem {
            mesh: self.cube,
            texture: self.texture,
            transform: base * Mat4::from_scale_rotation_translation(half * 2.0, rot, center),
            tint,
            unlit,
        }
    }

    /// Advances animation by `dt` (real time) and appends every player's parts.
    pub fn draw<'a>(
        &mut self,
        players: impl Iterator<Item = &'a EntityState>,
        dt: f32,
        out: &mut Vec<DrawItem>,
    ) {
        for a in self.anims.values_mut() {
            a.seen = false;
        }
        for e in players {
            let anim = self.anims.entry(e.id).or_default();
            anim.seen = true;
            let speed = Vec2::new(e.velocity.x, e.velocity.z).length();
            let low = e.flags & player_flags::LOW_PROFILE != 0;
            let airborne = e.velocity.y.abs() > 1.0;
            let run_target = if low || airborne {
                0.0
            } else {
                (speed / 16.0).min(1.0)
            };
            let k = 1.0 - (-10.0 * dt).exp();
            anim.run += (run_target - anim.run) * k;
            anim.crouch += ((low as u8 as f32) - anim.crouch) * (1.0 - (-14.0 * dt).exp());
            anim.phase += speed.min(30.0) * dt * std::f32::consts::PI / 2.6;
            let anim = *anim;

            let color = e.id.as_player().map_or(PLAYER_COLORS[0], player_color);
            // Squash while sliding (the capsule halves), leaning back.
            let squash = 1.0 - 0.48 * anim.crouch;
            let base = Mat4::from_rotation_translation(Quat::from_rotation_y(e.yaw), e.position)
                * Mat4::from_scale(Vec3::new(1.0, squash, 1.0))
                * Mat4::from_rotation_x(0.35 * anim.crouch);
            let swing = anim.phase.sin() * 0.7 * anim.run;
            let tuck = if airborne { 0.5 } else { 0.0 };
            let hip = Vec3::new(0.0, 0.84, 0.0);
            for (side, s) in [(-1.0f32, swing), (1.0, -swing)] {
                let rot = Quat::from_rotation_x(s + tuck * 0.8);
                let leg_center = hip + rot * Vec3::new(side * 0.13, -0.42, 0.0);
                out.push(self.part(
                    base,
                    leg_center,
                    Vec3::new(0.09, 0.42, 0.1),
                    rot,
                    DARK,
                    false,
                ));
            }
            out.push(self.part(
                base,
                Vec3::new(0.0, 1.17, 0.0),
                Vec3::new(0.24, 0.31, 0.15),
                Quat::IDENTITY,
                CHROME,
                false,
            ));
            // Chest plate in the player's colour.
            out.push(self.part(
                base,
                Vec3::new(0.0, 1.25, -0.15),
                Vec3::new(0.16, 0.12, 0.02),
                Quat::IDENTITY,
                color,
                false,
            ));
            let shoulder = Vec3::new(0.0, 1.42, 0.0);
            for (side, s) in [(-1.0f32, -swing), (1.0, swing)] {
                let rot = Quat::from_rotation_x(s * 0.8 - tuck);
                let arm_center =
                    shoulder + Vec3::new(side * 0.33, 0.0, 0.0) + rot * Vec3::new(0.0, -0.28, 0.0);
                out.push(self.part(
                    base,
                    arm_center,
                    Vec3::new(0.07, 0.29, 0.08),
                    rot,
                    CHROME,
                    false,
                ));
                out.push(self.part(
                    base,
                    shoulder + Vec3::new(side * 0.33, 0.04, 0.0),
                    Vec3::new(0.1, 0.06, 0.11),
                    Quat::IDENTITY,
                    color,
                    false,
                ));
            }
            let head_rot = Quat::from_rotation_x(e.pitch * 0.8);
            let head_center = Vec3::new(0.0, 1.62, 0.0);
            out.push(self.part(
                base,
                head_center,
                Vec3::new(0.14, 0.14, 0.15),
                head_rot,
                CHROME,
                false,
            ));
            out.push(self.part(
                base,
                head_center + head_rot * Vec3::new(0.0, 0.02, -0.15),
                Vec3::new(0.11, 0.035, 0.02),
                head_rot,
                color,
                true,
            ));
        }
        self.anims.retain(|_, a| a.seen);
    }

    pub fn free(self, renderer: &mut Renderer) {
        renderer.free_mesh(self.cube);
    }
}
