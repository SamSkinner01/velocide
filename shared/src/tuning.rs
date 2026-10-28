//! Every gameplay number lives here and is loaded from `assets/tuning.ron`, which is watched at
//! runtime so movement and combat feel can be iterated without recompiling.
//!
//! All structs use `#[serde(default)]`, so a tuning file only needs the values it overrides and
//! new fields never break old files. The server is authoritative: it sends its tuning to clients
//! on connect and whenever it reloads, so client prediction always uses the same values.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use balance::{BalanceProfile, BalanceScale, BalanceTuning};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    pub movement: MovementTuning,
    pub combat: CombatTuning,
    pub net: NetTuning,
    // Style meter / ranks and game-mode rules (`sim::style`, `sim::modes`,
    // docs/style_modes.md).
    pub style: StyleTuning,
    pub modes: ModeTuning,
    // PvE difficulty / co-op scaling and the grind mode (`sim::difficulty`, `sim::grind`,
    // docs/style_modes.md).
    pub difficulty: DifficultyTuning,
    pub grind: GrindTuning,
    /// Gun game: the weapon ladder (`sim::gungame`).
    pub gun_game: GunGameTuning,
    /// PvE / PvP balance profiles over `combat` ([`Tuning::for_mode`], docs/arsenal.md).
    pub balance: BalanceTuning,
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

/// Combat rules: player vitals, fist/parry, ember bursts, thorns, hit assist, explosions and
/// every weapon.
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
    /// Player-owned projectiles test *targets* with their radius plus this much (m), on top of
    /// the hit assist: a shot that visibly grazes a body counts. The level is tested with the
    /// plain radius, so shots don't snag on edges (`docs/feel.md`).
    pub projectile_hit_pad: f32,

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
    /// Parrying never makes a projectile faster than this (m/s) ...
    pub parry_max_speed: f32,
    /// ... or its direct / explosion damage higher than this, however often it is punched
    /// back and forth (a projectile already beyond a cap keeps its value).
    pub parry_max_damage: f32,
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

    /// Ember bursts (kindling). Old tuning files call this section `blood`.
    #[serde(alias = "blood")]
    pub embers: EmberTuning,
    pub hit_assist: HitAssistTuning,
    pub thorn: ThornTuning,
    pub quill: QuillTuning,
    pub bellows: BellowsTuning,
    pub arclash: ArclashTuning,
    pub tumbler: TumblerTuning,
    // ---- expansion arsenal (docs/arsenal.md): the new weapons, then the new abilities ----
    pub lancet: LancetTuning,
    pub thresher: ThresherTuning,
    pub riveter: RiveterTuning,
    pub cinderbox: CinderboxTuning,
    pub longwick: LongwickTuning,
    pub kiln: KilnTuning,
    pub flak: FlakTuning,
    pub limpet: LimpetTuning,
    pub pinning_arrow: PinningArrowTuning,
    /// Status effects: Burn (Kiln, Stoke, the Waxwork's burst).
    pub burn: BurnTuning,
    pub enemy_projectiles: EnemyProjectileTuning,
    // ---- refit (docs/refit.md): the melee weapons, their abilities, the knacks on Q ----
    pub trimmer: TrimmerTuning,
    pub tamper: TamperTuning,
    pub lunge: LungeTuning,
    pub whirl: WhirlTuning,
    pub hurl: HurlTuning,
    pub knacks: KnackTuning,
    // ---- rework (docs/rework.md): the sniper, then one section per new ability ----
    pub lamplighter: LamplighterTuning,
    pub wickline: WicklineTuning,
    pub tally: TallyTuning,
    pub waxwork: WaxworkTuning,
    pub bolas: BolasTuning,
    pub bulwark: BulwarkTuning,
    pub stoke: StokeTuning,
    pub lantern: LanternTuning,
    pub kite: KiteTuning,
    pub blink_arrow: BlinkArrowTuning,
    pub beacon_arrow: BeaconArrowTuning,
    pub riposte: RiposteTuning,
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
            projectile_hit_pad: 0.2,

            parry_window: 0.2,
            parry_range: 3.0,
            parry_cone_degrees: 70.0,
            parry_close_range: 1.2,
            parry_speed_multiplier: 1.5,
            parry_damage_multiplier: 2.0,
            parry_max_speed: 250.0,
            parry_max_damage: 400.0,
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

            embers: EmberTuning::default(),
            hit_assist: HitAssistTuning::default(),
            thorn: ThornTuning::default(),
            quill: QuillTuning::default(),
            bellows: BellowsTuning::default(),
            arclash: ArclashTuning::default(),
            tumbler: TumblerTuning::default(),
            lancet: LancetTuning::default(),
            thresher: ThresherTuning::default(),
            riveter: RiveterTuning::default(),
            cinderbox: CinderboxTuning::default(),
            longwick: LongwickTuning::default(),
            kiln: KilnTuning::default(),
            flak: FlakTuning::default(),
            limpet: LimpetTuning::default(),
            pinning_arrow: PinningArrowTuning::default(),
            burn: BurnTuning::default(),
            enemy_projectiles: EnemyProjectileTuning::default(),
            trimmer: TrimmerTuning::default(),
            tamper: TamperTuning::default(),
            lunge: LungeTuning::default(),
            whirl: WhirlTuning::default(),
            hurl: HurlTuning::default(),
            knacks: KnackTuning::default(),
            lamplighter: LamplighterTuning::default(),
            wickline: WicklineTuning::default(),
            tally: TallyTuning::default(),
            waxwork: WaxworkTuning::default(),
            bolas: BolasTuning::default(),
            bulwark: BulwarkTuning::default(),
            stoke: StokeTuning::default(),
            lantern: LanternTuning::default(),
            kite: KiteTuning::default(),
            blink_arrow: BlinkArrowTuning::default(),
            beacon_arrow: BeaconArrowTuning::default(),
            riposte: RiposteTuning::default(),
        }
    }
}

/// Ember bursts: close-range healing (kindling, `sim/embers.rs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmberTuning {
    /// Heal pool as a fraction of the damage dealt.
    pub heal_fraction: f32,
    /// Heal pool fraction for punches and parry damage.
    pub melee_heal_fraction: f32,
    /// Heal pool fraction for the melee weapons' strikes (Trimmer, Tamper, Lunge, Whirl,
    /// Riposte): between a gun's and the fist's, they hit often and several at once.
    pub melee_weapon_heal_fraction: f32,
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

impl Default for EmberTuning {
    fn default() -> Self {
        Self {
            heal_fraction: 0.4,
            melee_heal_fraction: 1.0,
            melee_weapon_heal_fraction: 0.5,
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

/// Hit assist: extra radius (m) added to hit shapes when *player-owned* attacks test against
/// them (rays, projectile sweeps, explosion distances, the fist, thorn hits). Enemy attacks use
/// the plain shapes. Applied at test time; the lag-compensation history keeps the plain shapes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HitAssistTuning {
    pub enemy_body_pad: f32,
    /// Enemy weak points (heads and cores).
    pub enemy_weak_pad: f32,
    /// PvP: other players' bodies ...
    pub player_body_pad: f32,
    /// ... and heads.
    pub player_head_pad: f32,
}

impl Default for HitAssistTuning {
    fn default() -> Self {
        Self {
            enemy_body_pad: 0.35,
            enemy_weak_pad: 0.15,
            player_body_pad: 0.12,
            player_head_pad: 0.06,
        }
    }
}

/// Thorns: thrown relay spikes (`THROW`). They plant in the world; Quill shots ricochet off
/// them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThornTuning {
    pub charges: u32,
    /// Seconds per recharged thorn.
    pub recharge_time: f32,
    pub throw_speed: f32,
    /// Fraction of the thrower's velocity the thorn inherits.
    pub inherit_velocity: f32,
    pub gravity: f32,
    /// Seconds a thorn flies before it fizzles if it never hits anything.
    pub flight_time: f32,
    /// Seconds a thorn lives once planted (or lying on the floor).
    pub lifetime: f32,
    /// Collision radius against the world and hit shapes.
    pub radius: f32,
    /// Damage to an enemy / player it hits in flight (it then drops to the floor).
    pub damage: f32,
    /// A Quill shot passing this close to a live thorn ricochets off it.
    pub ricochet_radius: f32,
    /// Another live thorn within this range is preferred as the next bounce (chain) ...
    pub chain_range: f32,
    /// ... else a branded hostile, else the nearest hostile within this range (weak point
    /// preferred, line of sight).
    pub target_range: f32,
    /// Damage multiplier per bounce.
    pub damage_multiplier: f32,
}

impl Default for ThornTuning {
    fn default() -> Self {
        Self {
            charges: 3,
            recharge_time: 3.0,
            throw_speed: 30.0,
            inherit_velocity: 0.5,
            gravity: 12.0,
            flight_time: 3.0,
            lifetime: 8.0,
            radius: 0.08,
            damage: 8.0,
            ricochet_radius: 0.6,
            chain_range: 30.0,
            target_range: 40.0,
            damage_multiplier: 1.5,
        }
    }
}

/// 1 — Quill: precision spike-caster (hitscan). Defaults Brand, Wickline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuillTuning {
    pub damage: f32,
    pub interval: f32,
    pub brand_damage: f32,
    pub brand_cooldown: f32,
    /// Seconds a brand lasts.
    pub brand_duration: f32,
    /// A branded target takes this much damage from every source.
    pub brand_multiplier: f32,
    /// Damage of the abilities this weapon carries, relative to their home weapon's
    /// (docs/arsenal.md, "Abilities on other weapons").
    pub ability_power: f32,
}

impl Default for QuillTuning {
    fn default() -> Self {
        Self {
            damage: 24.0,
            interval: 0.28,
            brand_damage: 10.0,
            brand_cooldown: 2.5,
            brand_duration: 5.0,
            brand_multiplier: 1.35,
            ability_power: 1.0,
        }
    }
}

/// 2 — Bellows: twin-barrel blast cannon. Defaults Gust, Bulwark.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BellowsTuning {
    pub pellets: u32,
    pub pellet_damage: f32,
    pub cone_degrees: f32,
    pub pellet_range: f32,
    pub interval: f32,
    /// Knockback per pellet along its ray.
    pub pellet_knockback: f32,
    pub gust_range: f32,
    pub gust_cone_degrees: f32,
    pub gust_damage: f32,
    /// Hostiles in the gust are knocked back this fast along the aim ...
    pub gust_knockback: f32,
    /// ... plus this upwards.
    pub gust_up: f32,
    /// The shooter is pushed back this fast (gust-jump).
    pub gust_recoil: f32,
    pub gust_cooldown: f32,
    /// A hostile projectile caught in the gust becomes yours: speed and damage multipliers.
    pub deflect_speed_multiplier: f32,
    pub deflect_damage_multiplier: f32,
    /// Damage of the abilities this weapon carries, relative to their home weapon's
    /// (docs/arsenal.md, "Abilities on other weapons").
    pub ability_power: f32,
}

impl Default for BellowsTuning {
    fn default() -> Self {
        Self {
            pellets: 12,
            pellet_damage: 8.0,
            cone_degrees: 8.0,
            pellet_range: 60.0,
            interval: 0.8,
            pellet_knockback: 2.0,
            gust_range: 7.0,
            gust_cone_degrees: 60.0,
            gust_damage: 6.0,
            gust_knockback: 18.0,
            gust_up: 4.0,
            gust_recoil: 10.0,
            gust_cooldown: 1.6,
            deflect_speed_multiplier: 1.25,
            deflect_damage_multiplier: 1.5,
            ability_power: 1.0,
        }
    }
}

