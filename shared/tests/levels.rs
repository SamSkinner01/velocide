//! Checks for the shipped levels in `assets/levels/`:
//!
//! - every level parses and its markup is consistent (materials, doors, enemy names);
//! - campaign / arena structure (spawns, checkpoints, encounters, exit, enemy mix);
//! - player, checkpoint and enemy spawns stand on ground and don't overlap geometry, harriers
//!   hover in open air;
//! - jump pads land somewhere solid;
//! - no z-fighting (coplanar overlapping faces of different materials);
//! - sealing: a flood fill through free space with every door closed never leaves the level
//!   and never connects two arenas (locked rooms really lock);
//! - chaotic players never leave the level;
//! - traversability: waypoint-following bots driven through the real movement code
//!   (`shared::movement::step`) walk the whole critical path from every spawn and every
//!   checkpoint to the exit with doors open, using only running, jumping, sliding and pads.
//!
//! `cargo test -p shared --test levels -- --nocapture` prints route times (par suggestions).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use engine::physics::{CollisionWorld, MASK_MOVEMENT};
use glam::{IVec3, Vec2, Vec3};
use shared::input::{Buttons, PlayerInput};
use shared::level::{Brush, LevelCollision, LevelDef, LevelKind, Volume};
use shared::movement::{self, MoveEnv, MoveEvents, MoveState};
use shared::tuning::MovementTuning;

const DT: f32 = 1.0 / 60.0;
const CAMPAIGN: &str = "crucible_of_veins";
const ARENA: &str = "cistern_nine";
const SHIPPED: [&str; 2] = [CAMPAIGN, ARENA];
const ENEMIES: [&str; 4] = ["gnawer", "censer", "harrier", "gaoler"];
const MATERIALS: [&str; 14] = [
    "concrete", "brick", "metal", "rust", "grate", "flesh", "bone", "lava", "tile", "wood",
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
    for b in level.brushes.iter().chain(level.doors.iter().map(|d| &d.brush)) {
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
        let strict = SHIPPED.contains(&name.as_str());
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
            assert!(MATERIALS.contains(&d.brush.material.as_str()), "{name}: door {} material", d.id);
        }
        for (i, e) in level.encounters.iter().enumerate() {
            for id in e.lock_doors.iter().chain(&e.unlock_doors) {
                assert!(ids.contains(id), "{name}: encounter {i} references missing door {id}");
            }
            assert!(!e.waves.is_empty(), "{name}: encounter {i} has no waves");
            for w in &e.waves {
                assert!(!w.spawns.is_empty(), "{name}: encounter {i} has an empty wave");
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
            assert!((v.max - v.min).min_element() > 0.0, "{name}: empty volume {v:?}");
        }
    }
}

#[test]
fn shipped_levels_fit_the_budget_and_have_a_safety_net() {
    for name in SHIPPED {
        let level = load(name);
        assert!(
            level.brushes.len() < MAX_BRUSHES,
            "{name}: {} brushes (budget {MAX_BRUSHES})",
            level.brushes.len()
        );
        let (lo, hi) = level_bounds(&level);
        // A kill volume under the whole level.
        assert!(
            level.kill_volumes.iter().any(|k| {
                k.min.x <= lo.x && k.min.z <= lo.z && k.max.x >= hi.x && k.max.z >= hi.z && k.max.y <= lo.y
            }),
            "{name}: no kill volume below the whole level ({lo} .. {hi})"
        );
        assert!(!level.lights.is_empty(), "{name}: no lights");
        assert!(!level.environment.music.is_empty(), "{name}: no music track");
    }
}

