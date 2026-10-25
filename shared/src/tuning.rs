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

/// Player movement. Distances in metres, speeds in m/s, accelerations in m/s², times in
/// seconds, angles in degrees. See `docs/movement.md` for what each value does and why it has
/// its default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MovementTuning {
    // Body
    pub radius: f32,
    pub standing_height: f32,
    /// Capsule height while sliding (and while stuck crouched under a low ceiling).
    pub slide_height: f32,
    pub eye_height: f32,
    pub slide_eye_height: f32,
    /// Ledges up to this height are climbed automatically while on the ground; also the
    /// maximum distance the player is snapped down to stay on the ground (stairs, ramps).
    pub step_height: f32,
    /// Steeper surfaces are not ground: you slide off them and can't walk up them.
    pub max_ground_slope_degrees: f32,

    // Ground / air
    pub ground_speed: f32,
    /// Acceleration towards the wish velocity (also used for braking and turning).
    pub ground_accel: f32,
    /// How fast speed above `ground_speed` bleeds off while running with input held.
    pub ground_overspeed_decel: f32,
    /// Quake-style friction, applied only when there is no movement input.
    pub ground_friction: f32,
    /// Below this speed friction acts as if moving at this speed (quick final stop).
    pub ground_stop_speed: f32,
    pub air_accel: f32,
    /// Air control accelerates up to this speed; it never reduces speed above it.
    pub air_max_speed: f32,
    pub gravity: f32,
    pub max_fall_speed: f32,
    /// Hard cap on total speed (safety net for stacked explosions).
    pub max_speed: f32,
    pub jump_speed: f32,
    pub coyote_time: f32,
    pub jump_buffer_time: f32,
    /// Horizontal distance between footstep events while walking.
    pub footstep_stride: f32,

    // Dash
    pub dash_speed: f32,
    pub dash_duration: f32,
    /// Invulnerability granted when a dash starts.
    pub dash_invuln_time: f32,
    pub dash_charges: f32,
    /// Charges regenerated per second.
    pub dash_regen_rate: f32,
    /// Horizontal speed kept when a dash ends normally (or the speed you dashed in with, if
    /// that was higher).
    pub dash_end_speed: f32,
    /// Fraction of dash speed kept when jumping out of a dash.
    pub dash_jump_keep: f32,

    // Slide
    pub slide_min_speed: f32,
    /// Slopes can't accelerate a slide beyond this (entry speed above it is kept).
    pub slide_max_speed: f32,
    /// Steering rate with full sideways input.
    pub slide_steer_degrees: f32,
    /// Downhill acceleration as a fraction of gravity × sin(slope).
    pub slide_slope_accel: f32,
    /// Multiplier on horizontal speed when jumping out of a slide.
    pub slide_jump_boost: f32,

    // Slam
    pub slam_speed: f32,
    /// Pressing crouch closer than this to the ground doesn't slam (you slide on landing).
    pub slam_min_height: f32,
    /// Fraction of horizontal speed kept when a slam starts.
    pub slam_horizontal_keep: f32,
    /// Slam fall height that gives full power (1.0) to the landing shockwave and bounce.
    pub slam_full_power_height: f32,
    pub slam_bounce_window: f32,
    pub slam_bounce_speed: f32,
    /// Extra bounce speed at full slam power.
    pub slam_bounce_power_bonus: f32,
    /// Used by the server for the landing shockwave.
    pub slam_shockwave_radius: f32,

    // Walls
    pub wall_jumps: u32,
    pub wall_jump_up_speed: f32,
    pub wall_jump_out_speed: f32,
    /// Extra speed in the input direction (along/away from the wall) on a wall jump.
    pub wall_jump_input_speed: f32,
    pub wall_cling_max_fall_speed: f32,
    pub wall_detect_distance: f32,

    // Camera feel (client only)
    pub fov_kick_degrees: f32,
    /// Speed at which the full FOV kick is reached (starts at `ground_speed`).
    pub fov_kick_speed: f32,
    pub slide_tilt_degrees: f32,
    pub wall_tilt_degrees: f32,
    /// Eye dip (metres) for a hard landing.
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
            step_height: 0.45,
            max_ground_slope_degrees: 50.0,

            ground_speed: 16.0,
            ground_accel: 160.0,
            ground_overspeed_decel: 30.0,
            ground_friction: 10.0,
            ground_stop_speed: 4.0,
            air_accel: 70.0,
            air_max_speed: 16.0,
            gravity: 38.0,
            max_fall_speed: 90.0,
            max_speed: 150.0,
            jump_speed: 14.0,
            coyote_time: 0.12,
            jump_buffer_time: 0.12,
            footstep_stride: 2.6,

            dash_speed: 48.0,
            dash_duration: 0.17,
            dash_invuln_time: 0.2,
            dash_charges: 3.0,
            dash_regen_rate: 0.85,
            dash_end_speed: 20.0,
            dash_jump_keep: 0.6,

            slide_min_speed: 24.0,
            slide_max_speed: 60.0,
            slide_steer_degrees: 70.0,
            slide_slope_accel: 1.0,
            slide_jump_boost: 1.0,

            slam_speed: 75.0,
            slam_min_height: 1.0,
            slam_horizontal_keep: 0.0,
            slam_full_power_height: 15.0,
            slam_bounce_window: 0.2,
            slam_bounce_speed: 21.0,
            slam_bounce_power_bonus: 5.0,
            slam_shockwave_radius: 6.0,

            wall_jumps: 3,
            wall_jump_up_speed: 13.0,
            wall_jump_out_speed: 11.0,
            wall_jump_input_speed: 4.0,
            wall_cling_max_fall_speed: 4.0,
            wall_detect_distance: 0.4,

            fov_kick_degrees: 10.0,
            fov_kick_speed: 45.0,
            slide_tilt_degrees: 4.0,
            wall_tilt_degrees: 7.0,
            landing_dip: 0.25,
        }
    }
}

