//! Brush-based level format (RON files in `assets/levels/`).
//!
//! A level is a list of boxes ("brushes") plus gameplay markup: spawn points, checkpoints,
//! encounter triggers with enemy waves, doors that lock arenas, jump pads and kill volumes.
//! Both the server (collision, AI) and the client (collision for prediction, meshes for
//! rendering) build their data from the same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine::physics::{ColliderId, CollisionWorld, GROUP_DOOR, GROUP_PLAYER_CLIP, GROUP_WORLD};
use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelKind {
    /// Single-player / co-op campaign level with encounters and an exit.
    #[default]
    Campaign,
    /// PvP deathmatch arena.
    Arena,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub sky_color: Vec3,
    pub fog_color: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    pub ambient: Vec3,
    /// Direction the key light travels (from light towards scene).
    pub light_dir: Vec3,
    pub light_color: Vec3,
    /// Music track name.
    pub music: String,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_color: Vec3::new(0.18, 0.05, 0.05),
            fog_color: Vec3::new(0.25, 0.08, 0.06),
            fog_start: 25.0,
            fog_end: 140.0,
            ambient: Vec3::new(0.35, 0.3, 0.32),
            light_dir: Vec3::new(-0.4, -1.0, -0.3),
            light_color: Vec3::new(0.9, 0.75, 0.6),
            music: "campaign".into(),
        }
    }
}

/// A solid box. `min`/`max` are the box corners before rotation; `rotation` (degrees, applied
/// as yaw around Y, then pitch around X, then roll around Z) rotates it around its centre,
/// which makes ramps easy to author.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub min: Vec3,
    pub max: Vec3,
    #[serde(default)]
    pub rotation: Vec3,
    /// Procedural texture name (e.g. "concrete", "brick", "metal", "grate", "flesh", "lava").
    #[serde(default = "default_material")]
    pub material: String,
    /// Colour multiplied with the texture.
    #[serde(default = "default_tint")]
    pub tint: Vec3,
    /// Invisible wall that only blocks players/enemies (not bullets).
    #[serde(default)]
    pub clip_only: bool,
    /// Visual only, no collision (decoration).
    #[serde(default)]
    pub no_collide: bool,
}

fn default_material() -> String {
    "concrete".into()
}

fn default_tint() -> Vec3 {
    Vec3::ONE
}

impl Brush {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min).abs() * 0.5
    }

    pub fn rotation_quat(&self) -> Quat {
        Quat::from_euler(
            EulerRot::YXZ,
            self.rotation.y.to_radians(),
            self.rotation.x.to_radians(),
            self.rotation.z.to_radians(),
        )
    }
}

/// Axis-aligned trigger volume.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Volume {
    pub min: Vec3,
    pub max: Vec3,
}

