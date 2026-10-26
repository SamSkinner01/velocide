//! Client-side "juice": trauma-based screen shake, hitstop (a *visual* time scale) and screen
//! flashes. Gameplay code calls these from its event handlers:
//!
//! ```ignore
//! game.feedback.shake(0.4);                      // add trauma (0..1), decays by itself
//! game.feedback.hitstop(0.12);                   // freeze what is drawn for 120 ms
//! game.feedback.parry_flash();                   // colours follow palette::palette()
//! game.feedback.damage_flash(damage / max_hp);   // red, scaled (also shield_ / death_flash)
//! ```
//!
//! Hitstop only slows what is *drawn* (camera shake, the viewmodel, particles and decal fades):
//! [`Feedback::update`] turns the real frame time into a visual frame time. The simulation,
//! prediction and networking always run on real time. Every effect respects the player's
//! `RetroSettings::{screen_shake, screen_flashes, hitstop}` toggles.

use engine::gfx::RetroSettings;
use glam::{Vec3, Vec4};

use crate::palette::palette;

/// Trauma lost per second.
const TRAUMA_DECAY: f32 = 1.4;
/// Maximum camera rotation from shake (radians): yaw, pitch, roll.
const MAX_SHAKE_ANGLES: Vec3 = Vec3::new(0.045, 0.045, 0.06);
/// Maximum camera offset from shake (metres).
const MAX_SHAKE_OFFSET: f32 = 0.08;
/// Seconds for a full-strength flash to fade out.
const FLASH_FADE: f32 = 0.28;
/// Longest hitstop accepted (a stack of hits shouldn't freeze the view for long).
const MAX_HITSTOP: f32 = 0.35;

/// Camera offsets produced by the shake this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShakeOffset {
    /// Yaw, pitch, roll in radians.
    pub angles: Vec3,
    /// View-space translation in metres.
    pub offset: Vec3,
}

#[derive(Clone, Debug)]
pub struct Feedback {
    trauma: f32,
    hitstop: f32,
    time_scale: f32,
    flash_color: Vec3,
    flash: f32,
    visual_time: f64,
    shake: ShakeOffset,
    enable_shake: bool,
    enable_flashes: bool,
    enable_hitstop: bool,
}

impl Default for Feedback {
    fn default() -> Self {
        Self {
            trauma: 0.0,
            hitstop: 0.0,
            time_scale: 1.0,
            flash_color: Vec3::ONE,
            flash: 0.0,
            visual_time: 0.0,
            shake: ShakeOffset::default(),
            enable_shake: true,
            enable_flashes: true,
            enable_hitstop: true,
        }
    }
}

/// Smooth pseudo-noise in about [-1, 1] (sum of incommensurate sines).
fn wobble(t: f64, seed: f64) -> f32 {
    let a = (t * 31.0 + seed * 1.7).sin();
    let b = (t * 47.3 + seed * 3.1).sin();
    let c = (t * 13.9 + seed * 5.3).sin();
    (a * 0.5 + b * 0.3 + c * 0.2) as f32
}

impl Feedback {
    /// The standard palette's flash colours; the helpers below follow the colour-vision
    /// palette (`palette::palette().flash_*`).
    pub const DAMAGE_RED: Vec3 = crate::palette::STANDARD.flash_damage;
    pub const PARRY_WHITE: Vec3 = crate::palette::STANDARD.flash_parry;

    pub fn new() -> Self {
        Self::default()
    }

    /// Follow the player's toggles.
    pub fn apply_settings(&mut self, retro: &RetroSettings) {
        self.enable_shake = retro.screen_shake;
        self.enable_flashes = retro.screen_flashes;
        self.enable_hitstop = retro.hitstop;
        if !self.enable_shake {
            self.trauma = 0.0;
            self.shake = ShakeOffset::default();
        }
        if !self.enable_flashes {
            self.flash = 0.0;
        }
        if !self.enable_hitstop {
            self.hitstop = 0.0;
        }
    }

    /// Adds screen-shake trauma (0..1). Shake strength is trauma², so small hits stay subtle
    /// and big ones stack up fast.
    pub fn shake(&mut self, trauma: f32) {
        if self.enable_shake && trauma.is_finite() {
            self.trauma = (self.trauma + trauma.max(0.0)).min(1.0);
        }
    }

    /// Freezes what is drawn for `seconds` (the longest pending hitstop wins).
    pub fn hitstop(&mut self, seconds: f32) {
        if self.enable_hitstop && seconds.is_finite() {
            self.hitstop = self.hitstop.max(seconds.clamp(0.0, MAX_HITSTOP));
        }
    }

