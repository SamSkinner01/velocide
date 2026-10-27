//! Data-driven enemy definitions: `assets/enemies/<name>.ron`.
//!
//! Every field has a default (`#[serde(default)]`), so a definition only spells out what makes
//! the enemy different. Local offsets (weak points, muzzles) are in the enemy's frame: +X right,
//! +Y up, −Z forward (the facing direction, like `input::look_dir` at yaw 0), from the feet.
//! See `docs/combat.md` → "Enemy definitions" for every field.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::sim::projectiles::ProjectileKind;
use crate::sim::{HitPart, HitShape, HitTarget, Hitbox};

/// Built-in definitions (compiled in, so the server works without an assets directory and
/// tests don't depend on edited files). Files in `assets/enemies/` override them by name.
const BUILTIN: &[(&str, &str)] = &[
    (
        "gnawer",
        include_str!("../../../../assets/enemies/gnawer.ron"),
    ),
    (
        "censer",
        include_str!("../../../../assets/enemies/censer.ron"),
    ),
    (
        "harrier",
        include_str!("../../../../assets/enemies/harrier.ron"),
    ),
    (
        "gaoler",
        include_str!("../../../../assets/enemies/gaoler.ron"),
    ),
    (
        "heron",
        include_str!("../../../../assets/enemies/heron.ron"),
    ),
    (
        "bellwether",
        include_str!("../../../../assets/enemies/bellwether.ron"),
    ),
    (
        "snuffer",
        include_str!("../../../../assets/enemies/snuffer.ron"),
    ),
    (
        "smelter",
        include_str!("../../../../assets/enemies/smelter.ron"),
    ),
    (
        "effigy",
        include_str!("../../../../assets/enemies/effigy.ron"),
    ),
];

/// How an enemy moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Locomotion {
    /// Walks on the level with gravity (capsule sweeps against the world).
    #[default]
    Ground,
    /// Flies without gravity, keeping its centre (the model's hover centre, half the height
    /// above `Enemy::position`) between `min_height` and `max_height` above the ground.
    Flying { min_height: f32, max_height: f32 },
}

/// What a weak point counts as for scoring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeakKind {
    /// Counts as a HEADSHOT.
    #[default]
    Head,
    /// Any other weak point (a glowing core, a back vent...).
    Core,
}

/// A sphere hitbox with a damage multiplier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeakPoint {
    pub name: String,
    pub kind: WeakKind,
    /// Local offset from the feet (see the module docs).
    pub offset: Vec3,
    pub radius: f32,
    pub multiplier: f32,
}

impl Default for WeakPoint {
    fn default() -> Self {
        Self {
            name: "head".into(),
            kind: WeakKind::Head,
            offset: Vec3::new(0.0, 1.6, 0.0),
            radius: 0.25,
            multiplier: 2.0,
        }
    }
}

/// An extra body hit shape (a capsule, a sphere when `a == b`) for what the upright body
/// capsule doesn't cover: wings, a long body or neck, a shield, a slung crucible.
/// `HitPart::Body` (body damage, armour applies as usual).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BodyShape {
    /// Local offsets of the capsule's ends (see the module docs).
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

impl Default for BodyShape {
    fn default() -> Self {
        Self {
            a: Vec3::ZERO,
            b: Vec3::ZERO,
            radius: 0.3,
        }
    }
}

/// Directional armour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Armour {
    /// Damage multiplier for hits arriving from the front arc.
    pub front_multiplier: f32,
    /// Full aperture of the front arc (degrees).
    pub front_arc: f32,
    /// Weak points (by name) that ignore armour.
    pub bypass: Vec<String>,
    /// Explosions from the front are reduced too.
    pub explosions: bool,
}

impl Default for Armour {
    fn default() -> Self {
        Self {
            front_multiplier: 0.35,
            front_arc: 140.0,
            bypass: vec!["head".into()],
            explosions: true,
        }
    }
}