impl Volume {
    pub fn contains(&self, p: Vec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnPoint {
    /// Feet position.
    pub position: Vec3,
    /// Degrees, 0 = facing -Z.
    #[serde(default)]
    pub yaw: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightDef {
    pub position: Vec3,
    pub color: Vec3,
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub trigger: Volume,
    pub spawn: SpawnPoint,
}

/// A door brush that can be closed (solid) or open (no collision, not drawn).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Door {
    pub id: u32,
    pub brush: Brush,
    #[serde(default = "default_true")]
    pub starts_open: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnemySpawn {
    /// Enemy definition name (file stem in `assets/enemies/`).
    pub enemy: String,
    pub position: Vec3,
    #[serde(default)]
    pub yaw: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wave {
    /// Seconds after the previous wave is cleared (or the encounter starts).
    #[serde(default)]
    pub delay: f32,
    pub spawns: Vec<EnemySpawn>,
}

/// A locked-room arena fight: entering `trigger` closes `lock_doors`, spawns `waves` one after
/// another, and when the last wave dies opens `unlock_doors`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Encounter {
    pub trigger: Volume,
    #[serde(default)]
    pub lock_doors: Vec<u32>,
    #[serde(default)]
    pub unlock_doors: Vec<u32>,
    pub waves: Vec<Wave>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JumpPad {
    pub trigger: Volume,
    pub velocity: Vec3,
}

/// A glTF model placed in the level (decoration; collision optional).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prop {
    /// Path relative to `assets/`.
    pub model: String,
    pub position: Vec3,
    #[serde(default)]
    pub rotation: Vec3,
    #[serde(default = "default_scale")]
    pub scale: f32,
}

fn default_scale() -> f32 {
    1.0
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LevelDef {
    pub name: String,
    pub kind: LevelKind,
    pub environment: Environment,
    pub brushes: Vec<Brush>,
    pub lights: Vec<LightDef>,
    /// Player spawns (campaign start / deathmatch spawn points).
    pub spawns: Vec<SpawnPoint>,
    pub checkpoints: Vec<Checkpoint>,
    pub doors: Vec<Door>,
    pub encounters: Vec<Encounter>,
    /// Reaching this volume completes a campaign level.
    pub exit: Option<Volume>,
    pub jump_pads: Vec<JumpPad>,
    /// Falling into one of these kills the player (bottomless pits, lava).
    pub kill_volumes: Vec<Volume>,
    pub props: Vec<Prop>,
}

/// Collision world for a level plus the collider ids of its doors.
pub struct LevelCollision {
    pub world: CollisionWorld,
    pub doors: BTreeMap<u32, ColliderId>,
}

impl LevelCollision {
    /// Opens (disables) or closes (enables) a door's collider.
    pub fn set_door_open(&mut self, door: u32, open: bool) {
        if let Some(&id) = self.doors.get(&door) {
            self.world.set_enabled(id, !open);
        }
    }

    pub fn is_door_open(&self, door: u32) -> bool {
        self.doors
            .get(&door)
            .is_none_or(|&id| !self.world.is_enabled(id))
    }
}

impl LevelDef {
    pub fn dir() -> PathBuf {
        crate::assets_dir().join("levels")
    }

    /// Path of a level by name (file stem).
    pub fn path_for(name: &str) -> PathBuf {
        Self::dir().join(format!("{name}.ron"))
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))
    }

    pub fn load_named(name: &str) -> anyhow::Result<Self> {
        Self::load(&Self::path_for(name))
    }

    /// Names (file stems) of all levels in `assets/levels`, sorted.
    pub fn list() -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(Self::dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                (p.extension()? == "ron").then(|| p.file_stem()?.to_str().map(str::to_owned))?
            })
            .collect();
        names.sort();
        names
    }

    /// Builds the collision world. Brush colliders get `tag` = brush index; door colliders get
    /// `tag` = door id. Doors start open or closed according to `starts_open`.
    pub fn build_collision(&self) -> LevelCollision {
        let mut world = CollisionWorld::new();
        for (i, brush) in self.brushes.iter().enumerate() {
            if brush.no_collide {
                continue;
            }
            let group = if brush.clip_only {
                GROUP_PLAYER_CLIP
            } else {
                GROUP_WORLD
            };
            world.add_box(
                brush.center(),
                brush.half_extents(),
                brush.rotation_quat(),
                group,
                i as u32,
            );
        }
        let mut doors = BTreeMap::new();
        for door in &self.doors {
            let id = world.add_box(
                door.brush.center(),
                door.brush.half_extents(),
                door.brush.rotation_quat(),
                GROUP_DOOR,
                door.id,
            );
            world.set_enabled(id, !door.starts_open);
            doors.insert(door.id, id);
        }
        LevelCollision { world, doors }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_level() {
        let text = r#"(
            name: "Test",
            brushes: [
                (min: (-10.0, -1.0, -10.0), max: (10.0, 0.0, 10.0)),
                (min: (2.0, 0.0, -1.0), max: (4.0, 2.0, 1.0), rotation: (0.0, 45.0, 0.0), material: "metal"),
            ],
            spawns: [(position: (0.0, 0.0, 0.0))],
            doors: [(id: 1, brush: (min: (5.0, 0.0, -1.0), max: (5.5, 3.0, 1.0)), starts_open: false)],
        )"#;
        let level: LevelDef = ron::from_str(text).unwrap();
        assert_eq!(level.brushes.len(), 2);
        let mut col = level.build_collision();
        assert_eq!(col.world.len(), 3);
        assert!(!col.is_door_open(1));
        col.set_door_open(1, true);
        assert!(col.is_door_open(1));
    }

    #[test]
    fn all_shipped_levels_parse() {
        for name in LevelDef::list() {
            LevelDef::load_named(&name).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