/// Combat rules: player vitals, fist/parry, blood healing, prisms, explosions and every weapon.
/// Damage in HP, distances in metres, speeds in m/s, times in seconds, angles in degrees
/// (cones are *full* apertures: a 70° cone accepts ±35° off the aim). See `docs/gameplay.md`
/// for the rules and `docs/combat.md` for how each value is used. Enemy numbers live in
/// `assets/enemies/*.ron`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CombatTuning {
    // Player
    pub max_health: f32,
    /// Fraction of incoming damage that becomes "hard damage" (temporarily caps max health).
    pub hard_damage_fraction: f32,
    pub hard_damage_recover_delay: f32,
    pub hard_damage_recover_rate: f32,
    /// Seconds from death to respawn (default rule; game modes replace it).
    pub respawn_delay: f32,
    /// No damage for this long after (re)spawning.
    pub spawn_protection: f32,
    /// Damage multiplier for hitting a player's head (PvP).
    pub player_head_multiplier: f32,
    /// Fraction of its own explosion's damage a player takes (knockback is always full).
    pub self_damage_fraction: f32,
    /// Maximum range of hitscan weapons.
    pub hitscan_range: f32,

    // Fist / parry
    /// Seconds the punch counts as a parry after pressing it.
    pub parry_window: f32,
    /// Parryable projectiles within this distance (and cone) of the eye are deflected.
    pub parry_range: f32,
    pub parry_cone_degrees: f32,
    /// Projectiles this close are parried at any angle (they are touching you).
    pub parry_close_range: f32,
    /// Deflected projectiles fly this much faster (flak uses its own multiplier).
    pub parry_speed_multiplier: f32,
    pub parry_damage_multiplier: f32,
    /// Deflections auto-aim at the nearest hostile within this cone / range.
    pub parry_auto_aim_degrees: f32,
    pub parry_auto_aim_range: f32,
    /// Fraction of hard damage a successful parry clears.
    pub parry_hard_damage_clear: f32,
    /// Delay damage from parryable attacks by the victim's view lag as well as
    /// `net.parry_tolerance_ms`, so a parry made on what the victim saw can still cancel it at
    /// real-world pings. `false` = delay by the tolerance only.
    pub parry_compensate_view_lag: bool,
    /// Range for parrying telegraphed melee attacks.
    pub melee_parry_range: f32,
    pub punch_damage: f32,
    pub punch_range: f32,
    pub punch_cone_degrees: f32,
    pub punch_knockback: f32,
    pub punch_cooldown: f32,

    // Explosions
    /// Damage/knockback fraction at the edge of the radius (linear from 1.0 at the centre).
    pub explosion_edge_fraction: f32,
    /// Upward bias added to the knockback direction before normalising.
    pub explosion_up_bias: f32,

    // Slam shockwave (radius: `movement.slam_shockwave_radius`)
    pub slam_damage: f32,
    pub slam_knockback: f32,
    /// Damage/knockback scale of a zero-power slam (full power = 1.0).
    pub slam_min_power_scale: f32,

    pub blood: BloodTuning,
    pub prism: PrismTuning,
    pub lancet: LancetTuning,
    pub thresher: ThresherTuning,
    pub riveter: RiveterTuning,
    pub cinderbox: CinderboxTuning,
    pub enemy_projectiles: EnemyProjectileTuning,
}

