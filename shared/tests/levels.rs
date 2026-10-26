//! Checks for the shipped levels in `assets/levels/`:
//!
//! - every level parses and its markup is consistent (materials, doors, enemy names);
//! - campaign / arena structure (spawns, checkpoints, encounters, exit, enemy mix and how the
//!   roster is introduced) — every `Campaign` level in [`CAMPAIGNS`] gets the same checks, and
//!   every `Arena` level in `assets/levels/` gets the arena checks;
//! - player, checkpoint and enemy spawns stand on ground and don't overlap geometry (with the
//!   roster's sizes), harriers hover in open air, censers sit on deep ledges, gaolers have
//!   room to be circled, herons have long sightlines;
//! - campaign Moulds can walk to their rally point on the nav grid (with their own footprint);
//! - every arena's grind spawns: room for any Mould, on the nav grid, out of sight of and away
//!   from the player spawns, spread round the arena; the newer arenas' PvP spawns can't see
//!   each other;
//! - jump pads land somewhere solid;
//! - no z-fighting (coplanar overlapping faces of different materials);
//! - sealing: a flood fill through free space with every door closed never leaves the level
//!   and never connects two arenas (locked rooms really lock);
//! - chaotic players never leave the level;
//! - traversability: waypoint-following bots driven through the real movement code
//!   (`shared::movement::step`) walk the whole critical path of every campaign level from
//!   every spawn and every checkpoint to the exit with doors open, using only running,
//!   jumping, sliding and pads; plus each level's alternate routes (which find its secrets)
//!   and scripted tech; every arena has a full lap through its ramps, pads, tunnels and
//!   catwalks, and scripted checks for its movement-tech spots (wall-jump shafts, slides).
//!
//! `cargo test -p shared --test levels -- --nocapture` prints route times (par suggestions).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use engine::physics::{CollisionWorld, MASK_MOVEMENT, MASK_SHOTS};
use glam::{IVec3, Vec2, Vec3};
use shared::input::{Buttons, PlayerInput};
use shared::level::{Brush, LevelCollision, LevelDef, LevelKind, Volume};
use shared::movement::{self, MoveEnv, MoveEvents, MoveState};
use shared::sim::EnemyRegistry;
use shared::sim::enemy::Locomotion;
use shared::sim::nav::{NavClass, NavGrid, NavScratch, PathResult};
use shared::tuning::MovementTuning;

const DT: f32 = 1.0 / 60.0;
const CRUCIBLE: &str = "crucible_of_veins";
const SPIRE: &str = "spire_of_wicks";
const DROWNED: &str = "drowned_chandlery";
/// Every shipped `Campaign` level: each gets the structure, spawn, sealing and bot checks
/// (its routes live in [`campaign_route`] / [`alternate_routes`]).
const CAMPAIGNS: [&str; 3] = [CRUCIBLE, SPIRE, DROWNED];
const ARENA: &str = "cistern_nine";
const KNELL: &str = "knell_yard";
const INGOT: &str = "ingot_run";
const FURNACE: &str = "furnace_ring";
const TALLOW: &str = "tallow_works";
const WICKMARKET: &str = "wickmarket";
/// Levels that must exist. Every level in `assets/levels/` except the playgrounds is held to
/// the shipping checks (see [`shipped`]).
const SHIPPED: [&str; 8] = [
    CRUCIBLE, SPIRE, DROWNED, ARENA, KNELL, INGOT, FURNACE, TALLOW,
];
/// Test levels that aren't held to the shipping standards.
const PLAYGROUNDS: [&str; 1] = ["movement_test"];
/// The roster (docs/roster.md); a campaign finale uses all of it.
const ENEMIES: [&str; 5] = ["gnawer", "censer", "harrier", "gaoler", "heron"];
const MATERIALS: [&str; 14] = [
    "concrete", "brick", "metal", "rust", "grate", "wax", "waxstone", "lava", "tile", "wood",
    "hazard", "glass", "dirt", "panel",
];
const MAX_BRUSHES: usize = 600;

// ------------------------------------------------------------------------------------------
// Helpers

fn levels_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/levels")
}

fn load(name: &str) -> LevelDef {
    LevelDef::load(&levels_dir().join(format!("{name}.ron")))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn all_level_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(levels_dir())
        .expect("assets/levels exists")
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "ron").then(|| p.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort();
    names
}

/// Every shipped level: everything in `assets/levels/` except the playgrounds.
fn shipped() -> Vec<String> {
    all_level_names()
        .into_iter()
        .filter(|n| !PLAYGROUNDS.contains(&n.as_str()))
        .collect()
}

/// Every shipped deathmatch arena.
fn arenas() -> Vec<(String, LevelDef)> {
    shipped()
        .into_iter()
        .map(|n| {
            let level = load(&n);
            (n, level)
        })
        .filter(|(_, l)| l.kind == LevelKind::Arena)
        .collect()
}

fn collision(level: &LevelDef, doors_open: Option<bool>) -> LevelCollision {
    let mut col = level.build_collision();
    if let Some(open) = doors_open {
        for d in &level.doors {
            col.set_door_open(d.id, open);
        }
    }
    col
}

/// World-space AABB of a (possibly rotated) brush.
fn brush_aabb(b: &Brush) -> (Vec3, Vec3) {
    let (c, h, q) = (b.center(), b.half_extents(), b.rotation_quat());
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for i in 0..8 {
        let s = Vec3::new(
            if i & 1 == 0 { -1.0 } else { 1.0 },
            if i & 2 == 0 { -1.0 } else { 1.0 },
            if i & 4 == 0 { -1.0 } else { 1.0 },
        );
        let p = c + q * (h * s);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

fn level_bounds(level: &LevelDef) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for b in level
        .brushes
        .iter()
        .chain(level.doors.iter().map(|d| &d.brush))
    {
        let (l, h) = brush_aabb(b);
        lo = lo.min(l);
        hi = hi.max(h);
    }
    (lo, hi)
}

fn in_kill_volume(level: &LevelDef, p: Vec3) -> bool {
    level.kill_volumes.iter().any(|k| k.contains(p))
}

/// Distance from `p` down to walkable ground (`None` if nothing within `max`).
fn ground_below(world: &CollisionWorld, p: Vec3, max: f32) -> Option<f32> {
    let from = p + Vec3::Y * 0.5;
    world
        .cast_ray(from, Vec3::NEG_Y, max + 0.5, MASK_MOVEMENT)
        .filter(|h| h.normal.y > 0.7)
        .map(|h| h.distance - 0.5)
}

/// Upright capsule with its feet at `feet`: (centre, inner half height).
fn capsule_at(feet: Vec3, height: f32, radius: f32) -> (Vec3, f32) {
    let half = (height * 0.5 - radius).max(0.0);
    (feet + Vec3::Y * height * 0.5, half)
}

/// Rough collision sizes per enemy (radius, height); the real numbers live in the enemy
/// definitions, these are generous so spawns keep working if they grow a little.
fn enemy_size(name: &str) -> (f32, f32) {
    match name {
        "gaoler" => (1.45, 4.0),
        "heron" => (0.6, 3.6),
        "censer" => (0.7, 2.7),
        "harrier" => (1.0, 1.6),
        _ => (0.7, 2.2),
    }
}

// ------------------------------------------------------------------------------------------
// Markup / structure

#[test]
fn every_level_parses_with_consistent_markup() {
    let names = all_level_names();
    for s in SHIPPED {
        assert!(names.iter().any(|n| n == s), "{s}.ron is missing");
    }
    for name in names {
        let level = load(&name);
        // (Only our shipped levels are held to "every brush has volume"; the movement test
        // level has a deliberate zero-height bottom stair.)
        let strict = !PLAYGROUNDS.contains(&name.as_str());
        for (i, b) in level.brushes.iter().enumerate() {
            assert!(
                !strict || (b.max - b.min).min_element() > 0.0,
                "{name}: brush {i} has no volume ({} .. {})",
                b.min,
                b.max
            );
            assert!(
                MATERIALS.contains(&b.material.as_str()),
                "{name}: brush {i} uses unknown material {:?}",
                b.material
            );
        }
        let mut ids = HashSet::new();
        for d in &level.doors {
            assert!(ids.insert(d.id), "{name}: duplicate door id {}", d.id);
            assert!(
                MATERIALS.contains(&d.brush.material.as_str()),
                "{name}: door {} material",
                d.id
            );
        }
        for (i, e) in level.encounters.iter().enumerate() {
            for id in e.lock_doors.iter().chain(&e.unlock_doors) {
                assert!(
                    ids.contains(id),
                    "{name}: encounter {i} references missing door {id}"
                );
            }
            assert!(!e.waves.is_empty(), "{name}: encounter {i} has no waves");
            for w in &e.waves {
                assert!(
                    !w.spawns.is_empty(),
                    "{name}: encounter {i} has an empty wave"
                );
                for s in &w.spawns {
                    assert!(
                        ENEMIES.contains(&s.enemy.as_str()),
                        "{name}: unknown enemy {:?}",
                        s.enemy
                    );
                }
            }
        }
        for v in level.kill_volumes.iter().chain(level.exit.iter()) {
            assert!(
                (v.max - v.min).min_element() > 0.0,
                "{name}: empty volume {v:?}"
            );
        }
    }
}

#[test]
fn shipped_levels_fit_the_budget_and_have_a_safety_net() {
    for name in shipped() {
        let level = load(&name);
        assert!(
            level.brushes.len() < MAX_BRUSHES,
            "{name}: {} brushes (budget {MAX_BRUSHES})",
            level.brushes.len()
        );
        let (lo, hi) = level_bounds(&level);
        // A kill volume under the whole level.
        assert!(
            level.kill_volumes.iter().any(|k| {
                k.min.x <= lo.x
                    && k.min.z <= lo.z
                    && k.max.x >= hi.x
                    && k.max.z >= hi.z
                    && k.max.y <= lo.y
            }),
            "{name}: no kill volume below the whole level ({lo} .. {hi})"
        );
        assert!(!level.lights.is_empty(), "{name}: no lights");
        assert!(
            !level.environment.music.is_empty(),
            "{name}: no music track"
        );
    }
}

fn kinds(e: &shared::level::Encounter) -> HashSet<String> {
    e.waves
        .iter()
        .flat_map(|w| &w.spawns)
        .map(|s| s.enemy.clone())
        .collect()
}

/// Index of the first encounter that spawns `enemy`.
fn first_encounter_with(level: &LevelDef, enemy: &str) -> Option<usize> {
    level
        .encounters
        .iter()
        .position(|e| kinds(e).contains(enemy))
}

#[test]
fn campaign_structure() {
    for name in CAMPAIGNS {
        let level = load(name);
        assert_eq!(level.kind, LevelKind::Campaign, "{name}");
        assert_eq!(level.spawns.len(), 4, "{name}: 4 co-op player spawns");
        assert!(level.exit.is_some(), "{name}: campaign needs an exit");
        let n = level.encounters.len();
        assert!((4..=5).contains(&n), "{name}: 4-5 arenas, got {n}");
        assert_eq!(
            level.checkpoints.len(),
            n,
            "{name}: one checkpoint per encounter"
        );

        let door = |id: u32| {
            level
                .doors
                .iter()
                .find(|d| d.id == id)
                .expect("door exists")
        };
        let mut used = HashSet::new();
        for (i, (e, cp)) in level.encounters.iter().zip(&level.checkpoints).enumerate() {
            assert!(
                !e.lock_doors.is_empty() && !e.unlock_doors.is_empty(),
                "{name}: encounter {i} locks and unlocks"
            );
            for &id in &e.lock_doors {
                assert!(
                    door(id).starts_open,
                    "{name}: encounter {i}: lock door {id} must start open"
                );
                used.insert(id);
            }
            for &id in &e.unlock_doors {
                assert!(
                    !door(id).starts_open,
                    "{name}: encounter {i}: exit door {id} must start closed"
                );
                used.insert(id);
            }
            // The checkpoint is touched before the fight starts: its spawn is outside the
            // encounter trigger, and its trigger is next to it.
            assert!(
                !e.trigger.contains(cp.spawn.position),
                "{name}: checkpoint {i} spawn inside encounter trigger"
            );
            assert!(
                cp.trigger.contains(cp.spawn.position + Vec3::Y * 0.1),
                "{name}: checkpoint {i} spawn outside its trigger"
            );
            assert!(
                cp.trigger.center().distance(e.trigger.center()) < 60.0,
                "{name}: checkpoint {i} is far from encounter {i}"
            );
        }
        for d in &level.doors {
            assert!(
                used.contains(&d.id),
                "{name}: door {} is not used by any encounter",
                d.id
            );
        }

        // Introduction curve: fodder first; harriers first in a tall room; the first gaoler
        // before the finale, in a room big enough to circle it; every type introduced before
        // the finale, which has all of them over 3+ waves. (Heron sightlines are checked
        // with the spawns.)
        let opener: HashSet<&str> = level.encounters[0].waves[0]
            .spawns
            .iter()
            .map(|s| s.enemy.as_str())
            .collect();
        assert_eq!(
            opener,
            HashSet::from(["gnawer"]),
            "{name}: the first wave is gnawers only"
        );
        let arena = |i: usize| {
            level.encounters[i]
                .arena
                .unwrap_or(level.encounters[i].trigger)
        };
        let harriers = first_encounter_with(&level, "harrier").expect("harriers appear");
        let a = arena(harriers);
        assert!(
            a.max.y - a.min.y >= 25.0,
            "{name}: harriers are introduced in a tall room"
        );
        let gaoler = first_encounter_with(&level, "gaoler").expect("a gaoler appears");
        let a = arena(gaoler);
        assert!(
            a.max.x - a.min.x >= 30.0 && a.max.z - a.min.z >= 30.0,
            "{name}: the first gaoler gets a room of 30 x 30 m or more"
        );
        for k in ENEMIES {
            let first = first_encounter_with(&level, k).unwrap_or_else(|| panic!("{name}: no {k}"));
            assert!(first < n - 1, "{name}: {k} is introduced before the finale");
        }
        let last = level.encounters.last().expect("encounters");
        assert!(last.waves.len() >= 3, "{name}: finale has 3+ waves");
        for k in ENEMIES {
            assert!(kinds(last).contains(k), "{name}: finale includes {k}");
        }
        let total: usize = level
            .encounters
            .iter()
            .flat_map(|e| &e.waves)
            .map(|w| w.spawns.len())
            .sum();
        println!("{name}: {n} encounters, {total} enemies (single player)");
    }

    // Crucible of Veins is the first campaign level: its whole first arena is gnawers and its
    // second introduces censers.
    let level = load(CRUCIBLE);
    let first = kinds(&level.encounters[0]);
    assert!(
        first.len() == 1 && first.contains("gnawer"),
        "arena 1 is gnawers only: {first:?}"
    );
    assert!(
        kinds(&level.encounters[1]).contains("censer"),
        "arena 2 introduces censers"
    );
}

#[test]
fn arena_structure() {
    let arenas = arenas();
    for a in [ARENA, KNELL, INGOT] {
        assert!(arenas.iter().any(|(n, _)| n == a), "{a} should be an arena");
    }
    for (name, level) in &arenas {
        assert!(level.spawns.len() >= 8, "{name}: 8+ deathmatch spawns");
        assert!(
            level.encounters.is_empty() && level.exit.is_none() && level.checkpoints.is_empty(),
            "{name}: no encounters, exit or checkpoints"
        );
        assert!(level.doors.is_empty(), "{name}: no doors in an arena");
        assert!(level.jump_pads.len() >= 2, "{name}: jump pads");
        assert_eq!(
            level.par_time, 0.0,
            "{name}: deathmatch arenas have no par time"
        );
        assert_eq!(level.environment.music, "arena", "{name}: arena music");
        let (lo, hi) = level_bounds(level);
        // A hazard inside the arena (not just the safety net under the world).
        assert!(
            level.kill_volumes.iter().any(|k| k.max.y > lo.y),
            "{name}: no kill hazard"
        );
        // An invisible clip ceiling over the whole arena, well above every spawn.
        let top_spawn = level
            .spawns
            .iter()
            .map(|s| s.position.y)
            .fold(f32::MIN, f32::max);
        assert!(
            level.brushes.iter().any(|b| {
                b.clip_only
                    && b.min.x <= lo.x
                    && b.min.z <= lo.z
                    && b.max.x >= hi.x
                    && b.max.z >= hi.z
                    && b.min.y >= top_spawn + 15.0
            }),
            "{name}: no clip ceiling over the arena"
        );
        for (i, a) in level.spawns.iter().enumerate() {
            for b in &level.spawns[i + 1..] {
                let d = a.position.distance(b.position);
                assert!(
                    d >= 10.0,
                    "{name}: spawns {} and {} only {d:.1} m apart",
                    a.position,
                    b.position
                );
            }
        }
        // Spawns spread over the arena: every quadrant (around the middle of the level) has
        // one, there are 3+ heights, and each one faces into the arena.
        let mid = (lo + hi) * 0.5;
        for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            assert!(
                level.spawns.iter().any(|s| (s.position.x - mid.x) * sx > 0.0 && (s.position.z - mid.z) * sz > 0.0),
                "{name}: no spawn in quadrant ({sx}, {sz})"
            );
        }
        let heights: HashSet<i32> = level
            .spawns
            .iter()
            .map(|s| s.position.y.round() as i32)
            .collect();
        assert!(
            heights.len() >= 3,
            "{name}: spawns on 3+ levels: {heights:?}"
        );
        for s in &level.spawns {
            let look = shared::input::look_dir(s.yaw.to_radians(), 0.0);
            let inward =
                Vec3::new(mid.x - s.position.x, 0.0, mid.z - s.position.z).normalize_or_zero();
            assert!(
                look.dot(inward) > 0.5,
                "{name}: spawn at {} (yaw {}) faces away from the arena",
                s.position,
                s.yaw
            );
        }
    }
}