/// How an attack strikes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttackKind {
    /// Hits players within `range` inside the `arc` in front (with `lunge`: leaps and rakes
    /// whoever it reaches during the strike).
    #[default]
    Melee,
    /// Fires `projectile`.
    Projectile,
    /// Ground slam: hits every player within `shockwave_radius`.
    Slam,
    /// Flyers: a straight 3D dive at the target (`lunge` m/s for up to `strike` s, aimed with
    /// `dive_lead` s of lead), raking every player it passes within reach once; hitting the
    /// level ends it early. Recover climbs back towards hover height.
    Dive,
    /// Support (the Bellwether's toll): no damage; at the strike every other Mould within
    /// `toll.radius` is frenzied (`toll`). Chosen without line of sight, only when at least
    /// `toll.allies` allies are in reach and the target is within `range` (centre to chest).
    Toll,
    /// Pours a line of molten slag along the ground towards the target (the Smelter, `pour`):
    /// the line is laid (and replicated, glowing) when the wind-up starts, aimed and fixed
    /// then, and only burns from the strike on. `range` / `min_range`: centre to the target's
    /// feet.
    Pour,
}

/// A frenzy (the Bellwether's toll): multipliers on the frenzied enemy's behaviour, on top of
/// `SimWorld::enemy_scale` and the elite bonus. Wind-ups keep their length (readable).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Frenzy {
    /// Movement speed (walking, flying; not leaps or dives).
    pub speed: f32,
    /// Attack cooldowns and recoveries are divided by this.
    pub aggression: f32,
    /// Damage dealt.
    pub damage: f32,
    /// Projectile speed.
    pub projectile_speed: f32,
}

impl Default for Frenzy {
    fn default() -> Self {
        Self {
            speed: 1.3,
            aggression: 1.4,
            damage: 1.25,
            projectile_speed: 1.15,
        }
    }
}

/// `AttackKind::Toll` parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toll {
    /// Allies within this distance of the ringer (centre to centre) are frenzied.
    pub radius: f32,
    /// Seconds the frenzy lasts.
    pub duration: f32,
    /// Only rings with at least this many allies (that it could frenzy) in reach.
    pub allies: u32,
    pub frenzy: Frenzy,
}

impl Default for Toll {
    fn default() -> Self {
        Self {
            radius: 18.0,
            duration: 5.0,
            allies: 1,
            frenzy: Frenzy::default(),
        }
    }
}

/// `AttackKind::Pour` parameters: a straight line of slag on the ground from just in front of
/// the pourer towards (and past) the target. The attack's `damage` is damage per second to a
/// player standing in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pour {
    /// The line starts this far beyond the pourer's body surface.
    pub start: f32,
    /// Longest line (m).
    pub length: f32,
    /// How far past the target's (predicted) feet it runs.
    pub overshoot: f32,
    /// Lead on the target's horizontal velocity (seconds).
    pub lead: f32,
    /// Full width of the burning strip.
    pub width: f32,
    /// Seconds it burns after igniting.
    pub lifetime: f32,
    /// Seconds between damage ticks.
    pub interval: f32,
}

impl Default for Pour {
    fn default() -> Self {
        Self {
            start: 0.8,
            length: 20.0,
            overshoot: 4.0,
            lead: 0.4,
            width: 1.8,
            lifetime: 5.0,
            interval: 0.25,
        }
    }
}

/// A leap that **latches** onto the first player it reaches instead of raking (the Snuffer):
/// it clings to them (`AiState::Latched`), smothering the wick until it is punched or dashed
/// off, its victim dies or `max_time` runs out. The attack's `damage` is the impact on latching.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Latch {
    /// Damage per second while latched; all of it is hard damage (smothered).
    pub damage: f32,
    /// Seconds between damage ticks.
    pub interval: f32,
    /// Lets go after this long.
    pub max_time: f32,
    /// The victim moves at this fraction of their speeds while latched (the movement state's
    /// predicted slow, `MoveState::slow_scale`; a modest slow, not a root).
    pub slow: f32,
    /// A punch from the victim throws it off: counter damage and stun.
    pub punch_damage: f32,
    pub punch_stun: f32,
    /// A dash shakes it off: stun.
    pub shake_stun: f32,
}

impl Default for Latch {
    fn default() -> Self {
        Self {
            damage: 16.0,
            interval: 0.25,
            max_time: 6.0,
            slow: 0.72,
            punch_damage: 30.0,
            punch_stun: 1.6,
            shake_stun: 1.0,
        }
    }
}

/// Damage to a weak point during the wind-up cancels the attack and staggers the enemy (the
/// Bellwether's bell mid-swing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interrupt {
    /// Weak point (by name) that must be hit.
    pub weak_point: String,
    /// Damage (after the weak point multiplier) it takes during one wind-up.
    pub damage: f32,
    /// Stun (seconds).
    pub stun: f32,
}