impl Default for CombatTuning {
    fn default() -> Self {
        Self {
            max_health: 100.0,
            hard_damage_fraction: 0.35,
            hard_damage_recover_delay: 1.5,
            hard_damage_recover_rate: 25.0,
            respawn_delay: 2.0,
            spawn_protection: 1.5,
            player_head_multiplier: 2.0,
            self_damage_fraction: 0.35,
            hitscan_range: 400.0,

            parry_window: 0.2,
            parry_range: 3.0,
            parry_cone_degrees: 70.0,
            parry_close_range: 1.2,
            parry_speed_multiplier: 1.5,
            parry_damage_multiplier: 2.0,
            parry_auto_aim_degrees: 15.0,
            parry_auto_aim_range: 80.0,
            parry_hard_damage_clear: 0.5,
            parry_compensate_view_lag: true,
            melee_parry_range: 4.0,
            punch_damage: 25.0,
            punch_range: 2.6,
            punch_cone_degrees: 50.0,
            punch_knockback: 10.0,
            punch_cooldown: 0.4,

            explosion_edge_fraction: 0.3,
            explosion_up_bias: 0.3,

            slam_damage: 20.0,
            slam_knockback: 12.0,
            slam_min_power_scale: 0.3,

            blood: BloodTuning::default(),
            prism: PrismTuning::default(),
            lancet: LancetTuning::default(),
            thresher: ThresherTuning::default(),
            riveter: RiveterTuning::default(),
            cinderbox: CinderboxTuning::default(),
            enemy_projectiles: EnemyProjectileTuning::default(),
        }
    }
}

/// Blood bursts: close-range healing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BloodTuning {
    /// Heal pool as a fraction of the damage dealt.
    pub heal_fraction: f32,
    /// Heal pool fraction for melee and parry damage.
    pub melee_heal_fraction: f32,
    pub radius: f32,
    pub lifetime: f32,
    /// Big burst when an enemy dies.
    pub death_pool: f32,
    pub death_radius: f32,
    pub death_lifetime: f32,
    /// A new burst this close to a burst younger than `merge_window` is merged into it
    /// (shotgun pellets make one burst, not ten).
    pub merge_distance: f32,
    pub merge_window: f32,
}

impl Default for BloodTuning {
    fn default() -> Self {
        Self {
            heal_fraction: 0.4,
            melee_heal_fraction: 1.0,
            radius: 3.5,
            lifetime: 0.6,
            death_pool: 25.0,
            death_radius: 4.5,
            death_lifetime: 1.0,
            merge_distance: 1.0,
            merge_window: 0.2,
        }
    }
}

/// Ricochet prisms.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrismTuning {
    pub charges: u32,
    /// Seconds per recharged prism.
    pub recharge_time: f32,
    pub throw_speed: f32,
    pub throw_up_speed: f32,
    pub gravity: f32,
    pub lifetime: f32,
    /// Collision radius against the world.
    pub radius: f32,
    /// A hitscan shot passing this close to a live prism ricochets off it.
    pub ricochet_radius: f32,
    /// Another live prism within this range is preferred as the next bounce (chain).
    pub chain_range: f32,
    /// Otherwise the nearest hostile within this range (weak point preferred, line of sight).
    pub target_range: f32,
    /// Damage multiplier per bounce.
    pub damage_multiplier: f32,
    /// Speed kept when bouncing off the world.
    pub restitution: f32,
}

