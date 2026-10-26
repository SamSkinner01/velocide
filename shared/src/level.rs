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
    /// PvP deathmatch arena (also gun game and grind).
    Arena,
    /// The warm-up hub players roam while the server is in its lobby (never offered as a
    /// match level).
    Hub,
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
    /// Procedural texture name (e.g. "concrete", "brick", "metal", "grate", "wax", "lava").
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
    /// The locked room itself. When the doors lock, living players outside it are pulled to
    /// `rally` so co-op teammates can't be locked out.
    #[serde(default)]
    pub arena: Option<Volume>,
    /// Where players outside `arena` are teleported when the encounter locks (usually the
    /// checkpoint just inside the entry door).
    #[serde(default)]
    pub rally: Option<SpawnPoint>,
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
    /// Path relative to `assets/` (plain names only, `.gltf` / `.glb`, at most 16 MiB; the
    /// client refuses anything else: `client::level_mesh::resolve_prop_model`).
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
    /// A player whose *feet position* enters one of these dies (bottomless pits, lava).
    pub kill_volumes: Vec<Volume>,
    pub props: Vec<Prop>,
    /// Par time in seconds for results-screen grading (0 = no par time).
    pub par_time: f32,
    /// Grind mode: where waves of Moulds may appear (arenas). Empty = the mode picks points on
    /// the nav grid away from the players.
    pub grind_spawns: Vec<Vec3>,
}

// ---- limits checked by [`LevelDef::validate`] -------------------------------------------------
//
// Levels arrive from servers (`ServerMessage::LoadLevel`) and from files anyone can drop into
// `assets/levels/`; the shipped ones are far inside every limit.

/// Every coordinate (brush corners, spawns, volumes, lights, props) lies within ± this (m).
pub const MAX_LEVEL_COORD: f32 = 1.0e4;
pub const MAX_BRUSHES: usize = 16_384;
pub const MAX_LIGHTS: usize = 1024;
pub const MAX_SPAWNS: usize = 256;
pub const MAX_CHECKPOINTS: usize = 256;
pub const MAX_DOORS: usize = 1024;
pub const MAX_ENCOUNTERS: usize = 256;
/// Per encounter.
pub const MAX_WAVES: usize = 64;
/// Per wave.
pub const MAX_WAVE_SPAWNS: usize = 128;
/// Doors an encounter locks / unlocks (each list).
pub const MAX_ENCOUNTER_DOORS: usize = 64;
pub const MAX_JUMP_PADS: usize = 1024;
pub const MAX_KILL_VOLUMES: usize = 1024;
pub const MAX_PROPS: usize = 4096;
/// Names, materials, enemy names, model paths, music (bytes).
pub const MAX_LEVEL_STRING_BYTES: usize = 256;
/// Jump pad launch speed (m/s).
pub const MAX_JUMP_PAD_SPEED: f32 = 1000.0;
/// Seconds (wave delays, par time).
const MAX_LEVEL_TIME: f32 = 1.0e5;

fn check_count(what: &str, n: usize, max: usize) -> Result<(), String> {
    if n > max {
        Err(format!("too many {what}: {n} (at most {max})"))
    } else {
        Ok(())
    }
}

fn check_text(what: &str, s: &str) -> Result<(), String> {
    if s.len() > MAX_LEVEL_STRING_BYTES {
        Err(format!(
            "{what} is longer than {MAX_LEVEL_STRING_BYTES} bytes"
        ))
    } else {
        Ok(())
    }
}

/// Finite and within `±max` on every axis.
fn check_vec(what: &str, v: Vec3, max: f32) -> Result<(), String> {
    if v.is_finite() && v.abs().max_element() <= max {
        Ok(())
    } else {
        Err(format!("{what} {v} is not finite or outside ±{max}"))
    }
}

fn check_point(what: &str, p: Vec3) -> Result<(), String> {
    check_vec(what, p, MAX_LEVEL_COORD)
}

fn check_num(what: &str, v: f32, lo: f32, hi: f32) -> Result<(), String> {
    if v.is_finite() && (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(format!("{what} {v} is not in {lo}..={hi}"))
    }
}

fn check_volume(v: &Volume) -> Result<(), String> {
    check_point("min", v.min)?;
    check_point("max", v.max)
}