impl Default for Interrupt {
    fn default() -> Self {
        Self {
            weak_point: "head".into(),
            damage: 20.0,
            stun: 1.5,
        }
    }
}

/// A training dummy (the Effigy): never targets, never attacks, never moves, can't be killed
/// (its health stops at 1) and heals back to full once nobody has hurt it for a while. Never
/// placed in encounters or waves ([`EnemyRegistry::wave_names`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dummy {
    /// Seconds without damage before it regenerates.
    pub regen_delay: f32,
    /// Health per second while regenerating.
    pub regen_rate: f32,
}

impl Default for Dummy {
    fn default() -> Self {
        Self {
            regen_delay: 3.0,
            regen_rate: 400.0,
        }
    }
}

/// What an elite of this type gets (any Mould can spawn as an elite, `SpawnOptions::elite`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EliteDef {
    /// Health multiplier (on top of the spawn's health scale).
    pub health: f32,
    /// Attack cooldowns and recoveries are divided by this (wind-ups unchanged).
    pub aggression: f32,
    /// Movement speed multiplier.
    pub speed: f32,
    /// Knockback multiplier (sturdier).
    pub knockback: f32,
    /// Heat (the death ember burst's heal pool) multiplier.
    pub heat: f32,
}

impl Default for EliteDef {
    fn default() -> Self {
        Self {
            health: 2.5,
            aggression: 1.25,
            speed: 1.1,
            knockback: 0.6,
            heat: 2.0,
        }
    }
}

/// Projectile parameters of a ranged attack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectileAttack {
    pub kind: ProjectileKind,
    pub speed: f32,
    /// Projectiles per strike (a burst).
    pub count: u32,
    /// Seconds between the projectiles of a burst.
    pub interval: f32,
    /// Random spread (full aperture, degrees).
    pub spread: f32,
    /// Local muzzle offset.
    pub muzzle: Vec3,
    /// 0 = aim at the target, 1 = full linear lead.
    pub lead: f32,
    /// Lobbed on a ballistic arc (the kind's gravity) that lands on the target's predicted
    /// feet: the launch speed is solved for the arc and capped at `speed` (out of reach it
    /// flies at `speed`, 45°). A burst is fanned evenly across `spread` (yaw) instead of
    /// random spread.
    pub lob: bool,
}

impl Default for ProjectileAttack {
    fn default() -> Self {
        Self {
            kind: ProjectileKind::Needle,
            speed: 22.0,
            count: 1,
            interval: 0.15,
            spread: 0.0,
            muzzle: Vec3::new(0.0, 1.4, -0.4),
            lead: 0.0,
            lob: false,
        }
    }
}

/// A visible aim laser during the wind-up (snipers): the aim tracks the target, then locks
/// for the last `lock` seconds so the target can dodge; the projectile flies along the locked
/// aim. Replicated (`enemy_flags::AIMING` / `AIM_LOCKED`, exact yaw and pitch) so every
/// client draws the laser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AimLaser {
    /// Seconds before the strike the aim stops tracking.
    pub lock: f32,
    /// Tracking speed while unlocked (degrees per second).
    pub track_rate: f32,
    /// Lead on the target's horizontal velocity while tracking (0 = none, 1 = full linear
    /// lead for the projectile's flight time).
    pub lead: f32,
}

impl Default for AimLaser {
    fn default() -> Self {
        Self {
            lock: 0.25,
            track_rate: 120.0,
            lead: 0.0,
        }
    }
}