impl Default for PrismTuning {
    fn default() -> Self {
        Self {
            charges: 4,
            recharge_time: 2.5,
            throw_speed: 18.0,
            throw_up_speed: 6.0,
            gravity: 20.0,
            lifetime: 4.0,
            radius: 0.2,
            ricochet_radius: 0.6,
            chain_range: 30.0,
            target_range: 40.0,
            damage_multiplier: 1.5,
            restitution: 0.35,
        }
    }
}

/// 1 — Lancet: hitscan sidearm. Alt A Overcharge (hold, release), alt B Fan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LancetTuning {
    pub damage: f32,
    pub interval: f32,
    pub overcharge_time: f32,
    pub overcharge_min_damage: f32,
    pub overcharge_max_damage: f32,
    /// Targets the beam passes through (the first one included).
    pub overcharge_pierce: u32,
    pub overcharge_cooldown: f32,
    pub fan_shots: u32,
    /// Outermost shot's angle off the aim (±).
    pub fan_spread_degrees: f32,
    pub fan_damage: f32,
    pub fan_cooldown: f32,
}

impl Default for LancetTuning {
    fn default() -> Self {
        Self {
            damage: 22.0,
            interval: 0.30,
            overcharge_time: 0.8,
            overcharge_min_damage: 25.0,
            overcharge_max_damage: 75.0,
            overcharge_pierce: 5,
            overcharge_cooldown: 0.5,
            fan_shots: 3,
            fan_spread_degrees: 6.0,
            fan_damage: 15.0,
            fan_cooldown: 0.9,
        }
    }
}

/// 2 — Thresher: hitscan scattergun. Alt A Slug, alt B Flak (parryable grenade).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThresherTuning {
    pub pellets: u32,
    pub pellet_damage: f32,
    pub cone_degrees: f32,
    pub interval: f32,
    pub pellet_knockback: f32,
    pub pellet_range: f32,
    pub slug_damage: f32,
    pub slug_cooldown: f32,
    pub slug_knockback: f32,
    /// Shooter is pushed back this much (server-side impulse).
    pub slug_recoil: f32,
    pub flak_speed: f32,
    pub flak_gravity: f32,
    pub flak_fuse: f32,
    pub flak_radius: f32,
    pub flak_damage: f32,
    pub flak_knockback: f32,
    pub flak_cooldown: f32,
    pub flak_projectile_radius: f32,
    /// Speed multiplier when a flak grenade is punched (instead of `parry_speed_multiplier`).
    pub flak_parry_speed_multiplier: f32,
}

impl Default for ThresherTuning {
    fn default() -> Self {
        Self {
            pellets: 10,
            pellet_damage: 9.0,
            cone_degrees: 7.0,
            interval: 0.75,
            pellet_knockback: 2.0,
            pellet_range: 120.0,
            slug_damage: 70.0,
            slug_cooldown: 1.0,
            slug_knockback: 14.0,
            slug_recoil: 6.0,
            flak_speed: 28.0,
            flak_gravity: 25.0,
            flak_fuse: 1.2,
            flak_radius: 5.0,
            flak_damage: 55.0,
            flak_knockback: 18.0,
            flak_cooldown: 1.0,
            flak_projectile_radius: 0.2,
            flak_parry_speed_multiplier: 2.0,
        }
    }
}

/// 3 — Riveter: projectile nailer. Alt A Sawblade, alt B Volley.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiveterTuning {
    pub rivet_speed: f32,
    pub rivet_damage: f32,
    /// Rivets per second while held.
    pub rate: f32,
    pub spread_degrees: f32,
    pub rivet_radius: f32,
    pub rivet_lifetime: f32,
    pub saw_speed: f32,
    /// Damage per `saw_damage_interval` of contact.
    pub saw_damage: f32,
    pub saw_damage_interval: f32,
    pub saw_bounces: u32,
    pub saw_lifetime: f32,
    pub saw_cooldown: f32,
    pub saw_radius: f32,
    pub volley_count: u32,
    /// Angle of the ring off the aim.
    pub volley_ring_degrees: f32,
    pub volley_damage: f32,
    pub volley_cooldown: f32,
}

