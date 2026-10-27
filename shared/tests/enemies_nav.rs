//! Nav grid on the shipped levels: builds everywhere (timed), every enemy class has room
//! (headroom / footprint per docs/roster.md sizes), encounter spawns and arena centres are
//! connected for the spawned type's class, paths never touch kill volumes (lava), stacked
//! floors are separate nodes, the edge guard refuses lava. Run with `-- --nocapture` for the
//! measurements.

use std::time::Instant;

use glam::Vec3;
use shared::level::{LevelDef, LevelKind};
use shared::sim::EnemyRegistry;
use shared::sim::enemy::Locomotion;
use shared::sim::nav::{MAX_DROP, NavClass, NavGrid, NavScratch, PathResult, STEP_UP, node_flags};

fn load(name: &str) -> (LevelDef, NavGrid, shared::level::LevelCollision) {
    let level = LevelDef::load_named(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    let col = level.build_collision();
    let grid = NavGrid::build(&level, &col.world);
    (level, grid, col)
}

/// The nav class of a definition (flyers have none).
fn class_of(enemy: &str) -> Option<NavClass> {
    let def = EnemyRegistry::builtin()
        .get(enemy)
        .unwrap_or_else(|| panic!("no enemy '{enemy}'"));
    match def.locomotion {
        Locomotion::Flying { .. } => None,
        Locomotion::Ground => Some(NavClass::for_size(def.radius, def.height)),
    }
}

fn in_kill_volume(level: &LevelDef, p: Vec3) -> bool {
    level.kill_volumes.iter().any(|v| v.contains(p))
}

/// Every raw node and every point along the string-pulled route is safe: no node in a kill
/// volume, and walking the smoothed line never puts the feet above lava.
fn assert_safe_path(level: &LevelDef, g: &NavGrid, path: &[u32], class: NavClass, what: &str) {
    for &n in path {
        let node = &g.nodes[n as usize];
        assert!(
            !node.is(node_flags::DEADLY),
            "{what}: deadly node {}",
            node.pos
        );
        assert!(
            !in_kill_volume(level, node.pos + Vec3::Y * 0.1),
            "{what}: node in a kill volume {}",
            node.pos
        );
    }
    let wps = g.smooth(path, class, &[]);
    let mut prev = g.nodes[path[0] as usize].pos;
    for w in wps {
        let steps = ((w - prev).length() / 0.25).ceil().max(1.0) as u32;
        for s in 0..=steps {
            let p = prev.lerp(w, s as f32 / steps as f32);
            assert!(
                g.ground_ok(p, prev.y.max(w.y) + 0.1, MAX_DROP + 0.2),
                "{what}: route from {prev} to {w} crosses unsafe ground at {p}"
            );
        }
        prev = w;
    }
}

#[test]
fn builds_on_every_shipped_level() {
    let mut names = LevelDef::list();
    names.sort();
    assert!(names.iter().any(|n| n == "crucible_of_veins"));
    for name in names {
        let started = Instant::now();
        let (level, g, _) = load(&name);
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        let s = &g.stats;
        println!(
            "{name:20} {}x{} cols, {} floors: {} walkable, {} large, {} hazard, {} deadly; {} links; {} rays, {} capsule tests; build {:.1} ms (incl. load {ms:.1} ms)",
            g.nx,
            g.nz,
            s.floors,
            s.small,
            s.large,
            s.hazard,
            s.deadly,
            s.links,
            s.rays,
            s.capsule_tests,
            s.build_ms
        );
        println!(
            "{:20} jumps: {} leaps ({} over lava / water), {} pad rides; {} arc sweeps, {:.1} ms",
            "", s.leaps, s.leaps_over_deadly, s.pad_links, s.arc_sweeps, s.jumps_ms
        );
        assert!(s.small > 1000, "{name}: only {} walkable nodes", s.small);
        assert!(s.links > s.small, "{name}: barely linked");
        // Room per enemy class (narrow + headroom, or the wide footprint).
        let mut per_class = Vec::new();
        for enemy in ["gnawer", "censer", "heron", "gaoler"] {
            let class = class_of(enemy).expect("ground enemy");
            let n = (0..g.nodes.len() as u32)
                .filter(|&n| g.passable(n, class, &[]))
                .count();
            per_class.push(format!("{enemy} {n}"));
            if level.kind == LevelKind::Campaign || enemy != "gaoler" {
                assert!(n > s.small / 3, "{name}: only {n} nodes for the {enemy}");
            }
        }
        println!("{:20} passable per class: {}", "", per_class.join(", "));
        // Every player spawn stands on a walkable node.
        for sp in &level.spawns {
            assert!(
                g.node_at(sp.position, 0.5, NavClass::SMALL, &[]).is_some(),
                "{name}: spawn {} not on the grid",
                sp.position
            );
        }
        if level.kind == LevelKind::Campaign {
            assert!(s.deadly > 50, "{name}: lava should be marked deadly");
            assert!(s.large > 500, "{name}: room for gaolers");
        }
    }
}

#[test]
fn crucible_encounters_are_connected_and_paths_avoid_lava() {
    let (level, g, _) = load("crucible_of_veins");
    let mut s = NavScratch::default();
    let mut checked = 0;
    for (ei, enc) in level.encounters.iter().enumerate() {
        let rally = enc.rally.expect("rally").position;
        // The arena centre at floor level: the walkable node nearest to it (islands, bridges).
        let arena = enc.arena.expect("arena");
        let centre = Vec3::new(arena.center().x, rally.y, arena.center().z);
        for (wi, wave) in enc.waves.iter().enumerate() {
            for sp in &wave.spawns {
                let Some(class) = class_of(&sp.enemy) else {
                    continue;
                };
                let what = format!(
                    "encounter {} wave {} {} at {}",
                    ei + 1,
                    wi + 1,
                    sp.enemy,
                    sp.position
                );
                let start = g
                    .node_at(sp.position, 0.6, class, &[])
                    .unwrap_or_else(|| panic!("{what}: spawn not on the grid for {class:?}"));
                let goal = g
                    .nearest_node(centre, 12, class, &[])
                    .expect("arena centre on the grid");
                let rally_node = g
                    .node_at(rally, 0.6, NavClass::SMALL, &[])
                    .expect("rally on the grid");
                let to_centre = g.find_path(&mut s, start, goal, class, &[], 100_000);
                let to_rally =
                    g.find_path(&mut s, start, rally_node, NavClass::SMALL, &[], 100_000);
                let from_rally =
                    g.find_path(&mut s, rally_node, start, NavClass::SMALL, &[], 100_000);
                if sp.enemy == "censer" {
                    // Censers may sit on pad-only ledges (docs/levels.md): they only need to be
                    // on the grid and able to come down.
                    if let PathResult::Found(p) = &to_rally {
                        assert_safe_path(&level, &g, p, NavClass::SMALL, &what);
                    }
                    continue;
                }
                let PathResult::Found(p) = to_centre else {
                    panic!("{what}: no path to the arena centre {centre}: {to_centre:?}");
                };
                assert_safe_path(&level, &g, &p, class, &format!("{what} -> centre"));
                let PathResult::Found(p) = to_rally else {
                    panic!("{what}: can't reach the rally point");
                };
                assert_safe_path(&level, &g, &p, NavClass::SMALL, &format!("{what} -> rally"));
                let PathResult::Found(p) = from_rally else {
                    panic!("{what}: unreachable from the rally point");
                };
                assert_safe_path(&level, &g, &p, NavClass::SMALL, &format!("rally -> {what}"));
                checked += 1;
            }
        }
    }
    assert!(checked > 40, "checked {checked} spawns");
}

#[test]
fn crucible_has_stacked_floors_and_lava_is_refused() {
    let (level, g, _) = load("crucible_of_veins");
    // What enemies can actually walk on: everything linked to a player spawn, checkpoint or
    // encounter spawn (roofs and wall tops are floors too, but nothing reaches them).
    let mut reach = vec![false; g.nodes.len()];
    let mut queue: Vec<u32> = Vec::new();
    let seeds = level
        .spawns
        .iter()
        .map(|s| s.position)
        .chain(level.checkpoints.iter().map(|c| c.spawn.position))
        .chain(
            level
                .encounters
                .iter()
                .flat_map(|e| e.waves.iter().flat_map(|w| w.spawns.iter()))
                .filter(|s| s.enemy != "harrier")
                .map(|s| s.position),
        );
    for p in seeds {
        if let Some(n) = g.node_at(p, 0.6, NavClass::SMALL, &[]) {
            queue.push(n);
        }
    }
    while let Some(n) = queue.pop() {
        if std::mem::replace(&mut reach[n as usize], true) {
            continue;
        }
        for &m in &g.nodes[n as usize].links {
            if m != shared::sim::nav::NONE && !reach[m as usize] {
                queue.push(m);
            }
        }
    }
    let reachable = reach.iter().filter(|r| **r).count();
    // Columns with two or more reachable floors (bridges over lava, galleries over aisles...).
    let mut stacked = 0;
    let mut deadly_under_walkable = 0;
    let mut by_column: std::collections::BTreeMap<(i32, i32), Vec<&shared::sim::nav::NavNode>> =
        Default::default();
    for (i, n) in g.nodes.iter().enumerate() {
        if !reach[i] && !n.is(node_flags::DEADLY) {
            continue;
        }
        by_column
            .entry((n.pos.x.floor() as i32, n.pos.z.floor() as i32))
            .or_default()
            .push(n);
    }
    println!(
        "{reachable} of {} walkable nodes are reachable",
        g.stats.small
    );
    assert!(reachable > 12_000);
    for nodes in by_column.values() {
        let walk = nodes.iter().filter(|n| n.is(node_flags::SMALL)).count();
        if walk >= 2 {
            stacked += 1;
        }
        if walk >= 1 && nodes.iter().any(|n| n.is(node_flags::DEADLY)) {
            deadly_under_walkable += 1;
            // Standing on the upper floor is fine, the lava below doesn't make it unsafe.
            let top = nodes
                .iter()
                .filter(|n| n.is(node_flags::SMALL))
                .max_by(|a, b| a.pos.y.total_cmp(&b.pos.y))
                .unwrap();
            let lava = nodes.iter().find(|n| n.is(node_flags::DEADLY)).unwrap();
            if lava.pos.y < top.pos.y {
                assert!(
                    g.ground_ok(top.pos, top.pos.y, STEP_UP),
                    "bridge at {} unsafe",
                    top.pos
                );
            }
        }
    }
    println!(
        "{stacked} columns with stacked walkable floors, {deadly_under_walkable} walkable floors over lava"
    );
    assert!(stacked > 100, "multi-floor areas found: {stacked}");
    assert!(
        deadly_under_walkable > 10,
        "bridges over lava: {deadly_under_walkable}"
    );

    // Every lava surface refuses the edge guard from the rim next to it.
    for v in &level.kill_volumes {
        let c = v.center();
        let top = v.max.y;
        if v.max.x - v.min.x > 200.0 {
            continue; // the safety volume under the whole level
        }
        let p = Vec3::new(c.x, top, c.z);
        assert!(
            !g.ground_ok(p, top + 0.5, MAX_DROP),
            "lava at {p} looks safe"
        );
    }

    // The stacked floors are separate nodes and the higher one is not reached by climbing a
    // wall: every up-link is a step or a ramp.
    for n in &g.nodes {
        for &m in &n.links {
            if m == shared::sim::nav::NONE {
                continue;
            }
            let o = &g.nodes[m as usize];
            assert!(
                o.pos.y - n.pos.y <= 1.25,
                "link climbs {} -> {}",
                n.pos,
                o.pos
            );
        }
    }
}

/// The full nav class of a ground Mould: size, leap limits, pads (as `Enemy::nav_class`).
fn full_class_of(enemy: &str) -> NavClass {
    let def = EnemyRegistry::builtin().get(enemy).expect("enemy");
    let class = NavClass::for_size(def.radius, def.height).with_pads(def.pads);
    match def.leap {
        Some(l) => class.with_leaps(l.distance, l.up, l.down),
        None => class,
    }
}

/// Nodes reachable from `seeds` for `class` (walking links, plus its jumps when `jumps`).
fn reachable(g: &NavGrid, seeds: &[u32], class: NavClass, jumps: bool) -> Vec<bool> {
    let walker = NavClass {
        leap_dm: 0,
        pads: false,
        ..class
    };
    let class = if jumps { class } else { walker };
    let mut seen = vec![false; g.nodes.len()];
    let mut queue: Vec<u32> = seeds.to_vec();
    while let Some(n) = queue.pop() {
        if std::mem::replace(&mut seen[n as usize], true) {
            continue;
        }
        g.for_each_successor(n, class, &[], |m| {
            if !seen[m as usize] {
                queue.push(m);
            }
        });
    }
    seen
}

#[test]
fn no_leap_lands_in_or_flies_through_a_kill_volume() {
    for name in LevelDef::list() {
        let (level, g, _) = load(&name);
        for j in g.jumps() {
            let (a, b) = (g.nodes[j.from as usize].pos, g.nodes[j.to as usize].pos);
            for n in [j.from, j.to] {
                let node = &g.nodes[n as usize];
                assert!(!node.is(node_flags::DEADLY), "{name}: leap {a} -> {b}");
                assert!(
                    !in_kill_volume(&level, node.pos + Vec3::Y * 0.1),
                    "{name}: leap {a} -> {b} ends in a kill volume"
                );
            }
            if j.kind == shared::sim::nav::JumpKind::Leap {
                for i in 1..40 {
                    let p = shared::sim::nav::leap_point(a, b, i as f32 / 40.0);
                    assert!(
                        !in_kill_volume(&level, p),
                        "{name}: leap {a} -> {b} passes through a kill volume at {p}"
                    );
                }
                assert!(j.dist <= shared::sim::nav::LEAP_MAX_DISTANCE + 1e-3);
            }
        }
    }
}

/// How much of the floor players can reach each ground Mould can reach (from the player
/// spawns and checkpoints, doors open), walking only (before the leap links) and with its
/// leaps and pads (`docs/enemies.md` measurements), counted over the part of that floor the
/// Mould fits on (headroom, the wide footprint). "Player-reachable" is approximated by
/// walking + every leap (8 m, 4 m up, 8 m down) + every pad from the same seeds; players do
/// more (wall jumps, dashes, rocket jumps), so this is a floor of what they reach.
#[test]
fn reachability_per_class_on_every_level() {
    let moulds = [
        "gnawer",
        "snuffer",
        "censer",
        "heron",
        "bellwether",
        "gaoler",
        "smelter",
    ];
    let mut names = LevelDef::list();
    names.sort();
    println!(
        "{:20} {:>7}  {}",
        "level",
        "player",
        moulds
            .iter()
            .map(|m| format!("{m:>16}"))
            .collect::<String>()
    );
    for name in names {
        if name == "movement_test" {
            continue;
        }
        let (level, g, _) = load(&name);
        let seeds: Vec<u32> = level
            .spawns
            .iter()
            .map(|s| s.position)
            .chain(level.checkpoints.iter().map(|c| c.spawn.position))
            .filter_map(|p| g.node_at(p, 0.6, NavClass::SMALL, &[]))
            .collect();
        let player = NavClass::SMALL.with_leaps(8.0, 4.0, 8.0).with_pads(true);
        let players = reachable(&g, &seeds, player, true);
        let total = players.iter().filter(|r| **r).count().max(1);
        let mut row = String::new();
        for m in moulds {
            let class = full_class_of(m);
            let own: Vec<u32> = seeds
                .iter()
                .filter_map(|&s| g.nearest_node(g.nodes[s as usize].pos, 6, class, &[]))
                .collect();
            let fits: Vec<bool> = (0..g.nodes.len() as u32)
                .map(|n| players[n as usize] && g.passable(n, class, &[]))
                .collect();
            let room = fits.iter().filter(|f| **f).count().max(1);
            let count = |r: &[bool]| r.iter().zip(&fits).filter(|(a, b)| **a && **b).count();
            let before = count(&reachable(&g, &own, class, false));
            let after = count(&reachable(&g, &own, class, true));
            assert!(after >= before, "{name} {m}: {before} -> {after}");
            row.push_str(&format!(
                "{:>16}",
                format!(
                    "{:.0}% -> {:.0}%",
                    100.0 * before as f32 / room as f32,
                    100.0 * after as f32 / room as f32
                )
            ));
            if m == "gnawer" && name == "drowned_chandlery" {
                assert!(
                    after as f32 > before as f32 * 1.05,
                    "{name}: leaps open up the flooded halls ({before} -> {after})"
                );
            }
        }
        println!("{name:20} {total:>7}  {row}");
    }
}

#[test]
fn arena_level_grid_and_queries_are_fast() {
    let (_, g, _) = load("crucible_of_veins");
    let mut s = NavScratch::default();
    // A long query across the whole campaign (start to exit), doors open.
    let start = g
        .node_at(Vec3::new(0.0, 0.0, 25.0), 0.5, NavClass::SMALL, &[])
        .unwrap();
    let exit = g
        .nearest_node(Vec3::new(91.0, 4.0, -362.0), 6, NavClass::SMALL, &[])
        .unwrap();
    let t = Instant::now();
    let r = g.find_path(&mut s, start, exit, NavClass::SMALL, &[], 1_000_000);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "start -> exit: {:?} nodes, {} expanded, {ms:.2} ms",
        match &r {
            PathResult::Found(p) | PathResult::Partial(p) => p.len(),
            PathResult::NotFound => 0,
        },
        s.last_expanded
    );
    // The campaign's critical path uses one jump pad (Bell Loft), so a walker can't do the
    // whole level; it must at least get far.
    assert!(!matches!(r, PathResult::NotFound));
    // Typical in-arena query (budgeted search).
    let a = g
        .node_at(Vec3::new(-10.0, 0.0, -57.0), 0.6, NavClass::SMALL, &[])
        .unwrap();
    let b = g
        .node_at(Vec3::new(0.0, 0.0, -25.5), 0.6, NavClass::SMALL, &[])
        .unwrap();
    let t = Instant::now();
    let mut n = 0;
    for _ in 0..100 {
        if let PathResult::Found(p) = g.find_path(&mut s, a, b, NavClass::SMALL, &[], 3000) {
            n = p.len();
        }
    }
    let us = t.elapsed().as_secs_f64() * 1e6 / 100.0;
    println!(
        "arena 1 spawn -> rally: {n} nodes, {} expanded, {us:.0} us per search",
        s.last_expanded
    );
    assert!(n > 0);
}