/// One attack. Timeline: wind-up (telegraph) → strike → recover, then `cooldown` before it
/// can be used again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttackDef {
    pub name: String,
    pub kind: AttackKind,
    /// Starts when the target is within this distance (between body surfaces; projectiles:
    /// between centres).
    pub range: f32,
    pub min_range: f32,
    /// Melee hit cone (full aperture, degrees).
    pub arc: f32,
    pub damage: f32,
    /// Knockback speed given to players hit.
    pub knockback: f32,
    /// Upward bias of the knockback direction (0 = flat along the hit).
    pub knockback_up: f32,
    /// Telegraph duration (seconds).
    pub windup: f32,
    /// Duration of the strike phase (projectile bursts fire during it).
    pub strike: f32,
    pub recover: f32,
    pub cooldown: f32,
    /// Movement speed fraction while winding up / striking.
    pub windup_move_scale: f32,
    /// Keep turning towards the target during the wind-up.
    pub track_target: bool,
    /// Parryable: the last `parry_window` seconds of the wind-up (telegraphed melee parry).
    pub parry_window: Option<f32>,
    /// Parryable during the whole strike instead (a punch mid-dive swats a Harrier).
    pub parry_strike: bool,
    /// Stun duration and counter damage when parried.
    pub parry_stun: f32,
    pub parry_damage: f32,
    /// How far from the puncher's eye a parry counts (metres beyond the enemy's radius, from
    /// its centre); 0 = the tuning's `combat.melee_parry_range`.
    pub parry_reach: f32,
    /// Slam radius.
    pub shockwave_radius: f32,
    pub projectile: Option<ProjectileAttack>,
    /// Melee lunge: forward speed (m/s) the enemy leaps at when the strike starts; 0 = none.
    /// A lunging strike hits every player it reaches during the whole strike phase (once) and
    /// ends early when a wall stops it. `Dive`: the dive speed.
    pub lunge: f32,
    /// Upward launch speed of a lunge (m/s): a pounce on a ballistic arc. 0 = along the
    /// ground.
    pub lunge_up: f32,
    /// `Dive`: seconds of lead on the target's velocity when the dive is aimed.
    pub dive_lead: f32,
    /// Only chosen against a target moving at least this fast (m/s, horizontal): gap closers
    /// for fleeing players. 0 = always.
    pub min_target_speed: f32,
    /// A low sweep: only hits targets whose feet are less than this many metres above the
    /// attacker's feet (jump it). `None` = any height the attack reaches.
    pub jumpable: Option<f32>,
    /// Aim laser during the wind-up (projectile attacks).
    pub laser: Option<AimLaser>,
    /// `Toll`: who is frenzied, for how long.
    pub toll: Option<Toll>,
    /// `Pour`: the slag line.
    pub pour: Option<Pour>,
    /// A lunge that latches onto its victim instead of raking (`lunge` > 0).
    pub latch: Option<Latch>,
    /// Hitting this weak point hard enough during the wind-up cancels the attack.
    pub interrupt: Option<Interrupt>,
}

impl Default for AttackDef {
    fn default() -> Self {
        Self {
            name: "attack".into(),
            kind: AttackKind::Melee,
            range: 2.0,
            min_range: 0.0,
            arc: 120.0,
            damage: 10.0,
            knockback: 5.0,
            knockback_up: 0.35,
            windup: 0.5,
            strike: 0.1,
            recover: 0.5,
            cooldown: 0.5,
            windup_move_scale: 0.15,
            track_target: true,
            parry_window: None,
            parry_strike: false,
            parry_stun: 2.5,
            parry_damage: 150.0,
            parry_reach: 0.0,
            shockwave_radius: 5.0,
            projectile: None,
            lunge: 0.0,
            lunge_up: 0.0,
            dive_lead: 0.15,
            min_target_speed: 0.0,
            jumpable: None,
            laser: None,
            toll: None,
            pour: None,
            latch: None,
            interrupt: None,
        }
    }
}

/// Senses and decision making.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiDef {
    pub sight_range: f32,
    /// Seconds between noticing a player and acting (the "alert" beat).
    pub reaction_time: f32,
    /// Keep chasing the last known position this long after losing sight.
    pub lose_target_time: f32,
    /// Ranged enemies try to stay within this band (min, max) of their target.
    pub preferred_range: Option<(f32, f32)>,
    /// Strafe sideways while in the preferred band.
    pub strafe: bool,
    /// Only switch to another visible player that is closer than this fraction of the
    /// current target's distance.
    pub target_stickiness: f32,
    /// Switch to a player who dealt this fraction of max health within the last 2 s.
    pub aggro_switch_fraction: f32,
    /// Seconds between target re-evaluations.
    pub retarget_interval: f32,
    /// Seconds between path re-plans while chasing (navigation, see `sim::nav`).
    pub repath_interval: f32,
    /// Ranged enemies: after each ranged strike, flip the strafe direction (snipers don't
    /// fire twice from the same spot).
    pub reposition: bool,
}

impl Default for AiDef {
    fn default() -> Self {
        Self {
            sight_range: 60.0,
            reaction_time: 0.35,
            lose_target_time: 4.0,
            preferred_range: None,
            strafe: false,
            target_stickiness: 0.6,
            aggro_switch_fraction: 0.3,
            retarget_interval: 0.25,
            repath_interval: 0.6,
            reposition: false,
        }
    }
}

