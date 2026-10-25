//! Drawing other players (interpolated `NetClient::remote_players()`) with the Shrike model
//! from `client::models`.
//!
//! The replicated player entity carries everything the third-person animation needs:
//! velocity, look angles, movement-state flags (`player_flags::{ON_GROUND, SLIDING, DASHING,
//! SLAMMING, WALL_CLING}`), the held weapon (`state`) and the wall normal (`aux`). The
//! locomotion phase is kept per player here ([`PlayerAnim::advance_cycle`]); combat adds
//! `firing` / `punching` / `downed` / `dead` from its events in [`PlayerVisual`].

use std::collections::HashMap;

use glam::Vec3;
use glam::Vec4;
use shared::input::look_dir;
use shared::protocol::{EntityState, NetId, dequantize_yaw, player_flags};
use shared::sim::PlayerId;

use crate::models::{ModelLibrary, PlayerAnim, Tint};
use engine::gfx::DrawItem;

/// Distinct colours per player slot (sRGB), used for the Shrike's team trim, visor, name tags,
/// chat names and the scoreboard.
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

/// Name-tag anchor without a model (above the capsule).
pub fn name_tag_anchor(e: &EntityState) -> Vec3 {
    let low = e.flags & player_flags::LOW_PROFILE != 0;
    e.position + Vec3::Y * if low { 1.35 } else { 2.15 }
}

/// Client-side animation state of one remote player.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerVisual {
    /// Locomotion phase (cycles).
    pub cycle: f32,
    /// Recoil 0..1 (combat sets 1 on a shot event; decays here).
    pub firing: f32,
    /// Punch progress 0..1 (combat starts it at a small value; advances here).
    pub punching: f32,
    pub downed: bool,
    pub dead: bool,
    /// World-space head of the last drawn pose (name tags).
    pub head: Vec3,
    /// World-space muzzle of the held weapon (tracers, muzzle flashes).
    pub muzzle: Vec3,
    pub muzzle_dir: Vec3,
    seen: bool,
}

/// The animation input for a replicated player entity.
pub fn player_anim(e: &EntityState, v: &PlayerVisual, time: f64) -> PlayerAnim {
    let f = |bit: u32| e.flags & bit != 0;
    PlayerAnim {
        velocity: e.velocity,
        yaw: e.yaw,
        pitch: e.pitch,
        on_ground: f(player_flags::ON_GROUND),
        sliding: f(player_flags::SLIDING),
        dashing: f(player_flags::DASHING),
        slamming: f(player_flags::SLAMMING),
        wall_cling: f(player_flags::WALL_CLING)
            .then(|| look_dir(dequantize_yaw(e.aux as u16), 0.0)),
        firing: v.firing,
        punching: v.punching,
        downed: v.downed,
        dead: v.dead,
        time,
        weapon: (e.state & 0xff) as u8,
        cycle: v.cycle,
    }
}

#[derive(Default)]
pub struct RemotePlayers {
    visuals: HashMap<NetId, PlayerVisual>,
    time: f64,
}

impl RemotePlayers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Animation state of a player (combat code sets `firing` / `punching` / `dead` here).
    pub fn visual_mut(&mut self, id: NetId) -> &mut PlayerVisual {
        self.visuals.entry(id).or_default()
    }

    pub fn visual(&self, id: NetId) -> Option<&PlayerVisual> {
        self.visuals.get(&id)
    }

    /// Where the name tag of `e` goes (above the model's head when drawn).
    pub fn name_tag_position(&self, e: &EntityState) -> Vec3 {
        match self.visuals.get(&e.id) {
            Some(v) if v.seen => v.head + Vec3::Y * 0.42,
            _ => name_tag_anchor(e),
        }
    }

    /// Advances animation by `dt` (real time) and appends every player.
    pub fn draw<'a>(
        &mut self,
        lib: Option<&ModelLibrary>,
        players: impl Iterator<Item = &'a EntityState>,
        dt: f32,
        out: &mut Vec<DrawItem>,
    ) {
        self.time += dt as f64;
        for v in self.visuals.values_mut() {
            v.seen = false;
        }
        for e in players {
            let v = self.visuals.entry(e.id).or_default();
            v.cycle = PlayerAnim::advance_cycle(v.cycle, e.velocity, dt);
            v.firing = (v.firing - dt / 0.2).max(0.0);
            if v.punching > 0.0 {
                v.punching += dt / 0.35;
                if v.punching >= 1.0 {
                    v.punching = 0.0;
                }
            }
            v.seen = true;
            let Some(lib) = lib else {
                v.head = name_tag_anchor(e) - Vec3::Y * 0.42;
                continue;
            };
            let color = e.id.as_player().map_or(PLAYER_COLORS[0], player_color);
            let anim = player_anim(e, v, self.time);
            let info = lib.draw_player(&anim, e.position, &Tint::team(color), out);
            v.head = info.head;
            v.muzzle = info.muzzle;
            v.muzzle_dir = info.muzzle_dir;
        }
        self.visuals.retain(|_, v| v.seen);
    }
}