/// 3 — Arclash: arc caster. Defaults Wisp, Discharge (hold to charge, release).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArclashTuning {
    /// Seconds between zaps while the primary is held.
    pub zap_interval: f32,
    pub zap_range: f32,
    pub zap_damage: f32,
    /// A zap that hits jumps to the nearest other hostile within this range of the target ...
    pub chain_range: f32,
    /// ... this many times at most ...
    pub chain_jumps: u32,
    /// ... each jump multiplying the damage by this.
    pub chain_falloff: f32,
    /// The Wisp orbits its caster (docs/rework.md): this far out from the body's axis ...
    pub wisp_orbit_radius: f32,
    /// ... at this height above the feet (chest) ...
    pub wisp_orbit_height: f32,
    /// ... going round at this many degrees per second, for this long.
    pub wisp_orbit_rate: f32,
    pub wisp_orbit_time: f32,
    /// Flung (ALT again while it orbits) it flies along the aim this fast; it pops on the
    /// first contact or after `wisp_lifetime`.
    pub wisp_speed: f32,
    pub wisp_lifetime: f32,
    pub wisp_radius: f32,
    /// The wisp zaps the nearest hostile within `wisp_zap_range` this often.
    pub wisp_zap_interval: f32,
    pub wisp_zap_range: f32,
    pub wisp_zap_damage: f32,
    /// On world contact or expiry it pops.
    pub wisp_pop_radius: f32,
    pub wisp_pop_damage: f32,
    pub wisp_pop_knockback: f32,
    /// Starts when the wisp is gone (popped, or the orbit ran out).
    pub wisp_cooldown: f32,
    /// Seconds to a full Discharge charge.
    pub discharge_charge_time: f32,
    /// Nova radius / damage / outward knockback with no charge and with a full charge.
    pub discharge_min_radius: f32,
    pub discharge_max_radius: f32,
    pub discharge_min_damage: f32,
    pub discharge_max_damage: f32,
    pub discharge_min_knockback: f32,
    pub discharge_max_knockback: f32,
    pub discharge_cooldown: f32,
    /// Damage of the abilities this weapon carries, relative to their home weapon's
    /// (docs/arsenal.md, "Abilities on other weapons").
    pub ability_power: f32,
}

impl Default for ArclashTuning {
    fn default() -> Self {
        Self {
            zap_interval: 0.1,
            zap_range: 30.0,
            zap_damage: 7.0,
            chain_range: 7.0,
            chain_jumps: 2,
            chain_falloff: 0.75,
            wisp_orbit_radius: 1.6,
            wisp_orbit_height: 1.2,
            wisp_orbit_rate: 240.0,
            wisp_orbit_time: 6.0,
            wisp_speed: 18.0,
            wisp_lifetime: 2.0,
            wisp_radius: 0.3,
            wisp_zap_interval: 0.25,
            wisp_zap_range: 7.0,
            wisp_zap_damage: 9.0,
            wisp_pop_radius: 4.0,
            wisp_pop_damage: 35.0,
            wisp_pop_knockback: 8.0,
            wisp_cooldown: 4.0,
            discharge_charge_time: 1.0,
            discharge_min_radius: 3.5,
            discharge_max_radius: 8.0,
            discharge_min_damage: 25.0,
            discharge_max_damage: 80.0,
            discharge_min_knockback: 12.0,
            discharge_max_knockback: 24.0,
            discharge_cooldown: 2.0,
            ability_power: 1.0,
        }
    }
}

/// 4 — Tumbler: bouncing-bomb launcher. Alt A Breach, alt B Gravewell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TumblerTuning {
    pub bomb_speed: f32,
    pub bomb_gravity: f32,
    /// Speed kept when bouncing off the world.
    pub bomb_restitution: f32,
    /// World contacts before it explodes (it explodes on this one).
    pub bomb_contacts: u32,
    /// Explodes after this long at the latest.
    pub bomb_fuse: f32,
    pub bomb_projectile_radius: f32,
    pub bomb_radius: f32,
    pub bomb_damage: f32,
    pub bomb_knockback: f32,
    pub interval: f32,
    /// Breach explodes at the first surface / hostile this far along the aim ...
    pub breach_reach: f32,
    /// ... pulled back this much towards the shooter.
    pub breach_pullback: f32,
    pub breach_radius: f32,
    pub breach_damage: f32,
    pub breach_knockback: f32,
    pub breach_cooldown: f32,
    pub gravewell_speed: f32,
    /// Launch angle above the aim.
    pub gravewell_angle_degrees: f32,
    pub gravewell_gravity: f32,
    pub gravewell_projectile_radius: f32,
    /// Seconds in flight before it implodes where it is (it normally sticks long before).
    pub gravewell_flight_time: f32,
    /// Once stuck it pulls for this long, then implodes.
    pub gravewell_pull_time: f32,
    pub gravewell_pull_radius: f32,
    /// Pull speed towards the centre (enemies: × their `knockback_scale`).
    pub gravewell_pull_speed: f32,
    /// PvP: hostile players are pulled at this fraction of the speed.
    pub gravewell_player_pull: f32,
    pub gravewell_radius: f32,
    pub gravewell_damage: f32,
    pub gravewell_knockback: f32,
    pub gravewell_cooldown: f32,
    /// Damage of the abilities this weapon carries, relative to their home weapon's
    /// (docs/arsenal.md, "Abilities on other weapons").
    pub ability_power: f32,
}

impl Default for TumblerTuning {
    fn default() -> Self {
        Self {
            bomb_speed: 30.0,
            bomb_gravity: 18.0,
            bomb_restitution: 0.6,
            bomb_contacts: 3,
            bomb_fuse: 1.6,
            bomb_projectile_radius: 0.2,
            bomb_radius: 4.5,
            bomb_damage: 72.0,
            bomb_knockback: 20.0,
            interval: 0.85,
            breach_reach: 3.0,
            breach_pullback: 0.2,
            breach_radius: 4.0,
            breach_damage: 70.0,
            breach_knockback: 24.0,
            breach_cooldown: 1.2,
            gravewell_speed: 20.0,
            gravewell_angle_degrees: 25.0,
            gravewell_gravity: 20.0,
            gravewell_projectile_radius: 0.25,
            gravewell_flight_time: 5.0,
            gravewell_pull_time: 1.5,
            gravewell_pull_radius: 9.0,
            gravewell_pull_speed: 9.0,
            gravewell_player_pull: 0.5,
            gravewell_radius: 5.0,
            gravewell_damage: 80.0,
            gravewell_knockback: 6.0,
            gravewell_cooldown: 3.0,
            ability_power: 1.0,
        }
    }
}

/// Lancet: needle pistol (hitscan, Precision). Spamming it blooms the spread.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LancetTuning {
    pub damage: f32,
    pub interval: f32,
    /// Each shot adds this much heat (seconds; it drains in real time) ...
    pub bloom_per_shot: f32,
    /// ... up to this much ...
    pub bloom_max: f32,
    /// ... and the spread (full aperture) grows with it to this at full heat.
    pub bloom_degrees: f32,
    pub ability_power: f32,
}

impl Default for LancetTuning {
    fn default() -> Self {
        Self {
            damage: 14.0,
            interval: 0.15,
            bloom_per_shot: 0.25,
            bloom_max: 0.6,
            bloom_degrees: 3.0,
            ability_power: 0.85,
        }
    }
}

/// Thresher: drum-fed auto-scattergun (Scatter); automatic while held.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThresherTuning {
    pub pellets: u32,
    pub pellet_damage: f32,
    pub cone_degrees: f32,
    pub pellet_range: f32,
    pub interval: f32,
    pub pellet_knockback: f32,
    pub ability_power: f32,
}

impl Default for ThresherTuning {
    fn default() -> Self {
        Self {
            pellets: 7,
            pellet_damage: 7.5,
            cone_degrees: 11.0,
            pellet_range: 45.0,
            interval: 0.42,
            pellet_knockback: 1.5,
            ability_power: 0.9,
        }
    }
}

/// Riveter: pneumatic rivet driver (Rapid): fast hot-rivet projectiles with light gravity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiveterTuning {
    pub damage: f32,
    /// 0.09 s = 5 ticks at 60 Hz: 12 rivets/s.
    pub interval: f32,
    pub rivet_speed: f32,
    pub rivet_gravity: f32,
    pub rivet_radius: f32,
    pub rivet_lifetime: f32,
    /// Spread (full aperture) of each rivet.
    pub spread_degrees: f32,
    pub ability_power: f32,
}

impl Default for RiveterTuning {
    fn default() -> Self {
        Self {
            damage: 7.0,
            interval: 0.09,
            rivet_speed: 90.0,
            rivet_gravity: 6.0,
            rivet_radius: 0.06,
            rivet_lifetime: 1.5,
            spread_degrees: 1.2,
            ability_power: 0.9,
        }
    }
}

/// Cinderbox: box launcher (Explosive). One trigger pull = a salvo of small cinder rockets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CinderboxTuning {
    /// Rockets per salvo ...
    pub rockets: u32,
    /// ... this far apart (seconds).
    pub salvo_gap: f32,
    /// Seconds from one salvo to the next.
    pub interval: f32,
    pub rocket_speed: f32,
    pub rocket_radius: f32,
    pub rocket_lifetime: f32,
    /// Each rocket leaves this far (full aperture) off the aim.
    pub spread_degrees: f32,
    pub explosion_radius: f32,
    pub damage: f32,
    pub knockback: f32,
    pub ability_power: f32,
}

impl Default for CinderboxTuning {
    fn default() -> Self {
        Self {
            rockets: 3,
            salvo_gap: 0.08,
            interval: 1.3,
            rocket_speed: 42.0,
            rocket_radius: 0.15,
            rocket_lifetime: 2.5,
            spread_degrees: 2.5,
            explosion_radius: 2.6,
            damage: 32.0,
            knockback: 14.0,
            ability_power: 0.95,
        }
    }
}

/// Longwick: recurve bow (Draw). Hold to draw, release to loose; speed and damage grow with
/// the draw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LongwickTuning {
    /// Seconds to a full draw.
    pub draw_time: f32,
    /// Seconds after a release before the next draw can start.
    pub interval: f32,
    /// Arrow speed / damage un-drawn and at full draw.
    pub min_speed: f32,
    pub max_speed: f32,
    pub min_damage: f32,
    pub max_damage: f32,
    pub arrow_gravity: f32,
    pub arrow_radius: f32,
    pub arrow_lifetime: f32,
    pub ability_power: f32,
}

impl Default for LongwickTuning {
    fn default() -> Self {
        Self {
            draw_time: 0.62,
            interval: 0.25,
            min_speed: 55.0,
            max_speed: 135.0,
            min_damage: 22.0,
            max_damage: 105.0,
            arrow_gravity: 10.0,
            arrow_radius: 0.05,
            arrow_lifetime: 4.0,
            ability_power: 1.0,
        }
    }
}

/// Kiln: ember sprayer (Rapid). A short cone of embers that sets targets alight (Burn).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KilnTuning {
    pub range: f32,
    pub cone_degrees: f32,
    /// Seconds between sprays while held (0.08 s = 5 ticks at 60 Hz).
    pub interval: f32,
    /// Per spray to everything in the cone ...
    pub damage: f32,
    /// ... plus this many Burn stacks.
    pub burn_stacks: u32,
    pub ability_power: f32,
}

impl Default for KilnTuning {
    fn default() -> Self {
        Self {
            range: 13.0,
            cone_degrees: 20.0,
            interval: 0.08,
            damage: 4.0,
            burn_stacks: 1,
            ability_power: 0.85,
        }
    }
}

/// Flak (Scatter): a shell that airbursts into a ring of pellets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlakTuning {
    pub shell_speed: f32,
    pub shell_radius: f32,
    /// Bursts this far from the muzzle (or on contact).
    pub burst_distance: f32,
    pub pellets: u32,
    pub pellet_damage: f32,
    /// The ring's pellets leave the burst at this angle off the shell's flight.
    pub ring_degrees: f32,
    pub pellet_range: f32,
    pub cooldown: f32,
}

impl Default for FlakTuning {
    fn default() -> Self {
        Self {
            shell_speed: 45.0,
            shell_radius: 0.12,
            burst_distance: 14.0,
            pellets: 10,
            pellet_damage: 9.0,
            ring_degrees: 25.0,
            pellet_range: 14.0,
            cooldown: 1.8,
        }
    }
}

/// Limpet (Explosive): sticky charges, detonated together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimpetTuning {
    pub speed: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    /// Most charges out at once.
    pub max_live: u32,
    /// A charge left alone detonates by itself after this long.
    pub fuse: f32,
    pub explosion_radius: f32,
    pub damage: f32,
    pub knockback: f32,
    /// Seconds between two throws.
    pub cooldown: f32,
    /// Holding the ability this long detonates every charge out (a tap throws one).
    pub detonate_hold: f32,
}