fn check_spawn(s: &SpawnPoint) -> Result<(), String> {
    check_point("position", s.position)?;
    check_num("yaw", s.yaw, -MAX_LEVEL_COORD, MAX_LEVEL_COORD)
}

fn check_brush(b: &Brush) -> Result<(), String> {
    check_point("min", b.min)?;
    check_point("max", b.max)?;
    check_vec("rotation", b.rotation, MAX_LEVEL_COORD)?;
    check_vec("tint", b.tint, MAX_LEVEL_COORD)?;
    check_text("material", &b.material)
}

/// Runs `check` on every item, prefixing errors with `what` and the item's index.
fn check_each<T>(
    what: &str,
    items: &[T],
    max: usize,
    check: impl Fn(&T) -> Result<(), String>,
) -> Result<(), String> {
    check_count(what, items.len(), max)?;
    items
        .iter()
        .enumerate()
        .try_for_each(|(i, item)| check(item).map_err(|e| format!("{what} {i}: {e}")))
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

    /// Reads, parses and [validates](LevelDef::validate) a level file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let level: LevelDef =
            ron::from_str(&text).map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        level
            .validate()
            .map_err(|e| anyhow::anyhow!("invalid level {}: {e}", path.display()))?;
        Ok(level)
    }

    /// Checks that the level is safe to build and simulate: every number finite, every
    /// coordinate within ±[`MAX_LEVEL_COORD`] m, at most [`MAX_BRUSHES`] brushes (and the
    /// other `MAX_*` counts), strings at most [`MAX_LEVEL_STRING_BYTES`] long. The server
    /// checks the level files it loads; clients check every level a server sends them (a
    /// brush with 100 km extents or a million brushes would stall or exhaust memory).
    pub fn validate(&self) -> Result<(), String> {
        check_text("name", &self.name)?;
        let env = &self.environment;
        for (what, v) in [
            ("sky_color", env.sky_color),
            ("fog_color", env.fog_color),
            ("ambient", env.ambient),
            ("light_dir", env.light_dir),
            ("light_color", env.light_color),
        ] {
            check_vec(what, v, MAX_LEVEL_COORD)?;
        }
        check_num(
            "fog_start",
            env.fog_start,
            -MAX_LEVEL_COORD,
            MAX_LEVEL_COORD,
        )?;
        check_num("fog_end", env.fog_end, -MAX_LEVEL_COORD, MAX_LEVEL_COORD)?;
        check_text("music", &env.music)?;
        check_each("brush", &self.brushes, MAX_BRUSHES, check_brush)?;
        check_each("light", &self.lights, MAX_LIGHTS, |l| {
            check_point("position", l.position)?;
            check_vec("color", l.color, MAX_LEVEL_COORD)?;
            check_num("radius", l.radius, 0.0, MAX_LEVEL_COORD)
        })?;
        check_each("spawn", &self.spawns, MAX_SPAWNS, check_spawn)?;
        check_each("checkpoint", &self.checkpoints, MAX_CHECKPOINTS, |c| {
            check_volume(&c.trigger)?;
            check_spawn(&c.spawn)
        })?;
        check_each("door", &self.doors, MAX_DOORS, |d| check_brush(&d.brush))?;
        check_each("encounter", &self.encounters, MAX_ENCOUNTERS, |e| {
            check_volume(&e.trigger)?;
            e.arena.as_ref().map_or(Ok(()), check_volume)?;
            e.rally.as_ref().map_or(Ok(()), check_spawn)?;
            check_count("lock_doors", e.lock_doors.len(), MAX_ENCOUNTER_DOORS)?;
            check_count("unlock_doors", e.unlock_doors.len(), MAX_ENCOUNTER_DOORS)?;
            check_each("wave", &e.waves, MAX_WAVES, |w| {
                check_num("delay", w.delay, 0.0, MAX_LEVEL_TIME)?;
                check_each("spawn", &w.spawns, MAX_WAVE_SPAWNS, |s| {
                    check_text("enemy", &s.enemy)?;
                    check_point("position", s.position)?;
                    check_num("yaw", s.yaw, -MAX_LEVEL_COORD, MAX_LEVEL_COORD)
                })
            })
        })?;
        self.exit
            .as_ref()
            .map_or(Ok(()), check_volume)
            .map_err(|e| format!("exit: {e}"))?;
        check_each("jump pad", &self.jump_pads, MAX_JUMP_PADS, |j| {
            check_volume(&j.trigger)?;
            check_vec("velocity", j.velocity, MAX_JUMP_PAD_SPEED)
        })?;
        check_each(
            "kill volume",
            &self.kill_volumes,
            MAX_KILL_VOLUMES,
            check_volume,
        )?;
        check_each("prop", &self.props, MAX_PROPS, |p| {
            check_text("model", &p.model)?;
            check_point("position", p.position)?;
            check_vec("rotation", p.rotation, MAX_LEVEL_COORD)?;
            check_num("scale", p.scale, -1000.0, 1000.0)
        })?;
        check_num("par_time", self.par_time, 0.0, MAX_LEVEL_TIME)
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
        let names = LevelDef::list();
        assert!(!names.is_empty());
        for name in names {
            // `load` validates too.
            let level = LevelDef::load_named(&name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(level.validate(), Ok(()), "{name}");
        }
        let builtin = crate::sim::builtin_level(crate::sim::BUILTIN_LEVEL).unwrap();
        assert_eq!(builtin.validate(), Ok(()));
    }

    #[test]
    fn hostile_levels_are_rejected() {
        let base = crate::sim::builtin_level(crate::sim::BUILTIN_LEVEL).unwrap();
        let bad = |f: &dyn Fn(&mut LevelDef)| {
            let mut level = base.clone();
            f(&mut level);
            level.validate().expect_err("must be rejected")
        };
        // A brush with 100 km extents (used to put a collider into ~150 million grid cells).
        let e = bad(&|l| l.brushes[0].max = Vec3::splat(1.0e5));
        assert!(e.starts_with("brush 0: max"), "{e}");
        bad(&|l| l.brushes[3].rotation.y = f32::NAN);
        bad(&|l| l.brushes[3].min.x = f32::NEG_INFINITY);
        bad(&|l| l.spawns[0].position.y = f32::NAN);
        bad(&|l| l.doors[0].brush.max.z = 2.0e4);
        bad(&|l| l.kill_volumes[0].min.y = -1.0e9);
        bad(&|l| l.jump_pads[0].velocity.y = 1.0e6);
        bad(&|l| {
            l.lights.push(LightDef {
                position: Vec3::ZERO,
                color: Vec3::ONE,
                radius: f32::NAN,
            })
        });
        bad(&|l| l.environment.fog_end = f32::INFINITY);
        bad(&|l| l.par_time = -1.0);
        bad(&|l| l.name = "x".repeat(MAX_LEVEL_STRING_BYTES + 1));
        bad(&|l| l.brushes[1].material = "m".repeat(10_000));
        let e = bad(&|l| {
            let b = l.brushes[5].clone();
            l.brushes.resize(MAX_BRUSHES + 1, b);
        });
        assert!(e.contains("too many brush"), "{e}");
        bad(&|l| {
            l.encounters.push(Encounter {
                trigger: Volume {
                    min: Vec3::ZERO,
                    max: Vec3::ONE,
                },
                arena: None,
                rally: None,
                lock_doors: vec![],
                unlock_doors: vec![],
                waves: vec![Wave {
                    delay: 0.0,
                    spawns: vec![
                        EnemySpawn {
                            enemy: "gnawer".into(),
                            position: Vec3::ZERO,
                            yaw: 0.0
                        };
                        MAX_WAVE_SPAWNS + 1
                    ],
                }],
            })
        });
        bad(&|l| {
            l.props.push(Prop {
                model: "models/x.glb".into(),
                position: Vec3::ZERO,
                rotation: Vec3::ZERO,
                scale: f32::NAN,
            })
        });
        // At the limits is fine.
        let mut ok = base.clone();
        ok.brushes[0].min = Vec3::splat(-MAX_LEVEL_COORD);
        ok.brushes[0].max = Vec3::splat(MAX_LEVEL_COORD);
        assert_eq!(ok.validate(), Ok(()));
        ok.build_collision();
    }
}