// ------------------------------------------------------------------------------------------
// Spawns

fn check_player_spot(level: &LevelDef, world: &CollisionWorld, p: Vec3, what: &str) {
    let t = MovementTuning::default();
    let ground = ground_below(world, p, 0.6);
    assert!(
        ground.is_some_and(|d| d.abs() < 0.3),
        "{}: {what} at {p} has no ground right below (got {ground:?})",
        level.name
    );
    let (c, hh) = capsule_at(p + Vec3::Y * 0.02, t.standing_height, t.radius);
    assert!(
        !world.intersects_capsule(c, hh, t.radius, MASK_MOVEMENT),
        "{}: {what} at {p} overlaps geometry",
        level.name
    );
    // Head room for a jump.
    let (c, hh) = capsule_at(p + Vec3::Y * 0.6, t.standing_height, t.radius);
    assert!(
        !world.intersects_capsule(c, hh, t.radius - 0.05, MASK_MOVEMENT),
        "{}: {what} at {p} has no head room",
        level.name
    );
    assert!(
        !in_kill_volume(level, p + Vec3::Y * 0.1),
        "{}: {what} at {p} is in a kill volume",
        level.name
    );
    // It settles where it is.
    let env = MoveEnv {
        world,
        jump_pads: &[],
    };
    let mut s = MoveState::spawn(p);
    for seq in 1..=30 {
        let input = PlayerInput {
            seq,
            ..Default::default()
        };
        movement::step(&mut s, &input, &env, &t, DT);
    }
    assert!(
        s.on_ground,
        "{}: {what} at {p} doesn't settle on the ground",
        level.name
    );
    assert!(
        s.position.distance(p) < 0.15,
        "{}: {what} at {p} drifts to {}",
        level.name,
        s.position
    );
}

#[test]
fn player_and_checkpoint_spawns_are_valid() {
    for name in shipped() {
        let level = load(&name);
        // Closed doors must not overlap spawn points either.
        for doors in [None, Some(false)] {
            let col = collision(&level, doors);
            for (i, s) in level.spawns.iter().enumerate() {
                check_player_spot(&level, &col.world, s.position, &format!("spawn {i}"));
            }
            for (i, c) in level.checkpoints.iter().enumerate() {
                check_player_spot(
                    &level,
                    &col.world,
                    c.spawn.position,
                    &format!("checkpoint {} spawn", i + 1),
                );
            }
        }
        for (i, a) in level.spawns.iter().enumerate() {
            for b in &level.spawns[i + 1..] {
                assert!(
                    a.position.distance(b.position) >= 1.5,
                    "{name}: overlapping spawns"
                );
            }
        }
    }
}

/// Whether there is floor at `p`'s height (within `dy`) `dist` metres away in each of the
/// eight compass directions.
fn floor_all_around(world: &CollisionWorld, p: Vec3, dist: f32, dy: f32) -> Result<(), Vec3> {
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let q = p + Vec3::new(a.cos(), 0.0, a.sin()) * dist;
        if !ground_below(world, q, dy + 0.3).is_some_and(|g| g.abs() <= dy) {
            return Err(q);
        }
    }
    Ok(())
}