impl Default for LimpetTuning {
    fn default() -> Self {
        Self {
            speed: 24.0,
            gravity: 16.0,
            projectile_radius: 0.12,
            max_live: 3,
            fuse: 20.0,
            explosion_radius: 3.5,
            damage: 40.0,
            knockback: 14.0,
            cooldown: 0.9,
            detonate_hold: 0.25,
        }
    }
}

/// Pinning Arrow (Draw): the next release pins what it hits (status Pinned).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PinningArrowTuning {
    /// Seconds an enemy is rooted ...
    pub enemy_time: f32,
    /// ... a heavy one (base health at least `heavy_health`) ...
    pub heavy_time: f32,
    pub heavy_health: f32,
    /// ... and a player is slowed ...
    pub player_time: f32,
    /// ... to this fraction of their speed.
    pub player_slow: f32,
    /// Damage as a fraction of a plain arrow at the same draw.
    pub damage_scale: f32,
    pub cooldown: f32,
}

impl Default for PinningArrowTuning {
    fn default() -> Self {
        Self {
            enemy_time: 1.4,
            heavy_time: 0.6,
            heavy_health: 300.0,
            player_time: 1.0,
            player_slow: 0.5,
            damage_scale: 1.0,
            cooldown: 2.5,
        }
    }
}

/// Burn: damage over time from embers (the Kiln, a stoked weapon, the Waxwork's burst),
/// enemies and PvP players alike.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BurnTuning {
    /// Damage per second per stack.
    pub dps: f32,
    pub max_stacks: u32,
    /// Seconds a burn lasts; every new stack refreshes it.
    pub duration: f32,
    /// Its damage is dealt in steps this far apart.
    pub tick_interval: f32,
}

impl Default for BurnTuning {
    fn default() -> Self {
        Self {
            dps: 5.0,
            max_stacks: 3,
            duration: 3.0,
            tick_interval: 0.25,
        }
    }
}

/// Physical properties of enemy projectiles (speed and damage come from the enemy's attack).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnemyProjectileTuning {
    /// Censer ember bomb: collision radius, gravity (the lob is solved for it), fuse.
    pub ember_radius: f32,
    pub ember_gravity: f32,
    pub ember_lifetime: f32,
    /// Ember bomb explosion (damage comes from the attack; falloff as every explosion).
    pub ember_explosion_radius: f32,
    pub ember_knockback: f32,
    /// Heron needle.
    pub needle_radius: f32,
    pub needle_gravity: f32,
    pub needle_lifetime: f32,
}

impl Default for EnemyProjectileTuning {
    fn default() -> Self {
        Self {
            ember_radius: 0.3,
            ember_gravity: 20.0,
            ember_lifetime: 6.0,
            ember_explosion_radius: 2.5,
            ember_knockback: 9.0,
            needle_radius: 0.12,
            needle_gravity: 0.0,
            needle_lifetime: 2.0,
        }
    }
}

// ---- refit (docs/refit.md, docs/arsenal.md): melee weapons, their abilities, knacks ----

/// Trimmer (Melee): hooked wick-shears. Every `interval` a slash in an arc in front of the eye
/// hits up to `max_targets` (nearest first) and deflects the parryable hostile projectiles in
/// it, like a punch parry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrimmerTuning {
    pub damage: f32,
    pub interval: f32,
    /// Reach from the eye and the arc (full aperture) a swing covers.
    pub reach: f32,
    pub arc_degrees: f32,
    pub max_targets: u32,
    pub knockback: f32,
    /// Damage of the abilities this weapon carries, relative to their home weapon's.
    pub ability_power: f32,
}

impl Default for TrimmerTuning {
    fn default() -> Self {
        Self {
            damage: 36.0,
            interval: 0.32,
            reach: 3.0,
            arc_degrees: 70.0,
            max_targets: 3,
            knockback: 4.0,
            ability_power: 1.0,
        }
    }
}

/// Tamper (Melee): a foundry ramming hammer. Every `interval` a heavy swing hits everything in
/// its arc, with heavy knockback; no deflect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TamperTuning {
    pub damage: f32,
    pub interval: f32,
    pub reach: f32,
    pub arc_degrees: f32,
    pub knockback: f32,
    pub ability_power: f32,
}

impl Default for TamperTuning {
    fn default() -> Self {
        Self {
            damage: 85.0,
            interval: 0.85,
            reach: 3.4,
            arc_degrees: 100.0,
            knockback: 16.0,
            ability_power: 1.0,
        }
    }
}

/// Lunge (Melee): an impulse along the aim; for `window` seconds the first target the body
/// reaches takes `damage_scale` × the carrier's primary damage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LungeTuning {
    pub speed: f32,
    /// Share of the aim's vertical part the impulse keeps (mostly horizontal).
    pub vertical: f32,
    /// Upward speed it gives at least: off the ground the lunge glides (ground friction would
    /// stop a standing lunge within ~1.5 m).
    pub lift: f32,
    pub window: f32,
    /// A target this close to the body (beyond the hit shapes) counts as reached.
    pub reach: f32,
    pub damage_scale: f32,
    pub knockback: f32,
    pub cooldown: f32,
}

impl Default for LungeTuning {
    fn default() -> Self {
        Self {
            speed: 16.0,
            vertical: 0.35,
            lift: 4.5,
            window: 0.35,
            reach: 0.8,
            damage_scale: 1.5,
            knockback: 10.0,
            cooldown: 3.0,
        }
    }
}

/// Whirl (Melee): a spin hitting everything within `radius` for `damage_scale` × the carrier's
/// primary damage, knocking it outward.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhirlTuning {
    pub radius: f32,
    pub damage_scale: f32,
    pub knockback: f32,
    /// Upward bias of the knockback (added before normalising).
    pub up: f32,
    pub cooldown: f32,
}

impl Default for WhirlTuning {
    fn default() -> Self {
        Self {
            radius: 3.5,
            damage_scale: 1.5,
            knockback: 12.0,
            up: 0.35,
            cooldown: 4.0,
        }
    }
}

/// Hurl (Melee): throws a spinning blade / hammer-head that flies out up to `max_distance`
/// (or until it hits the level), then comes back to the thrower's chest (docs/rework.md),
/// hitting each hostile at most once per leg: `damage_scale` × the carrier's primary damage
/// on the way out, `return_damage_scale` × on the way back. The Trimmer's blade passes
/// through targets, the Tamper's head turns back at the first one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HurlTuning {
    pub speed: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    /// Seconds in flight (both legs) before it drops out of the air.
    pub flight_time: f32,
    pub damage_scale: f32,
    pub knockback: f32,
    pub cooldown: f32,
    /// The outward leg turns back this far from the throw.
    pub max_distance: f32,
    /// Speed of the way back (homing on the thrower's chest).
    pub return_speed: f32,
    pub return_damage_scale: f32,
    /// Caught (it reached the thrower): this fraction of the cooldown back.
    pub catch_refund: f32,
}

impl Default for HurlTuning {
    fn default() -> Self {
        Self {
            speed: 30.0,
            gravity: 6.0,
            projectile_radius: 0.25,
            flight_time: 3.0,
            damage_scale: 1.5,
            knockback: 6.0,
            cooldown: 2.5,
            max_distance: 14.0,
            return_speed: 30.0,
            return_damage_scale: 1.0,
            catch_refund: 0.4,
        }
    }
}

/// The knacks on Q (`sim::knacks`), one section each. The Thorn keeps `combat.thorn`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnackTuning {
    pub kindle_flask: KindleFlaskTuning,
    pub snare: SnareTuning,
    pub soot_pot: SootPotTuning,
    pub tallow_skin: TallowSkinTuning,
    pub surge: SurgeTuning,
}

/// Kindle Flask: a lobbed flask; where it lands a pool heals the Shrikes standing in it
/// (teammates and you; in PvP only you) through the normal healing rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KindleFlaskTuning {
    pub charges: u32,
    pub recharge_time: f32,
    pub speed: f32,
    /// Launch angle above the aim.
    pub angle_degrees: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    pub flight_time: f32,
    pub radius: f32,
    pub duration: f32,
    /// HP per second to everyone it heals.
    pub heal_rate: f32,
    /// Drunk (docs/rework.md): haste for this long ...
    pub haste_time: f32,
    /// ... ground, air, slide and dash speeds and accelerations × this ...
    pub haste_scale: f32,
    /// ... and this many HP every `haste_heal_interval` (never above the effective max).
    pub haste_heal: f32,
    pub haste_heal_interval: f32,
}

impl Default for KindleFlaskTuning {
    fn default() -> Self {
        Self {
            charges: 1,
            recharge_time: 14.0,
            speed: 16.0,
            angle_degrees: 15.0,
            gravity: 16.0,
            projectile_radius: 0.15,
            flight_time: 3.0,
            radius: 3.5,
            duration: 4.0,
            heal_rate: 10.0,
            haste_time: 5.0,
            haste_scale: 1.3,
            haste_heal: 8.0,
            haste_heal_interval: 1.0,
        }
    }
}

/// Snare: a wax trap thrown to the floor. It arms after `arm_time`; the first hostile within
/// `trigger_radius` is Pinned (the Pinning Arrow's status, `pinning_arrow` times) and takes
/// `damage`. Up to `max_live` out, each lasts `lifetime`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnareTuning {
    pub charges: u32,
    pub recharge_time: f32,
    pub speed: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    pub arm_time: f32,
    pub trigger_radius: f32,
    pub damage: f32,
    pub lifetime: f32,
    pub max_live: u32,
}

impl Default for SnareTuning {
    fn default() -> Self {
        Self {
            charges: 2,
            recharge_time: 10.0,
            speed: 12.0,
            gravity: 20.0,
            projectile_radius: 0.15,
            arm_time: 0.6,
            trigger_radius: 1.8,
            damage: 15.0,
            lifetime: 20.0,
            max_live: 2,
        }
    }
}

/// Soot Pot: bursts into a soot cloud Moulds can't see into, out of or through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SootPotTuning {
    pub charges: u32,
    pub recharge_time: f32,
    pub speed: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    /// Seconds in flight before it bursts where it is.
    pub flight_time: f32,
    pub radius: f32,
    pub duration: f32,
}

impl Default for SootPotTuning {
    fn default() -> Self {
        Self {
            charges: 1,
            recharge_time: 15.0,
            speed: 18.0,
            gravity: 16.0,
            projectile_radius: 0.15,
            flight_time: 2.0,
            radius: 5.0,
            duration: 7.0,
        }
    }
}

/// Tallow Skin: for `duration` the player takes `damage_scale` × damage and no knockback.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TallowSkinTuning {
    pub charges: u32,
    pub recharge_time: f32,
    pub duration: f32,
    pub damage_scale: f32,
}

impl Default for TallowSkinTuning {
    fn default() -> Self {
        Self {
            charges: 1,
            recharge_time: 16.0,
            duration: 3.0,
            damage_scale: 0.5,
        }
    }
}

/// Surge: an instant burst of speed along the look (an impulse, mostly horizontal).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SurgeTuning {
    pub charges: u32,
    pub recharge_time: f32,
    pub speed: f32,
    /// Share of the look's vertical part the impulse keeps.
    pub vertical: f32,
}

impl Default for SurgeTuning {
    fn default() -> Self {
        Self {
            charges: 2,
            recharge_time: 6.0,
            speed: 18.0,
            vertical: 0.3,
        }
    }
}

// ---- rework (docs/rework.md): the sniper and the new abilities (`sim::abilities`) ----

/// Lamplighter (Precision): hold to focus, release a hitscan beam, exact; its damage grows with
/// the focus and a full focus pierces the first target into one more behind it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LamplighterTuning {
    /// Seconds to a full focus (it starts once off `interval`).
    pub focus_time: f32,
    /// Seconds after a release before the next focus can start.
    pub interval: f32,
    /// Beam damage at no focus (a tap) and at full focus (weak points ×2 as any ray).
    pub min_damage: f32,
    pub max_damage: f32,
    /// Targets a full-focus beam passes through (the last one stops it) ...
    pub pierce: u32,
    /// ... each one multiplying the damage by this.
    pub pierce_falloff: f32,
    /// Scope (client): at full focus the view's tan(fov/2) × this (eased like the bow's zoom).
    pub zoom: f32,
    pub ability_power: f32,
}

