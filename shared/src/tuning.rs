//! Every gameplay number lives here and is loaded from `assets/tuning.ron`, which is watched at
//! runtime so movement and combat feel can be iterated without recompiling.
//!
//! All structs use `#[serde(default)]`, so a tuning file only needs the values it overrides and
//! new fields never break old files. The server is authoritative: it sends its tuning to clients
//! on connect and whenever it reloads, so client prediction always uses the same values.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    pub movement: MovementTuning,
    pub combat: CombatTuning,
    pub net: NetTuning,
}

/// Player movement. Distances in metres, speeds in m/s, times in seconds, angles in degrees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MovementTuning {
    // Body
    pub radius: f32,
    pub standing_height: f32,
    pub slide_height: f32,
    pub eye_height: f32,
    pub slide_eye_height: f32,
    pub step_height: f32,
    pub max_ground_slope_degrees: f32,

    // Ground / air
    pub ground_speed: f32,
    pub ground_accel: f32,
    pub ground_friction: f32,
    pub air_accel: f32,
    pub air_max_speed: f32,
    pub gravity: f32,
    pub max_fall_speed: f32,
    pub jump_speed: f32,
    pub coyote_time: f32,
    pub jump_buffer_time: f32,

    // Dash
    pub dash_speed: f32,
    pub dash_duration: f32,
    pub dash_charges: f32,
    /// Charges regenerated per second.
    pub dash_regen_rate: f32,
    /// Fraction of dash speed kept when jumping out of a dash.
    pub dash_jump_keep: f32,

    // Slide
    pub slide_min_speed: f32,
    pub slide_jump_boost: f32,

    // Slam
    pub slam_speed: f32,
    pub slam_bounce_window: f32,
    pub slam_bounce_speed: f32,
    pub slam_shockwave_radius: f32,

    // Walls
    pub wall_jumps: u32,
    pub wall_jump_up_speed: f32,
    pub wall_jump_out_speed: f32,
    pub wall_cling_max_fall_speed: f32,
    pub wall_detect_distance: f32,

    // Camera feel (client only)
    pub fov_kick_degrees: f32,
    pub fov_kick_speed: f32,
    pub slide_tilt_degrees: f32,
    pub wall_tilt_degrees: f32,
    pub landing_dip: f32,
}

impl Default for MovementTuning {
    fn default() -> Self {
        Self {
            radius: 0.4,
            standing_height: 1.8,
            slide_height: 0.9,
            eye_height: 1.6,
            slide_eye_height: 0.75,
            step_height: 0.4,
            max_ground_slope_degrees: 50.0,

            ground_speed: 16.0,
            ground_accel: 160.0,
            ground_friction: 12.0,
            air_accel: 70.0,
            air_max_speed: 16.0,
            gravity: 38.0,
            max_fall_speed: 90.0,
            jump_speed: 14.0,
            coyote_time: 0.12,
            jump_buffer_time: 0.12,

            dash_speed: 48.0,
            dash_duration: 0.17,
            dash_charges: 3.0,
            dash_regen_rate: 0.85,
            dash_jump_keep: 0.6,

            slide_min_speed: 24.0,
            slide_jump_boost: 1.0,

            slam_speed: 85.0,
            slam_bounce_window: 0.2,
            slam_bounce_speed: 24.0,
            slam_shockwave_radius: 6.0,

            wall_jumps: 3,
            wall_jump_up_speed: 13.0,
            wall_jump_out_speed: 13.0,
            wall_cling_max_fall_speed: 4.0,
            wall_detect_distance: 0.25,

            fov_kick_degrees: 10.0,
            fov_kick_speed: 45.0,
            slide_tilt_degrees: 4.0,
            wall_tilt_degrees: 6.0,
            landing_dip: 0.25,
        }
    }
}

/// Combat rules. Extended as weapons/enemies are added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatTuning {
    pub max_health: f32,
    /// Fraction of incoming damage that becomes "hard damage" (temporarily caps max health).
    pub hard_damage_fraction: f32,
    pub hard_damage_recover_delay: f32,
    pub hard_damage_recover_rate: f32,
    /// Seconds the punch counts as a parry after pressing it.
    pub parry_window: f32,
}

impl Default for CombatTuning {
    fn default() -> Self {
        Self {
            max_health: 100.0,
            hard_damage_fraction: 0.35,
            hard_damage_recover_delay: 1.5,
            hard_damage_recover_rate: 25.0,
            parry_window: 0.2,
        }
    }
}

/// Networking feel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetTuning {
    /// Snapshots per second sent by the server.
    pub snapshot_rate: f32,
    /// Extra interpolation delay (ms) on top of one snapshot interval.
    pub interp_delay_ms: f32,
    /// Max rewind for lag compensation.
    pub lag_comp_max_ms: f32,
    /// Extra time (ms) a parry is accepted after the server thinks it was too late.
    pub parry_tolerance_ms: f32,
    /// Prediction errors are smoothed out at this rate (fraction of remaining error per second).
    pub correction_smoothing: f32,
    /// Errors larger than this (metres) snap instead of smoothing.
    pub correction_snap_distance: f32,
}

impl Default for NetTuning {
    fn default() -> Self {
        Self {
            snapshot_rate: 30.0,
            interp_delay_ms: 50.0,
            lag_comp_max_ms: 250.0,
            parry_tolerance_ms: 80.0,
            correction_smoothing: 12.0,
            correction_snap_distance: 3.0,
        }
    }
}

impl Tuning {
    pub fn default_path() -> PathBuf {
        crate::assets_dir().join("tuning.ron")
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(ron::from_str(&text)?)
    }

    /// Loads the tuning file, falling back to defaults (with a warning) if it is missing or
    /// invalid so a typo while hot-reloading never crashes the game.
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load(path) {
            Ok(t) => t,
            Err(err) => {
                log::warn!("using default tuning ({}: {err})", path.display());
                Self::default()
            }
        }
    }

    pub fn to_ron(&self) -> String {
        let pretty = ron::ser::PrettyConfig::new().depth_limit(3);
        ron::ser::to_string_pretty(self, pretty).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_ron() {
        let t = Tuning::default();
        let back: Tuning = ron::from_str(&t.to_ron()).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn partial_files_use_defaults() {
        let t: Tuning = ron::from_str("(movement: (jump_speed: 20.0))").unwrap();
        assert_eq!(t.movement.jump_speed, 20.0);
        assert_eq!(t.movement.gravity, MovementTuning::default().gravity);
    }
}
