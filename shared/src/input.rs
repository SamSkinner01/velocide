//! Player input commands. Clients send these to the server every tick; the server never trusts
//! anything else from a client.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Held buttons as a bitmask. Edge-triggered actions (jump pressed, dash pressed...) are derived
/// by comparing with the previous tick's buttons, which is stored in the predicted state so
/// client and server agree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Buttons(pub u16);

impl Buttons {
    pub const NONE: Buttons = Buttons(0);
    pub const JUMP: Buttons = Buttons(1 << 0);
    /// Slide on the ground, slam in the air.
    pub const CROUCH: Buttons = Buttons(1 << 1);
    pub const DASH: Buttons = Buttons(1 << 2);
    pub const FIRE: Buttons = Buttons(1 << 3);
    pub const ALT_FIRE: Buttons = Buttons(1 << 4);
    /// Melee punch; parries projectiles and telegraphed attacks when timed right.
    pub const PUNCH: Buttons = Buttons(1 << 5);
    /// Throw a ricochet prism.
    pub const THROW: Buttons = Buttons(1 << 6);
    /// Interact / revive a downed teammate.
    pub const USE: Buttons = Buttons(1 << 7);

    pub fn contains(self, other: Buttons) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }

    pub fn set(&mut self, other: Buttons, on: bool) {
        if on {
            self.0 |= other.0;
        } else {
            self.0 &= !other.0;
        }
    }

    /// True if `which` is held now but wasn't in `prev`.
    pub fn pressed(self, prev: Buttons, which: Buttons) -> bool {
        self.contains(which) && !prev.contains(which)
    }

    /// True if `which` was held in `prev` but isn't now.
    pub fn released(self, prev: Buttons, which: Buttons) -> bool {
        !self.contains(which) && prev.contains(which)
    }
}

impl std::ops::BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, rhs: Buttons) -> Buttons {
        Buttons(self.0 | rhs.0)
    }
}

/// One tick of player input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Monotonically increasing per client; used for reconciliation.
    pub seq: u32,
    /// Movement axes in `-127..=127` (quantised so client and server see identical values).
    pub forward: i8,
    pub right: i8,
    /// Absolute view angles in radians. Yaw 0 looks down -Z, positive yaw turns left
    /// (counter-clockwise seen from above). Pitch positive looks up.
    pub yaw: f32,
    pub pitch: f32,
    pub buttons: Buttons,
    /// Selected weapon slot (absolute, so a lost packet can't desync the selection).
    pub weapon: u8,
    /// Selected alt-fire mode of that weapon.
    pub weapon_mode: u8,
    /// The server tick the client was *displaying* remote entities at when this input was
    /// sampled (interpolation time), plus a fraction in 1/256ths. Used for lag compensation.
    pub view_tick: u32,
    pub view_frac: u8,
}

impl PlayerInput {
    /// Movement axes as floats, length clamped to 1. x = right, y = forward.
    pub fn move_axes(&self) -> Vec2 {
        let v = Vec2::new(self.right as f32, self.forward as f32) / 127.0;
        if v.length_squared() > 1.0 {
            v.normalize()
        } else {
            v
        }
    }

    /// Horizontal world-space direction the player wants to move in (length <= 1).
    pub fn wish_dir(&self) -> Vec3 {
        let axes = self.move_axes();
        let (s, c) = self.yaw.sin_cos();
        let forward = Vec3::new(-s, 0.0, -c);
        let right = Vec3::new(c, 0.0, -s);
        forward * axes.y + right * axes.x
    }

    /// Unit view direction.
    pub fn look_dir(&self) -> Vec3 {
        look_dir(self.yaw, self.pitch)
    }

    /// Quantises a float axis in [-1, 1] to the wire format.
    pub fn quantize_axis(v: f32) -> i8 {
        (v.clamp(-1.0, 1.0) * 127.0).round() as i8
    }

    /// The (fractional) server tick this input was viewing remote entities at.
    pub fn view_time_ticks(&self) -> f64 {
        self.view_tick as f64 + self.view_frac as f64 / 256.0
    }
}

/// Unit direction for a yaw/pitch pair (same convention as [`PlayerInput`]).
pub fn look_dir(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    Vec3::new(-sy * cp, sp, -cy * cp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_zero_looks_down_negative_z() {
        let d = look_dir(0.0, 0.0);
        assert!((d - Vec3::NEG_Z).length() < 1e-6);
        let input = PlayerInput {
            forward: 127,
            ..Default::default()
        };
        assert!((input.wish_dir() - Vec3::NEG_Z).length() < 1e-6);
        let input = PlayerInput {
            right: 127,
            ..Default::default()
        };
        assert!((input.wish_dir() - Vec3::X).length() < 1e-6);
    }

    #[test]
    fn positive_yaw_turns_left() {
        let d = look_dir(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((d - Vec3::NEG_X).length() < 1e-6);
    }

    #[test]
    fn button_edges() {
        let prev = Buttons::NONE;
        let now = Buttons::JUMP | Buttons::FIRE;
        assert!(now.pressed(prev, Buttons::JUMP));
        assert!(!now.pressed(now, Buttons::JUMP));
        assert!(prev.released(now, Buttons::FIRE));
    }
}