impl Default for LamplighterTuning {
    fn default() -> Self {
        Self {
            focus_time: 0.9,
            interval: 0.35,
            min_damage: 35.0,
            max_damage: 130.0,
            pierce: 2,
            pierce_falloff: 0.7,
            zoom: 0.4,
            ability_power: 1.0,
        }
    }
}

/// Wickline (Precision, move): a burning wick-thread, hitscan to `range`. Anchored in the
/// level it zips you along it; on a light hostile it yanks the target to you, on a heavy one
/// you to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WicklineTuning {
    pub range: f32,
    /// One impulse towards the anchor (or a heavy target) this fast ...
    pub zip_speed: f32,
    /// ... with at least this much of it upwards.
    pub zip_lift: f32,
    /// A light hostile (base health below `heavy_health`, any PvP player) is yanked towards
    /// you this fast (enemies × their knockback scale) ...
    pub yank_speed: f32,
    /// ... a PvP player at this fraction of it ...
    pub player_yank_scale: f32,
    /// ... and takes this.
    pub damage: f32,
    pub heavy_health: f32,
    /// How long the line is drawn.
    pub line_time: f32,
    pub cooldown: f32,
    /// A miss serves only this fraction of the cooldown.
    pub miss_cooldown: f32,
}

impl Default for WicklineTuning {
    fn default() -> Self {
        Self {
            range: 34.0,
            zip_speed: 24.0,
            zip_lift: 4.0,
            yank_speed: 16.0,
            player_yank_scale: 0.5,
            damage: 10.0,
            heavy_health: 300.0,
            line_time: 0.35,
            cooldown: 4.0,
            miss_cooldown: 0.5,
        }
    }
}

/// Tally (Precision, set up): hold for up to `hold_time`; every hostile the crosshair passes
/// over (within `range`, line of sight, lag-compensated, `assist_degrees` of assist) is
/// tallied, up to `max_targets`; the release shoots each one still in sight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TallyTuning {
    pub hold_time: f32,
    pub range: f32,
    /// Full aperture of the assist cone around the crosshair.
    pub assist_degrees: f32,
    /// At most this many (PvP: `balance.pvp` sets 3).
    pub max_targets: u32,
    /// Per shot, at the part the crosshair passed over (weak point / head if that was it).
    pub damage: f32,
    pub cooldown: f32,
    /// Released with nothing tallied: no shot, only this fraction of the cooldown.
    pub empty_cooldown: f32,
}

impl Default for TallyTuning {
    fn default() -> Self {
        Self {
            hold_time: 1.2,
            range: 45.0,
            assist_degrees: 1.5,
            max_targets: 4,
            damage: 30.0,
            cooldown: 6.0,
            empty_cooldown: 0.25,
        }
    }
}

/// Waxwork (Precision, defend): a wax double where you stand for `duration` with `health`;
/// Moulds within `lure_range` that could see it prefer it to you. Destroyed or at the end it
/// bursts in hot wax. One at a time; the cooldown starts at its end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WaxworkTuning {
    pub duration: f32,
    pub health: f32,
    pub lure_range: f32,
    pub burst_radius: f32,
    pub burst_damage: f32,
    pub burst_knockback: f32,
    pub burn_stacks: u32,
    pub cooldown: f32,
}

impl Default for WaxworkTuning {
    fn default() -> Self {
        Self {
            duration: 5.0,
            health: 60.0,
            lure_range: 25.0,
            burst_radius: 3.5,
            burst_damage: 30.0,
            burst_knockback: 8.0,
            burn_stacks: 1,
            cooldown: 10.0,
        }
    }
}

/// Bolas (Scatter, control): two wax-weighted balls on a cord. The first hostile it touches is
/// bound (Pinned: the Pinning Arrow's times); a second hostile within `link_range` of it is
/// bound too, and for `link_time` `link_share` of the damage either takes is dealt to the
/// other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BolasTuning {
    pub speed: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    /// Seconds in flight before it drops.
    pub flight_time: f32,
    /// Damage of the hit itself.
    pub damage: f32,
    pub link_range: f32,
    pub link_time: f32,
    pub link_share: f32,
    pub cooldown: f32,
}

impl Default for BolasTuning {
    fn default() -> Self {
        Self {
            speed: 30.0,
            gravity: 8.0,
            projectile_radius: 0.35,
            flight_time: 2.0,
            damage: 0.0,
            link_range: 6.0,
            link_time: 4.0,
            link_share: 0.6,
            cooldown: 5.0,
        }
    }
}

/// Bulwark (Scatter, defend): hold up a slab of hardened wax (`width` × `height`, `distance`
/// out, turning with the aim's yaw, pitch clamped to `pitch_limit_degrees`) for up to
/// `hold_time`: the shared guard absorbs hostile hits from its `cone_degrees` front until it
/// has absorbed `absorb`, then it shatters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BulwarkTuning {
    pub width: f32,
    pub height: f32,
    pub distance: f32,
    pub pitch_limit_degrees: f32,
    pub hold_time: f32,
    /// Full aperture of the front it guards.
    pub cone_degrees: f32,
    pub absorb: f32,
    /// Your speeds while it is up.
    pub move_scale: f32,
    /// From when it drops ...
    pub cooldown: f32,
    /// ... or this if it shattered.
    pub broken_cooldown: f32,
}

impl Default for BulwarkTuning {
    fn default() -> Self {
        Self {
            width: 2.2,
            height: 1.8,
            distance: 1.2,
            pitch_limit_degrees: 30.0,
            hold_time: 4.0,
            cone_degrees: 110.0,
            absorb: 150.0,
            move_scale: 0.75,
            cooldown: 6.0,
            broken_cooldown: 9.0,
        }
    }
}

/// Stoke (Rapid, damage): for `duration` the primary fires at `rate` × its rate and every hit
/// sets `burn_stacks` Burn stacks (the Kiln: `kiln_reach` × its cone's reach instead); then it
/// overheats for `overheat` (no primary). Predicted (`WeaponState::stoke` / `overheat`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StokeTuning {
    pub duration: f32,
    pub rate: f32,
    pub burn_stacks: u32,
    pub kiln_reach: f32,
    pub overheat: f32,
    /// From the use.
    pub cooldown: f32,
}

impl Default for StokeTuning {
    fn default() -> Self {
        Self {
            duration: 4.0,
            rate: 1.6,
            burn_stacks: 1,
            kiln_reach: 1.3,
            overheat: 1.5,
            cooldown: 10.0,
        }
    }
}

/// Lantern (Rapid, set up): tossed (`toss_speed`), it settles on the floor (never over a pit)
/// and for `duration` fires the carrier's primary at the nearest hostile within `range` (line
/// of sight) at `rate_scale` × its rate and `damage_scale` × its damage (the Kiln's: a
/// `kiln_range` cone). Moulds may attack it. One at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanternTuning {
    pub toss_speed: f32,
    pub gravity: f32,
    pub projectile_radius: f32,
    pub duration: f32,
    pub range: f32,
    pub rate_scale: f32,
    pub damage_scale: f32,
    pub kiln_range: f32,
    pub health: f32,
    pub cooldown: f32,
}

impl Default for LanternTuning {
    fn default() -> Self {
        Self {
            toss_speed: 10.0,
            gravity: 20.0,
            projectile_radius: 0.2,
            duration: 8.0,
            range: 18.0,
            rate_scale: 0.5,
            damage_scale: 0.6,
            kiln_range: 8.0,
            health: 80.0,
            cooldown: 12.0,
        }
    }
}

/// Kite (Explosive, damage): an ember kite at `speed` that turns towards the point under its
/// owner's crosshair at up to `turn_degrees` per second while ALT is held (at most
/// `steer_time`), then flies straight; it explodes on contact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KiteTuning {
    pub speed: f32,
    pub turn_degrees: f32,
    pub steer_time: f32,
    /// Seconds in flight before it bursts where it is.
    pub flight_time: f32,
    pub projectile_radius: f32,
    pub explosion_radius: f32,
    pub damage: f32,
    pub knockback: f32,
    pub cooldown: f32,
}

impl Default for KiteTuning {
    fn default() -> Self {
        Self {
            speed: 16.0,
            turn_degrees: 120.0,
            steer_time: 4.0,
            flight_time: 7.0,
            projectile_radius: 0.3,
            explosion_radius: 4.5,
            damage: 75.0,
            knockback: 18.0,
            cooldown: 5.0,
        }
    }
}

/// Blink Arrow (Draw, move): an arrow at the draw; ALT while it flies or within `window` after
/// it sticks steps you to it (the nearest spot your body fits), at `exit_speed` along its
/// flight. The cooldown starts at the blink or when the window passes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlinkArrowTuning {
    pub window: f32,
    pub exit_speed: f32,
    /// The arrow's own hit, of a plain arrow's at the same draw.
    pub damage_scale: f32,
    pub cooldown: f32,
}

impl Default for BlinkArrowTuning {
    fn default() -> Self {
        Self {
            window: 1.5,
            exit_speed: 6.0,
            damage_scale: 0.8,
            cooldown: 6.0,
        }
    }
}

/// Beacon Arrow (Draw, set up): an arrow at the draw that lights a beacon where it sticks (in a
/// target it rides along) for `duration`: every hostile within `radius` of it is revealed to
/// the shooter's side and takes `damage_multiplier` × the damage from every player.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BeaconArrowTuning {
    /// The arrow's own hit, of a plain arrow's at the same draw.
    pub damage_scale: f32,
    pub duration: f32,
    pub radius: f32,
    pub damage_multiplier: f32,
    pub cooldown: f32,
}

impl Default for BeaconArrowTuning {
    fn default() -> Self {
        Self {
            damage_scale: 0.6,
            duration: 6.0,
            radius: 16.0,
            damage_multiplier: 1.15,
            cooldown: 8.0,
        }
    }
}

/// Riposte (Melee, defend): hold to guard for up to `hold_time` at `move_scale` of your speeds:
/// the shared guard blocks hits from its `cone_degrees` front (melee attackers are staggered
/// for `stagger`). The release counter-swings in the carrier's arc at `damage_scale` × its
/// swing (+`per_block` per further block, at most `max_scale`) if anything was blocked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiposteTuning {
    pub hold_time: f32,
    pub move_scale: f32,
    pub cone_degrees: f32,
    pub stagger: f32,
    pub damage_scale: f32,
    pub per_block: f32,
    pub max_scale: f32,
    pub knockback: f32,
    pub cooldown: f32,
}