/// What it leaves behind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Drops {
    /// The heat it releases on death: heal pool (HP) of its big ember burst.
    #[serde(alias = "blood_pool")]
    pub heat: f32,
    /// Radius (m) of that burst.
    #[serde(alias = "blood_radius")]
    pub heat_radius: f32,
}

impl Default for Drops {
    fn default() -> Self {
        Self {
            heat: 25.0,
            heat_radius: 4.5,
        }
    }
}

/// Presentation colours (sRGB 0..1); clients use them for hit sprays, telegraph glints and
/// death bursts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnemyColors {
    /// Wax-flesh.
    pub body: Vec3,
    /// Slag / brass / chain.
    pub accent: Vec3,
    /// Weak point glow (the heat collected inside).
    pub glow: Vec3,
    /// Wind-up telegraph flash.
    pub telegraph: Vec3,
    /// Parryable-phase flash.
    pub parry: Vec3,
}

impl Default for EnemyColors {
    fn default() -> Self {
        Self {
            body: Vec3::splat(0.5),
            accent: Vec3::splat(0.3),
            glow: Vec3::new(1.0, 0.2, 0.1),
            telegraph: Vec3::new(1.0, 0.1, 0.05),
            parry: Vec3::new(1.0, 0.9, 0.1),
        }
    }
}

/// One enemy type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnemyDef {
    pub name: String,
    /// Replicated entity kind (`16..64`, unique per type; see `protocol::kind`).
    pub net_kind: u16,
    /// Model name for the client (`assets/models/<model>.gltf` or a procedural model).
    pub model: String,
    /// Drawn size of the model: the client scales it by this about its origin (the feet; the
    /// Harrier's hover centre). Presentation only: the movement capsule (`radius`, `height`)
    /// keeps the nav grid and the level's doors and ledges as they are, and the hit shapes
    /// (`hit_radius`, `body_top`, `body_shapes`, `weak_points`) are sized to the scaled model
    /// in this file (docs/enemies.md "Sizes and hit shapes").
    pub model_scale: f32,
    /// Base health (co-op scaling multiplies it).
    pub health: f32,
    pub speed: f32,
    pub acceleration: f32,
    /// Turn rate (degrees per second).
    pub turn_rate: f32,
    /// Collision capsule (movement) radius and height.
    pub radius: f32,
    pub height: f32,
    /// Top of the *body hitbox* capsule (so a head weak point can sit above it); `None` =
    /// `height`.
    pub body_top: Option<f32>,
    /// Radius of the body hitbox capsule; `None` = the movement `radius`. Hit shapes cover
    /// the (scaled) model, which can be wider than the capsule it moves with.
    pub hit_radius: Option<f32>,
    /// Extra body hit shapes beside the upright body capsule ([`BodyShape`]).
    pub body_shapes: Vec<BodyShape>,
    pub eye_height: f32,
    pub step_height: f32,
    pub locomotion: Locomotion,
    /// Knockback multiplier (light enemies > 1, heavies < 1).
    pub knockback_scale: f32,
    pub weak_points: Vec<WeakPoint>,
    pub armour: Option<Armour>,
    pub attacks: Vec<AttackDef>,
    pub ai: AiDef,
    pub drops: Drops,
    pub colors: EnemyColors,
    /// Seconds the corpse stays (Dying state) before it is removed.
    pub death_time: f32,
    /// A training dummy (the Effigy): see [`Dummy`].
    pub dummy: Option<Dummy>,
    /// What an elite of this type gets.
    pub elite: EliteDef,
}

impl Default for EnemyDef {
    fn default() -> Self {
        Self {
            name: "Enemy".into(),
            net_kind: crate::protocol::kind::ENEMY_FIRST,
            model: String::new(),
            model_scale: 1.0,
            health: 50.0,
            speed: 6.0,
            acceleration: 50.0,
            turn_rate: 540.0,
            radius: 0.45,
            height: 1.9,
            body_top: None,
            hit_radius: None,
            body_shapes: Vec::new(),
            eye_height: 1.6,
            step_height: 0.45,
            locomotion: Locomotion::Ground,
            knockback_scale: 1.0,
            weak_points: Vec::new(),
            armour: None,
            attacks: Vec::new(),
            ai: AiDef::default(),
            drops: Drops::default(),
            colors: EnemyColors::default(),
            death_time: 0.6,
            dummy: None,
            elite: EliteDef::default(),
        }
    }
}