#[test]
fn campaign_structure() {
    let level = load(CAMPAIGN);
    assert_eq!(level.kind, LevelKind::Campaign);
    assert_eq!(level.spawns.len(), 4, "4 co-op player spawns");
    assert!(level.exit.is_some(), "campaign needs an exit");
    let n = level.encounters.len();
    assert!((4..=5).contains(&n), "4-5 arenas, got {n}");
    assert_eq!(level.checkpoints.len(), n, "one checkpoint per encounter");

    let door = |id: u32| level.doors.iter().find(|d| d.id == id).expect("door exists");
    let mut used = HashSet::new();
    for (i, (e, cp)) in level.encounters.iter().zip(&level.checkpoints).enumerate() {
        assert!(!e.lock_doors.is_empty() && !e.unlock_doors.is_empty(), "encounter {i} locks and unlocks");
        for &id in &e.lock_doors {
            assert!(door(id).starts_open, "encounter {i}: lock door {id} must start open");
            used.insert(id);
        }
        for &id in &e.unlock_doors {
            assert!(!door(id).starts_open, "encounter {i}: exit door {id} must start closed");
            used.insert(id);
        }
        // The checkpoint is touched before the fight starts: its spawn is outside the
        // encounter trigger, and its trigger is next to it.
        assert!(!e.trigger.contains(cp.spawn.position), "checkpoint {i} spawn inside encounter trigger");
        assert!(cp.trigger.contains(cp.spawn.position + Vec3::Y * 0.1), "checkpoint {i} spawn outside its trigger");
        assert!(
            cp.trigger.center().distance(e.trigger.center()) < 60.0,
            "checkpoint {i} is far from encounter {i}"
        );
    }
    for d in &level.doors {
        assert!(used.contains(&d.id), "door {} is not used by any encounter", d.id);
    }

    // Enemy progression: gnawers first, censers next, harriers in a tall room, a gaoler
    // mid-level, and a finale with every type over 3+ waves.
    let kinds = |e: &shared::level::Encounter| -> HashSet<String> {
        e.waves.iter().flat_map(|w| &w.spawns).map(|s| s.enemy.clone()).collect()
    };
    let first = kinds(&level.encounters[0]);
    assert!(first.len() == 1 && first.contains("gnawer"), "arena 1 is gnawers only: {first:?}");
    assert!(kinds(&level.encounters[1]).contains("censer"), "arena 2 introduces censers");
    let harrier_room = level
        .encounters
        .iter()
        .position(|e| kinds(e).contains("harrier"))
        .expect("harriers appear");
    let t = &level.encounters[harrier_room].trigger;
    assert!(t.max.y - t.min.y >= 25.0, "harriers are introduced in a tall room");
    let gaoler = level
        .encounters
        .iter()
        .position(|e| kinds(e).contains("gaoler"))
        .expect("a gaoler appears");
    assert!(gaoler < n - 1, "the first gaoler comes before the finale");
    let last = level.encounters.last().expect("encounters");
    assert!(last.waves.len() >= 3, "finale has 3+ waves");
    for k in ENEMIES {
        assert!(kinds(last).contains(k), "finale includes {k}");
    }
    let total: usize = level.encounters.iter().flat_map(|e| &e.waves).map(|w| w.spawns.len()).sum();
    println!("{CAMPAIGN}: {n} encounters, {total} enemies (single player)");
}

#[test]
fn arena_structure() {
    let level = load(ARENA);
    assert_eq!(level.kind, LevelKind::Arena);
    assert!(level.spawns.len() >= 8, "8+ deathmatch spawns");
    assert!(level.encounters.is_empty() && level.exit.is_none() && level.checkpoints.is_empty());
    assert!(level.doors.is_empty(), "no doors in the arena");
    assert!(level.jump_pads.len() >= 2, "jump pads");
    for (i, a) in level.spawns.iter().enumerate() {
        for b in &level.spawns[i + 1..] {
            let d = a.position.distance(b.position);
            assert!(d >= 10.0, "spawns {} and {} only {d:.1} m apart", a.position, b.position);
        }
    }
    // Spawns spread over the arena: every quadrant has one, and there are 3+ heights.
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        assert!(
            level.spawns.iter().any(|s| s.position.x * sx > 0.0 && s.position.z * sz > 0.0),
            "no spawn in quadrant ({sx}, {sz})"
        );
    }
    let heights: HashSet<i32> = level.spawns.iter().map(|s| s.position.y.round() as i32).collect();
    assert!(heights.len() >= 3, "spawns on 3+ levels: {heights:?}");
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
    assert!(!in_kill_volume(level, p + Vec3::Y * 0.1), "{}: {what} at {p} is in a kill volume", level.name);
    // It settles where it is.
    let env = MoveEnv { world, jump_pads: &[] };
    let mut s = MoveState::spawn(p);
    for seq in 1..=30 {
        let input = PlayerInput { seq, ..Default::default() };
        movement::step(&mut s, &input, &env, &t, DT);
    }
    assert!(s.on_ground, "{}: {what} at {p} doesn't settle on the ground", level.name);
    assert!(s.position.distance(p) < 0.15, "{}: {what} at {p} drifts to {}", level.name, s.position);
}