impl Default for RiposteTuning {
    fn default() -> Self {
        Self {
            hold_time: 1.0,
            move_scale: 0.6,
            cone_degrees: 120.0,
            stagger: 0.8,
            damage_scale: 2.0,
            per_block: 0.5,
            max_scale: 3.0,
            knockback: 12.0,
            cooldown: 4.0,
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

/// Style meter, ranks, bonuses and freshness (`sim::style`). Points are meter points: one rank's
/// bar holds `points_per_rank`. Rules and rationale: `docs/gameplay.md` ("Style") and
/// `docs/style_modes.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleTuning {
    /// Size of one rank's bar.
    pub points_per_rank: f32,
    /// Meter decay (points/s) at the lowest rank (DULL) ...
    pub decay_lowest: f32,
    /// ... rising linearly to this at the highest (VELOCIDE).
    pub decay_highest: f32,
    /// After dropping a rank the meter restarts at this fraction of the lower rank's bar.
    pub rank_down_refill: f32,
    /// After ranking up, the new rank can't be lost for this long (the bar may empty, but no
    /// instant up/down flicker when a bonus just reaches the next rank).
    pub rank_grace: f32,
    /// Dying (or going down in co-op) empties the meter back to DULL.
    pub reset_on_death: bool,
    // ---- bonuses (base points, multiplied by freshness) ----
    pub kill: f32,
    /// PvP player kill (instead of `kill`).
    pub frag: f32,
    pub headshot: f32,
    /// The killer was airborne.
    pub airkill: f32,
    /// The victim was airborne (flying enemies don't count).
    pub airshot: f32,
    /// Per extra kill within `multikill_window` (n kills: `multikill × (n − 1)`).
    pub multikill: f32,
    pub multikill_window: f32,
    pub parry: f32,
    /// Kill with a parried projectile.
    pub return_to_sender: f32,
    /// A ricochet shot hit something ...
    pub ricochet: f32,
    /// ... plus this per thorn bounce after the first.
    pub ricochet_per_bounce: f32,
    /// A Gust turned a hostile projectile around.
    pub deflect: f32,
    /// Explosive kill.
    pub exploded: f32,
    pub slam_kill: f32,
    /// Punch kill.
    pub fisted: f32,
    /// Kills while the killer's health is below this ...
    pub close_call_health: f32,
    /// ... score this much more (the extra is shown as CLOSE CALL).
    pub close_call_multiplier: f32,
    // ---- freshness (variety) ----
    /// Each scoring use of a weapon mode / action costs this much of its freshness ...
    pub freshness_cost: f32,
    /// ... and restores every other one by this much ...
    pub freshness_restore: f32,
    /// ... and everything recovers this much per second.
    pub freshness_regen: f32,
    /// Freshness never drops below this (it multiplies every gain).
    pub freshness_min: f32,
    /// Below this the weapon / action is STALE (HUD indicator, "- STALE" in the feed).
    pub stale_threshold: f32,
    // ---- potg (docs/potg.md) ----
    /// LONGSHOT: a kill with a hitscan shot or an arrow from at least `longshot_distance` m.
    pub longshot: f32,
    pub longshot_distance: f32,
}

impl Default for StyleTuning {
    fn default() -> Self {
        Self {
            points_per_rank: 100.0,
            decay_lowest: 4.0,
            decay_highest: 30.0,
            rank_down_refill: 0.6,
            rank_grace: 1.0,
            reset_on_death: true,
            kill: 30.0,
            frag: 100.0,
            headshot: 15.0,
            airkill: 40.0,
            airshot: 20.0,
            multikill: 40.0,
            multikill_window: 0.6,
            parry: 60.0,
            return_to_sender: 80.0,
            ricochet: 30.0,
            ricochet_per_bounce: 20.0,
            deflect: 30.0,
            exploded: 20.0,
            slam_kill: 40.0,
            fisted: 30.0,
            close_call_health: 30.0,
            close_call_multiplier: 1.5,
            freshness_cost: 0.12,
            freshness_restore: 0.04,
            freshness_regen: 0.02,
            freshness_min: 0.2,
            stale_threshold: 0.5,
            longshot: 50.0,
            longshot_distance: 40.0,
        }
    }
}

/// Game-mode rules (`sim::modes`): campaign checkpoints, death / downed / revive, the co-op exit,
/// deathmatch, results grading. Deathmatch respawn delay and spawn protection are
/// `combat.respawn_delay` / `combat.spawn_protection`; frag and time limits are match settings
/// (`sim::modes::ModeRules`), not tuning. See `docs/style_modes.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModeTuning {
    /// Single player: seconds of death cam before restarting at the last checkpoint.
    pub sp_death_time: f32,
    /// Co-op: seconds a downed player waits for a revive before respawning at the checkpoint.
    pub downed_time: f32,
    /// A teammate holding USE this close (metres, feet to feet) ...
    pub revive_range: f32,
    /// ... for this long revives ...
    pub revive_time: f32,
    /// ... with this much health.
    pub revive_health: f32,
    /// Co-op deaths in kill volumes leave nothing to revive: respawn at the checkpoint after this.
    pub fall_respawn_time: f32,
    /// Co-op: everyone down at once → the team respawns at the checkpoint after this.
    pub wipe_respawn_time: f32,
    /// Co-op exit: the first player in starts this countdown; the level ends when it runs out
    /// or as soon as every living player is in the exit.
    pub exit_countdown: f32,
    /// Deathmatch: a kill-volume death within this many seconds of being hit by another player
    /// also frags that player.
    pub knockoff_credit: f32,
    /// Seconds between scoreboard broadcasts while something changed.
    pub scoreboard_interval: f32,
    /// Results grade thresholds, best first: S, A, B, C (anything worse is D).
    /// Level time as a multiple of the par time (at most).
    pub grade_time: [f32; 4],
    /// Kills as a fraction of the player's share of the level's enemies (at least).
    pub grade_kills: [f32; 4],
    /// Style points earned per enemy share (at least).
    pub grade_style: [f32; 4],
    /// Damage taken per enemy share (at most).
    pub grade_damage: [f32; 4],
}

impl Default for ModeTuning {
    fn default() -> Self {
        Self {
            sp_death_time: 1.5,
            downed_time: 15.0,
            revive_range: 2.5,
            revive_time: 1.5,
            revive_health: 40.0,
            fall_respawn_time: 4.0,
            wipe_respawn_time: 3.0,
            exit_countdown: 10.0,
            knockoff_credit: 3.0,
            scoreboard_interval: 0.25,
            grade_time: [1.0, 1.25, 1.65, 2.2],
            grade_kills: [1.0, 0.9, 0.75, 0.5],
            grade_style: [120.0, 85.0, 60.0, 35.0],
            grade_damage: [1.0, 3.0, 6.0, 10.0],
        }
    }
}

/// PvE difficulty and co-op scaling (`sim::difficulty`): what EASY .. INFERNO and every extra
/// player do to the Moulds and to the co-op rules, in campaign encounters and grind waves.
/// The per-difficulty tables are in `Difficulty::ALL` order (EASY, NORMAL, HARD, BRUTAL,
/// INFERNO); NORMAL with one player is the reference (1.0 = the enemy definitions and the
/// `modes` values as written). See `docs/style_modes.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DifficultyTuning {
    /// Enemy health at spawn.
    pub health: [f32; 5],
    /// Enemies per encounter wave / grind wave budget.
    pub count: [f32; 5],
    /// Damage enemies deal (`EnemyScale::damage`).
    pub damage: [f32; 5],
    /// Attack cooldowns and recoveries are divided by this (`EnemyScale::aggression`;
    /// wind-ups keep their length).
    pub aggression: [f32; 5],
    /// Enemy projectile speed (`EnemyScale::projectile_speed`).
    pub projectile_speed: [f32; 5],
    /// Chance (0..1) that a spawned Mould is an elite.
    pub elite_chance: [f32; 5],
    /// Multipliers on `modes.downed_time` and `modes.revive_health`.
    pub downed_time: [f32; 5],
    pub revive_health: [f32; 5],
    /// Per extra player (co-op): health, count, damage and aggression are multiplied by
    /// `1 + x (players − 1)` ...
    pub player_health: f32,
    pub player_count: f32,
    pub player_damage: f32,
    pub player_aggression: f32,
    /// ... the elite chance grows by `x (players − 1)` ...
    pub player_elite_chance: f32,
    /// ... and from the third player on the downed time shrinks, `× (1 − x (players − 2))`
    /// (more teammates to revive you), never below `min_downed_time` seconds.
    pub player_downed_time: f32,
    pub min_downed_time: f32,
}

impl Default for DifficultyTuning {
    fn default() -> Self {
        Self {
            health: [0.7, 1.0, 1.3, 1.65, 2.1],
            count: [0.75, 1.0, 1.2, 1.4, 1.6],
            damage: [0.6, 1.0, 1.25, 1.5, 1.85],
            aggression: [0.8, 1.0, 1.15, 1.3, 1.5],
            projectile_speed: [0.85, 1.0, 1.1, 1.2, 1.3],
            elite_chance: [0.0, 0.02, 0.06, 0.12, 0.2],
            downed_time: [1.5, 1.0, 0.8, 0.65, 0.5],
            revive_health: [1.5, 1.0, 0.85, 0.7, 0.55],
            player_health: 0.6,
            player_count: 0.6,
            player_damage: 0.1,
            player_aggression: 0.08,
            player_elite_chance: 0.02,
            player_downed_time: 0.1,
            min_downed_time: 4.0,
        }
    }
}

/// One Mould of the grind table ([`GrindTuning::roster`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrindEnemy {
    /// Enemy definition name (`assets/enemies/<enemy>.ron`). Names the server doesn't know are
    /// skipped, so the table can list Moulds that arrive later.
    pub enemy: String,
    /// What it takes out of the wave budget.
    pub cost: f32,
    /// First (normal) wave it appears in.
    pub from_wave: u32,
    /// Relative pick weight among the unlocked Moulds.
    pub weight: f32,
    /// A heavy: heavy waves bring some (from `heavy_every` waves before its own unlock), and
    /// the boss-grade elites are heavies.
    pub heavy: bool,
}

impl Default for GrindEnemy {
    fn default() -> Self {
        Self {
            enemy: String::new(),
            cost: 1.0,
            from_wave: 1,
            weight: 1.0,
            heavy: false,
        }
    }
}

impl GrindEnemy {
    fn new(enemy: &str, cost: f32, from_wave: u32, weight: f32, heavy: bool) -> Self {
        Self {
            enemy: enemy.into(),
            cost,
            from_wave,
            weight,
            heavy,
        }
    }
}

/// Grind (`sim::grind`): endless waves on an arena until everyone is down at once. Wave
/// sizes come from a budget in enemy cost units; the difficulty and the player count scale it
/// like the campaign encounters (`difficulty.count`, `difficulty.player_count`). See
/// `docs/style_modes.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GrindTuning {
    /// Seconds before wave 1 ...
    pub first_delay: f32,
    /// ... and between waves (the breather: dead players are back when it ends).
    pub breather: f32,
    /// Wave budget: `budget_base + budget_per_wave (w − 1) + budget_per_wave_sq (w − 1)²`,
    /// times the difficulty × player count factor.
    pub budget_base: f32,
    pub budget_per_wave: f32,
    pub budget_per_wave_sq: f32,
    /// Enemy health `× (1 + health_per_wave (w − 1))` on top of the difficulty.
    pub health_per_wave: f32,
    /// Every `heavy_every`th wave is a heavy wave: budget × `heavy_budget`, `1 + (w − 1) /
    /// boss_every` heavies guaranteed, elite chance + `heavy_elite_chance`.
    pub heavy_every: u32,
    pub heavy_budget: f32,
    pub heavy_elite_chance: f32,
    /// Every `boss_every`th wave also brings a boss-grade elite: a heavy with `boss_health` ×
    /// its health, on top of the elite health multiplier (1.6 × the elite's 2.5 = ×4 overall).
    pub boss_every: u32,
    pub boss_health: f32,
    /// Most wave enemies alive at once (the rest wait their turn): `max_alive +
    /// alive_per_player (players − 1)`.
    pub max_alive: u32,
    pub alive_per_player: u32,
    /// A wave opens with `spawn_burst` Moulds, then one more every `spawn_interval` seconds.
    pub spawn_burst: u32,
    pub spawn_interval: f32,
    /// Most Moulds one wave holds (budget beyond this is dropped).
    pub max_wave_enemies: u32,
    /// Moulds appear at least this far (m) from every living player when the arena allows.
    pub min_spawn_distance: f32,
    /// Seconds from everyone being down to the results (the last death sinks in).
    pub end_delay: f32,
    /// Results grade for the wave reached: thresholds for S, A, B, C (at least).
    pub grade_wave: [f32; 4],
    /// The Moulds a wave is built from.
    pub roster: Vec<GrindEnemy>,
}

impl Default for GrindTuning {
    fn default() -> Self {
        Self {
            first_delay: 5.0,
            breather: 6.0,
            budget_base: 6.0,
            budget_per_wave: 2.4,
            budget_per_wave_sq: 0.06,
            health_per_wave: 0.03,
            heavy_every: 5,
            heavy_budget: 1.25,
            heavy_elite_chance: 0.15,
            boss_every: 10,
            boss_health: 1.6,
            max_alive: 14,
            alive_per_player: 5,
            spawn_burst: 6,
            spawn_interval: 0.35,
            max_wave_enemies: 150,
            min_spawn_distance: 14.0,
            end_delay: 2.5,
            grade_wave: [20.0, 15.0, 10.0, 5.0],
            roster: vec![
                GrindEnemy::new("gnawer", 1.0, 1, 10.0, false),
                GrindEnemy::new("harrier", 1.5, 2, 4.0, false),
                GrindEnemy::new("censer", 2.0, 3, 5.0, false),
                GrindEnemy::new("snuffer", 1.5, 4, 4.0, false),
                GrindEnemy::new("heron", 3.0, 6, 3.0, false),
                GrindEnemy::new("bellwether", 4.0, 7, 2.0, false),
                GrindEnemy::new("gaoler", 6.0, 8, 2.0, true),
                GrindEnemy::new("smelter", 6.0, 12, 2.0, true),
            ],
        }
    }
}