    /// Visual slow motion (1 = normal). Multiplies with hitstop.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = if scale.is_finite() {
            scale.clamp(0.0, 4.0)
        } else {
            1.0
        };
    }

    /// Full-screen colour flash (sRGB colour, strength 0..1 = alpha at its peak).
    pub fn flash(&mut self, color: Vec3, strength: f32) {
        if !self.enable_flashes || !strength.is_finite() {
            return;
        }
        let strength = strength.clamp(0.0, 1.0);
        if strength >= self.flash {
            self.flash_color = color;
        }
        self.flash = self.flash.max(strength);
    }

    /// Damage flash (red; orange-red for red-blind palettes) scaled by how much of the
    /// player's health a hit took (0..1).
    pub fn damage_flash(&mut self, fraction: f32) {
        let color = palette().flash_damage;
        self.flash(color, (0.18 + fraction * 0.9).clamp(0.0, 0.55));
    }

    /// White parry flash.
    pub fn parry_flash(&mut self) {
        self.flash(palette().flash_parry, 0.6);
    }

    /// Faint pale-blue flash: a hit blocked by spawn protection.
    pub fn shield_flash(&mut self) {
        self.flash(palette().flash_shield, 0.12);
    }

    /// Dark red flash when the local player dies.
    pub fn death_flash(&mut self) {
        self.flash(palette().flash_death, 0.5);
    }

    /// Faint flash when the local player heals `amount` hp (kindling).
    pub fn heal_flash(&mut self, amount: f32) {
        let strength = (0.08 + amount.max(0.0) / 120.0).min(0.25);
        self.flash(palette().flash_heal, strength);
    }

    /// Advances by a real frame and returns the *visual* frame time (0 during hitstop).
    pub fn update(&mut self, real_dt: f32) -> f32 {
        let real_dt = real_dt.max(0.0);
        let mut visual = real_dt * self.time_scale;
        if self.hitstop > 0.0 {
            let frozen = self.hitstop.min(real_dt);
            self.hitstop -= frozen;
            visual = (real_dt - frozen) * self.time_scale;
        }
        // Flashes fade on real time: a parry flash should read even during the hitstop.
        self.flash = (self.flash - real_dt / FLASH_FADE).max(0.0);

        self.visual_time += visual as f64;
        self.trauma = (self.trauma - TRAUMA_DECAY * visual).max(0.0);
        let s = self.trauma * self.trauma;
        let t = self.visual_time;
        self.shake = if s > 0.0 {
            ShakeOffset {
                angles: Vec3::new(
                    MAX_SHAKE_ANGLES.x * s * wobble(t, 1.0),
                    MAX_SHAKE_ANGLES.y * s * wobble(t, 2.0),
                    MAX_SHAKE_ANGLES.z * s * wobble(t, 3.0),
                ),
                offset: Vec3::new(wobble(t, 4.0), wobble(t, 5.0), 0.0) * MAX_SHAKE_OFFSET * s,
            }
        } else {
            ShakeOffset::default()
        };
        visual
    }

    pub fn shake_offset(&self) -> ShakeOffset {
        self.shake
    }

    pub fn trauma(&self) -> f32 {
        self.trauma
    }

    pub fn in_hitstop(&self) -> bool {
        self.hitstop > 0.0
    }

    /// Seconds of visual time so far (drives visual-only animation).
    pub fn visual_time(&self) -> f64 {
        self.visual_time
    }

    /// `FrameDesc::screen_flash` value for this frame.
    pub fn screen_flash(&self) -> Vec4 {
        // Ease out: bright at the start, quick tail.
        let a = self.flash * self.flash.sqrt();
        self.flash_color.extend(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hitstop_freezes_visual_time_only() {
        let mut f = Feedback::new();
        f.hitstop(0.1);
        assert_eq!(f.update(0.05), 0.0);
        let v = f.update(0.08);
        assert!((v - 0.03).abs() < 1e-6, "partially frozen frame: {v}");
        assert_eq!(f.update(0.016), 0.016);
    }

    #[test]
    fn settings_disable_effects() {
        let mut f = Feedback::new();
        let retro = RetroSettings {
            screen_shake: false,
            screen_flashes: false,
            hitstop: false,
            ..Default::default()
        };
        f.apply_settings(&retro);
        f.shake(1.0);
        f.flash(Vec3::ONE, 1.0);
        f.hitstop(0.2);
        assert_eq!(f.update(0.016), 0.016);
        assert_eq!(f.shake_offset(), ShakeOffset::default());
        assert_eq!(f.screen_flash().w, 0.0);
    }

    #[test]
    fn trauma_decays() {
        let mut f = Feedback::new();
        f.shake(0.8);
        f.update(0.016);
        assert!(f.shake_offset().angles.length() > 0.0 || f.trauma() > 0.0);
        for _ in 0..120 {
            f.update(0.016);
        }
        assert_eq!(f.trauma(), 0.0);
        assert_eq!(f.shake_offset(), ShakeOffset::default());
    }
}