impl Default for RiveterTuning {
    fn default() -> Self {
        Self {
            rivet_speed: 90.0,
            rivet_damage: 8.0,
            rate: 12.0,
            spread_degrees: 1.5,
            rivet_radius: 0.08,
            rivet_lifetime: 2.0,
            saw_speed: 40.0,
            saw_damage: 10.0,
            saw_damage_interval: 0.1,
            saw_bounces: 3,
            saw_lifetime: 3.0,
            saw_cooldown: 1.0,
            saw_radius: 0.35,
            volley_count: 8,
            volley_ring_degrees: 10.0,
            volley_damage: 12.0,
            volley_cooldown: 1.2,
        }
    }
}

/// 4 — Cinderbox: explosive launcher. Alt A Sticky (alt again detonates), alt B Mortar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CinderboxTuning {
    pub rocket_speed: f32,
    pub rocket_radius: f32,
    pub rocket_damage: f32,
    pub rocket_knockback: f32,
    pub rocket_interval: f32,
    pub rocket_projectile_radius: f32,
    pub rocket_lifetime: f32,
    pub sticky_speed: f32,
    pub sticky_gravity: f32,
    pub sticky_radius: f32,
    pub sticky_damage: f32,
    pub sticky_knockback: f32,
    pub sticky_max: u32,
    pub sticky_cooldown: f32,
    /// Pressing alt again within this time of the last throw throws another sticky (up to
    /// `sticky_max`); pressing it later detonates every sticky.
    pub sticky_chain_window: f32,
    /// Unexploded stickies detonate on their own after this long.
    pub sticky_lifetime: f32,
    pub mortar_speed: f32,
    /// Launch angle above the aim.
    pub mortar_angle_degrees: f32,
    pub mortar_gravity: f32,
    pub mortar_radius: f32,
    pub mortar_damage: f32,
    pub mortar_knockback: f32,
    pub mortar_cooldown: f32,
}

impl Default for CinderboxTuning {
    fn default() -> Self {
        Self {
            rocket_speed: 45.0,
            rocket_radius: 4.5,
            rocket_damage: 70.0,
            rocket_knockback: 22.0,
            rocket_interval: 0.9,
            rocket_projectile_radius: 0.2,
            rocket_lifetime: 5.0,
            sticky_speed: 30.0,
            sticky_gravity: 20.0,
            sticky_radius: 4.0,
            sticky_damage: 60.0,
            sticky_knockback: 18.0,
            sticky_max: 3,
            sticky_cooldown: 0.2,
            sticky_chain_window: 0.4,
            sticky_lifetime: 20.0,
            mortar_speed: 22.0,
            mortar_angle_degrees: 35.0,
            mortar_gravity: 25.0,
            mortar_radius: 7.0,
            mortar_damage: 90.0,
            mortar_knockback: 24.0,
            mortar_cooldown: 2.0,
        }
    }
}

/// Physical properties of enemy projectiles (speed and damage come from the enemy's attack).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnemyProjectileTuning {
    pub orb_radius: f32,
    pub orb_gravity: f32,
    pub orb_lifetime: f32,
    pub bolt_radius: f32,
    pub bolt_gravity: f32,
    pub bolt_lifetime: f32,
}

impl Default for EnemyProjectileTuning {
    fn default() -> Self {
        Self {
            orb_radius: 0.35,
            orb_gravity: 0.0,
            orb_lifetime: 6.0,
            bolt_radius: 0.15,
            bolt_gravity: 0.0,
            bolt_lifetime: 3.0,
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
    /// Remote entities are extrapolated at most this long (ms) past the newest snapshot when
    /// snapshots are late, then frozen.
    pub max_extrapolation_ms: f32,
    /// Extra interpolation delay per unit of measured snapshot arrival jitter (multiplier).
    pub interp_jitter_factor: f32,
    /// Upper bound (ms) for the adaptive interpolation delay.
    pub interp_delay_max_ms: f32,
}

impl Default for NetTuning {
    fn default() -> Self {
        Self {
            snapshot_rate: 30.0,
            interp_delay_ms: 50.0,
            lag_comp_max_ms: 350.0,
            parry_tolerance_ms: 80.0,
            correction_smoothing: 12.0,
            correction_snap_distance: 3.0,
            max_extrapolation_ms: 100.0,
            interp_jitter_factor: 2.0,
            interp_delay_max_ms: 250.0,
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