/// Gun game (`sim::gungame`): the fixed ladder of weapon builds players climb with kills (the
/// melee finale comes after the last one) and what a punch kill costs the victim. See
/// `docs/style_modes.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GunGameTuning {
    /// One build per tier, first to last. A kill with your tier's weapon moves you up.
    pub ladder: Vec<crate::loadout::WeaponBuild>,
    /// The tier after the ladder: a kill with this weapon wins the match.
    pub finale: crate::loadout::WeaponBuild,
    /// Tiers a punch kill knocks the victim down.
    pub punch_demotes: u32,
}

impl Default for GunGameTuning {
    fn default() -> Self {
        use crate::loadout::WeaponKind::*;
        // From forgiving (chaining beams, cones, spreads) to demanding (splash, precision,
        // the bow), each with its default build.
        let ladder = [
            Arclash,
            Kiln,
            Thresher,
            Riveter,
            Bellows,
            Tumbler,
            Cinderbox,
            Lancet,
            Quill,
            Longwick,
            Lamplighter,
        ];
        Self {
            ladder: ladder.iter().map(|w| w.default_build()).collect(),
            finale: Trimmer.default_build(),
            punch_demotes: 1,
        }
    }
}

// ---- range checks ([`Tuning::sanitize`]) ------------------------------------------------------
//
// Tuning comes from a file anyone can edit and, on clients, from the server. Absurd values
// (u32::MAX pellets, 1e30 m ranges, NaN speeds) would allocate billions of rays, stall
// collision queries or poison the simulation, so every field is clamped to a sane range.
// The ranges are generous: they exist to keep the game running, not to judge game design.

/// Distances, ranges, radii, heights (m).
const MAX_DIST: f32 = 1.0e4;
/// Speeds and knockbacks (m/s).
const MAX_SPEED: f32 = 1.0e3;
/// Accelerations, gravity (m/s²).
const MAX_ACCEL: f32 = 1.0e4;
/// Durations, cooldowns, lifetimes (s).
const MAX_TIME: f32 = 3600.0;
/// Damage, health and heal amounts (HP).
const MAX_DAMAGE: f32 = 1.0e5;
/// Multipliers.
const MAX_MULT: f32 = 100.0;
/// Shot / projectile / charge counts.
const MAX_COUNT: u32 = 64;
/// Network timings (ms).
const MAX_MS: f32 = 1000.0;
/// Style points.
const MAX_POINTS: f32 = 1.0e6;

/// `v` clamped to `lo..=hi`; NaN becomes `default`.
fn clamped(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_nan() { default } else { v.clamp(lo, hi) }
}

/// `clamp!(self, defaults, lo, hi; field, ...)`: clamps float fields (NaN → their default).
macro_rules! clamp {
    ($s:ident, $d:ident, $lo:expr, $hi:expr; $($f:ident),+ $(,)?) => {
        $( $s.$f = clamped($s.$f, $lo, $hi, $d.$f); )+
    };
}

/// `clamp_count!(self; field, ...)`: caps counts at [`MAX_COUNT`].
macro_rules! clamp_count {
    ($s:ident; $($f:ident),+ $(,)?) => {
        $( $s.$f = $s.$f.min(MAX_COUNT); )+
    };
}

// Declared after the macros so it can use them.
mod balance;

impl MovementTuning {
    /// Clamps every value to a sane range (see [`Tuning::sanitize`]).
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.05, 5.0; radius);
        clamp!(self, d, 0.2, 20.0; standing_height, slide_height);
        clamp!(self, d, 0.0, 20.0; eye_height, slide_eye_height);
        clamp!(self, d, 0.0, 5.0; step_height);
        clamp!(self, d, 0.0, 90.0; max_ground_slope_degrees, fov_kick_degrees,
            slide_tilt_degrees, wall_tilt_degrees);
        clamp!(self, d, 0.0, MAX_SPEED; ground_speed, ground_stop_speed, air_max_speed,
            max_fall_speed, max_speed, jump_speed, dash_speed, dash_end_speed, slide_min_speed,
            slide_max_speed, slam_speed, slam_bounce_speed, slam_bounce_power_bonus,
            wall_jump_up_speed, wall_jump_out_speed, wall_jump_input_speed,
            wall_cling_max_fall_speed, fov_kick_speed);
        clamp!(self, d, 0.0, MAX_ACCEL; ground_accel, ground_overspeed_decel, ground_friction,
            air_accel, gravity);
        clamp!(self, d, 0.0, MAX_TIME; coyote_time, jump_buffer_time, dash_duration,
            dash_invuln_time, slam_bounce_window);
        clamp!(self, d, 0.0, MAX_DIST; footstep_stride, slam_min_height, slam_full_power_height,
            slam_shockwave_radius);
        clamp!(self, d, 0.0, MAX_COUNT as f32; dash_charges);
        clamp!(self, d, 0.0, MAX_MULT; dash_regen_rate, dash_jump_keep, slide_slope_accel,
            slide_jump_boost, slam_horizontal_keep);
        clamp!(self, d, 0.0, 3600.0; slide_steer_degrees);
        clamp!(self, d, 0.0, 10.0; wall_detect_distance, landing_dip);
        clamp_count!(self; wall_jumps);
    }
}

impl CombatTuning {
    /// Clamps every value to a sane range (see [`Tuning::sanitize`]).
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 1.0, MAX_DAMAGE; max_health);
        clamp!(self, d, 0.0, MAX_DAMAGE; hard_damage_recover_rate, parry_max_damage,
            punch_damage, slam_damage);
        clamp!(self, d, 0.0, 1.0; hard_damage_fraction, self_damage_fraction,
            parry_hard_damage_clear, explosion_edge_fraction, slam_min_power_scale);
        clamp!(self, d, 0.0, MAX_TIME; hard_damage_recover_delay, respawn_delay,
            spawn_protection, parry_window, punch_cooldown);
        clamp!(self, d, 0.0, MAX_MULT; player_head_multiplier, parry_speed_multiplier,
            parry_damage_multiplier, explosion_up_bias);
        clamp!(self, d, 0.0, MAX_DIST; hitscan_range, parry_range, parry_close_range,
            parry_auto_aim_range, melee_parry_range, punch_range);
        clamp!(self, d, 0.0, 2.0; projectile_hit_pad);
        clamp!(self, d, 0.0, 360.0; parry_cone_degrees, parry_auto_aim_degrees,
            punch_cone_degrees);
        clamp!(self, d, 0.0, MAX_SPEED; parry_max_speed, punch_knockback, slam_knockback);
        self.embers.sanitize();
        self.hit_assist.sanitize();
        self.thorn.sanitize();
        self.quill.sanitize();
        self.bellows.sanitize();
        self.arclash.sanitize();
        self.tumbler.sanitize();
        self.lancet.sanitize();
        self.thresher.sanitize();
        self.riveter.sanitize();
        self.cinderbox.sanitize();
        self.longwick.sanitize();
        self.kiln.sanitize();
        self.flak.sanitize();
        self.limpet.sanitize();
        self.pinning_arrow.sanitize();
        self.burn.sanitize();
        self.enemy_projectiles.sanitize();
        self.trimmer.sanitize();
        self.tamper.sanitize();
        self.lunge.sanitize();
        self.whirl.sanitize();
        self.hurl.sanitize();
        self.knacks.sanitize();
        self.lamplighter.sanitize();
        self.wickline.sanitize();
        self.tally.sanitize();
        self.waxwork.sanitize();
        self.bolas.sanitize();
        self.bulwark.sanitize();
        self.stoke.sanitize();
        self.lantern.sanitize();
        self.kite.sanitize();
        self.blink_arrow.sanitize();
        self.beacon_arrow.sanitize();
        self.riposte.sanitize();
    }
}

impl EmberTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_MULT; heal_fraction, melee_heal_fraction,
            melee_weapon_heal_fraction);
        clamp!(self, d, 0.0, MAX_DIST; radius, death_radius, merge_distance);
        clamp!(self, d, 0.0, MAX_TIME; lifetime, death_lifetime, merge_window);
        clamp!(self, d, 0.0, MAX_DAMAGE; death_pool);
    }
}

impl HitAssistTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, 2.0; enemy_body_pad, enemy_weak_pad, player_body_pad,
            player_head_pad);
    }
}

impl ThornTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp_count!(self; charges);
        clamp!(self, d, 0.0, MAX_TIME; recharge_time, flight_time, lifetime);
        clamp!(self, d, 0.0, MAX_SPEED; throw_speed);
        clamp!(self, d, 0.0, 1.0; inherit_velocity);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_DIST; ricochet_radius, chain_range, target_range);
        clamp!(self, d, 0.0, MAX_MULT; damage_multiplier);
    }
}

impl QuillTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DAMAGE; damage, brand_damage);
        clamp!(self, d, 0.0, MAX_TIME; interval, brand_cooldown, brand_duration);
        clamp!(self, d, 0.0, MAX_MULT; ability_power, brand_multiplier);
    }
}

impl BellowsTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp_count!(self; pellets);
        clamp!(self, d, 0.0, MAX_DAMAGE; pellet_damage, gust_damage);
        clamp!(self, d, 0.0, 180.0; cone_degrees);
        clamp!(self, d, 0.0, 360.0; gust_cone_degrees);
        clamp!(self, d, 0.0, MAX_DIST; pellet_range, gust_range);
        clamp!(self, d, 0.0, MAX_TIME; interval, gust_cooldown);
        clamp!(self, d, 0.0, MAX_SPEED; pellet_knockback, gust_knockback, gust_up, gust_recoil);
        clamp!(self, d, 0.0, MAX_MULT; ability_power, deflect_speed_multiplier,
            deflect_damage_multiplier);
    }
}

impl ArclashTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; zap_interval, wisp_lifetime, wisp_zap_interval,
            wisp_cooldown, wisp_orbit_time, discharge_charge_time, discharge_cooldown);
        clamp!(self, d, 0.0, MAX_DIST; zap_range, chain_range, wisp_zap_range, wisp_pop_radius,
            wisp_orbit_radius, wisp_orbit_height, discharge_min_radius, discharge_max_radius);
        clamp!(self, d, -3600.0, 3600.0; wisp_orbit_rate);
        clamp!(self, d, 0.0, MAX_DAMAGE; zap_damage, wisp_zap_damage, wisp_pop_damage,
            discharge_min_damage, discharge_max_damage);
        clamp!(self, d, 0.0, MAX_SPEED; wisp_speed, wisp_pop_knockback, discharge_min_knockback,
            discharge_max_knockback);
        clamp!(self, d, 0.0, 10.0; wisp_radius);
        clamp!(self, d, 0.0, MAX_MULT; ability_power, chain_falloff);
        clamp_count!(self; chain_jumps);
    }
}

impl TumblerTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_SPEED; bomb_speed, bomb_knockback, breach_knockback,
            gravewell_speed, gravewell_pull_speed, gravewell_knockback);
        clamp!(self, d, 0.0, MAX_ACCEL; bomb_gravity, gravewell_gravity);
        clamp!(self, d, 0.0, 1.0; bomb_restitution, gravewell_player_pull);
        clamp!(self, d, 0.0, MAX_TIME; bomb_fuse, interval, breach_cooldown,
            gravewell_flight_time, gravewell_pull_time, gravewell_cooldown);
        clamp!(self, d, 0.0, 10.0; bomb_projectile_radius, gravewell_projectile_radius);
        clamp!(self, d, 0.0, MAX_DIST; bomb_radius, breach_reach, breach_pullback, breach_radius,
            gravewell_pull_radius, gravewell_radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; bomb_damage, breach_damage, gravewell_damage);
        clamp!(self, d, 0.0, 90.0; gravewell_angle_degrees);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
        clamp_count!(self; bomb_contacts);
    }
}

