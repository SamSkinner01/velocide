//! Data-driven enemy definitions: `assets/enemies/<name>.ron`.
//!
//! Every field has a default (`#[serde(default)]`), so a definition only spells out what makes
//! the enemy different. Local offsets (weak points, muzzles) are in the enemy's frame: +X right,
//! +Y up, −Z forward (the facing direction, like `input::look_dir` at yaw 0), from the feet.
//! See `docs/combat.md` → "Enemy definitions" for every field.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::sim::projectiles::ProjectileKind;

/// Built-in definitions (compiled in, so the server works without an assets directory and
/// tests don't depend on edited files). Files in `assets/enemies/` override them by name.
const BUILTIN: &[(&str, &str)] = &[
    ("husk", include_str!("../../../../assets/enemies/husk.ron")),
    ("spitter", include_str!("../../../../assets/enemies/spitter.ron")),
    ("gnat", include_str!("../../../../assets/enemies/gnat.ron")),
    ("bastion", include_str!("../../../../assets/enemies/bastion.ron")),
    // Roster contract stubs (docs/roster.md).
    ("gnawer", include_str!("../../../../assets/enemies/gnawer.ron")),
    ("censer", include_str!("../../../../assets/enemies/censer.ron")),
    ("harrier", include_str!("../../../../assets/enemies/harrier.ron")),
    ("gaoler", include_str!("../../../../assets/enemies/gaoler.ron")),
    ("heron", include_str!("../../../../assets/enemies/heron.ron")),
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
    /// Hits players within `range` inside the `arc` in front.
    #[default]
    Melee,
    /// Fires `projectile`.
    Projectile,
    /// Ground slam: hits every player within `shockwave_radius`.
    Slam,
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
}

impl Default for ProjectileAttack {
    fn default() -> Self {
        Self {
            kind: ProjectileKind::SpitterOrb,
            speed: 22.0,
            count: 1,
            interval: 0.15,
            spread: 0.0,
            muzzle: Vec3::new(0.0, 1.4, -0.4),
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
    /// Stun duration and counter damage when parried.
    pub parry_stun: f32,
    pub parry_damage: f32,
    /// Slam radius.
    pub shockwave_radius: f32,
    pub projectile: Option<ProjectileAttack>,
    /// Melee lunge: forward speed (m/s) the enemy leaps at when the strike starts; 0 = none.
    /// A lunging strike hits every player it reaches during the whole strike phase (once).
    pub lunge: f32,
    /// Only chosen against a target moving at least this fast (m/s, horizontal): gap closers
    /// for fleeing players. 0 = always.
    pub min_target_speed: f32,
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
            windup: 0.5,
            strike: 0.1,
            recover: 0.5,
            cooldown: 0.5,
            windup_move_scale: 0.15,
            track_target: true,
            parry_window: None,
            parry_stun: 2.5,
            parry_damage: 150.0,
            shockwave_radius: 5.0,
            projectile: None,
            lunge: 0.0,
            min_target_speed: 0.0,
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
        }
    }
}

/// What it leaves behind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Drops {
    /// Heal pool of the death burst.
    pub blood_pool: f32,
    pub blood_radius: f32,
}

impl Default for Drops {
    fn default() -> Self {
        Self {
            blood_pool: 25.0,
            blood_radius: 4.5,
        }
    }
}

/// Placeholder-art colours (sRGB 0..1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnemyColors {
    pub body: Vec3,
    pub accent: Vec3,
    /// Weak point glow.
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
}

impl Default for EnemyDef {
    fn default() -> Self {
        Self {
            name: "Enemy".into(),
            net_kind: crate::protocol::kind::ENEMY_FIRST,
            model: String::new(),
            health: 50.0,
            speed: 6.0,
            acceleration: 50.0,
            turn_rate: 540.0,
            radius: 0.45,
            height: 1.9,
            body_top: None,
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

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}