impl EnemyDef {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(ron::from_str(text)?)
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))
    }

    /// Weak point index by name.
    pub fn weak_point(&self, name: &str) -> Option<usize> {
        self.weak_points.iter().position(|w| w.name == name)
    }

    /// [`EnemyDef::model_scale`], kept sane (a file can say anything): 0.25..4, else 1.
    pub fn draw_scale(&self) -> f32 {
        if self.model_scale.is_finite() && self.model_scale > 0.0 {
            self.model_scale.clamp(0.25, 4.0)
        } else {
            1.0
        }
    }

    /// The hit shapes of one of these standing with its feet at `feet`, facing `yaw`: the
    /// upright body capsule (`hit_radius`, from the feet to `body_top`), the extra
    /// `body_shapes`, then a sphere per weak point (`HitPart::Weak(i)`). The server's hit tests
    /// and lag compensation use these (`Enemy::hitbox`); clients draw them in debug views.
    pub fn hitbox(&self, target: HitTarget, feet: Vec3, yaw: f32) -> Hitbox {
        let rot = Quat::from_rotation_y(yaw);
        let at = |offset: Vec3| feet + rot * offset;
        let r = self.hit_radius.unwrap_or(self.radius).max(0.05);
        let h = self.body_top.unwrap_or(self.height).max(2.0 * r);
        let mut shapes = Vec::with_capacity(1 + self.body_shapes.len() + self.weak_points.len());
        shapes.push(HitShape::capsule(
            HitPart::Body,
            feet + Vec3::Y * r,
            feet + Vec3::Y * (h - r),
            r,
        ));
        for s in &self.body_shapes {
            shapes.push(HitShape::capsule(
                HitPart::Body,
                at(s.a),
                at(s.b),
                s.radius.max(0.01),
            ));
        }
        for (i, w) in self.weak_points.iter().enumerate() {
            shapes.push(HitShape::sphere(
                HitPart::Weak(i.min(u8::MAX as usize) as u8),
                at(w.offset),
                w.radius,
            ));
        }
        Hitbox { target, shapes }
    }
}

/// All enemy definitions by (lower-case) name.
#[derive(Clone, Debug, Default)]
pub struct EnemyRegistry {
    defs: BTreeMap<String, Arc<EnemyDef>>,
}

impl EnemyRegistry {
    /// The compiled-in definitions only.
    pub fn builtin() -> Self {
        let mut r = Self::default();
        for (name, text) in BUILTIN {
            match EnemyDef::parse(text) {
                Ok(def) => r.insert(name, def),
                Err(err) => log::error!("built-in enemy '{name}' is invalid: {err}"),
            }
        }
        r
    }

    /// Built-ins overridden/extended by every `*.ron` in `dir` (invalid files are logged and
    /// skipped).
    pub fn load_dir(dir: &Path) -> Self {
        let mut r = Self::builtin();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return r;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ron"))
            .collect();
        paths.sort();
        for path in paths {
            let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            match EnemyDef::load(&path) {
                Ok(def) => r.insert(&name, def),
                Err(err) => log::warn!("skipping enemy definition: {err}"),
            }
        }
        r
    }

    /// `assets/enemies/` (with built-in fallbacks).
    pub fn load_default() -> Self {
        Self::load_dir(&crate::assets_dir().join("enemies"))
    }

    pub fn insert(&mut self, name: &str, def: EnemyDef) {
        self.defs.insert(name.to_ascii_lowercase(), Arc::new(def));
    }

    pub fn get(&self, name: &str) -> Option<Arc<EnemyDef>> {
        self.defs.get(&name.to_ascii_lowercase()).cloned()
    }

    pub fn by_kind(&self, net_kind: u16) -> Option<Arc<EnemyDef>> {
        self.defs.values().find(|d| d.net_kind == net_kind).cloned()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.defs.keys().map(String::as_str)
    }

    /// Names of the types that may appear in encounters and waves: everything but training
    /// dummies (the Effigy).
    pub fn wave_names(&self) -> impl Iterator<Item = &str> {
        self.defs
            .iter()
            .filter(|(_, d)| d.dummy.is_none())
            .map(|(n, _)| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}