impl LancetTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_TIME; interval, bloom_per_shot, bloom_max);
        clamp!(self, d, 0.0, 180.0; bloom_degrees);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
    }
}

impl ThresherTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp_count!(self; pellets);
        clamp!(self, d, 0.0, MAX_DAMAGE; pellet_damage);
        clamp!(self, d, 0.0, 180.0; cone_degrees);
        clamp!(self, d, 0.0, MAX_DIST; pellet_range);
        clamp!(self, d, 0.0, MAX_TIME; interval);
        clamp!(self, d, 0.0, MAX_SPEED; pellet_knockback);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
    }
}

impl RiveterTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_TIME; interval, rivet_lifetime);
        clamp!(self, d, 0.0, MAX_SPEED; rivet_speed);
        clamp!(self, d, 0.0, MAX_ACCEL; rivet_gravity);
        clamp!(self, d, 0.0, 10.0; rivet_radius);
        clamp!(self, d, 0.0, 180.0; spread_degrees);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
    }
}

impl CinderboxTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp_count!(self; rockets);
        clamp!(self, d, 0.0, MAX_TIME; salvo_gap, interval, rocket_lifetime);
        clamp!(self, d, 0.0, MAX_SPEED; rocket_speed, knockback);
        clamp!(self, d, 0.0, 10.0; rocket_radius);
        clamp!(self, d, 0.0, 180.0; spread_degrees);
        clamp!(self, d, 0.0, MAX_DIST; explosion_radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
    }
}

impl LongwickTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; draw_time, interval, arrow_lifetime);
        clamp!(self, d, 0.0, MAX_SPEED; min_speed, max_speed);
        clamp!(self, d, 0.0, MAX_DAMAGE; min_damage, max_damage);
        clamp!(self, d, 0.0, MAX_ACCEL; arrow_gravity);
        clamp!(self, d, 0.0, 10.0; arrow_radius);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
    }
}

impl KilnTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DIST; range);
        clamp!(self, d, 0.0, 360.0; cone_degrees);
        clamp!(self, d, 0.0, MAX_TIME; interval);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
        clamp_count!(self; burn_stacks);
    }
}

impl FlakTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp_count!(self; pellets);
        clamp!(self, d, 0.0, MAX_SPEED; shell_speed);
        clamp!(self, d, 0.0, 10.0; shell_radius);
        clamp!(self, d, 0.0, MAX_DIST; burst_distance, pellet_range);
        clamp!(self, d, 0.0, MAX_DAMAGE; pellet_damage);
        clamp!(self, d, 0.0, 180.0; ring_degrees);
        clamp!(self, d, 0.0, MAX_TIME; cooldown);
    }
}

impl LimpetTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp_count!(self; max_live);
        clamp!(self, d, 0.0, MAX_SPEED; speed, knockback);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_TIME; fuse, cooldown, detonate_hold);
        clamp!(self, d, 0.0, MAX_DIST; explosion_radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
    }
}

impl PinningArrowTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; enemy_time, heavy_time, player_time, cooldown);
        clamp!(self, d, 0.0, MAX_DAMAGE; heavy_health);
        clamp!(self, d, 0.0, 1.0; player_slow);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale);
    }
}

impl BurnTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DAMAGE; dps);
        clamp!(self, d, 0.0, MAX_TIME; duration, tick_interval);
        clamp_count!(self; max_stacks);
    }
}

impl EnemyProjectileTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, 10.0; ember_radius, needle_radius);
        clamp!(self, d, 0.0, MAX_ACCEL; ember_gravity, needle_gravity);
        clamp!(self, d, 0.0, MAX_TIME; ember_lifetime, needle_lifetime);
        clamp!(self, d, 0.0, MAX_DIST; ember_explosion_radius);
        clamp!(self, d, 0.0, MAX_SPEED; ember_knockback);
    }
}

impl TrimmerTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_TIME; interval);
        clamp!(self, d, 0.0, MAX_DIST; reach);
        clamp!(self, d, 0.0, 360.0; arc_degrees);
        clamp!(self, d, 0.0, MAX_SPEED; knockback);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
        clamp_count!(self; max_targets);
    }
}

impl TamperTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_TIME; interval);
        clamp!(self, d, 0.0, MAX_DIST; reach);
        clamp!(self, d, 0.0, 360.0; arc_degrees);
        clamp!(self, d, 0.0, MAX_SPEED; knockback);
        clamp!(self, d, 0.0, MAX_MULT; ability_power);
    }
}

impl LungeTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_SPEED; speed, knockback, lift);
        clamp!(self, d, 0.0, 1.0; vertical);
        clamp!(self, d, 0.0, MAX_TIME; window, cooldown);
        clamp!(self, d, 0.0, MAX_DIST; reach);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale);
    }
}

impl WhirlTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DIST; radius);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale, up);
        clamp!(self, d, 0.0, MAX_SPEED; knockback);
        clamp!(self, d, 0.0, MAX_TIME; cooldown);
    }
}

impl HurlTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_SPEED; speed, knockback, return_speed);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_TIME; flight_time, cooldown);
        clamp!(self, d, 0.0, MAX_DIST; max_distance);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale, return_damage_scale);
        clamp!(self, d, 0.0, 1.0; catch_refund);
    }
}

impl LamplighterTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; focus_time, interval);
        clamp!(self, d, 0.0, MAX_DAMAGE; min_damage, max_damage);
        clamp!(self, d, 0.0, MAX_MULT; pierce_falloff, ability_power);
        clamp!(self, d, 0.05, 1.0; zoom);
        clamp_count!(self; pierce);
    }
}

impl WicklineTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DIST; range);
        clamp!(self, d, 0.0, MAX_SPEED; zip_speed, zip_lift, yank_speed);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage, heavy_health);
        clamp!(self, d, 0.0, MAX_TIME; line_time, cooldown);
        clamp!(self, d, 0.0, 1.0; player_yank_scale, miss_cooldown);
    }
}

impl TallyTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; hold_time, cooldown);
        clamp!(self, d, 0.0, MAX_DIST; range);
        clamp!(self, d, 0.0, 180.0; assist_degrees);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, 1.0; empty_cooldown);
        // The owner's own state holds at most `abilities::tally::MAX_TALLY` marks.
        self.max_targets = self.max_targets.min(4);
    }
}

impl WaxworkTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; duration, cooldown);
        clamp!(self, d, 1.0, MAX_DAMAGE; health);
        clamp!(self, d, 0.0, MAX_DAMAGE; burst_damage);
        clamp!(self, d, 0.0, MAX_DIST; lure_range, burst_radius);
        clamp!(self, d, 0.0, MAX_SPEED; burst_knockback);
        clamp_count!(self; burn_stacks);
    }
}

impl BolasTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_SPEED; speed);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_TIME; flight_time, link_time, cooldown);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp!(self, d, 0.0, MAX_DIST; link_range);
        clamp!(self, d, 0.0, 1.0; link_share);
    }
}

impl BulwarkTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_DIST; width, height, distance);
        clamp!(self, d, 0.0, 90.0; pitch_limit_degrees);
        clamp!(self, d, 0.0, 360.0; cone_degrees);
        clamp!(self, d, 0.0, MAX_TIME; hold_time, cooldown, broken_cooldown);
        clamp!(self, d, 0.0, MAX_DAMAGE; absorb);
        clamp!(self, d, 0.0, 1.0; move_scale);
    }
}

impl StokeTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; duration, overheat, cooldown);
        clamp!(self, d, 0.1, 10.0; rate);
        clamp!(self, d, 0.0, MAX_MULT; kiln_reach);
        clamp_count!(self; burn_stacks);
    }
}

impl LanternTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_SPEED; toss_speed);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_TIME; duration, cooldown);
        clamp!(self, d, 0.0, MAX_DIST; range, kiln_range);
        clamp!(self, d, 0.01, MAX_MULT; rate_scale);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale);
        clamp!(self, d, 1.0, MAX_DAMAGE; health);
    }
}

impl KiteTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_SPEED; speed, knockback);
        clamp!(self, d, 0.0, 3600.0; turn_degrees);
        clamp!(self, d, 0.0, MAX_TIME; steer_time, flight_time, cooldown);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_DIST; explosion_radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
    }
}

impl BlinkArrowTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; window, cooldown);
        clamp!(self, d, 0.0, MAX_SPEED; exit_speed);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale);
    }
}

impl BeaconArrowTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; duration, cooldown);
        clamp!(self, d, 0.0, MAX_DIST; radius);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale, damage_multiplier);
    }
}

impl RiposteTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; hold_time, stagger, cooldown);
        clamp!(self, d, 0.0, 1.0; move_scale);
        clamp!(self, d, 0.0, 360.0; cone_degrees);
        clamp!(self, d, 0.0, MAX_MULT; damage_scale, per_block, max_scale);
        clamp!(self, d, 0.0, MAX_SPEED; knockback);
    }
}

impl KnackTuning {
    pub fn sanitize(&mut self) {
        self.kindle_flask.sanitize();
        self.snare.sanitize();
        self.soot_pot.sanitize();
        self.tallow_skin.sanitize();
        self.surge.sanitize();
    }
}

impl KindleFlaskTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; recharge_time, flight_time, duration, haste_time,
            haste_heal_interval);
        clamp!(self, d, 0.0, MAX_SPEED; speed);
        clamp!(self, d, 0.0, 90.0; angle_degrees);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius, haste_scale);
        clamp!(self, d, 0.0, MAX_DIST; radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; heal_rate, haste_heal);
        clamp_count!(self; charges);
    }
}

impl SnareTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; recharge_time, arm_time, lifetime);
        clamp!(self, d, 0.0, MAX_SPEED; speed);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_DIST; trigger_radius);
        clamp!(self, d, 0.0, MAX_DAMAGE; damage);
        clamp_count!(self; charges, max_live);
    }
}

impl SootPotTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; recharge_time, flight_time, duration);
        clamp!(self, d, 0.0, MAX_SPEED; speed);
        clamp!(self, d, 0.0, MAX_ACCEL; gravity);
        clamp!(self, d, 0.0, 10.0; projectile_radius);
        clamp!(self, d, 0.0, MAX_DIST; radius);
        clamp_count!(self; charges);
    }
}

impl TallowSkinTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; recharge_time, duration);
        clamp!(self, d, 0.0, 1.0; damage_scale);
        clamp_count!(self; charges);
    }
}

impl SurgeTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; recharge_time);
        clamp!(self, d, 0.0, MAX_SPEED; speed);
        clamp!(self, d, 0.0, 1.0; vertical);
        clamp_count!(self; charges);
    }
}

impl NetTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 1.0, 1000.0; snapshot_rate);
        clamp!(self, d, 0.0, MAX_MS; interp_delay_ms, lag_comp_max_ms, parry_tolerance_ms,
            max_extrapolation_ms, interp_delay_max_ms);
        clamp!(self, d, 0.0, 1000.0; correction_smoothing);
        clamp!(self, d, 0.0, MAX_DIST; correction_snap_distance);
        clamp!(self, d, 0.0, MAX_MULT; interp_jitter_factor);
    }
}

impl StyleTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 1.0, MAX_POINTS; points_per_rank);
        clamp!(self, d, 0.0, MAX_POINTS; decay_lowest, decay_highest, kill, frag, headshot,
            airkill, airshot, multikill, parry, return_to_sender, ricochet, ricochet_per_bounce,
            deflect, exploded, slam_kill, fisted);
        clamp!(self, d, 0.0, 1.0; rank_down_refill, freshness_cost, freshness_restore,
            freshness_regen, freshness_min, stale_threshold);
        clamp!(self, d, 0.0, MAX_TIME; rank_grace, multikill_window);
        clamp!(self, d, 0.0, MAX_DAMAGE; close_call_health);
        clamp!(self, d, 0.0, MAX_MULT; close_call_multiplier);
        clamp!(self, d, 0.0, MAX_POINTS; longshot);
        clamp!(self, d, 0.0, MAX_DIST; longshot_distance);
    }
}

