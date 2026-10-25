//! Sounds for movement.
//!
//! - The local player's sounds come from the **predicted** [`MoveEvents`] returned by
//!   `NetClient::tick` (instant, no round trip), played 2D.
//! - Remote players don't send events, so [`RemoteSounds`] derives footsteps, jumps, landings,
//!   dashes and slides from their interpolated entity state and plays them positionally.
//!
//! Combat sounds belong next to the code that handles the combat events (see docs/client.md).

use std::collections::HashMap;

use engine::audio::{Audio, PlayParams, Sfx};
use glam::{Vec2, Vec3};
use shared::movement::MoveEvents;
use shared::protocol::{EntityState, NetId, player_flags};

/// Landing speed above which the heavy landing sound plays.
const HEAVY_LANDING: f32 = 22.0;

/// Plays the local player's movement sounds for one predicted tick.
pub fn play_local_move_events(audio: &mut Audio, ev: &MoveEvents) {
    if ev.footstep {
        audio.play_2d(Sfx::Footstep);
    }
    if ev.wall_jumped {
        audio.play_2d(Sfx::WallJump);
    } else if ev.jumped {
        let pitch = if ev.slam_bounced {
            0.85
        } else if ev.dash_jumped || ev.slide_jumped {
            1.08
        } else {
            1.0
        };
        audio.play(Sfx::Jump, PlayParams::default().with_pitch(pitch));
        if ev.slam_bounced {
            audio.play(
                Sfx::JumpPad,
                PlayParams::default().with_volume(0.5).with_pitch(1.3),
            );
        }
    }
    if let Some(speed) = ev.landed
        && ev.slam_landed.is_none()
    {
        if speed >= HEAVY_LANDING {
            audio.play_2d(Sfx::HeavyLand);
        } else if speed > 3.0 {
            audio.play(
                Sfx::Land,
                PlayParams::default().with_volume((speed / 14.0).clamp(0.35, 1.0)),
            );
        }
    }
    if ev.dashed {
        audio.play_2d(Sfx::Dash);
    }
    if ev.dash_failed {
        audio.play_2d(Sfx::NoStamina);
    }
    if ev.slide_started {
        audio.play_2d(Sfx::Slide);
    }
    if ev.slide_ended {
        audio.stop(Sfx::Slide);
    }
    if ev.slam_started {
        audio.play_2d(Sfx::Slam);
    }
    if let Some(slam) = ev.slam_landed {
        audio.stop(Sfx::Slam);
        audio.play(
            Sfx::SlamImpact,
            PlayParams::default().with_volume(0.6 + 0.4 * slam.power),
        );
    }
    if ev.jump_pad.is_some() {
        audio.play_2d(Sfx::JumpPad);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Track {
    last_pos: Vec3,
    last_vel: Vec3,
    stride: f32,
    sliding: bool,
    dashing: bool,
    seen: bool,
}

/// Positional movement sounds of remote players, inferred from interpolated state.
#[derive(Default)]
pub struct RemoteSounds {
    tracks: HashMap<NetId, Track>,
}

impl RemoteSounds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update<'a>(
        &mut self,
        audio: &mut Audio,
        players: impl Iterator<Item = &'a EntityState>,
    ) {
        for t in self.tracks.values_mut() {
            t.seen = false;
        }
        for e in players {
            let pos = e.position;
            let vel = e.velocity;
            let Some(t) = self.tracks.get_mut(&e.id) else {
                self.tracks.insert(
                    e.id,
                    Track {
                        last_pos: pos,
                        last_vel: vel,
                        seen: true,
                        ..Default::default()
                    },
                );
                continue;
            };
            t.seen = true;
            let moved = pos - t.last_pos;
            if moved.length() > 4.0 {
                // Teleport / respawn: just resync.
                *t = Track {
                    last_pos: pos,
                    last_vel: vel,
                    seen: true,
                    ..Default::default()
                };
                continue;
            }
            let at = PlayParams::at(pos);
            let grounded = vel.y.abs() < 0.5 && t.last_vel.y.abs() < 0.5;
            let low = e.flags & player_flags::LOW_PROFILE != 0;
            let hspeed = Vec2::new(vel.x, vel.z).length();

            if vel.y > 8.0 && t.last_vel.y < 3.0 {
                audio.play(Sfx::Jump, at);
            }
            if t.last_vel.y < -6.0 && vel.y > -0.5 {
                let sfx = if t.last_vel.y < -HEAVY_LANDING {
                    Sfx::HeavyLand
                } else {
                    Sfx::Land
                };
                audio.play(sfx, at);
            }
            let dashing = hspeed > 40.0;
            if dashing && !t.dashing {
                audio.play(Sfx::Dash, at);
            }
            t.dashing = dashing;
            let sliding = low && grounded && hspeed > 10.0;
            if sliding && !t.sliding {
                audio.play(Sfx::Slide, at);
            }
            t.sliding = sliding;
            if grounded && !low && hspeed > 3.0 {
                t.stride += Vec2::new(moved.x, moved.z).length();
                if t.stride >= 2.6 {
                    t.stride = 0.0;
                    audio.play(Sfx::Footstep, at);
                }
            }
            t.last_pos = pos;
            t.last_vel = vel;
        }
        self.tracks.retain(|_, t| t.seen);
    }
}