#[test]
fn player_and_checkpoint_spawns_are_valid() {
    for name in SHIPPED {
        let level = load(name);
        // Closed doors must not overlap spawn points either.
        for doors in [None, Some(false)] {
            let col = collision(&level, doors);
            for (i, s) in level.spawns.iter().enumerate() {
                check_player_spot(&level, &col.world, s.position, &format!("spawn {i}"));
            }
            for (i, c) in level.checkpoints.iter().enumerate() {
                check_player_spot(&level, &col.world, c.spawn.position, &format!("checkpoint {} spawn", i + 1));
            }
        }
        for (i, a) in level.spawns.iter().enumerate() {
            for b in &level.spawns[i + 1..] {
                assert!(a.position.distance(b.position) >= 1.5, "{name}: overlapping spawns");
            }
        }
    }
}

#[test]
fn enemy_spawns_are_valid() {
    let level = load(CAMPAIGN);
    let col = collision(&level, Some(false));
    let world = &col.world;
    for (ei, e) in level.encounters.iter().enumerate() {
        let cp = level.checkpoints[ei].spawn.position;
        for (wi, w) in e.waves.iter().enumerate() {
            for (si, s) in w.spawns.iter().enumerate() {
                let what = format!("encounter {} wave {} {} #{si} at {}", ei + 1, wi + 1, s.enemy, s.position);
                let p = s.position;
                assert!(e.trigger.contains(p + Vec3::Y * 0.1), "{what}: outside the locked room trigger");
                assert!(!in_kill_volume(&level, p), "{what}: in a kill volume");
                assert!(p.distance(cp) >= 8.0, "{what}: too close to the checkpoint");
                let (r, h) = enemy_size(&s.enemy);
                if s.enemy == "harrier" {
                    assert!(
                        !world.intersects_capsule(p, 0.0, r, MASK_MOVEMENT),
                        "{what}: not in open air"
                    );
                    let below = ground_below(world, p, 20.0);
                    assert!(
                        below.is_some_and(|d| (3.5..=9.0).contains(&d)),
                        "{what}: should hover 4-8 m above solid ground, ground is {below:?} below"
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
                    assert!(!world.intersects_capsule(c, hh, r, MASK_MOVEMENT), "{what}: overlaps geometry");
                    // Standing on solid floor all around (not hanging over a ledge).
                    for d in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
                        let edge = ground_below(world, p + d * r * 0.8, 0.6);
                        assert!(edge.is_some_and(|g| g.abs() < 0.3), "{what}: overhangs a ledge");
                    }
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

// ------------------------------------------------------------------------------------------
// Jump pads

#[test]
fn jump_pads_land_on_higher_ground() {
    let t = MovementTuning::default();
    for name in SHIPPED {
        let level = load(name);
        let col = collision(&level, Some(true));
        let env = MoveEnv { world: &col.world, jump_pads: &level.jump_pads };
        for (i, pad) in level.jump_pads.iter().enumerate() {
            let start = Vec3::new(pad.trigger.center().x, pad.trigger.min.y, pad.trigger.center().z);
            assert!(
                ground_below(&col.world, start, 0.3).is_some(),
                "{name}: pad {i} is not on the floor"
            );
            // No input: the pad alone must deliver the player onto a platform above it.
            let mut s = MoveState::spawn(start + Vec3::Y * 0.02);
            let mut fired = false;
            let mut landed = None;
            for seq in 1..=360 {
                let input = PlayerInput { seq, ..Default::default() };
                let air_before = s.air_time;
                let ev = movement::step(&mut s, &input, &env, &t, DT);
                fired |= ev.jump_pad.is_some();
                if fired && air_before > 0.2 && ev.landed.is_some() {
                    landed = Some(s.position);
                    break;
                }
                assert!(!in_kill_volume(&level, s.position), "{name}: pad {i} throws into a kill volume");
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
    for name in SHIPPED {
        let level = load(name);
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
                    let overlap = |k: usize| (a.max[k].min(b.max[k]) - a.min[k].max(b.min[k])).max(0.0);
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
        assert!(bad.is_empty(), "{name}: coplanar faces would z-fight:\n{}", bad.join("\n"));
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
        for d in [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z] {
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
    if a.distance(inside) > b.distance(inside) { a } else { b }
}

fn volume_inside(inner: &Volume, outer: &Volume) -> bool {
    inner.min.cmpge(outer.min).all() && inner.max.cmple(outer.max).all()
}

#[test]
fn encounter_arenas_and_rally_points() {
    let level = load(CAMPAIGN);
    let closed = collision(&level, Some(false));
    let open = collision(&level, Some(true));
    let arenas: Vec<Volume> = level
        .encounters
        .iter()
        .enumerate()
        .map(|(i, e)| e.arena.unwrap_or_else(|| panic!("encounter {} has no arena volume", i + 1)))
        .collect();
    for (i, e) in level.encounters.iter().enumerate() {
        let n = i + 1;
        let arena = arenas[i];
        let rally = e.rally.unwrap_or_else(|| panic!("encounter {n} has no rally point"));
        assert!(volume_inside(&e.trigger, &arena), "encounter {n}: trigger not inside the arena");
        // The rally point is the checkpoint just inside the entry door: a valid standing spot
        // inside the locked room, whether the doors are open or closed.
        assert!(arena.contains(rally.position + Vec3::Y * 0.1), "encounter {n}: rally outside the arena");
        assert_eq!(rally, level.checkpoints[i].spawn, "encounter {n}: rally should be its checkpoint");
        for col in [&closed, &open] {
            check_player_spot(&level, &col.world, rally.position, &format!("encounter {n} rally"));
        }
        // The arena is the room, not the corridors: the far side of its doors, other rooms'
        // checkpoints, the player spawns and the exit are all outside it; every enemy is in it.
        for &id in e.lock_doors.iter().chain(&e.unlock_doors) {
            let p = outside_door(&level, id, e.trigger.center());
            assert!(!arena.contains(p), "encounter {n}: arena reaches past door {id} ({p})");
        }
        for (j, c) in level.checkpoints.iter().enumerate() {
            assert_eq!(arena.contains(c.spawn.position + Vec3::Y * 0.1), i == j, "encounter {n} vs checkpoint {}", j + 1);
        }
        for s in &level.spawns {
            assert!(!arena.contains(s.position + Vec3::Y * 0.1), "encounter {n}: arena contains a player spawn");
        }
        if let Some(x) = level.exit {
            assert!(!arena.contains(x.center()), "encounter {n}: arena contains the exit");
        }
        for s in e.waves.iter().flat_map(|w| &w.spawns) {
            assert!(arena.contains(s.position), "encounter {n}: {} at {} outside the arena", s.enemy, s.position);
        }
        for (j, other) in arenas.iter().enumerate().skip(i + 1) {
            let overlap = arena.min.cmplt(other.max).all() && other.min.cmplt(arena.max).all();
            assert!(!overlap, "arenas {n} and {} overlap", j + 1);
        }
    }
}

#[test]
fn par_times() {
    let campaign = load(CAMPAIGN);
    assert!(
        (180.0..=900.0).contains(&campaign.par_time),
        "campaign par time {} s",
        campaign.par_time
    );
    assert_eq!(load(ARENA).par_time, 0.0, "deathmatch arenas have no par time");
}

#[test]
fn campaign_compartments_are_sealed_when_doors_close() {
    let level = load(CAMPAIGN);
    let col = collision(&level, Some(false));
    let (lo, hi) = level_bounds(&level);
    let bounds = (lo - Vec3::ONE, hi + Vec3::ONE);
    // Compartments with all doors closed: the start area, each arena (its checkpoint is just
    // inside the entry door), and the exit hall.
    let mut points: Vec<(usize, Vec3, String)> = level
        .spawns
        .iter()
        .enumerate()
        .map(|(i, s)| (0, s.position + Vec3::Y, format!("player spawn {i}")))
        .collect();
    for (i, c) in level.checkpoints.iter().enumerate() {
        points.push((i + 1, c.spawn.position + Vec3::Y, format!("checkpoint {}", i + 1)));
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
        if let Some(e) = compartment.checked_sub(1).and_then(|i| level.encounters.get(i)) {
            for &id in e.lock_doors.iter().chain(&e.unlock_doors) {
                forbidden.push((outside_door(id, e.trigger.center()), format!("the far side of door {id}")));
            }
        }
        let cells = flood(&col.world, start, bounds, &forbidden)
            .unwrap_or_else(|e| panic!("compartment {compartment} (from {start}) leaks: {e}"));
        println!("compartment {compartment}: {cells} free cells");
    }
    // With the doors open everything is one connected space.
    let open = collision(&level, Some(true));
    let all: Vec<(Vec3, String)> = vec![(exit_point, "the exit".into())];
    let err = flood(&open.world, level.spawns[0].position + Vec3::Y, bounds, &all)
        .expect_err("with open doors the exit is reachable from the start");
    assert!(err.contains("the exit"), "{err}");
}

#[test]
fn arena_is_sealed() {
    let level = load(ARENA);
    let col = collision(&level, None);
    let (lo, hi) = level_bounds(&level);
    for s in level.spawns.iter().take(1) {
        let cells = flood(&col.world, s.position + Vec3::Y, (lo - Vec3::ONE, hi + Vec3::ONE), &[])
            .unwrap_or_else(|e| panic!("arena leaks: {e}"));
        println!("arena: {cells} free cells");
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
    Wp { p: Vec3::new(x, y, z), act: Act::Run }
}
fn slide(x: f32, y: f32, z: f32) -> Wp {
    Wp { p: Vec3::new(x, y, z), act: Act::Slide }
}
fn pad(x: f32, y: f32, z: f32) -> Wp {
    Wp { p: Vec3::new(x, y, z), act: Act::Pad }
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
        let mut bot = Self {
            level,
            s: MoveState::spawn(start),
            t: MovementTuning::default(),
            env: MoveEnv { world: &col.world, jump_pads: &level.jump_pads },
            seq: 0,
            ticks: 0,
            prev_jump: false,
            prev_crouch: false,
            slow: 0,
            trail: VecDeque::new(),
            entered: Vec::new(),
            marks,
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
        let input = PlayerInput { seq: self.seq, ..input };
        let ev = movement::step(&mut self.s, &input, &self.env, &self.t, DT);
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
        match self.env.world.cast_ray(from, Vec3::NEG_Y, 4.0, MASK_MOVEMENT) {
            Some(h) => h.distance - 0.6,
            None => 4.0,
        }
    }

    fn hurdle_ahead(&self, dir: Vec3) -> bool {
        let knee = self.s.position + Vec3::Y * 0.6;
        let low = self.env.world.cast_ray(knee, dir, 1.6, MASK_MOVEMENT);
        let high = self.env.world.cast_ray(self.s.position + Vec3::Y * 2.6, dir, 2.2, MASK_MOVEMENT);
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
                    return Err(self.fail(format!("waypoint {wi} {:?} {} not reached in {timeout} s", wp.act, wp.p)));
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
                    return Err(self.fail(format!("fell into a kill volume heading to waypoint {wi} {}", wp.p)));
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

/// The campaign's critical path, one segment per checkpoint: segment 0 leads from the
/// player spawns into arena 1; segment i from checkpoint i through arena i into arena i+1
/// (the last one to the exit).
fn campaign_route() -> Vec<Vec<Wp>> {
    vec![
        // Receiving Nave -> Split Gullet (hurdle lane) -> Rendering Floor vestibule.
        vec![run(1.75, 0.0, -3.0), run(1.75, 0.0, -15.0), run(0.0, 0.0, -26.0)],
        // Rendering Floor (around the vat) -> Sluice (jump the lava trench) -> Cloister.
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
    let level = load(CAMPAIGN);
    let col = collision(&level, Some(true));
    let route = campaign_route();
    assert_eq!(route.len(), level.checkpoints.len() + 1);
    let exit = level.exit.expect("exit");

    // Whole route from every player spawn.
    for (si, s) in level.spawns.iter().enumerate() {
        let mut bot = Bot::new(&level, &col, s.position);
        let mut splits = Vec::new();
        for (i, seg) in route.iter().enumerate() {
            bot.follow(seg, 15.0)
                .unwrap_or_else(|e| panic!("spawn {si}, segment {i}: {e}"));
            splits.push(format!("{:.1}", bot.seconds()));
        }
        assert!(exit.contains(bot.s.position + Vec3::Y * 0.1), "spawn {si}: route ends outside the exit");
        let mut expected = Vec::new();
        for i in 1..=level.checkpoints.len() {
            expected.push(format!("checkpoint {i}"));
            expected.push(format!("encounter {i}"));
        }
        expected.push("exit".into());
        assert_eq!(bot.entered, expected, "spawn {si}: triggers entered out of order");
        println!(
            "spawn {si}: exit reached in {:.1} s of pure traversal (splits {})",
            bot.seconds(),
            splits.join(", ")
        );
    }

    // Each checkpoint respawn can make progress on its own.
    for (i, c) in level.checkpoints.iter().enumerate() {
        let mut bot = Bot::new(&level, &col, c.spawn.position);
        for seg in &route[i + 1..] {
            bot.follow(seg, 15.0)
                .unwrap_or_else(|e| panic!("from checkpoint {}: {e}", i + 1));
        }
        assert!(exit.contains(bot.s.position + Vec3::Y * 0.1), "checkpoint {}: route ends outside the exit", i + 1);
    }
}

#[test]
fn campaign_alternate_routes_are_traversable() {
    let level = load(CAMPAIGN);
    let col = collision(&level, Some(true));
    let routes: Vec<(&str, Vec3, Vec<Wp>)> = vec![
        (
            "slide slot (west lane of the Split Gullet)",
            level.spawns[0].position,
            vec![run(-1.75, 0.0, -4.0), slide(-1.75, 0.0, -15.0), run(0.0, 0.0, -26.0)],
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
            "Cathedral east aisle beside the lava pool, east pad to the gallery",
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
    ];
    for (name, start, wps) in routes {
        let mut bot = Bot::new(&level, &col, start);
        bot.follow(&wps, 12.0).unwrap_or_else(|e| panic!("{name}: {e}"));
        println!("alternate route '{name}': {:.1} s", bot.seconds());
    }
}

#[test]
fn arena_loop_is_traversable() {
    let level = load(ARENA);
    let col = collision(&level, None);
    // A full lap: NW tower, north catwalk, NE tower, ramp down, east slide tunnel into the
    // bowl, pad onto the core, south ramp out, SE tower, south catwalk, SW tower, ramp down,
    // west slide tunnel, pad onto the core again.
    let lap = vec![
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
    ];
    let s = level.spawns[0];
    let mut bot = Bot::new(&level, &col, s.position);
    bot.follow(&lap, 12.0).unwrap_or_else(|e| panic!("arena lap: {e}"));
    println!("arena lap: {:.1} s", bot.seconds());
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
fn climb_shaft(level: &LevelDef, col: &LevelCollision, start: Vec3, exit: Vec3, top: f32) -> MoveState {
    let t = MovementTuning::default();
    let env = MoveEnv { world: &col.world, jump_pads: &level.jump_pads };
    let mut s = MoveState::spawn(start);
    let mut push = Vec3::ZERO;
    let mut prev_jump = false;
    for seq in 1..=600u32 {
        let mut jump = false;
        if s.on_ground && seq > 5 && s.position.y < top - 1.0 {
            jump = !prev_jump;
        } else if !s.on_ground && s.wall_normal != Vec3::ZERO && s.velocity.y < 3.0 && s.position.y < top + 0.5 {
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
    let level = load(CAMPAIGN);
    let col = collision(&level, Some(true));
    let env = MoveEnv { world: &col.world, jump_pads: &level.jump_pads };

    // Rendering Floor: dash-jump from the west terrace onto the perch above the lava vat.
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
    let s = climb_shaft(&level, &col, Vec3::new(-43.5, 10.0, -177.0), Vec3::NEG_Z, 16.0);
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
        let b = if seq > 5 { Buttons::CROUCH } else { Buttons::NONE };
        let mut input = input_towards(east, b);
        input.seq = seq;
        movement::step(&mut s, &input, &env, &t, DT);
        top_speed = top_speed.max(s.horizontal_speed());
        assert!(!in_kill_volume(&level, s.position));
        if s.position.x > 62.0 {
            break;
        }
    }
    assert!(s.position.x > 62.0 && s.position.y.abs() < 0.1, "slid to the bottom: {}", s.position);
    assert!(top_speed > 40.0, "spillway slide reached only {top_speed:.1} m/s");
    println!("spillway slide: {top_speed:.1} m/s at the bottom");

    // Cistern Nine: wall-jump between a tower and its fin onto the tower top.
    let arena = load(ARENA);
    let acol = collision(&arena, None);
    let s = climb_shaft(&arena, &acol, Vec3::new(-27.5, 0.0, -30.25), Vec3::NEG_X, 6.0);
    assert!(
        s.on_ground && (s.position.y - 6.0).abs() < 0.05 && s.position.x < -28.0,
        "arena fin shaft should reach the tower, got {}",
        s.position
    );
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
    for name in SHIPPED {
        let level = load(name);
        let (lo, hi) = level_bounds(&level);
        for doors in [Some(true), Some(false)] {
            let col = collision(&level, doors);
            let env = MoveEnv { world: &col.world, jump_pads: &level.jump_pads };
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