impl ModeTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; sp_death_time, downed_time, revive_time,
            fall_respawn_time, wipe_respawn_time, exit_countdown, knockoff_credit,
            scoreboard_interval);
        clamp!(self, d, 0.0, MAX_DIST; revive_range);
        clamp!(self, d, 0.0, MAX_DAMAGE; revive_health);
        for (values, defaults) in [
            (&mut self.grade_time, d.grade_time),
            (&mut self.grade_kills, d.grade_kills),
            (&mut self.grade_style, d.grade_style),
            (&mut self.grade_damage, d.grade_damage),
        ] {
            for (v, default) in values.iter_mut().zip(defaults) {
                *v = clamped(*v, 0.0, MAX_POINTS, default);
            }
        }
    }
}

/// Clamps every entry of a per-difficulty table (NaN → the default entry).
fn clamp_table(values: &mut [f32; 5], defaults: [f32; 5], lo: f32, hi: f32) {
    for (v, d) in values.iter_mut().zip(defaults) {
        *v = clamped(*v, lo, hi, d);
    }
}

impl DifficultyTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        for (values, defaults) in [
            (&mut self.health, d.health),
            (&mut self.count, d.count),
            (&mut self.damage, d.damage),
            (&mut self.aggression, d.aggression),
            (&mut self.projectile_speed, d.projectile_speed),
            (&mut self.downed_time, d.downed_time),
            (&mut self.revive_health, d.revive_health),
        ] {
            clamp_table(values, defaults, 0.01, MAX_MULT);
        }
        clamp_table(&mut self.elite_chance, d.elite_chance, 0.0, 1.0);
        clamp!(self, d, 0.0, MAX_MULT; player_health, player_count, player_damage,
            player_aggression);
        clamp!(self, d, 0.0, 1.0; player_elite_chance, player_downed_time);
        clamp!(self, d, 0.0, MAX_TIME; min_downed_time);
    }
}

/// Longest grind table / enemy name accepted (a server's tuning is untrusted on clients).
const MAX_GRIND_ROSTER: usize = 64;
const MAX_ENEMY_NAME: usize = 64;

/// Longest gun game ladder accepted (tiers ride in a byte on the scoreboard).
const MAX_LADDER: usize = 32;

impl GunGameTuning {
    pub fn sanitize(&mut self) {
        self.ladder.truncate(MAX_LADDER);
        for b in &mut self.ladder {
            *b = b.sanitized();
        }
        if self.ladder.is_empty() {
            self.ladder = Self::default().ladder;
        }
        self.finale = self.finale.sanitized();
        self.punch_demotes = self.punch_demotes.min(MAX_LADDER as u32);
    }
}

impl GrindTuning {
    pub fn sanitize(&mut self) {
        let d = Self::default();
        clamp!(self, d, 0.0, MAX_TIME; first_delay, breather, spawn_interval, end_delay);
        clamp!(self, d, 0.0, MAX_POINTS; budget_base, budget_per_wave, budget_per_wave_sq);
        clamp!(self, d, 0.0, MAX_MULT; health_per_wave, heavy_budget, boss_health);
        clamp!(self, d, 0.0, 1.0; heavy_elite_chance);
        clamp!(self, d, 0.0, MAX_DIST; min_spawn_distance);
        self.heavy_every = self.heavy_every.min(10_000);
        self.boss_every = self.boss_every.min(10_000);
        self.max_alive = self.max_alive.clamp(1, 256);
        self.alive_per_player = self.alive_per_player.min(256);
        self.spawn_burst = self.spawn_burst.min(256);
        self.max_wave_enemies = self.max_wave_enemies.clamp(1, 10_000);
        for (v, default) in self.grade_wave.iter_mut().zip(d.grade_wave) {
            *v = clamped(*v, 0.0, MAX_POINTS, default);
        }
        self.roster.truncate(MAX_GRIND_ROSTER);
        let fallback = GrindEnemy::default();
        for (i, e) in self.roster.iter_mut().enumerate() {
            let de = d.roster.get(i).unwrap_or(&fallback);
            // Costs below 0.1 would let one wave hold thousands of Moulds.
            e.cost = clamped(e.cost, 0.1, MAX_POINTS, de.cost);
            e.weight = clamped(e.weight, 0.0, MAX_POINTS, de.weight);
            e.from_wave = e.from_wave.min(10_000);
            if e.enemy.len() > MAX_ENEMY_NAME {
                e.enemy = e.enemy.chars().take(MAX_ENEMY_NAME).collect();
            }
        }
    }
}

impl Tuning {
    /// `tuning.ron` in the assets folder on disk: hot reload watches it, the tuning editor
    /// writes it. [`Tuning::load_default`] falls back to its embedded copy.
    pub fn default_path() -> PathBuf {
        crate::assets_dir().join("tuning.ron")
    }

    /// Clamps every value to a sane range: NaN becomes the field's default, floats are
    /// limited (distances ≤ 1e4 m, speeds ≤ 1e3 m/s, times ≤ 1 h, rates > 0...), counts
    /// ≤ 64. Idempotent; the defaults (and any reasonable file) are unchanged. Applied on
    /// every load and by clients to the tuning a server sends. Returns true if anything was
    /// changed.
    pub fn sanitize(&mut self) -> bool {
        let before = self.clone();
        self.movement.sanitize();
        self.combat.sanitize();
        self.net.sanitize();
        self.style.sanitize();
        self.modes.sanitize();
        self.difficulty.sanitize();
        self.grind.sanitize();
        self.gun_game.sanitize();
        self.balance.sanitize();
        *self != before
    }

    /// Loads and sanitizes a tuning file (out-of-range values are clamped with a warning).
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text, &path.display().to_string())
    }

    /// Parses and sanitizes tuning RON; the clamping warning names it `what`.
    fn parse(text: &str, what: &str) -> anyhow::Result<Self> {
        let mut tuning: Tuning = ron::from_str(text)?;
        if tuning.sanitize() {
            log::warn!("{what}: out-of-range values were clamped");
        }
        Ok(tuning)
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

    /// The game's tuning: `tuning.ron` from the assets folder on disk, else its embedded copy
    /// (`shared::asset_files`); defaults (with a warning) if it is invalid.
    pub fn load_default() -> Self {
        let loaded = crate::asset_files::read_to_string("tuning.ron").and_then(|text| {
            Self::parse(&text, "tuning.ron").map_err(|e| anyhow::anyhow!("tuning.ron: {e}"))
        });
        match loaded {
            Ok(t) => t,
            Err(err) => {
                log::warn!("using default tuning ({err})");
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

    /// The defaults as RON with every float replaced by `float` and every integer by `int`.
    fn defaults_with(float: &str, int: &str) -> Tuning {
        let text = ron::to_string(&Tuning::default()).unwrap();
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        let mut prev = ' ';
        while let Some(c) = chars.next() {
            let starts_number =
                (c.is_ascii_digit() || c == '-') && matches!(prev, ':' | ',' | '(' | '[');
            if !starts_number {
                out.push(c);
                prev = c;
                continue;
            }
            let mut token = c.to_string();
            while let Some(&n) = chars.peek() {
                if !(n.is_ascii_alphanumeric() || matches!(n, '.' | '-' | '+')) {
                    break;
                }
                token.push(n);
                chars.next();
            }
            out.push_str(if token.contains(['.', 'e']) {
                float
            } else {
                int
            });
            prev = '0';
        }
        ron::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"))
    }

    #[test]
    fn sanitize_keeps_the_defaults_and_the_shipped_file() {
        let mut t = Tuning::default();
        assert!(!t.sanitize());
        assert_eq!(t, Tuning::default());
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
        let mut shipped: Tuning = ron::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(
            !shipped.sanitize(),
            "assets/tuning.ron has out-of-range values"
        );
    }

    #[test]
    fn the_default_tuning_is_the_shipped_file_also_when_embedded() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
        let shipped = Tuning::load(&path).unwrap();
        assert_eq!(Tuning::load_default(), shipped);
        let embedded = crate::asset_files::embedded("tuning.ron").expect("embedded");
        let embedded = Tuning::parse(std::str::from_utf8(embedded).unwrap(), "embedded").unwrap();
        assert_eq!(embedded, shipped);
    }

    #[test]
    fn sanitize_checks_every_field() {
        // NaN everywhere: every float falls back to its default (a field missing from
        // `sanitize` would stay NaN and fail the comparison).
        let mut t = defaults_with("NaN", "3");
        assert!(t.sanitize());
        let mut expected = Tuning::default();
        for n in [
            &mut expected.movement.wall_jumps,
            &mut expected.combat.thorn.charges,
            &mut expected.combat.bellows.pellets,
            &mut expected.combat.arclash.chain_jumps,
            &mut expected.combat.tumbler.bomb_contacts,
            &mut expected.grind.heavy_every,
            &mut expected.grind.boss_every,
            &mut expected.grind.max_alive,
            &mut expected.grind.alive_per_player,
            &mut expected.grind.spawn_burst,
            &mut expected.grind.max_wave_enemies,
            &mut expected.combat.thresher.pellets,
            &mut expected.combat.cinderbox.rockets,
            &mut expected.combat.flak.pellets,
            &mut expected.combat.limpet.max_live,
            &mut expected.combat.burn.max_stacks,
            &mut expected.combat.kiln.burn_stacks,
            &mut expected.combat.lamplighter.pierce,
            &mut expected.combat.tally.max_targets,
            &mut expected.combat.waxwork.burn_stacks,
            &mut expected.combat.stoke.burn_stacks,
            &mut expected.combat.trimmer.max_targets,
            &mut expected.combat.knacks.kindle_flask.charges,
            &mut expected.combat.knacks.snare.charges,
            &mut expected.combat.knacks.snare.max_live,
            &mut expected.combat.knacks.soot_pot.charges,
            &mut expected.combat.knacks.tallow_skin.charges,
            &mut expected.combat.knacks.surge.charges,
        ] {
            *n = 3;
        }
        expected.gun_game.punch_demotes = 3;
        for e in &mut expected.grind.roster {
            e.from_wave = 3;
        }
        assert_eq!(t, expected);

        // Absurdly large values (what a hostile server could send): clamped, and stable.
        for float in ["1e30", "inf", "-1e30", "-inf"] {
            let mut t = defaults_with(float, &u32::MAX.to_string());
            assert!(t.sanitize());
            let c = &t.combat;
            assert_eq!(c.bellows.pellets, MAX_COUNT);
            assert_eq!(c.flak.pellets, MAX_COUNT);
            assert_eq!(c.tally.max_targets, 4, "the owner's state holds four marks");
            assert!(c.hitscan_range <= MAX_DIST && c.bellows.pellet_range <= MAX_DIST);
            assert!(c.parry_max_speed <= MAX_SPEED && c.tumbler.bomb_speed <= MAX_SPEED);
            assert!(c.hit_assist.enemy_body_pad <= 2.0 && t.net.snapshot_rate >= 1.0);
            assert!(t.movement.radius >= 0.05 && t.style.points_per_rank >= 1.0);
            assert!(t.net.lag_comp_max_ms <= MAX_MS);
            let text = ron::to_string(&t).unwrap();
            assert!(!text.contains("inf") && !text.contains("NaN"), "{text}");
            let before = t.clone();
            assert!(!t.sanitize(), "idempotent");
            assert_eq!(t, before);
        }
    }

    #[test]
    fn partial_files_use_defaults() {
        let t: Tuning = ron::from_str("(movement: (jump_speed: 20.0))").unwrap();
        assert_eq!(t.movement.jump_speed, 20.0);
        assert_eq!(t.movement.gravity, MovementTuning::default().gravity);
    }

    #[test]
    fn old_blood_section_still_loads() {
        // Tuning files written before the ember rename call the section `blood`.
        let t: Tuning = ron::from_str("(combat: (blood: (heal_fraction: 0.25)))").unwrap();
        assert_eq!(t.combat.embers.heal_fraction, 0.25);
        assert_eq!(t.combat.embers.radius, EmberTuning::default().radius);
    }
}