#[test]
fn enemy_spawns_are_valid() {
    for name in CAMPAIGNS {
        let level = load(name);
        let col = collision(&level, Some(false));
        let world = &col.world;
        for (ei, e) in level.encounters.iter().enumerate() {
            let cp = level.checkpoints[ei].spawn.position;
            let rally = e.rally.map_or(cp, |r| r.position);
            for (wi, w) in e.waves.iter().enumerate() {
                for (si, s) in w.spawns.iter().enumerate() {
                    let what = format!(
                        "{name}: encounter {} wave {} {} #{si} at {}",
                        ei + 1,
                        wi + 1,
                        s.enemy,
                        s.position
                    );
                    let p = s.position;
                    assert!(
                        e.trigger.contains(p + Vec3::Y * 0.1),
                        "{what}: outside the locked room trigger"
                    );
                    assert!(!in_kill_volume(&level, p), "{what}: in a kill volume");
                    assert!(p.distance(cp) >= 8.0, "{what}: too close to the checkpoint");
                    let (r, h) = enemy_size(&s.enemy);
                    if s.enemy == "harrier" {
                        // Authored at the hover centre: open air, 4-11 m above solid ground
                        // (they hover 5-10 m up), room to climb.
                        assert!(
                            !world.intersects_capsule(p, 0.0, r, MASK_MOVEMENT),
                            "{what}: not in open air"
                        );
                        let below = ground_below(world, p, 20.0);
                        assert!(
                            below.is_some_and(|d| (4.0..=11.0).contains(&d)),
                            "{what}: should hover 4-11 m above solid ground, ground is {below:?} below"
                        );
                        let above = world.cast_ray(p, Vec3::Y, 3.0, MASK_MOVEMENT);
                        assert!(above.is_none(), "{what}: ceiling too close");
                    } else {
                        let ground = ground_below(world, p, 0.6);
                        assert!(
                            ground.is_some_and(|d| d.abs() < 0.3),
                            "{what}: no ground right below ({ground:?})"
                        );
                        let (c, hh) = capsule_at(p + Vec3::Y * 0.05, h, r);
                        assert!(
                            !world.intersects_capsule(c, hh, r, MASK_MOVEMENT),
                            "{what}: overlaps geometry"
                        );
                        // Standing on solid floor all around (not hanging over a ledge).
                        for d in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
                            let edge = ground_below(world, p + d * r * 0.8, 0.6);
                            assert!(
                                edge.is_some_and(|g| g.abs() < 0.3),
                                "{what}: overhangs a ledge"
                            );
                        }
                    }
                    match s.enemy.as_str() {
                        // Ledges deep enough that co-op copies placed around them stay on it.
                        "censer" => {
                            if let Err(q) = floor_all_around(world, p, 3.0, 0.3) {
                                panic!("{what}: ledge too shallow for a censer (no floor at {q})");
                            }
                        }
                        // Room to circle a 1.3 m heavy: level floor 4 m around it.
                        "gaoler" => {
                            if let Err(q) = floor_all_around(world, p, 4.0, 0.6) {
                                panic!("{what}: no room to circle the gaoler (no floor at {q})");
                            }
                        }
                        // Snipers get a long sightline: ≥ 20 m from the rally point, which
                        // they can see from their lens.
                        "heron" => {
                            let eye = p + Vec3::Y * 3.1;
                            let target = rally + Vec3::Y * 1.6;
                            let d = Vec2::new(target.x - eye.x, target.z - eye.z).length();
                            assert!(d >= 20.0, "{what}: only {d:.1} m from the rally point");
                            let to = target - eye;
                            let hit = world.cast_ray(eye, to.normalize(), to.length(), MASK_SHOTS);
                            assert!(
                                hit.is_none(),
                                "{what}: can't see the rally point (blocked at {:?})",
                                hit.map(|h| h.point)
                            );
                        }
                        _ => {}
                    }
                    // Spread out within the wave.
                    for o in &w.spawns[si + 1..] {
                        let (r2, _) = enemy_size(&o.enemy);
                        assert!(
                            p.distance(o.position) >= r + r2 + 1.0,
                            "{what}: crowded with the {} at {}",
                            o.enemy,
                            o.position
                        );
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------------------------------
// Jump pads

#[test]
fn jump_pads_land_on_higher_ground() {
    let t = MovementTuning::default();
    for name in shipped() {
        let level = load(&name);
        let col = collision(&level, Some(true));
        let env = MoveEnv {
            world: &col.world,
            jump_pads: &level.jump_pads,
        };
        for (i, pad) in level.jump_pads.iter().enumerate() {
            let start = Vec3::new(
                pad.trigger.center().x,
                pad.trigger.min.y,
                pad.trigger.center().z,
            );
            assert!(
                ground_below(&col.world, start, 0.3).is_some(),
                "{name}: pad {i} is not on the floor"
            );
            // No input: the pad alone must deliver the player onto a platform above it.
            let mut s = MoveState::spawn(start + Vec3::Y * 0.02);
            let mut fired = false;
            let mut landed = None;
            for seq in 1..=360 {
                let input = PlayerInput {
                    seq,
                    ..Default::default()
                };
                let air_before = s.air_time;
                let ev = movement::step(&mut s, &input, &env, &t, DT);
                fired |= ev.jump_pad.is_some();
                if fired && air_before > 0.2 && ev.landed.is_some() {
                    landed = Some(s.position);
                    break;
                }
                assert!(
                    !in_kill_volume(&level, s.position),
                    "{name}: pad {i} throws into a kill volume"
                );
            }
            assert!(fired, "{name}: pad {i} did not fire");
            let p = landed.unwrap_or_else(|| panic!("{name}: pad {i} never lands"));
            assert!(
                p.y >= start.y + 2.0,
                "{name}: pad {i} lands at {p}, not on higher ground (from {start})"
            );
        }
    }
}

// ------------------------------------------------------------------------------------------
// Z-fighting

#[test]
fn no_coplanar_faces_of_different_materials() {
    for name in shipped() {
        let level = load(&name);
        let brushes: Vec<&Brush> = level
            .brushes
            .iter()
            .chain(level.doors.iter().map(|d| &d.brush))
            .filter(|b| b.rotation == Vec3::ZERO && !b.clip_only)
            .collect();
        let mut bad = Vec::new();
        for (i, a) in brushes.iter().enumerate() {
            for b in &brushes[i + 1..] {
                if a.material == b.material && a.tint == b.tint {
                    continue;
                }
                for axis in 0..3 {
                    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                    let overlap =
                        |k: usize| (a.max[k].min(b.max[k]) - a.min[k].max(b.min[k])).max(0.0);
                    let area = overlap(u) * overlap(v);
                    if area < 1e-3 {
                        continue;
                    }
                    let same_max = (a.max[axis] - b.max[axis]).abs() < 1e-3;
                    let same_min = (a.min[axis] - b.min[axis]).abs() < 1e-3;
                    if same_max || same_min {
                        bad.push(format!(
                            "{} {}..{} vs {} {}..{} (axis {axis})",
                            a.material, a.min, a.max, b.material, b.min, b.max
                        ));
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "{name}: coplanar faces would z-fight:\n{}",
            bad.join("\n")
        );
    }
}

// ------------------------------------------------------------------------------------------
// Sealing

/// Flood fill through free space (balls of 1 m diameter on a 1 m grid, 6-connected) from
/// `start`. Every wall, however thin, stops it; openings smaller than ~1 m may be missed.
/// Fails if it leaves `bounds` or comes within 1 m of a `forbidden` point.
fn flood(
    world: &CollisionWorld,
    start: Vec3,
    bounds: (Vec3, Vec3),
    forbidden: &[(Vec3, String)],
) -> Result<usize, String> {
    let free = |c: IVec3| {
        let p = c.as_vec3() + Vec3::splat(0.5);
        !world.intersects_capsule(p, 0.01, 0.49, MASK_MOVEMENT)
    };
    let origin = start.floor().as_ivec3();
    let first = (-1..=1)
        .flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| IVec3::new(x, y, z))))
        .map(|d| origin + d)
        .find(|&c| free(c))
        .ok_or_else(|| format!("no free cell near {start}"))?;
    let mut seen = HashSet::from([first]);
    let mut queue = VecDeque::from([first]);
    let mut count = 0;
    while let Some(c) = queue.pop_front() {
        count += 1;
        if count > 3_000_000 {
            return Err("flood fill too large".into());
        }
        let p = c.as_vec3() + Vec3::splat(0.5);
        if p.cmplt(bounds.0).any() || p.cmpgt(bounds.1).any() {
            return Err(format!("escapes the level at {p}"));
        }
        if let Some((_, what)) = forbidden.iter().find(|(f, _)| f.distance(p) < 1.0) {
            return Err(format!("reaches {what} (via {p})"));
        }
        for d in [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ] {
            let n = c + d;
            if seen.insert(n) && free(n) {
                queue.push_back(n);
            }
        }
    }
    Ok(count)
}

/// A point 1.5 m beyond door `id` (1.2 m above its sill), on the side away from `inside`.
fn outside_door(level: &LevelDef, id: u32, inside: Vec3) -> Vec3 {
    let d = level.doors.iter().find(|d| d.id == id).expect("door");
    let ext = d.brush.max - d.brush.min;
    let axis = if ext.x < ext.z { Vec3::X } else { Vec3::Z };
    let c = Vec3::new(d.brush.center().x, d.brush.min.y + 1.2, d.brush.center().z);
    let a = c + axis * 1.5;
    let b = c - axis * 1.5;
    if a.distance(inside) > b.distance(inside) {
        a
    } else {
        b
    }
}

fn volume_inside(inner: &Volume, outer: &Volume) -> bool {
    inner.min.cmpge(outer.min).all() && inner.max.cmple(outer.max).all()
}

#[test]
fn encounter_arenas_and_rally_points() {
    for name in CAMPAIGNS {
        let level = load(name);
        let closed = collision(&level, Some(false));
        let open = collision(&level, Some(true));
        let arenas: Vec<Volume> = level
            .encounters
            .iter()
            .enumerate()
            .map(|(i, e)| {
                e.arena
                    .unwrap_or_else(|| panic!("{name}: encounter {} has no arena volume", i + 1))
            })
            .collect();
        for (i, e) in level.encounters.iter().enumerate() {
            let n = i + 1;
            let arena = arenas[i];
            let rally = e
                .rally
                .unwrap_or_else(|| panic!("{name}: encounter {n} has no rally point"));
            assert!(
                volume_inside(&e.trigger, &arena),
                "{name}: encounter {n}: trigger not inside the arena"
            );
            // The rally point is the checkpoint just inside the entry door: a valid standing
            // spot inside the locked room, whether the doors are open or closed.
            assert!(
                arena.contains(rally.position + Vec3::Y * 0.1),
                "{name}: encounter {n}: rally outside the arena"
            );
            assert_eq!(
                rally, level.checkpoints[i].spawn,
                "{name}: encounter {n}: rally should be its checkpoint"
            );
            for col in [&closed, &open] {
                check_player_spot(
                    &level,
                    &col.world,
                    rally.position,
                    &format!("encounter {n} rally"),
                );
            }
            // The arena is the room, not the corridors: the far side of its doors, other
            // rooms' checkpoints, the player spawns and the exit are all outside it; every
            // enemy is in it.
            for &id in e.lock_doors.iter().chain(&e.unlock_doors) {
                let p = outside_door(&level, id, e.trigger.center());
                assert!(
                    !arena.contains(p),
                    "{name}: encounter {n}: arena reaches past door {id} ({p})"
                );
            }
            for (j, c) in level.checkpoints.iter().enumerate() {
                assert_eq!(
                    arena.contains(c.spawn.position + Vec3::Y * 0.1),
                    i == j,
                    "{name}: encounter {n} vs checkpoint {}",
                    j + 1
                );
            }
            for s in &level.spawns {
                assert!(
                    !arena.contains(s.position + Vec3::Y * 0.1),
                    "{name}: encounter {n}: arena contains a player spawn"
                );
            }
            if let Some(x) = level.exit {
                assert!(
                    !arena.contains(x.center()),
                    "{name}: encounter {n}: arena contains the exit"
                );
            }
            for s in e.waves.iter().flat_map(|w| &w.spawns) {
                assert!(
                    arena.contains(s.position),
                    "{name}: encounter {n}: {} at {} outside the arena",
                    s.enemy,
                    s.position
                );
            }
            for (j, other) in arenas.iter().enumerate().skip(i + 1) {
                let overlap = arena.min.cmplt(other.max).all() && other.min.cmplt(arena.max).all();
                assert!(!overlap, "{name}: arenas {n} and {} overlap", j + 1);
            }
        }
    }
}

#[test]
fn par_times() {
    for name in CAMPAIGNS {
        let campaign = load(name);
        assert!(
            (180.0..=900.0).contains(&campaign.par_time),
            "{name}: campaign par time {} s",
            campaign.par_time
        );
    }
    for (name, level) in arenas() {
        assert_eq!(
            level.par_time, 0.0,
            "{name}: deathmatch arenas have no par time"
        );
    }
}

#[test]
fn campaign_compartments_are_sealed_when_doors_close() {
    for name in CAMPAIGNS {
        let level = load(name);
        let col = collision(&level, Some(false));
        let (lo, hi) = level_bounds(&level);
        let bounds = (lo - Vec3::ONE, hi + Vec3::ONE);
        // Compartments with all doors closed: the start area, each arena (its checkpoint is
        // just inside the entry door), and the exit hall.
        let mut points: Vec<(usize, Vec3, String)> = level
            .spawns
            .iter()
            .enumerate()
            .map(|(i, s)| (0, s.position + Vec3::Y, format!("player spawn {i}")))
            .collect();
        for (i, c) in level.checkpoints.iter().enumerate() {
            points.push((
                i + 1,
                c.spawn.position + Vec3::Y,
                format!("checkpoint {}", i + 1),
            ));
        }
        let exit = level.exit.expect("exit");
        let exit_point = Vec3::new(exit.center().x, exit.min.y + 1.0, exit.center().z);
        points.push((level.checkpoints.len() + 1, exit_point, "the exit".into()));
        let outside_door = |id: u32, inside: Vec3| outside_door(&level, id, inside);
        for compartment in 0..=level.checkpoints.len() + 1 {
            let start = points.iter().find(|p| p.0 == compartment).expect("point").1;
            let mut forbidden: Vec<(Vec3, String)> = points
                .iter()
                .filter(|p| p.0 != compartment)
                .map(|p| (p.1, p.2.clone()))
                .collect();
            // An arena must also be closed towards both of its corridors.
            if let Some(e) = compartment
                .checked_sub(1)
                .and_then(|i| level.encounters.get(i))
            {
                for &id in e.lock_doors.iter().chain(&e.unlock_doors) {
                    forbidden.push((
                        outside_door(id, e.trigger.center()),
                        format!("the far side of door {id}"),
                    ));
                }
            }
            let cells = flood(&col.world, start, bounds, &forbidden).unwrap_or_else(|e| {
                panic!("{name}: compartment {compartment} (from {start}) leaks: {e}")
            });
            println!("{name}: compartment {compartment}: {cells} free cells");
        }
        // With the doors open everything is one connected space.
        let open = collision(&level, Some(true));
        let all: Vec<(Vec3, String)> = vec![(exit_point, "the exit".into())];
        let err = flood(
            &open.world,
            level.spawns[0].position + Vec3::Y,
            bounds,
            &all,
        )
        .expect_err("with open doors the exit is reachable from the start");
        assert!(err.contains("the exit"), "{name}: {err}");
    }
}

#[test]
fn arenas_are_sealed() {
    for (name, level) in arenas() {
        let col = collision(&level, None);
        let (lo, hi) = level_bounds(&level);
        let start = level.spawns[0].position + Vec3::Y;
        let cells = flood(&col.world, start, (lo - Vec3::ONE, hi + Vec3::ONE), &[])
            .unwrap_or_else(|e| panic!("{name} leaks: {e}"));
        println!("{name}: {cells} free cells");
    }
}

// ------------------------------------------------------------------------------------------
// Bots

#[derive(Clone, Copy, PartialEq, Debug)]
enum Act {
    /// Run towards the point (jumping over hurdles and gaps).
    Run,
    /// Slide (crouch held) towards the point.
    Slide,
    /// Run onto a jump pad; reached when it fires.
    Pad,
}

#[derive(Clone, Copy, Debug)]
struct Wp {
    p: Vec3,
    act: Act,
}

fn run(x: f32, y: f32, z: f32) -> Wp {
    Wp {
        p: Vec3::new(x, y, z),
        act: Act::Run,
    }
}
fn slide(x: f32, y: f32, z: f32) -> Wp {
    Wp {
        p: Vec3::new(x, y, z),
        act: Act::Slide,
    }
}
fn pad(x: f32, y: f32, z: f32) -> Wp {
    Wp {
        p: Vec3::new(x, y, z),
        act: Act::Pad,
    }
}

struct Bot<'a> {
    level: &'a LevelDef,
    s: MoveState,
    t: MovementTuning,
    env: MoveEnv<'a>,
    seq: u32,
    ticks: u32,
    prev_jump: bool,
    prev_crouch: bool,
    slow: u32,
    /// Recent positions for failure messages.
    trail: VecDeque<Vec3>,
    /// Volumes entered (by label), in order of first entry.
    entered: Vec<String>,
    marks: Vec<(String, Volume)>,
    /// Jump pads fired (indices, in order of first use).
    pads: Vec<u16>,
}

impl<'a> Bot<'a> {
    fn new(level: &'a LevelDef, col: &'a LevelCollision, start: Vec3) -> Self {
        let mut marks = Vec::new();
        for (i, c) in level.checkpoints.iter().enumerate() {
            marks.push((format!("checkpoint {}", i + 1), c.trigger));
        }
        for (i, e) in level.encounters.iter().enumerate() {
            marks.push((format!("encounter {}", i + 1), e.trigger));
        }
        if let Some(x) = level.exit {
            marks.push(("exit".into(), x));
        }
        for (i, v) in level.secrets.iter().enumerate() {
            marks.push((format!("secret {}", i + 1), *v));
        }
        let mut bot = Self {
            level,
            s: MoveState::spawn(start),
            t: MovementTuning::default(),
            env: MoveEnv {
                world: &col.world,
                jump_pads: &level.jump_pads,
            },
            seq: 0,
            ticks: 0,
            prev_jump: false,
            prev_crouch: false,
            slow: 0,
            trail: VecDeque::new(),
            entered: Vec::new(),
            marks,
            pads: Vec::new(),
        };
        bot.mark();
        bot
    }

    fn mark(&mut self) {
        let p = self.s.position + Vec3::Y * 0.1;
        for (label, v) in &self.marks {
            if v.contains(p) && !self.entered.contains(label) {
                self.entered.push(label.clone());
            }
        }
    }

    fn step(&mut self, input: PlayerInput) -> MoveEvents {
        self.seq += 1;
        self.ticks += 1;
        let input = PlayerInput {
            seq: self.seq,
            ..input
        };
        let ev = movement::step(&mut self.s, &input, &self.env, &self.t, DT);
        if let Some(i) = ev.jump_pad.filter(|i| !self.pads.contains(i)) {
            self.pads.push(i);
        }
        self.trail.push_back(self.s.position);
        if self.trail.len() > 12 {
            self.trail.pop_front();
        }
        self.mark();
        ev
    }

    fn fail(&self, msg: String) -> String {
        format!(
            "{msg}\n  at {} vel {} mode {:?} ground {} after {:.1} s\n  trail: {:?}",
            self.s.position,
            self.s.velocity,
            self.s.mode,
            self.s.on_ground,
            self.ticks as f32 * DT,
            self.trail.iter().step_by(3).collect::<Vec<_>>()
        )
    }

    fn ground_drop_ahead(&self, dir: Vec3) -> f32 {
        let from = self.s.position + dir * 1.4 + Vec3::Y * 0.6;
        match self
            .env
            .world
            .cast_ray(from, Vec3::NEG_Y, 4.0, MASK_MOVEMENT)
        {
            Some(h) => h.distance - 0.6,
            None => 4.0,
        }
    }

    fn hurdle_ahead(&self, dir: Vec3) -> bool {
        let knee = self.s.position + Vec3::Y * 0.6;
        let low = self.env.world.cast_ray(knee, dir, 1.6, MASK_MOVEMENT);
        let high =
            self.env
                .world
                .cast_ray(self.s.position + Vec3::Y * 2.6, dir, 2.2, MASK_MOVEMENT);
        low.is_some_and(|h| h.normal.y < 0.5) && high.is_none()
    }

    /// Follows the waypoints; errors if one takes longer than `timeout` seconds.
    fn follow(&mut self, wps: &[Wp], timeout: f32) -> Result<(), String> {
        for (wi, wp) in wps.iter().enumerate() {
            let start_tick = self.ticks;
            loop {
                let to = wp.p - self.s.position;
                let dist = Vec2::new(to.x, to.z).length();
                if wp.act != Act::Pad && dist < 1.5 && to.y.abs() < 2.5 {
                    break;
                }
                if (self.ticks - start_tick) as f32 * DT > timeout {
                    return Err(self.fail(format!(
                        "waypoint {wi} {:?} {} not reached in {timeout} s",
                        wp.act, wp.p
                    )));
                }
                let dir = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
                let yaw = f32::atan2(-to.x, -to.z);
                let mut buttons = Buttons::NONE;
                // Crouch only from the ground (a crouch press in the air would slam).
                let crouch = wp.act == Act::Slide && (self.s.on_ground || self.prev_crouch);
                if crouch {
                    buttons = buttons | Buttons::CROUCH;
                }
                self.prev_crouch = crouch;
                let mut jump = false;
                if self.s.on_ground && !self.prev_jump && wp.act != Act::Slide {
                    if to.y > 0.6 && dist < 6.0 {
                        jump = true;
                    }
                    if to.y > -1.0 && self.ground_drop_ahead(dir) > 1.0 {
                        jump = true;
                    }
                    if to.y > -1.0 && self.hurdle_ahead(dir) {
                        jump = true;
                    }
                    if self.slow > 8 {
                        jump = true;
                        self.slow = 0;
                    }
                }
                if jump {
                    buttons = buttons | Buttons::JUMP;
                }
                self.prev_jump = jump;
                let input = PlayerInput {
                    forward: 127,
                    yaw,
                    buttons,
                    ..Default::default()
                };
                let ev = self.step(input);
                if self.s.on_ground && self.s.horizontal_speed() < 4.0 && dist > 1.5 {
                    self.slow += 1;
                } else {
                    self.slow = 0;
                }
                if in_kill_volume(self.level, self.s.position) {
                    return Err(self.fail(format!(
                        "fell into a kill volume heading to waypoint {wi} {}",
                        wp.p
                    )));
                }
                if wp.act == Act::Pad && ev.jump_pad.is_some() {
                    break;
                }
            }
        }
        Ok(())
    }

    fn seconds(&self) -> f32 {
        self.ticks as f32 * DT
    }
}

/// A campaign level's critical path, one segment per checkpoint: segment 0 leads from the
/// player spawns into arena 1; segment i from checkpoint i through arena i into arena i+1
/// (the last one to the exit).
fn campaign_route(name: &str) -> Vec<Vec<Wp>> {
    match name {
        CRUCIBLE => crucible_route(),
        SPIRE => spire_route(),
        DROWNED => drowned_route(),
        _ => panic!("{name}: every campaign level needs a bot route in levels.rs"),
    }
}

/// The Drowned Chandlery: two plain jumps over deep water (the Culvert's sluice, the Spool
/// Walk's break), a hop over the Pipe Gallery's valve box and one over the Wax Press's drowned
/// floor past a wax floe; ramps everywhere else.
fn drowned_route() -> Vec<Vec<Wp>> {
    vec![
        // Loading Dock -> Culvert (jump the sluice) -> Dipping Hall vestibule.
        vec![
            run(0.0, 0.0, 66.0),
            run(0.0, 0.0, 57.0),
            run(0.0, 0.0, 48.0),
            run(0.0, 0.0, 41.0),
        ],
        // Dipping Hall: east ramp to the gantry, exit door -> Spool Walk (jump the break) ->
        // Cistern vestibule.
        vec![
            run(20.0, 0.0, 38.0),
            run(20.0, 0.0, 31.0),
            run(20.0, 4.0, 15.0),
            run(16.5, 4.0, 12.0),
            run(16.5, 4.0, 5.0),
            run(16.5, 4.0, -8.0),
            run(16.5, 4.0, -16.0),
        ],
        // Cistern: east ramp, landing, north gallery, west gallery, exit -> Pipe Gallery (hop
        // the valve box) -> Wax Press balcony.
        vec![
            run(26.0, 4.0, -18.0),
            run(31.5, 4.0, -15.0),
            run(31.5, 14.0, -43.0),
            run(26.0, 14.0, -50.0),
            run(0.0, 14.0, -50.0),
            run(0.0, 14.0, -37.5),
            run(-6.0, 14.0, -36.0),
            run(-14.0, 14.0, -36.0),
            run(-19.0, 14.0, -37.5),
        ],
        // Wax Press: down the balcony ramp, west across the floor, jump the drowned floor past
        // the floe, exit door -> Wick Passage -> Great Wick vestibule.
        vec![
            run(-20.5, 14.0, -33.0),
            run(-31.0, 11.0, -33.0),
            run(-33.0, 11.0, -29.0),
            run(-40.5, 11.0, -27.5),
            run(-51.5, 11.0, -27.5),
            run(-57.0, 11.0, -27.5),
            run(-63.0, 11.0, -27.5),
            run(-69.0, 11.0, -27.5),
        ],
        // Great Wick: round the east floor to the south ramp, up to the west gallery, the
        // north gallery, exit door -> C5 -> the Wick Terrace.
        vec![
            run(-70.0, 11.0, -15.0),
            run(-76.0, 11.4, -9.0),
            run(-103.0, 21.0, -9.0),
            run(-106.5, 21.0, -12.0),
            run(-106.5, 21.0, -46.5),
            run(-88.5, 21.0, -46.5),
            run(-88.5, 21.0, -55.0),
            run(-88.5, 21.0, -68.0),
            run(-88.5, 21.3, -75.5),
        ],
    ]
}

/// Spire of Wicks: no tech needed (one jump over the fallen bell's crown, ramps all the way).
fn spire_route() -> Vec<Vec<Wp>> {
    vec![
        // Wax Crypt -> Fallen Bell (hop the crown in the east lane) -> Casting Floor vestibule.
        vec![
            run(0.0, 0.0, 63.0),
            run(2.25, 0.0, 56.0),
            run(2.25, 0.0, 46.0),
            run(0.0, 0.0, 39.0),
        ],
        // Casting Floor, west of the moulds -> Wick Gate -> Hoist vestibule.
        vec![
            run(-14.5, 0.0, 33.0),
            run(-14.5, 0.0, 20.0),
            run(-21.0, 0.0, 17.0),
            run(-27.0, 0.0, 17.0),
            run(-33.0, 0.0, 17.0),
        ],
        // Hoist: one turn of ramps (W, S, E, N walls) to the top exit -> Rope Walk -> Rood.
        vec![
            run(-45.0, 0.0, 4.0),
            run(-57.5, 0.0, -11.0),
            run(-57.5, 0.0, -6.0),
            run(-57.5, 6.5, 19.0),
            run(-38.0, 13.0, 19.5),
            run(-33.0, 13.0, 17.0),
            run(-32.5, 19.5, -10.0),
            run(-35.0, 19.5, -11.5),
            run(-55.0, 26.0, -11.5),
            run(-57.0, 26.0, -16.0),
            run(-57.0, 26.0, -36.0),
            run(-57.0, 26.0, -42.0),
        ],
        // Rood: down the wax run, south bridge, far floor, loft ramp, exit -> Wick Stair ->
        // Belfry vestibule.
        vec![
            run(-50.0, 26.0, -45.0),
            run(-28.0, 14.0, -45.0),
            run(-6.0, 14.0, -45.0),
            run(5.0, 14.0, -60.0),
            run(13.5, 14.0, -67.0),
            run(13.5, 24.0, -48.0),
            run(13.0, 24.0, -41.0),
            run(13.0, 24.0, -35.0),
            run(13.0, 24.0, -31.5),
            run(-21.0, 36.0, -31.5),
            run(-21.0, 36.0, -27.0),
            run(-21.0, 36.0, -21.5),
        ],
        // Belfry: west ramp to the south gallery, exit door -> the Lantern.
        vec![
            run(-21.5, 36.0, -3.0),
            run(-21.5, 44.0, 17.5),
            run(-10.0, 44.0, 20.0),
            run(0.0, 44.0, 20.0),
            run(0.0, 44.0, 28.0),
            run(0.0, 44.3, 37.0),
        ],
    ]
}

/// Crucible of Veins.
fn crucible_route() -> Vec<Vec<Wp>> {
    vec![
        // Receiving Nave -> Split Flue (hurdle lane) -> Smelting Floor vestibule.
        vec![
            run(1.75, 0.0, -3.0),
            run(1.75, 0.0, -15.0),
            run(0.0, 0.0, -26.0),
        ],
        // Smelting Floor (around the vat) -> Sluice (jump the slag trench) -> Cloister.
        vec![
            run(0.0, 0.0, -36.0),
            run(-6.0, 0.0, -43.0),
            run(0.0, 0.0, -58.0),
            run(0.0, 0.0, -68.0),
            run(0.0, 0.0, -86.0),
            run(0.0, 0.0, -94.0),
        ],
        // Cloister: east bridge, east ramp to the mezzanine, exit door -> Vein Stair -> Bell Loft.
        vec![
            run(0.0, 0.0, -102.0),
            run(15.5, 0.0, -111.0),
            run(15.5, 0.0, -124.0),
            run(27.5, 0.0, -121.5),
            run(27.5, 5.0, -136.0),
            run(-22.5, 5.0, -137.0),
            run(-22.5, 5.0, -146.0),
            run(-22.5, 10.0, -165.0),
            run(-22.5, 10.0, -175.0),
        ],
        // Bell Loft: pad to the upper platform, exit door -> Spillway -> Crucible Heart.
        vec![
            pad(-10.0, 10.0, -184.0),
            run(-4.0, 22.0, -203.0),
            run(-2.0, 22.0, -208.5),
            run(4.0, 22.0, -208.5),
            run(60.0, 0.0, -208.5),
            run(68.5, 0.0, -208.5),
        ],
        // Crucible Heart: north side of the moat, exit door -> Pipe Run (slide) -> Cathedral.
        vec![
            run(72.0, 0.0, -226.0),
            run(91.0, 0.0, -229.0),
            run(91.0, 0.0, -238.0),
            slide(91.0, 0.0, -258.0),
            run(91.0, 0.0, -266.0),
        ],
        // Cathedral: bridge over the chasm, west dais ramp, door behind the altar -> exit.
        vec![
            run(91.0, 0.0, -280.0),
            run(91.0, 0.0, -302.0),
            run(80.5, 0.0, -304.0),
            run(80.5, 4.0, -317.0),
            run(91.0, 4.0, -326.0),
            run(91.0, 4.0, -336.0),
            run(91.0, 4.0, -350.0),
            run(91.0, 4.0, -362.0),
        ],
    ]
}

#[test]
fn campaign_critical_path_is_traversable() {
    for name in CAMPAIGNS {
        let level = load(name);
        let col = collision(&level, Some(true));
        let route = campaign_route(name);
        assert_eq!(
            route.len(),
            level.checkpoints.len() + 1,
            "{name}: one route segment per checkpoint + 1"
        );
        let exit = level.exit.expect("exit");

        // Whole route from every player spawn.
        for (si, s) in level.spawns.iter().enumerate() {
            let mut bot = Bot::new(&level, &col, s.position);
            let mut splits = Vec::new();
            for (i, seg) in route.iter().enumerate() {
                bot.follow(seg, 15.0)
                    .unwrap_or_else(|e| panic!("{name}: spawn {si}, segment {i}: {e}"));
                splits.push(format!("{:.1}", bot.seconds()));
            }
            assert!(
                exit.contains(bot.s.position + Vec3::Y * 0.1),
                "{name}: spawn {si}: route ends outside the exit"
            );
            let mut expected = Vec::new();
            for i in 1..=level.checkpoints.len() {
                expected.push(format!("checkpoint {i}"));
                expected.push(format!("encounter {i}"));
            }
            expected.push("exit".into());
            assert_eq!(
                bot.entered, expected,
                "{name}: spawn {si}: triggers entered out of order"
            );
            println!(
                "{name}: spawn {si}: exit reached in {:.1} s of pure traversal (splits {})",
                bot.seconds(),
                splits.join(", ")
            );
        }

        // Each checkpoint respawn can make progress on its own.
        for (i, c) in level.checkpoints.iter().enumerate() {
            let mut bot = Bot::new(&level, &col, c.spawn.position);
            for seg in &route[i + 1..] {
                bot.follow(seg, 15.0)
                    .unwrap_or_else(|e| panic!("{name}: from checkpoint {}: {e}", i + 1));
            }
            assert!(
                exit.contains(bot.s.position + Vec3::Y * 0.1),
                "{name}: checkpoint {}: route ends outside the exit",
                i + 1
            );
        }
    }
}

/// A campaign level's optional routes: (description, start, waypoints).
fn alternate_routes(name: &str, level: &LevelDef) -> Vec<(&'static str, Vec3, Vec<Wp>)> {
    match name {
        CRUCIBLE => crucible_alternate_routes(level),
        SPIRE => spire_alternate_routes(level),
        DROWNED => drowned_alternate_routes(level),
        _ => panic!("{name}: every campaign level needs alternate routes in levels.rs"),
    }
}

#[test]
fn campaign_alternate_routes_are_traversable() {
    for name in CAMPAIGNS {
        let level = load(name);
        let col = collision(&level, Some(true));
        let routes = alternate_routes(name, &level);
        assert!(routes.len() >= 4, "{name}: alternate routes");
        // Every secret is found by one of them (and never on the critical path: the bots
        // there must enter exactly the checkpoints, encounters and the exit).
        let mut found = HashSet::new();
        for (route, start, wps) in routes {
            let mut bot = Bot::new(&level, &col, start);
            bot.follow(&wps, 12.0)
                .unwrap_or_else(|e| panic!("{name}: {route}: {e}"));
            println!("{name}: alternate route '{route}': {:.1} s", bot.seconds());
            found.extend(bot.entered.into_iter().filter(|l| l.starts_with("secret")));
        }
        for i in 1..=level.secrets.len() {
            assert!(
                found.contains(&format!("secret {i}")),
                "{name}: no route finds secret {i}"
            );
        }
    }
}

fn drowned_alternate_routes(level: &LevelDef) -> Vec<(&'static str, Vec3, Vec<Wp>)> {
    vec![
        (
            "Dipping Hall pad onto the gantry",
            level.checkpoints[0].spawn.position,
            vec![
                run(5.5, 0.0, 33.0),
                run(5.5, 0.0, 24.0),
                pad(0.0, 0.0, 23.0),
                run(0.0, 4.0, 13.0),
                run(16.5, 4.0, 12.0),
            ],
        ),
        (
            "Cistern pad onto the west gallery",
            level.checkpoints[1].spawn.position,
            vec![
                run(10.0, 4.0, -18.0),
                run(0.5, 4.0, -21.0),
                pad(0.2, 4.0, -24.8),
                run(0.0, 14.0, -37.5),
            ],
        ),
        (
            "slide under the Pipe Gallery's pipes",
            Vec3::new(-5.5, 14.0, -39.0),
            vec![slide(-14.0, 14.0, -39.0), run(-19.0, 14.0, -38.0)],
        ),
        (
            "Wax Press: drop off the balcony, pad onto the press bed",
            level.checkpoints[2].spawn.position,
            vec![
                run(-23.0, 11.0, -43.0),
                run(-31.0, 11.0, -43.0),
                pad(-34.0, 11.0, -38.0),
                run(-34.0, 15.0, -48.0),
            ],
        ),
        (
            "Great Wick: floor pad onto the north gallery",
            level.checkpoints[3].spawn.position,
            vec![
                pad(-78.0, 11.0, -38.0),
                run(-82.0, 21.0, -47.0),
                run(-88.5, 21.0, -46.5),
            ],
        ),
        (
            "secret 1: hop onto the barge and into its cabin",
            level.spawns[0].position,
            vec![
                run(-4.5, 0.0, 74.5),
                run(-7.5, 0.0, 74.5),
                run(-10.0, 0.0, 74.5),
                run(-9.6, 0.0, 76.4),
            ],
        ),
        (
            "secret 2: the floe pad onto the top of the Great Wick",
            level.checkpoints[3].spawn.position,
            vec![
                run(-70.0, 11.0, -15.0),
                run(-94.0, 11.0, -13.5),
                pad(-94.36, 11.2, -21.64),
                run(-90.5, 38.0, -25.5),
            ],
        ),
    ]
}

fn spire_alternate_routes(level: &LevelDef) -> Vec<(&'static str, Vec3, Vec<Wp>)> {
    vec![
        (
            "slide under the fallen bell (west lane)",
            level.spawns[0].position,
            vec![
                run(-1.75, 0.0, 63.0),
                run(-1.75, 0.0, 57.0),
                slide(-1.75, 0.0, 45.0),
                run(0.0, 0.0, 39.0),
            ],
        ),
        (
            "Casting Floor east balcony via its ramp",
            level.checkpoints[0].spawn.position,
            vec![
                run(21.5, 0.0, 39.0),
                run(21.5, 7.0, 24.0),
                run(20.0, 7.0, 13.0),
            ],
        ),
        (
            "Hoist pad line: floor pad, cage A, cage B, NW landing",
            level.checkpoints[1].spawn.position,
            vec![
                pad(-46.0, 0.0, 12.0),
                pad(-49.0, 9.0, 5.5),
                pad(-51.0, 17.5, -2.5),
                run(-57.0, 26.0, -11.0),
                run(-57.0, 26.0, -20.0),
            ],
        ),
        (
            "Rood north bridge",
            level.checkpoints[2].spawn.position,
            vec![
                run(-50.0, 26.0, -65.0),
                run(-28.0, 14.0, -65.0),
                run(-6.0, 14.0, -65.0),
                run(5.0, 14.0, -60.0),
                run(13.5, 14.0, -67.0),
                run(13.5, 24.0, -48.0),
            ],
        ),
        (
            "Belfry pads to the east and south galleries, round the gallery loop",
            level.checkpoints[3].spawn.position,
            vec![
                run(-12.0, 36.0, -14.0),
                run(10.75, 36.0, -13.0),
                pad(10.75, 36.0, 0.0),
                run(20.0, 44.0, 0.0),
                run(20.0, 44.0, 20.0),
                run(-21.5, 44.0, 20.0),
                run(-21.5, 36.0, -3.0),
                run(-14.0, 36.0, 4.0),
                pad(-11.0, 36.0, 10.75),
                run(-11.0, 44.0, 20.0),
            ],
        ),
    ]
}

fn crucible_alternate_routes(level: &LevelDef) -> Vec<(&'static str, Vec3, Vec<Wp>)> {
    vec![
        (
            "slide slot (west lane of the Split Flue)",
            level.spawns[0].position,
            vec![
                run(-1.75, 0.0, -4.0),
                slide(-1.75, 0.0, -15.0),
                run(0.0, 0.0, -26.0),
            ],
        ),
        (
            "Cloister west bridge + jump pad to the mezzanine",
            level.checkpoints[1].spawn.position,
            vec![
                run(-15.5, 0.0, -111.0),
                run(-15.5, 0.0, -124.0),
                pad(0.0, 0.0, -126.0),
                run(0.0, 5.0, -137.0),
                run(-22.5, 5.0, -137.0),
                run(-22.5, 5.0, -146.0),
            ],
        ),
        (
            "Bell Loft west balcony via pad, catwalk and ramp to the upper platform",
            level.checkpoints[2].spawn.position,
            vec![
                pad(-32.0, 10.0, -184.0),
                run(-40.0, 16.0, -190.0),
                run(-40.0, 16.0, -211.5),
                run(-21.0, 16.0, -211.5),
                run(-6.0, 22.0, -211.5),
                run(-2.0, 22.0, -208.5),
            ],
        ),
        (
            "Crucible Heart across the island and up a corner pad",
            level.checkpoints[3].spawn.position,
            vec![
                run(76.0, 0.0, -208.0),
                run(91.0, 1.0, -208.0),
                run(98.0, 0.0, -208.0),
                run(102.5, 0.0, -212.0),
                pad(102.5, 0.0, -228.5),
                run(111.0, 4.0, -228.5),
            ],
        ),
        (
            "Cathedral east aisle beside the slag pool, east pad to the gallery",
            level.checkpoints[4].spawn.position,
            vec![
                run(115.0, 0.0, -270.0),
                run(110.5, 0.0, -288.0),
                run(110.5, 0.0, -302.0),
                run(108.0, 0.0, -286.0),
                pad(105.5, 0.0, -278.5),
                run(116.0, 8.0, -278.5),
                run(116.0, 8.0, -318.0),
            ],
        ),
    ]
}

/// A full lap of every arena: (level, start spawn, waypoints, distinct pads it must take).
fn arena_laps() -> Vec<(&'static str, usize, Vec<Wp>, usize)> {
    vec![
        // Cistern Nine: NW tower, north catwalk, NE tower, ramp down, east slide tunnel into
        // the bowl, pad onto the core, south ramp out, SE tower, south catwalk, SW tower, ramp
        // down, west slide tunnel, pad onto the core again.
        (
            ARENA,
            0,
            vec![
                run(-26.0, 6.0, -34.5),
                run(26.0, 6.0, -34.5),
                run(32.0, 6.0, -30.0),
                run(34.0, 3.0, -19.0),
                run(34.0, 0.0, -8.0),
                run(30.0, 0.0, 0.0),
                run(27.0, 0.0, 0.0),
                slide(14.0, -3.0, 0.0),
                pad(9.25, -3.0, 0.0),
                run(0.0, 4.0, 0.0),
                run(-6.0, -3.0, 6.0),
                run(0.0, -3.0, 7.0),
                run(0.0, 0.0, 17.0),
                run(10.0, 0.0, 18.0),
                run(34.0, 0.0, 9.0),
                run(34.0, 6.0, 30.0),
                run(26.0, 6.0, 34.5),
                run(-26.0, 6.0, 34.5),
                run(-32.0, 6.0, 30.0),
                run(-34.0, 3.0, 19.0),
                run(-34.0, 0.0, 8.0),
                run(-30.0, 0.0, 0.0),
                run(-27.0, 0.0, 0.0),
                slide(-14.0, -3.0, 0.0),
                pad(-9.25, -3.0, 0.0),
                run(0.0, 4.0, 0.0),
            ],
            2,
        ),
        // Knell Yard, all four quarters of the pinwheel: north shed roof, down the north ramp,
        // kiln pad onto the kiln roof, over the flask rack onto the east shed roof, east deck
        // bridge, corner pad onto the crown, off it onto the west deck bridge, west shed roof,
        // down the west ramp, moat pad onto the deck,
        // south deck bridge, south shed roof, down the south ramp, south moat bridge through
        // the arcade and out over the east one, slide under the east casting table straight
        // onto the south-east kiln pad.
        (
            KNELL,
            1,
            vec![
                run(4.0, 5.0, -22.0),
                run(15.5, 0.0, -22.0),
                run(15.0, 0.0, -18.5),
                pad(12.5, 0.0, -12.5),
                run(20.5, 7.5, -20.5),
                run(22.0, 5.0, -7.0),
                run(21.5, 5.0, -2.75),
                run(8.0, 5.0, -2.75),
                run(5.5, 5.0, 2.0),
                pad(6.2, 5.0, 6.2),
                run(1.5, 10.0, 1.5),
                run(-12.0, 5.0, 2.75),
                run(-19.0, 5.0, 2.75),
                run(-22.0, 5.0, -4.0),
                run(-22.0, 0.0, -15.5),
                run(-18.5, 0.0, -11.0),
                run(-17.0, 0.0, -3.0),
                pad(-12.3, 0.0, -0.1),
                run(-5.5, 5.0, 2.0),
                run(-4.8, 5.0, 4.8),
                run(2.75, 5.0, 5.5),
                run(2.75, 5.0, 19.0),
                run(-4.0, 5.0, 22.0),
                run(-15.5, 0.0, 22.0),
                run(-15.0, 0.0, 19.0),
                run(-11.0, 0.0, 18.8),
                run(-5.5, 0.0, 19.0),
                run(-3.0, 0.0, 11.0),
                run(-3.0, 0.0, 7.0),
                run(-3.0, 0.0, 3.0),
                run(3.0, 0.0, 3.0),
                run(8.0, 0.0, 3.0),
                run(15.0, 0.0, 2.0),
                run(15.0, 0.0, 4.5),
                slide(15.0, 0.0, 11.5),
                pad(12.5, 0.0, 12.5),
                run(20.5, 7.5, 20.5),
            ],
            4,
        ),
        // Ingot Run, end to end three ways: east walkway, the whole crane gantry west, west
        // walkway, down the west ramp, into the ingot trough and slide its whole length east
        // (cooling beds, the duct under the slag), up the trough ramp, furnace pad onto the
        // furnace, over the walkway down to the casting floor, east floor pad onto the
        // gantry, off it and up again with the west floor pad, over the pour bridge, step
        // crate onto the east ladle stand.
        (
            INGOT,
            0,
            vec![
                run(41.0, 7.0, -18.0),
                run(34.0, 7.0, -19.0),
                run(28.0, 7.0, -19.0),
                run(18.0, 7.0, -17.0),
                run(-18.0, 7.0, -17.0),
                run(-28.0, 7.0, -19.0),
                run(-34.0, 7.0, -19.0),
                run(-41.0, 7.0, -17.0),
                run(-41.0, 7.0, 11.0),
                run(-38.0, 7.0, 12.0),
                run(-25.0, 0.0, 12.0),
                run(-33.0, 0.0, 15.0),
                run(-40.5, -1.5, 18.0),
                run(-37.0, -1.5, 18.0),
                slide(-10.0, -1.5, 18.0),
                slide(10.0, -1.5, 18.0),
                slide(38.0, -1.5, 18.0),
                run(46.0, 0.0, 18.0),
                pad(45.5, 0.0, 9.5),
                run(45.5, 11.0, 5.0),
                run(30.0, 0.0, 0.0),
                run(20.0, 0.0, -6.0),
                pad(10.2, 0.0, -9.0),
                run(6.0, 7.0, -18.0),
                run(-14.0, 7.0, -17.0),
                run(-14.0, 0.0, -6.0),
                pad(-10.2, 0.0, -9.0),
                run(-6.0, 7.0, -17.0),
                run(-6.0, 0.0, -6.0),
                run(-13.0, 0.0, -5.0),
                run(-13.0, 0.0, 0.0),
                run(-4.0, 3.0, 0.0),
                run(4.0, 3.0, 0.0),
                run(13.0, 0.0, 0.0),
                run(12.5, 0.0, 6.0),
                run(14.25, 1.2, 6.0),
                run(17.0, 3.0, 5.5),
            ],
            3,
        ),
        // Tallow Works, every layer: from the west loft down the north belt to the floor, the
        // east boiler pad onto the boiler's top, round the flue, off it, the west boiler pad
        // back up, off it to the north, up the belt to the loft, drop into the wall-jump shaft,
        // along the floor to the north-east corner, up the mezzanine ramp and along the
        // mezzanine, its pad back onto the loft. (The skimmers' pads are checked by
        // `jump_pads_land_on_higher_ground`.)
        (
            TALLOW,
            0,
            vec![
                run(-25.0, 12.0, -14.0),
                run(0.0, 3.0, -14.0),
                run(8.0, 0.0, -14.0),
                run(10.5, 0.0, -9.0),
                pad(12.0, 0.0, -3.5),
                run(3.5, 9.0, -3.5),
                run(3.5, 9.0, 3.5),
                run(-3.5, 9.0, 3.5),
                run(-9.0, 0.0, 6.5),
                pad(-12.0, 0.0, 3.5),
                run(-3.5, 9.0, 3.5),
                run(-3.5, 9.0, -3.5),
                run(0.0, 0.0, -9.0),
                run(10.0, 0.0, -14.0),
                run(0.0, 3.0, -14.0),
                run(-25.0, 12.0, -14.0),
                run(-28.0, 12.0, -14.5),
                run(-28.0, 0.0, -17.25),
                run(-18.0, 0.0, -17.25),
                run(20.0, 0.0, -17.5),
                run(29.0, 0.0, -19.0),
                run(29.0, 0.0, -23.5),
                run(14.0, 6.0, -23.5),
                run(-17.0, 6.0, -22.5),
                pad(-20.0, 6.0, -22.5),
                run(-28.0, 12.0, -9.0),
            ],
            3,
        ),
        // Wickmarket: out of the east candle shop across the market, up the east guild stair to
        // the guild walk, along it past both kiosks, down the west stair, onto the gallows dais
        // and its pad up onto the beam, off it, the east pad back up onto the guild walk.
        (
            WICKMARKET,
            0,
            vec![
                run(33.0, 0.0, -18.0),
                run(22.0, 0.0, -19.5),
                run(20.5, 0.0, -23.5),
                run(36.0, 7.0, -23.5),
                run(34.0, 7.0, -28.0),
                run(20.0, 7.0, -27.5),
                run(-20.0, 7.0, -27.5),
                run(-34.0, 7.0, -27.5),
                run(-36.0, 7.0, -23.5),
                run(-20.0, 0.0, -23.5),
                run(-8.0, 0.0, -12.0),
                run(-3.0, 0.45, -6.0),
                pad(-3.0, 0.9, 3.0),
                run(-3.0, 14.2, 0.0),
                run(0.0, 14.2, 0.0),
                run(0.0, 0.0, -9.0),
                run(6.0, 0.0, -14.0),
                pad(8.0, 0.0, -19.0),
                run(8.0, 7.0, -31.0),
                run(16.0, 7.0, -30.0),
            ],
            2,
        ),
        // Furnace Ring, all three rings: out of the north floor alcove, the crucible pad onto
        // the crucible's top, jump off over the slag, the south-east floor pad onto tier 1, up
        // the south-east ramp to the rim, round the rim (east and north sides, past the rim
        // gates and round the wall-jump slots), drop to tier 1 and the floor, the north-west
        // floor pad onto tier 1, along it, drop, the west crucible pad, jump off the crucible.
        (
            FURNACE,
            0,
            vec![
                run(-5.0, 0.0, -16.0),
                pad(0.0, 0.0, -11.2),
                run(0.0, 10.0, -3.5),
                run(3.5, 10.0, 0.0),
                run(14.0, 0.0, 0.0),
                run(13.0, 0.0, 7.0),
                pad(8.63, 0.0, 8.63),
                run(17.7, 7.0, 17.7),
                run(12.0, 7.0, 26.0),
                run(24.4, 12.8, 14.5),
                run(29.7, 14.0, 18.4),
                run(36.0, 14.0, 10.0),
                run(36.0, 14.0, -12.0),
                run(27.0, 14.0, -27.0),
                run(10.0, 14.0, -35.0),
                run(-10.0, 14.0, -35.0),
                run(0.0, 7.0, -25.0),
                run(0.0, 0.0, -16.0),
                pad(-8.63, 0.0, -8.63),
                run(-17.7, 7.0, -17.7),
                run(-25.0, 7.0, 0.0),
                run(-15.0, 0.0, 0.0),
                pad(-11.2, 0.0, 0.0),
                run(-3.5, 10.0, 0.0),
                run(0.0, 10.0, -3.5),
                run(0.0, 0.0, -15.0),
            ],
            4,
        ),
    ]
}

#[test]
fn arena_laps_are_traversable() {
    let laps = arena_laps();
    for (name, _) in arenas() {
        assert!(laps.iter().any(|l| l.0 == name), "{name}: no traversal lap");
    }
    for (name, start, lap, min_pads) in laps {
        let level = load(name);
        let col = collision(&level, None);
        let mut bot = Bot::new(&level, &col, level.spawns[start].position);
        bot.follow(&lap, 12.0)
            .unwrap_or_else(|e| panic!("{name} lap: {e}"));
        assert!(
            bot.pads.len() >= min_pads,
            "{name}: the lap took pads {:?}, expected {min_pads}+",
            bot.pads
        );
        println!("{name} lap: {:.1} s (pads {:?})", bot.seconds(), bot.pads);
    }
}

// ------------------------------------------------------------------------------------------
// Advanced-tech shortcuts (scripted inputs): the rewards for good movement really exist.

fn input_towards(dir: Vec3, buttons: Buttons) -> PlayerInput {
    PlayerInput {
        forward: if dir == Vec3::ZERO { 0 } else { 127 },
        yaw: f32::atan2(-dir.x, -dir.z),
        buttons,
        ..Default::default()
    }
}

/// Wall-jumps up a shaft from `start` (right next to one of its walls), then moves along `exit` once above `top`; returns the
/// state once standing again.
fn climb_shaft(
    level: &LevelDef,
    col: &LevelCollision,
    start: Vec3,
    exit: Vec3,
    top: f32,
) -> MoveState {
    let t = MovementTuning::default();
    let env = MoveEnv {
        world: &col.world,
        jump_pads: &level.jump_pads,
    };
    let mut s = MoveState::spawn(start);
    let mut push = Vec3::ZERO;
    let mut prev_jump = false;
    for seq in 1..=600u32 {
        let mut jump = false;
        if s.on_ground && seq > 5 && s.position.y < top - 1.0 {
            jump = !prev_jump;
        } else if !s.on_ground
            && s.wall_normal != Vec3::ZERO
            && s.velocity.y < 3.0
            && s.position.y < top + 0.5
        {
            // Kick off the wall towards the opposite one.
            push = Vec3::new(s.wall_normal.x, 0.0, s.wall_normal.z).normalize_or_zero();
            jump = !prev_jump;
        }
        let dir = if s.position.y > top + 0.3 { exit } else { push };
        let mut input = input_towards(dir, if jump { Buttons::JUMP } else { Buttons::NONE });
        input.seq = seq;
        prev_jump = jump;
        movement::step(&mut s, &input, &env, &t, DT);
        if s.on_ground && s.position.y > top - 0.1 {
            break;
        }
    }
    s
}

#[test]
fn advanced_shortcuts_work() {
    let t = MovementTuning::default();
    let level = load(CRUCIBLE);
    let col = collision(&level, Some(true));
    let env = MoveEnv {
        world: &col.world,
        jump_pads: &level.jump_pads,
    };

    // Smelting Floor: dash-jump from the west terrace onto the perch above the slag vat.
    let mut s = MoveState::spawn(Vec3::new(-18.0, 4.0, -43.0));
    let east = Vec3::X;
    let mut seq = 0;
    let mut tick = |s: &mut MoveState, b: Buttons| {
        seq += 1;
        let mut input = input_towards(east, b);
        input.seq = seq;
        movement::step(s, &input, &env, &t, DT)
    };
    for _ in 0..5 {
        tick(&mut s, Buttons::NONE);
    }
    tick(&mut s, Buttons::DASH);
    let ev = tick(&mut s, Buttons::JUMP);
    assert!(ev.dash_jumped, "dash-jump off the terrace");
    for _ in 0..120 {
        if tick(&mut s, Buttons::NONE).landed.is_some() {
            break;
        }
    }
    assert!(
        s.on_ground && (s.position.y - 5.6).abs() < 0.05 && s.position.x.abs() < 2.0,
        "dash-jump should land on the vat perch, got {}",
        s.position
    );

    // Bell Loft: wall-jump up the shaft beside the fin onto the west balcony (6 m up).
    let s = climb_shaft(
        &level,
        &col,
        Vec3::new(-43.5, 10.0, -177.0),
        Vec3::NEG_Z,
        16.0,
    );
    assert!(
        s.on_ground && (s.position.y - 16.0).abs() < 0.05 && s.position.z < -180.0,
        "wall-jump shaft should reach the balcony, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );

    // Spillway: a slide down the 24.6° chute builds well past running speed.
    let mut s = MoveState::spawn(Vec3::new(3.0, 22.0, -208.5));
    let mut top_speed: f32 = 0.0;
    for seq in 1..=600 {
        let b = if seq > 5 {
            Buttons::CROUCH
        } else {
            Buttons::NONE
        };
        let mut input = input_towards(east, b);
        input.seq = seq;
        movement::step(&mut s, &input, &env, &t, DT);
        top_speed = top_speed.max(s.horizontal_speed());
        assert!(!in_kill_volume(&level, s.position));
        if s.position.x > 62.0 {
            break;
        }
    }
    assert!(
        s.position.x > 62.0 && s.position.y.abs() < 0.1,
        "slid to the bottom: {}",
        s.position
    );
    assert!(
        top_speed > 40.0,
        "spillway slide reached only {top_speed:.1} m/s"
    );
    println!("spillway slide: {top_speed:.1} m/s at the bottom");

    // Cistern Nine: wall-jump between a tower and its fin onto the tower top.
    let arena = load(ARENA);
    let acol = collision(&arena, None);
    let s = climb_shaft(
        &arena,
        &acol,
        Vec3::new(-27.5, 0.0, -30.25),
        Vec3::NEG_X,
        6.0,
    );
    assert!(
        s.on_ground && (s.position.y - 6.0).abs() < 0.05 && s.position.x < -28.0,
        "arena fin shaft should reach the tower, got {}",
        s.position
    );
}

/// Slides from `start` along `dir` (crouch held from the 5th tick) until `done` or 600 ticks;
/// returns the state, the slowest horizontal speed once sliding, and the seconds taken.
fn slide_run(
    level: &LevelDef,
    col: &LevelCollision,
    start: Vec3,
    dir: Vec3,
    done: impl Fn(Vec3) -> bool,
) -> (MoveState, f32, f32) {
    let t = MovementTuning::default();
    let env = MoveEnv {
        world: &col.world,
        jump_pads: &level.jump_pads,
    };
    let mut s = MoveState::spawn(start);
    let mut slowest = f32::MAX;
    let mut ticks = 0;
    for seq in 1..=600u32 {
        let b = if seq > 5 {
            Buttons::CROUCH
        } else {
            Buttons::NONE
        };
        let mut input = input_towards(dir, b);
        input.seq = seq;
        movement::step(&mut s, &input, &env, &t, DT);
        assert!(
            !in_kill_volume(level, s.position),
            "{}: slide from {start} hit a kill volume at {}",
            level.name,
            s.position
        );
        if s.is_sliding() {
            slowest = slowest.min(s.horizontal_speed());
        }
        ticks = seq;
        if done(s.position) {
            break;
        }
    }
    (s, slowest, ticks as f32 * DT)
}

/// Runs (no crouch) from `start` along `dir` for `secs`; returns the final state.
fn run_for(level: &LevelDef, col: &LevelCollision, start: Vec3, dir: Vec3, secs: f32) -> MoveState {
    let t = MovementTuning::default();
    let env = MoveEnv {
        world: &col.world,
        jump_pads: &level.jump_pads,
    };
    let mut s = MoveState::spawn(start);
    for seq in 1..=(secs / DT) as u32 {
        let mut input = input_towards(dir, Buttons::NONE);
        input.seq = seq;
        movement::step(&mut s, &input, &env, &t, DT);
    }
    s
}

/// Like [`climb_shaft`], but once above `top` it steers towards `land` (x, z) and lets go of
/// the stick when it's above it, so it also lands on narrow ledges.
fn climb_shaft_to(
    level: &LevelDef,
    col: &LevelCollision,
    start: Vec3,
    land: Vec3,
    top: f32,
) -> MoveState {
    let t = MovementTuning::default();
    let env = MoveEnv {
        world: &col.world,
        jump_pads: &level.jump_pads,
    };
    let mut s = MoveState::spawn(start);
    let mut push = Vec3::ZERO;
    let mut prev_jump = false;
    for seq in 1..=600u32 {
        let mut jump = false;
        if s.on_ground && seq > 5 && s.position.y < top - 1.0 {
            jump = !prev_jump;
        } else if !s.on_ground
            && s.wall_normal != Vec3::ZERO
            && s.velocity.y < 3.0
            && s.position.y < top + 0.5
        {
            push = Vec3::new(s.wall_normal.x, 0.0, s.wall_normal.z).normalize_or_zero();
            jump = !prev_jump;
        }
        let to = Vec3::new(land.x - s.position.x, 0.0, land.z - s.position.z);
        let dir = if s.position.y <= top + 0.3 {
            push
        } else if to.length() > 0.5 {
            to.normalize()
        } else {
            Vec3::ZERO
        };
        let mut input = input_towards(dir, if jump { Buttons::JUMP } else { Buttons::NONE });
        input.seq = seq;
        prev_jump = jump;
        movement::step(&mut s, &input, &env, &t, DT);
        if s.on_ground && s.position.y > top - 0.1 {
            break;
        }
    }
    s
}

#[test]
fn knell_yard_tech() {
    let level = load(KNELL);
    let col = collision(&level, None);

    // Wall-jump shaft between a kiln and its flask rack, up onto the kiln roof (7.5 m). The
    // pinwheel has four identical copies; this is the north-east one.
    let s = climb_shaft_to(
        &level,
        &col,
        Vec3::new(21.0, 0.0, -13.9),
        Vec3::new(21.0, 0.0, -20.0),
        7.5,
    );
    assert!(
        s.on_ground && (s.position.y - 7.5).abs() < 0.05 && s.position.z < -16.8,
        "kiln shaft should reach the kiln roof, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );

    // Casting tables are slide-only: running into one stops you, sliding passes under the
    // 1.2 m gap at full slide speed.
    let start = Vec3::new(3.5, 0.0, -15.0);
    let s = run_for(&level, &col, start, Vec3::X, 1.0);
    assert!(
        s.position.x < 7.0 && s.position.y < 0.1,
        "a runner should be stopped by the casting table, got {}",
        s.position
    );
    let (s, slowest, secs) = slide_run(&level, &col, start, Vec3::X, |p| p.x > 11.5);
    assert!(
        s.position.x > 11.5 && s.position.y.abs() < 0.1,
        "slide under the casting table, got {}",
        s.position
    );
    assert!(
        slowest >= 23.9,
        "the slide under the table slowed to {slowest:.1} m/s"
    );
    println!("knell yard: casting-table slide {secs:.2} s");
}

#[test]
fn ingot_run_tech() {
    let level = load(INGOT);
    let col = collision(&level, None);

    // Wall-jump shaft between the ingot racks up onto the crane gantry (7 m).
    let s = climb_shaft_to(
        &level,
        &col,
        Vec3::new(20.4, 0.0, -13.5),
        Vec3::new(21.75, 0.0, -18.0),
        7.0,
    );
    assert!(
        s.on_ground && (s.position.y - 7.0).abs() < 0.05 && s.position.z < -16.0,
        "rack shaft should reach the gantry, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );
    // Wall-jump shaft between the furnace and the buttress up onto the end walkway (7 m).
    let s = climb_shaft_to(
        &level,
        &col,
        Vec3::new(45.5, 0.0, -10.1),
        Vec3::new(41.0, 0.0, -8.75),
        7.0,
    );
    assert!(
        s.on_ground && (s.position.y - 7.0).abs() < 0.05 && s.position.x < 43.0,
        "furnace shaft should reach the end walkway, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );

    // The ingot trough: one slide from the west tapping floor to the east one, under both
    // cooling beds and through the duct below the slag, never slower than a slide.
    let (s, slowest, secs) = slide_run(&level, &col, Vec3::new(-46.0, 0.0, 18.0), Vec3::X, |p| {
        p.x > 44.5
    });
    assert!(
        s.position.x > 44.5 && s.position.y.abs() < 0.1,
        "trough slide should reach the east end, got {}",
        s.position
    );
    assert!(
        slowest >= 23.9,
        "the trough slide slowed to {slowest:.1} m/s"
    );
    assert!(secs < 4.5, "the end-to-end slide took {secs:.2} s");
    println!("ingot run: end-to-end trough slide {secs:.2} s (slowest {slowest:.1} m/s)");
}

#[test]
fn drowned_chandlery_tech() {
    let level = load(DROWNED);
    let col = collision(&level, Some(true));
    let (east, west, north) = (Vec3::X, Vec3::NEG_X, Vec3::NEG_Z);

    // The Cistern: dash-jump across the pool from the west walk to the east walk (a shortcut
    // to the east ramp; the pool snuffs a short jump).
    let mut d = Script::new(&level, &col, Vec3::new(3.0, 4.0, -33.0));
    for _ in 0..5 {
        d.tick(east, Buttons::NONE);
    }
    d.tick(east, Buttons::DASH);
    assert!(
        d.tick(east, Buttons::JUMP).dash_jumped,
        "dash-jump off the west walk"
    );
    assert!(d.until_landed(east, Buttons::NONE, 120), "lands somewhere");
    assert!(
        d.s.position.x > 24.0 && (d.s.position.y - 4.0).abs() < 0.2,
        "the dash-jump should clear the pool, landed at {}",
        d.s.position
    );

    // The Wax Press: jump, slam, bounce off the floor onto the press bed (4 m up).
    let mut b = Script::new(&level, &col, Vec3::new(-31.0, 11.0, -41.0));
    for _ in 0..5 {
        b.tick(Vec3::ZERO, Buttons::NONE);
    }
    b.tick(Vec3::ZERO, Buttons::JUMP);
    while b.s.velocity.y > 0.0 {
        b.tick(Vec3::ZERO, Buttons::NONE);
    }
    assert!(
        b.tick(Vec3::ZERO, Buttons::CROUCH).slam_started,
        "slam from the jump's apex"
    );
    assert!(
        (0..60).any(|_| b.tick(Vec3::ZERO, Buttons::JUMP).slam_bounced),
        "slam bounce off the floor"
    );
    let mut landed = false;
    for _ in 0..120 {
        let dir = if b.s.position.y > 15.3 {
            north
        } else {
            Vec3::ZERO
        };
        if b.tick(dir, Buttons::NONE).landed.is_some() {
            landed = true;
            break;
        }
    }
    assert!(
        landed && (b.s.position.y - 15.0).abs() < 0.05 && b.s.position.z < -44.0,
        "the slam bounce should land on the press bed, got {}",
        b.s.position
    );

    // The Wax Press: a running jump onto a wax floe, braking in the air over it.
    let mut f = Script::new(&level, &col, Vec3::new(-39.5, 11.0, -47.0));
    let mut jumped = false;
    let mut landed = false;
    for _ in 0..240 {
        let x = f.s.position.x;
        let (dir, buttons) = if !jumped {
            (
                west,
                if x < -41.8 {
                    Buttons::JUMP
                } else {
                    Buttons::NONE
                },
            )
        } else if x > -44.0 {
            (west, Buttons::NONE)
        } else if f.s.velocity.x < -0.5 {
            (east, Buttons::NONE)
        } else {
            (Vec3::ZERO, Buttons::NONE)
        };
        let ev = f.tick(dir, buttons);
        jumped |= ev.jumped;
        if jumped && ev.landed.is_some() {
            landed = true;
            break;
        }
    }
    assert!(
        landed && (f.s.position.y - 11.2).abs() < 0.05 && f.s.position.x < -44.6,
        "the hop should land on the floe, got {}",
        f.s.position
    );
}

#[test]
fn wickmarket_tech() {
    let level = load(WICKMARKET);
    let col = collision(&level, None);

    // Slam bounce from the cobbles onto a stall's awning (3.6 m).
    let mut b = Script::new(&level, &col, Vec3::new(15.8, 0.0, -18.0));
    for _ in 0..5 {
        b.tick(Vec3::ZERO, Buttons::NONE);
    }
    b.tick(Vec3::ZERO, Buttons::JUMP);
    while b.s.velocity.y > 0.0 {
        b.tick(Vec3::ZERO, Buttons::NONE);
    }
    assert!(
        b.tick(Vec3::ZERO, Buttons::CROUCH).slam_started,
        "slam from the jump's apex"
    );
    assert!(
        (0..60).any(|_| b.tick(Vec3::ZERO, Buttons::JUMP).slam_bounced),
        "slam bounce off the cobbles"
    );
    let mut landed = false;
    for _ in 0..120 {
        // Steer over the awning once above it, then brake.
        let dir = if b.s.position.y > 3.9 && b.s.position.x > 12.8 {
            Vec3::NEG_X
        } else if b.s.velocity.x < -0.5 {
            Vec3::X
        } else {
            Vec3::ZERO
        };
        if b.tick(dir, Buttons::NONE).landed.is_some() {
            landed = true;
            break;
        }
    }
    assert!(
        landed && (b.s.position.y - 3.6).abs() < 0.05,
        "the slam bounce should land on the awning, got {}",
        b.s.position
    );

    // Dash-jump from the gallows' beam over the market onto the guild walk (31 m north,
    // 7 m down, over the parapet).
    let north = Vec3::NEG_Z;
    let mut d = Script::new(&level, &col, Vec3::new(-2.0, 14.2, 0.0));
    for _ in 0..3 {
        d.tick(north, Buttons::NONE);
    }
    d.tick(north, Buttons::DASH);
    assert!(
        d.tick(north, Buttons::JUMP).dash_jumped,
        "dash-jump off the beam"
    );
    assert!(d.until_landed(north, Buttons::NONE, 120), "lands somewhere");
    assert!(
        (d.s.position.y - 7.0).abs() < 0.05 && d.s.position.z < -26.0,
        "the dash-jump should land on the guild walk, got {}",
        d.s.position
    );
}

#[test]
fn tallow_works_tech() {
    let level = load(TALLOW);
    let col = collision(&level, None);

    // Wall-jump shaft between the mezzanine's solid end and the press house, from the floor
    // onto the mezzanine (6 m).
    let s = climb_shaft_to(
        &level,
        &col,
        Vec3::new(-28.0, 0.0, -18.55),
        Vec3::new(-28.0, 0.0, -22.5),
        6.0,
    );
    assert!(
        s.on_ground && (s.position.y - 6.0).abs() < 0.05 && s.position.z < -19.2,
        "the shaft should reach the mezzanine, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );

    // The belt is a slide: from the loft down to the floor it builds well past slide speed.
    let mut b = Script::new(&level, &col, Vec3::new(-25.5, 12.0, -14.0));
    let mut top_speed: f32 = 0.0;
    for tick in 0..300 {
        b.tick(
            Vec3::X,
            if tick > 3 {
                Buttons::CROUCH
            } else {
                Buttons::NONE
            },
        );
        top_speed = top_speed.max(b.s.horizontal_speed());
        if b.s.position.x > 12.0 {
            break;
        }
    }
    assert!(
        b.s.position.x > 12.0 && b.s.position.y.abs() < 0.1,
        "slid down the belt onto the floor: {}",
        b.s.position
    );
    assert!(
        top_speed > 33.0,
        "the belt slide built only {top_speed:.1} m/s"
    );
    println!("tallow works: belt slide {top_speed:.1} m/s");
}

#[test]
fn furnace_ring_tech() {
    let level = load(FURNACE);
    let col = collision(&level, None);

    // Wall-jump slot in the tier 2 face (north side): from tier 1 (y 7) up onto the rim (14).
    let s = climb_shaft_to(
        &level,
        &col,
        Vec3::new(4.45, 7.0, -31.0),
        Vec3::new(5.75, 0.0, -35.0),
        14.0,
    );
    assert!(
        s.on_ground && (s.position.y - 14.0).abs() < 0.05 && s.position.z < -32.0,
        "the slot should reach the rim, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );

    // Dash-jump from the rim's edge across the pit onto the crucible's top (y 10), beside its
    // flue.
    let south = Vec3::Z;
    let mut d = Script::new(&level, &col, Vec3::new(3.5, 14.0, -31.0));
    for _ in 0..5 {
        d.tick(south, Buttons::NONE);
    }
    d.tick(south, Buttons::DASH);
    assert!(
        d.tick(south, Buttons::JUMP).dash_jumped,
        "dash-jump off the rim"
    );
    assert!(d.until_landed(south, Buttons::NONE, 120), "lands somewhere");
    assert!(
        (d.s.position.y - 10.0).abs() < 0.1 && d.s.position.z.abs() < 3.6,
        "the dash-jump should land on the crucible's top, got {}",
        d.s.position
    );
}

/// Scripted input on a level: one tick at a time towards a direction with some buttons.
struct Script<'a> {
    s: MoveState,
    env: MoveEnv<'a>,
    t: MovementTuning,
    seq: u32,
    level: &'a LevelDef,
}

impl<'a> Script<'a> {
    fn new(level: &'a LevelDef, col: &'a LevelCollision, start: Vec3) -> Self {
        Self {
            s: MoveState::spawn(start),
            env: MoveEnv {
                world: &col.world,
                jump_pads: &level.jump_pads,
            },
            t: MovementTuning::default(),
            seq: 0,
            level,
        }
    }

    fn tick(&mut self, dir: Vec3, buttons: Buttons) -> MoveEvents {
        self.seq += 1;
        let mut input = input_towards(dir, buttons);
        input.seq = self.seq;
        let ev = movement::step(&mut self.s, &input, &self.env, &self.t, DT);
        assert!(
            !in_kill_volume(self.level, self.s.position),
            "scripted move fell into a kill volume at {}",
            self.s.position
        );
        ev
    }

    /// Ticks until landing (at most `max` ticks); true if it landed.
    fn until_landed(&mut self, dir: Vec3, buttons: Buttons, max: u32) -> bool {
        (0..max).any(|_| self.tick(dir, buttons).landed.is_some())
    }
}

#[test]
fn spire_tech_shortcuts_work() {
    let level = load(SPIRE);
    let col = collision(&level, Some(true));

    // Casting Floor: wall-jump up the 3.5 m chimney between the north wall and the mould
    // rack, then step west onto the balcony (7 m up).
    let s = climb_shaft(&level, &col, Vec3::new(-11.5, 0.0, 11.5), Vec3::NEG_X, 7.0);
    assert!(
        s.on_ground && (s.position.y - 7.0).abs() < 0.05 && s.position.x < -16.0,
        "the chimney should reach the west balcony, got {} (wall jumps used {})",
        s.position,
        s.wall_jumps_used
    );

    // Casting Floor: jump, slam, bounce off the floor and land on top of a bell mould (5.5 m).
    let mut b = Script::new(&level, &col, Vec3::new(-11.0, 0.0, 30.0));
    let north = Vec3::NEG_Z;
    for _ in 0..5 {
        b.tick(Vec3::ZERO, Buttons::NONE);
    }
    b.tick(Vec3::ZERO, Buttons::JUMP);
    while b.s.velocity.y > 0.0 {
        b.tick(Vec3::ZERO, Buttons::NONE);
    }
    assert!(
        b.tick(Vec3::ZERO, Buttons::CROUCH).slam_started,
        "slam from the jump's apex"
    );
    let mut bounced = false;
    for _ in 0..60 {
        // Jump held from just before the impact: buffered into the bounce.
        if b.tick(Vec3::ZERO, Buttons::JUMP).slam_bounced {
            bounced = true;
            break;
        }
    }
    assert!(bounced, "slam bounce off the floor");
    let mut landed = false;
    for _ in 0..120 {
        let dir = if b.s.position.y > 5.8 {
            north
        } else {
            Vec3::ZERO
        };
        if b.tick(dir, Buttons::NONE).landed.is_some() {
            landed = true;
            break;
        }
    }
    assert!(
        landed && (b.s.position.y - 5.5).abs() < 0.05 && b.s.position.z < 29.0,
        "slam bounce should land on the mould, got {}",
        b.s.position
    );

    // The Rood: slide down the wax run and slide-jump the chasm from its lip, straight onto
    // the far floor (no bridge).
    let east = Vec3::X;
    let mut r = Script::new(&level, &col, Vec3::new(-51.0, 26.0, -58.0));
    let mut top_speed: f32 = 0.0;
    let mut jumped = false;
    for _ in 0..600 {
        let at_lip = r.s.position.x > -24.6;
        let ev = r.tick(
            east,
            if at_lip && !jumped {
                Buttons::CROUCH | Buttons::JUMP
            } else {
                Buttons::CROUCH
            },
        );
        top_speed = top_speed.max(r.s.horizontal_speed());
        jumped |= ev.slide_jumped;
        if jumped && ev.landed.is_some() {
            break;
        }
    }
    assert!(jumped, "slide-jump off the chasm lip");
    assert!(
        r.s.on_ground && (r.s.position.y - 14.0).abs() < 0.05 && r.s.position.x > -9.0,
        "slide-jump should clear the chasm, landed at {}",
        r.s.position
    );
    assert!(
        top_speed > 33.0,
        "the wax run built only {top_speed:.1} m/s"
    );
    println!(
        "wax run slide: {top_speed:.1} m/s, landed at x {:.1}",
        r.s.position.x
    );

    // Wick Stair: sliding climbs the long ramp at slide speed, well ahead of running.
    let west = Vec3::NEG_X;
    let time_up = |buttons: Buttons| {
        let mut w = Script::new(&level, &col, Vec3::new(15.0, 24.0, -31.5));
        for tick in 1..=600u32 {
            w.tick(west, if tick > 2 { buttons } else { Buttons::NONE });
            if w.s.position.x < -19.0 {
                assert!(
                    (w.s.position.y - 36.0).abs() < 0.1,
                    "reached the top landing: {}",
                    w.s.position
                );
                return tick as f32 * DT;
            }
        }
        panic!(
            "never reached the top of the Wick Stair ({buttons:?}): {}",
            w.s.position
        );
    };
    let (slid, ran) = (time_up(Buttons::CROUCH), time_up(Buttons::NONE));
    assert!(
        slid < ran * 0.8,
        "slide-climb {slid:.2} s vs running {ran:.2} s"
    );
    println!("Wick Stair: slide {slid:.2} s, run {ran:.2} s");

    // Belfry: dash-jump from the east gallery onto the great bell's rim (over the shaft).
    let mut d = Script::new(&level, &col, Vec3::new(17.5, 44.0, 0.0));
    for _ in 0..5 {
        d.tick(west, Buttons::NONE);
    }
    d.tick(west, Buttons::DASH);
    assert!(
        d.tick(west, Buttons::JUMP).dash_jumped,
        "dash-jump off the gallery"
    );
    assert!(d.until_landed(west, Buttons::NONE, 120), "lands somewhere");
    assert!(
        (d.s.position.y - 46.0).abs() < 0.05
            && d.s.position.x.abs() <= 7.5
            && d.s.position.x >= 6.0,
        "dash-jump should land on the bell's rim, got {}",
        d.s.position
    );
}

// ------------------------------------------------------------------------------------------
// Grind spawns and PvP spawn fairness

/// The nav class of an enemy definition (flyers have none).
fn nav_class(enemy: &str) -> Option<NavClass> {
    let def = EnemyRegistry::builtin()
        .get(enemy)
        .unwrap_or_else(|| panic!("no enemy '{enemy}'"));
    match def.locomotion {
        Locomotion::Flying { .. } => None,
        Locomotion::Ground => Some(NavClass::for_size(def.radius, def.height)),
    }
}

/// Whether a shot from `from` reaches `to` (nothing that stops bullets in between).
fn sees(world: &CollisionWorld, from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    world
        .cast_ray(from, d.normalize_or_zero(), d.length(), MASK_SHOTS)
        .is_none_or(|h| h.distance >= d.length() - 0.05)
}

/// Compass sector (0 N, 1 E, 2 S, 3 W) of `p` seen from `mid`.
fn sector(mid: Vec3, p: Vec3) -> usize {
    let (dx, dz) = (p.x - mid.x, p.z - mid.z);
    if dz.abs() >= dx.abs() {
        if dz < 0.0 { 0 } else { 2 }
    } else if dx > 0.0 {
        1
    } else {
        3
    }
}

/// Every arena is a grind arena: 8-16 grind spawns, where any Mould fits (the Gaoler's
/// footprint and headroom), on the nav grid with paths into the arena, out of sight of every
/// player spawn and well away from them, spread over 3+ compass sectors.
#[test]
fn grind_spawns_are_valid() {
    let gaoler = nav_class("gaoler").expect("ground enemy");
    for (name, level) in arenas() {
        let col = collision(&level, None);
        let world = &col.world;
        let grid = NavGrid::build(&level, world);
        let mut scratch = NavScratch::default();
        let mut bad = Vec::new();
        let n = level.grind_spawns.len();
        if !(8..=16).contains(&n) {
            bad.push(format!("{n} grind spawns (8-16)"));
        }
        let (lo, hi) = level_bounds(&level);
        let mid = (lo + hi) * 0.5;
        let sectors: HashSet<usize> = level.grind_spawns.iter().map(|&p| sector(mid, p)).collect();
        if sectors.len() < 3 {
            bad.push(format!("grind spawns only in compass sectors {sectors:?}"));
        }
        // Player spawns on the nav grid (narrow footprint), and their nearest wide-footprint
        // node (spawns on ledges a Gaoler can't stand on are skipped for it).
        let player_nodes: Vec<Option<u32>> = level
            .spawns
            .iter()
            .map(|s| grid.node_at(s.position, 0.6, NavClass::SMALL, &[]))
            .collect();
        let player_wide: Vec<Option<u32>> = level
            .spawns
            .iter()
            .map(|s| grid.nearest_node(s.position, 10, gaoler, &[]))
            .collect();
        for (i, &p) in level.grind_spawns.iter().enumerate() {
            let what = format!("grind spawn {i} at {p}");
            let ground = ground_below(world, p, 0.6);
            if !ground.is_some_and(|d| d.abs() < 0.3) {
                bad.push(format!("{what}: no ground right below ({ground:?})"));
            }
            if in_kill_volume(&level, p + Vec3::Y * 0.1) {
                bad.push(format!("{what}: in a kill volume"));
            }
            let (r, h) = enemy_size("gaoler");
            let (c, hh) = capsule_at(p + Vec3::Y * 0.05, h, r);
            if world.intersects_capsule(c, hh, r, MASK_MOVEMENT) {
                bad.push(format!("{what}: no room for a gaoler"));
            }
            if let Err(q) = floor_all_around(world, p, 1.5, 0.3) {
                bad.push(format!("{what}: no floor at {q}"));
            }
            for (j, o) in level.grind_spawns.iter().enumerate().skip(i + 1) {
                if p.distance(*o) < 4.0 {
                    bad.push(format!("{what}: crowded with grind spawn {j}"));
                }
            }
            // Out of sight of every player spawn (eye to the Mould's feet, middle and head),
            // and not right next to one.
            for (j, s) in level.spawns.iter().enumerate() {
                let eye = s.position + Vec3::Y * 1.6;
                for dy in [0.3, 1.5, 3.0] {
                    if sees(world, eye, p + Vec3::Y * dy) {
                        bad.push(format!(
                            "{what}: visible from player spawn {j} at {}",
                            s.position
                        ));
                        break;
                    }
                }
                let d = s.position.distance(p);
                if d < 12.0 {
                    bad.push(format!("{what}: only {d:.1} m from player spawn {j}"));
                }
            }
            // Navigable: a Gaoler can stand there and walk out to a player spawn; narrow
            // Moulds walk to at least half the player spawns, and back from at least one (the
            // gate isn't a one-way trap; drops off ledges are one-way, so not from all).
            let Some(node) = grid.node_at(p, 0.6, gaoler, &[]) else {
                bad.push(format!("{what}: not on the nav grid for a gaoler"));
                continue;
            };
            let wide_out = player_wide.iter().flatten().any(|&goal| {
                matches!(
                    grid.find_path(&mut scratch, node, goal, gaoler, &[], 200_000),
                    PathResult::Found(_)
                )
            });
            if !wide_out {
                bad.push(format!("{what}: a gaoler can't walk to any player spawn"));
            }
            let small = grid
                .node_at(p, 0.6, NavClass::SMALL, &[])
                .expect("a wide node is a narrow one too");
            let (mut there, mut back) = (0, 0);
            for &goal in player_nodes.iter().flatten() {
                let found = |r: PathResult| matches!(r, PathResult::Found(_));
                there +=
                    found(grid.find_path(&mut scratch, small, goal, NavClass::SMALL, &[], 200_000))
                        as usize;
                back +=
                    found(grid.find_path(&mut scratch, goal, small, NavClass::SMALL, &[], 200_000))
                        as usize;
            }
            if there * 2 < level.spawns.len() || back == 0 {
                bad.push(format!(
                    "{what}: walkers reach {there} of {} player spawns, come back from {back}",
                    level.spawns.len()
                ));
            }
        }
        assert!(bad.is_empty(), "{name}: grind spawns:\n{}", bad.join("\n"));
        println!("{name}: {n} grind spawns in sectors {sectors:?}");
    }
}

/// Arenas built with the PvP spawn rule: no player spawn can see another (eye to chest).
const FAIR_SPAWNS: [&str; 3] = [FURNACE, TALLOW, WICKMARKET];

#[test]
fn pvp_spawns_cannot_see_each_other() {
    for name in FAIR_SPAWNS {
        let level = load(name);
        let col = collision(&level, None);
        let mut bad = Vec::new();
        for (i, a) in level.spawns.iter().enumerate() {
            for (j, b) in level.spawns.iter().enumerate().skip(i + 1) {
                let (ea, eb) = (a.position + Vec3::Y * 1.6, b.position + Vec3::Y * 1.6);
                let (ca, cb) = (a.position + Vec3::Y * 1.1, b.position + Vec3::Y * 1.1);
                if sees(&col.world, ea, cb) || sees(&col.world, eb, ca) {
                    bad.push(format!(
                        "spawn {i} {} sees spawn {j} {}",
                        a.position, b.position
                    ));
                }
            }
        }
        assert!(
            bad.is_empty(),
            "{name}: spawns in sight of each other:\n{}",
            bad.join("\n")
        );
    }
}

/// Campaign encounter spawns are connected on the nav grid for their Mould's class (doors
/// open): every walker can walk to its rally point (the Gaoler with its wide footprint, to the
/// nearest spot by the rally it fits) and players can walk to it. Censers may sit on pad-only
/// ledges (they only need to be on the grid).
#[test]
fn campaign_enemy_spawns_are_on_connected_nav() {
    for name in CAMPAIGNS {
        let level = load(name);
        let col = collision(&level, Some(true));
        let grid = NavGrid::build(&level, &col.world);
        let mut scratch = NavScratch::default();
        let mut bad = Vec::new();
        for (ei, e) in level.encounters.iter().enumerate() {
            let rally = e.rally.expect("rally").position;
            let rally_node = grid
                .node_at(rally, 0.6, NavClass::SMALL, &[])
                .unwrap_or_else(|| panic!("{name}: encounter {} rally off the grid", ei + 1));
            for (wi, w) in e.waves.iter().enumerate() {
                for s in &w.spawns {
                    let Some(class) = nav_class(&s.enemy) else {
                        continue;
                    };
                    let what = format!(
                        "encounter {} wave {} {} at {}",
                        ei + 1,
                        wi + 1,
                        s.enemy,
                        s.position
                    );
                    let Some(start) = grid.node_at(s.position, 0.6, class, &[]) else {
                        bad.push(format!("{what}: not on the grid for {class:?}"));
                        continue;
                    };
                    if s.enemy == "censer" {
                        continue;
                    }
                    let found = |r: PathResult| matches!(r, PathResult::Found(_));
                    let goal = grid.nearest_node(rally, 8, class, &[]);
                    let to = goal.is_some_and(|goal| {
                        found(grid.find_path(&mut scratch, start, goal, class, &[], 200_000))
                    });
                    let small = grid
                        .node_at(s.position, 0.6, NavClass::SMALL, &[])
                        .expect("a node for a class is a narrow node too");
                    let from = found(grid.find_path(
                        &mut scratch,
                        rally_node,
                        small,
                        NavClass::SMALL,
                        &[],
                        200_000,
                    ));
                    if !to || !from {
                        bad.push(format!(
                            "{what}: can't walk to the rally point ({to}) or be reached from it ({from})"
                        ));
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "{name}: enemy spawns off the nav:\n{}",
            bad.join("\n")
        );
    }
}

// ------------------------------------------------------------------------------------------
// Chaos

/// Tiny deterministic RNG.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0
    }
}

#[test]
fn chaotic_players_never_leave_the_level() {
    let t = MovementTuning::default();
    for name in shipped() {
        let level = load(&name);
        let (lo, hi) = level_bounds(&level);
        for doors in [Some(true), Some(false)] {
            let col = collision(&level, doors);
            let env = MoveEnv {
                world: &col.world,
                jump_pads: &level.jump_pads,
            };
            let starts: Vec<Vec3> = level
                .spawns
                .iter()
                .map(|s| s.position)
                .chain(level.checkpoints.iter().map(|c| c.spawn.position))
                .collect();
            for (si, &start) in starts.iter().enumerate() {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((si as u64 + 1) * 7919));
                let mut s = MoveState::spawn(start);
                let mut input = PlayerInput::default();
                for seq in 1..=60 * 40 {
                    if seq % 20 == 1 {
                        input.forward = PlayerInput::quantize_axis(rng.unit() * 2.0 - 0.6);
                        input.right = PlayerInput::quantize_axis(rng.unit() * 2.0 - 1.0);
                        input.yaw = rng.unit() * std::f32::consts::TAU;
                    }
                    let mut b = Buttons::NONE;
                    let r = rng.next() % 100;
                    if r < 12 {
                        b = b | Buttons::JUMP;
                    }
                    if (12..16).contains(&r) {
                        b = b | Buttons::DASH;
                    }
                    if (16..30).contains(&r) {
                        b = b | Buttons::CROUCH;
                    }
                    input.buttons = b;
                    input.seq = seq;
                    movement::step(&mut s, &input, &env, &t, DT);
                    let p = s.position;
                    if in_kill_volume(&level, p) {
                        break;
                    }
                    assert!(
                        p.cmpge(lo - Vec3::ONE).all() && p.cmple(hi + Vec3::ONE).all(),
                        "{name}: a chaotic player from {start} escaped to {p} (doors {doors:?})"
                    );
                }
            }
        }
    }
}
