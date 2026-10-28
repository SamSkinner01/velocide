//! Drawing other players (interpolated `NetClient::remote_players()`) with the Shrike model
//! from `client::models`.
//!
//! The replicated player entity carries everything the third-person animation needs:
//! velocity, look angles, movement-state flags (`player_flags::{ON_GROUND, SLIDING, DASHING,
//! SLAMMING, WALL_CLING}`), the held weapon (`state`) and the wall normal (`aux`). The
//! locomotion phase is kept per player here ([`PlayerAnim::advance_cycle`]); combat adds
//! `firing` / `punching` / `downed` / `dead` from its events in [`PlayerVisual`].
//!
//! Every Shrike wears its player's cosmetics (`PlayerInfo::cosmetics`: mask, crest, cloak,
//! emblem, coat / trim / flame colours) on top of the identity colour of its slot
//! ([`player_color`]: scarf, pauldron edging, eye slits, name tag), which stays readable in
//! every colour-vision palette whatever the player picked.

use std::collections::HashMap;

use glam::Vec3;
use glam::Vec4;
use shared::cosmetics::Cosmetics;
use shared::input::look_dir;
use shared::protocol::{
    EntityState, NetId, dequantize_yaw, player_armed, player_flags, player_weapon,
};
use shared::sim::PlayerId;

use super::combat::knacks::tallow_tint;
use crate::models::shrike::REMOTE_SCALE;
use crate::models::weapons::melee::{self, Swing};
use crate::models::{ModelLibrary, PlayerAnim, Tint};
use engine::gfx::DrawItem;

/// Distinct colours per player slot (sRGB), used for the Shrike's team trim, visor, name tags,
/// chat names and the scoreboard: the standard palette's (`palette::STANDARD.team`).
pub const PLAYER_COLORS: [Vec4; 8] = crate::palette::STANDARD.team;

/// The colour of a player slot in the active colour-vision palette (`palette::palette()`).
pub fn player_color(id: PlayerId) -> Vec4 {
    crate::palette::palette().team(id.0 as usize)
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
    /// A melee swing (combat starts it with `melee::Swing::start`; advances here).
    pub swing: Swing,
    /// Their Hurl's piece is out (combat sets it from the `HURL` entities they own).
    pub hurled: bool,
    pub downed: bool,
    pub dead: bool,
    /// World-space head of the last drawn pose (name tags).
    pub head: Vec3,
    /// World-space muzzle of the held weapon (tracers, muzzle flashes).
    pub muzzle: Vec3,
    pub muzzle_dir: Vec3,
    seen: bool,
}

/// The animation input for a replicated player entity wearing `look`.
pub fn player_anim(e: &EntityState, v: &PlayerVisual, time: f64, look: Cosmetics) -> PlayerAnim {
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
            .then(|| look_dir(dequantize_yaw((e.aux >> 16) as u16), 0.0)),
        firing: v.firing,
        punching: v.punching,
        downed: v.downed,
        dead: v.dead,
        time,
        weapon: player_weapon(e.state).0.id(),
        ability: player_weapon(e.state).1.id(),
        draw: if f(player_flags::DRAWING) {
            player_weapon(e.state).2 as f32 / 255.0
        } else {
            0.0
        },
        charge: if f(player_flags::CHARGING) {
            player_weapon(e.state).2 as f32 / 255.0
        } else {
            0.0
        },
        cycle: v.cycle,
        cosmetics: look,
        unarmed: !player_armed(e.state),
        swing: v.swing,
        hurled: v.hurled,
    }
}

#[derive(Default)]
pub struct RemotePlayers {
    visuals: HashMap<NetId, PlayerVisual>,
    time: f64,
    /// A player posed but not drawn (the teammate a downed player watches from their eyes,
    /// `ingame::spectate`): muzzle, head and name tag anchors stay up to date.
    pub hidden: Option<NetId>,
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

    /// Advances animation by `dt` (real time) and appends every player, each wearing
    /// `look(id)`.
    pub fn draw<'a>(
        &mut self,
        lib: Option<&ModelLibrary>,
        players: impl Iterator<Item = &'a EntityState>,
        look: &dyn Fn(PlayerId) -> Cosmetics,
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
            match melee::of_id(player_weapon(e.state).0.id()) {
                Some(m) => v.swing.advance(m, dt),
                None => v.swing = Swing::default(),
            }
            v.seen = true;
            let Some(lib) = lib else {
                v.head = name_tag_anchor(e) - Vec3::Y * 0.42;
                continue;
            };
            let id = e.id.as_player();
            let color = id.map_or(PLAYER_COLORS[0], player_color);
            let anim = player_anim(e, v, self.time, id.map(look).unwrap_or_default());
            let mut tint = Tint::team(color);
            // Tallow Skin: a pale, waxy, glistening coat (knacks.rs).
            if e.flags & player_flags::TALLOW != 0 {
                tint.body = tallow_tint(1.0, self.time);
            }
            // Other Shrikes are drawn a little larger than life so they read at a distance
            // (visual only: hit shapes and movement are unchanged).
            let mut unseen = Vec::new();
            let into = if self.hidden == Some(e.id) {
                &mut unseen
            } else {
                &mut *out
            };
            let info = lib.draw_player_scaled(&anim, e.position, REMOTE_SCALE, &tint, into);
            v.head = info.head;
            v.muzzle = info.muzzle;
            v.muzzle_dir = info.muzzle_dir;
        }
        self.visuals.retain(|_, v| v.seen);
    }
}
