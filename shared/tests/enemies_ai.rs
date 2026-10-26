//! Enemy AI in small test arenas: every type acquires a player and telegraphs before every
//! attack, Spitters keep their distance and need line of sight, Gnats hover and never clip the
//! level, the Bastion slam is parryable only in its window (stun + 150), front armour, light vs
//! heavy knockback, nobody walks into lava, separation, untargetable players, determinism.

mod combat_support;

use std::collections::BTreeMap;
use std::f32::consts::PI;

use combat_support::*;
use engine::physics::{MASK_MOVEMENT, MASK_SHOTS};
use glam::Vec3;
use shared::input::Buttons;
use shared::level::{LevelDef, LevelKind, Volume};
use shared::protocol::{NetFrame, NetId, enemy_flags};
use shared::sim::{
    AiState, Attack, DamageFlags, DamageInfo, EnemyAttackPhase, HitPart, HitTarget, Owner,
    ParryTarget, PlayerId, SimEvent,
};

/// Per-tick log of one enemy: state, and the attack events it produced.
#[derive(Default)]
struct Log {
    states: Vec<(u32, AiState)>,
    windups: Vec<u32>,
    strikes: Vec<u32>,
    fired: Vec<u32>,
    /// (tick, damage event tick) of hits on players from this enemy (blocked ones too).
    hits: Vec<u32>,
    alerted: bool,
}

/// Runs `ticks` ticks, logging everything enemy `id` does.
fn run_logged(a: &mut Arena, id: NetId, ticks: u32, log: &mut Log) {
    for _ in 0..ticks {
        let ev = a.tick();
        let now = a.world.tick;
        if let Some(e) = a.world.enemies.get(&id) {
            log.states.push((now, e.state));
        }
        for e in &ev {
            match e {
                SimEvent::EnemyAttack {
                    id: eid, phase, ..
                } if *eid == id => match phase {
                    EnemyAttackPhase::Windup => log.windups.push(now),
                    EnemyAttackPhase::Strike => log.strikes.push(now),
                },
                SimEvent::EnemyFired { id: eid, .. } if *eid == id => log.fired.push(now),
                SimEvent::EnemyAlerted { id: eid, .. } if *eid == id => log.alerted = true,
                SimEvent::Damage(d)
                    if d.info.source == Owner::Enemy(id)
                        && matches!(d.target, HitTarget::Player(_)) =>
                {
                    log.hits.push(now)
                }
                _ => {}
            }
        }
    }
}

fn god(a: &mut Arena, p: PlayerId) {
    a.world.players.get_mut(&p).unwrap().combat.god = true;
}

fn spawn(a: &mut Arena, name: &str, pos: Vec3, yaw: f32) -> NetId {
    a.world.spawn_enemy(name, pos, yaw).expect("enemy exists")
}

/// Asserts the telegraph contract on a log: every strike comes exactly one wind-up after a
/// wind-up event, every projectile leaves during a strike, and no hit lands before a strike.
fn assert_telegraphs(a: &Arena, id: NetId, log: &Log, what: &str) {
    let def = a.world.enemy_defs.get(what).unwrap();
    assert!(!log.strikes.is_empty(), "{what}: never attacked");
    let windup_ticks: Vec<u32> = def
        .attacks
        .iter()
        .map(|at| (at.windup * 60.0).round() as u32)
        .collect();
    for &s in &log.strikes {
        let ok = log
            .windups
            .iter()
            .any(|&w| windup_ticks.iter().any(|&len| w + len == s));
        assert!(ok, "{what}: strike at tick {s} without a full wind-up ({:?} / windups {:?})", windup_ticks, log.windups);
        // And it was in the wind-up state right before.
        let before = log.states.iter().find(|(t, _)| *t == s - 1).map(|x| x.1);
        assert_eq!(before, Some(AiState::Windup), "{what}: state before the strike at {s}");
    }
    for &f in &log.fired {
        let state = log.states.iter().find(|(t, _)| *t == f).map(|x| x.1);
        assert_eq!(state, Some(AiState::Strike), "{what}: fired outside a strike at {f}");
    }
    let first_strike = log.strikes[0];
    for &h in &log.hits {
        assert!(h >= first_strike, "{what}: hit at {h} before any strike ({first_strike})");
    }
    let _ = id;
}

#[test]
fn every_type_acquires_a_player_and_telegraphs_every_attack() {
    for (name, distance, secs) in [
        ("husk", 15.0, 8.0),
        ("spitter", 18.0, 10.0),
        ("gnat", 15.0, 10.0),
        ("bastion", 10.0, 12.0),
    ] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        let pos = if name == "gnat" {
            Vec3::new(0.0, 6.0, -distance)
        } else {
            Vec3::new(0.0, 0.0, -distance)
        };
        let id = spawn(&mut a, name, pos, 0.0);
        let mut log = Log::default();
        run_logged(&mut a, id, (secs * 60.0) as u32, &mut log);
        assert!(log.alerted, "{name}: never noticed the player");
        assert_eq!(a.world.enemies[&id].target, Some(p), "{name}: target");
        assert!(!log.hits.is_empty(), "{name}: never landed a hit");
        assert_telegraphs(&a, id, &log, name);
        let alert_tick = log.states.iter().find(|s| s.1 == AiState::Alert).map(|s| s.0);
        assert!(alert_tick.is_some_and(|t| t < log.windups[0]), "{name}: alert before the first wind-up");
        println!(
            "{name}: {} attacks, {} hits, {} projectiles in {secs} s",
            log.strikes.len(),
            log.hits.len(),
            log.fired.len()
        );
        if name == "gnat" {
            // Bursts of three bolts.
            assert_eq!(log.fired.len(), 3 * log.strikes.len(), "gnat bursts");
        }
    }
}

#[test]
fn spitter_keeps_its_distance_and_strafes() {
    for start in [5.0f32, 45.0] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        let id = spawn(&mut a, "spitter", Vec3::new(0.0, 0.0, -start), 0.0);
        if start > 30.0 {
            // Out of sight range is fine: it knows where the fight is (arena spawn).
            a.world.enemies.get_mut(&id).unwrap().aware = true;
        }
        let mut in_band = 0;
        let mut total = 0;
        let mut lateral = 0.0f32;
        let mut fired = 0;
        let mut last = a.world.enemies[&id].position;
        for i in 0..(14 * 60) {
            let ev = a.tick();
            fired += ev
                .iter()
                .filter(|e| matches!(e, SimEvent::EnemyFired { id: x, .. } if *x == id))
                .count();
            let e = &a.world.enemies[&id];
            let d = (e.eye() - Vec3::new(0.0, 1.2, 0.0)).length();
            if i > 6 * 60 {
                total += 1;
                if (11.0..=26.0).contains(&d) {
                    in_band += 1;
                }
                // Movement across the line to the player = strafing.
                let to = (-e.position).normalize_or_zero();
                let step = e.position - last;
                lateral += (step - to * step.dot(to)).length();
            }
            last = e.position;
        }
        let frac = in_band as f32 / total as f32;
        println!("spitter from {start} m: {:.0} % of the time in 12-25 m, strafed {lateral:.1} m, {fired} spits", frac * 100.0);
        assert!(frac > 0.9, "spitter from {start} m: in band {frac}");
        assert!(lateral > 5.0, "spitter strafes ({lateral} m)");
        assert!(fired >= 2, "spits ({fired})");
    }
}

#[test]
fn spitter_needs_line_of_sight_and_walks_around_cover() {
    // A wall between the spitter and the player, open at its east end.
    let level = flat_level(
        LevelKind::Campaign,
        vec![brush([-20.0, 0.0, -11.0], [8.0, 6.0, -10.0])],
    );
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "spitter", Vec3::new(-6.0, 0.0, -24.0), 0.0);
    a.world.enemies.get_mut(&id).unwrap().aware = true;
    let mut first_shot = None;
    let mut shots = 0;
    for i in 0..(15 * 60) {
        let ev = a.tick();
        for e in &ev {
            if let SimEvent::EnemyFired { id: x, position, .. } = e
                && *x == id
            {
                let chest = a.eye(p) - Vec3::Y * 0.3;
                assert!(
                    !a.world.collision.world.line_blocked(*position, chest, MASK_SHOTS),
                    "spat without line of sight from {position}"
                );
                first_shot.get_or_insert(i);
                shots += 1;
            }
        }
    }
    let first_shot = first_shot.expect("found a firing position");
    println!(
        "spitter behind cover fired first after {:.1} s, {shots} spits in 15 s",
        first_shot as f32 / 60.0
    );
    assert!(shots >= 3, "keeps a firing position ({shots} spits)");
}

#[test]
fn gnat_hovers_at_height_and_never_clips_walls() {
    // Pillars and a wall around the player.
    let level = flat_level(
        LevelKind::Campaign,
        vec![
            brush([-6.0, 0.0, -6.0], [-4.0, 14.0, -4.0]),
            brush([4.0, 0.0, 4.0], [6.0, 14.0, 6.0]),
            brush([-14.0, 0.0, 7.0], [14.0, 14.0, 8.0]),
            brush([9.0, 0.0, -12.0], [10.0, 14.0, 2.0]),
            // A low roof over part of the arena.
            brush([-20.0, 7.0, -20.0], [-8.0, 8.0, -8.0]),
        ],
    );
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let gnats: Vec<NetId> = (0..3)
        .map(|i| spawn(&mut a, "gnat", Vec3::new(-12.0 + 8.0 * i as f32, 5.0, -15.0), 0.0))
        .collect();
    let mut low = f32::INFINITY;
    let mut high = 0.0f32;
    let mut angle_travel = 0.0f32;
    let mut last_angle = BTreeMap::new();
    for i in 0..(25 * 60) {
        a.tick();
        for &g in &gnats {
            let e = &a.world.enemies[&g];
            assert!(!e.on_ground, "gnat landed");
            let r = e.def.radius;
            let half = (e.def.height * 0.5 - r).max(0.0);
            assert!(
                !a.world
                    .collision
                    .world
                    .intersects_capsule(e.center(), half, r - 0.05, MASK_MOVEMENT),
                "gnat inside the level at {} (tick {i})",
                e.position
            );
            if i > 3 * 60 {
                let ground = a
                    .world
                    .collision
                    .world
                    .cast_ray(e.center(), Vec3::NEG_Y, 50.0, MASK_MOVEMENT)
                    .map_or(50.0, |h| h.distance);
                low = low.min(ground);
                high = high.max(ground);
                let ang = e.position.z.atan2(e.position.x);
                if let Some(prev) = last_angle.insert(g, ang) {
                    let mut d: f32 = ang - prev;
                    if d > PI {
                        d -= 2.0 * PI;
                    }
                    if d < -PI {
                        d += 2.0 * PI;
                    }
                    angle_travel += d.abs();
                }
            }
        }
    }
    println!("gnats hovered {low:.1}..{high:.1} m above the ground, circled {:.1} rad", angle_travel);
    assert!(low >= 2.5, "too low: {low}");
    assert!(high <= 9.5, "too high: {high}");
    assert!(angle_travel > 2.0 * PI, "gnats circle the target");
}

/// A bastion facing a player 1.5 m in front of it (surface to surface).
fn bastion_duel() -> (Arena, PlayerId, NetId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, PI);
    let id = spawn(&mut a, "bastion", Vec3::new(0.0, 0.0, -3.0), 0.0);
    a.world.enemies.get_mut(&id).unwrap().health = 5000.0;
    a.world.enemies.get_mut(&id).unwrap().max_health = 5000.0;
    // The player faces the bastion (yaw 0 looks down -Z).
    a.held.get_mut(&p).unwrap().yaw = 0.0;
    (a, p, id)
}

/// Ticks until the bastion starts winding up; returns the wind-up start tick.
fn until_windup(a: &mut Arena, id: NetId) -> u32 {
    for _ in 0..300 {
        a.tick();
        if a.world.enemies[&id].state == AiState::Windup {
            return a.world.tick;
        }
    }
    panic!("bastion never wound up");
}

fn parryable_flag(a: &Arena, id: NetId) -> bool {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).unwrap().flags & enemy_flags::PARRYABLE != 0
}

#[test]
fn bastion_slam_is_parryable_only_in_its_window() {
    // In the window (last 0.35 s of the 1.0 s wind-up): stunned 2.5 s, 150 damage, no slam.
    let (mut a, p, id) = bastion_duel();
    let w0 = until_windup(&mut a, id);
    assert!(!parryable_flag(&a, id), "not parryable at the start of the wind-up");
    a.ticks(45 - 1);
    assert_eq!(a.world.tick, w0 + 44);
    assert!(parryable_flag(&a, id), "parryable in the window");
    let ev = a.tap(p, Buttons::PUNCH, 2);
    let parried = ev.iter().any(|e| {
        matches!(e, SimEvent::Parried(pe) if pe.target == ParryTarget::Melee { enemy: id })
    });
    assert!(parried, "the punch parried the slam");
    let e = &a.world.enemies[&id];
    assert_eq!(e.state, AiState::Stunned);
    assert_eq!(e.state_len, 150, "2.5 s stun");
    // The counter: 150, through the front armour (the punch itself is armoured).
    let counter: f32 = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d)
                if d.target == HitTarget::Entity(id)
                    && d.info.flags.contains(DamageFlags::PARRIED) =>
            {
                Some(d.dealt)
            }
            _ => None,
        })
        .sum();
    assert_eq!(counter, 150.0, "parry counter damage");
    let before = a.health(p);
    a.ticks(60);
    assert_eq!(a.health(p), before, "the slam never landed");
    a.ticks(100);
    assert_ne!(a.world.enemies[&id].state, AiState::Stunned, "stun wears off");

    // Too early (first half of the wind-up): nothing, and the slam lands.
    let (mut a, p, id) = bastion_duel();
    let w0 = until_windup(&mut a, id);
    a.ticks(5);
    let ev = a.tap(p, Buttons::PUNCH, 0);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Parried(_))), "parried too early");
    let mut hurt = 0.0;
    for _ in 0..90 {
        hurt += damage_to(&a.tick(), HitTarget::Player(p));
    }
    assert!(hurt >= 20.0, "slam landed: {hurt}");
    assert_ne!(a.world.enemies[&id].state, AiState::Stunned);
    let _ = w0;

    // Too late (after the strike's damage was applied): nothing.
    let (mut a, p, id) = bastion_duel();
    let w0 = until_windup(&mut a, id);
    let mut hurt_at = None;
    for _ in 0..120 {
        let ev = a.tick();
        if damage_to(&ev, HitTarget::Player(p)) > 0.0 {
            hurt_at = Some(a.world.tick);
            break;
        }
    }
    let hurt_at = hurt_at.expect("slam hit");
    assert!(hurt_at >= w0 + 60, "damage only after the full 1 s wind-up");
    a.ticks(3);
    let ev = a.tap(p, Buttons::PUNCH, 0);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Parried(_))), "parried after the hit");
    assert_ne!(a.world.enemies[&id].state, AiState::Stunned);
}

#[test]
fn bastion_front_armour_and_weak_points() {
    let mut a = Arena::coop();
    a.player(0, Vec3::new(0.0, 0.0, 40.0), 0.0);
    let id = spawn(&mut a, "bastion", Vec3::ZERO, 0.0); // faces -Z
    a.world.enemies.get_mut(&id).unwrap().health = 10_000.0;
    let t = a.tuning.clone();
    let fwd = a.world.enemies[&id].forward();
    let hit = |a: &mut Arena, dir: Vec3, part: HitPart, flags: DamageFlags| {
        let info = DamageInfo::new(100.0, Owner::Player(PlayerId(0)), Attack::Quill)
            .at(Vec3::new(0.0, 1.5, 0.0), dir)
            .part(part)
            .flags(flags);
        a.world.apply_damage(&t, HitTarget::Entity(id), info)
    };
    // Shot travelling against the facing = from the front.
    let front = hit(&mut a, -fwd, HitPart::Body, DamageFlags::HITSCAN);
    let back = hit(&mut a, fwd, HitPart::Body, DamageFlags::HITSCAN);
    let side = hit(&mut a, Vec3::X, HitPart::Body, DamageFlags::HITSCAN);
    assert!((front.dealt - 35.0).abs() < 0.01, "front {}", front.dealt);
    assert!(front.info.flags.contains(DamageFlags::ARMOURED));
    assert_eq!(back.dealt, 100.0, "back");
    assert_eq!(side.dealt, 100.0, "90° from the front is outside the 140° arc");
    let head = hit(&mut a, -fwd, HitPart::Weak(0), DamageFlags::HITSCAN);
    assert_eq!(head.dealt, 200.0, "head ignores the armour (x2)");
    let furnace = hit(&mut a, fwd, HitPart::Weak(1), DamageFlags::HITSCAN);
    assert_eq!(furnace.dealt, 200.0, "back furnace x2");
    let blast = hit(&mut a, -fwd, HitPart::Body, DamageFlags::EXPLOSIVE);
    assert!((blast.dealt - 35.0).abs() < 0.01, "explosions from the front are armoured");
}

#[test]
fn light_enemies_fly_bastions_barely_move() {
    let mut a = Arena::coop();
    a.player(0, Vec3::new(0.0, 0.0, 40.0), 0.0);
    let husk = spawn(&mut a, "husk", Vec3::new(-5.0, 0.0, -15.0), 0.0);
    let bastion = spawn(&mut a, "bastion", Vec3::new(5.0, 0.0, 0.0), 0.0);
    let t = a.tuning.clone();
    for id in [husk, bastion] {
        let info = DamageInfo::new(1.0, Owner::Player(PlayerId(0)), Attack::Quill)
            .at(a.world.enemies[&id].center(), Vec3::X)
            .knockback(Vec3::new(20.0, 4.0, 0.0));
        a.world.apply_damage(&t, HitTarget::Entity(id), info);
    }
    let hv = a.world.enemies[&husk].velocity.length();
    let bv = a.world.enemies[&bastion].velocity.length();
    assert!(hv > 20.0, "husk knocked back at {hv}");
    assert!(bv < 4.0, "bastion barely moves: {bv}");
    a.ticks(40);
    let moved_h = a.world.enemies[&husk].position.x + 5.0;
    let moved_b = a.world.enemies[&bastion].position.x - 5.0;
    assert!(moved_h > 3.0 && moved_b < 0.6, "husk {moved_h} m, bastion {moved_b} m");
}

/// Two floors separated by a lava trench (kill volume) with one bridge on the east side.
fn lava_level() -> LevelDef {
    let mut level = flat_level(LevelKind::Campaign, Vec::new());
    level.brushes = vec![
        // North and south floors.
        brush([-30.0, -1.0, -40.0], [30.0, 0.0, -6.0]),
        brush([-30.0, -1.0, 6.0], [30.0, 0.0, 40.0]),
        // Trench bottom (lava surface at -2).
        brush([-30.0, -3.0, -6.0], [30.0, -2.0, 6.0]),
        // Bridge.
        brush([10.0, -1.0, -6.0], [14.0, 0.0, 6.0]),
        // Walls all around so the bridge is the only way.
        brush([-31.0, -3.0, -41.0], [-30.0, 8.0, 41.0]),
        brush([30.0, -3.0, -41.0], [31.0, 8.0, 41.0]),
        brush([-31.0, -3.0, -41.0], [31.0, 8.0, -40.0]),
        brush([-31.0, -3.0, 40.0], [31.0, 8.0, 41.0]),
    ];
    level.kill_volumes = vec![
        Volume {
            min: Vec3::new(-30.0, -3.0, -6.0),
            max: Vec3::new(30.0, -1.0, 6.0),
        },
        Volume {
            min: Vec3::new(-500.0, -100.0, -500.0),
            max: Vec3::new(500.0, -20.0, 500.0),
        },
    ];
    level
}

#[test]
fn enemies_take_the_bridge_and_never_walk_into_lava() {
    let mut a = Arena::new(lava_level());
    let p = a.player(0, Vec3::new(-8.0, 0.0, 20.0), 0.0);
    god(&mut a, p);
    let mut ids = Vec::new();
    for (i, name) in ["husk", "husk", "husk", "husk", "bastion", "spitter"].iter().enumerate() {
        let id = spawn(&mut a, name, Vec3::new(-15.0 + 5.0 * i as f32, 0.0, -20.0), 0.0);
        a.world.enemies.get_mut(&id).unwrap().aware = true;
        ids.push(id);
    }
    let kill_volume = a.world.level.kill_volumes[0];
    let mut reached = BTreeMap::new();
    for tick in 0..(30 * 60) {
        let ev = a.tick();
        for e in &ev {
            if let SimEvent::Killed(k) = e {
                panic!("{:?} died ({:?})", k.victim, k.attack);
            }
        }
        for &id in &ids {
            let e = &a.world.enemies[&id];
            assert!(!kill_volume.contains(e.feet()), "{} in the lava at {}", e.def.name, e.position);
            if e.position.z > 6.0 {
                reached.entry(id).or_insert(tick);
            }
        }
    }
    println!("crossed the bridge after (s): {:?}", reached.values().map(|t| *t as f32 / 60.0).collect::<Vec<_>>());
    // The melee ones crossed (the spitter may shoot from the far side).
    for &id in &ids[..5] {
        assert!(reached.contains_key(&id), "{} never crossed", a.world.enemies[&id].def.name);
    }
    // Knocked into the lava, an enemy dies.
    let husk = ids[0];
    a.world.enemies.get_mut(&husk).unwrap().position = Vec3::new(0.0, -1.5, 0.0);
    let ev = a.tick();
    assert!(
        ev.iter().any(|e| matches!(e, SimEvent::Killed(k) if k.victim == HitTarget::Entity(husk) && k.attack == Attack::World)),
        "lava kills enemies"
    );
}

#[test]
fn melee_enemies_route_around_walls() {
    // A long wall with a gap at the west end; the husk can't see the player.
    let level = flat_level(
        LevelKind::Campaign,
        vec![brush([-14.0, 0.0, -11.0], [30.0, 4.0, -10.0])],
    );
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::new(10.0, 0.0, 0.0), 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "husk", Vec3::new(10.0, 0.0, -20.0), 0.0);
    a.world.enemies.get_mut(&id).unwrap().aware = true;
    let mut hit = None;
    for i in 0..(10 * 60) {
        if damage_to(&a.tick(), HitTarget::Player(p)) > 0.0 || a.events.iter().any(|e| matches!(e, SimEvent::Damage(d) if d.target == HitTarget::Player(p))) {
            hit = Some(i);
            break;
        }
    }
    let hit = hit.expect("the husk found the way around");
    println!("husk around the wall: {:.1} s, {:?}", hit as f32 / 60.0, a.world.ai.stats);
    // Walking ~24 m around + 20 m back at 9 m/s: well under 8 s.
    assert!(hit < 8 * 60);
    assert!(a.world.ai.stats.paths >= 1, "planned with A*");
}

#[test]
fn separation_keeps_enemies_apart() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(spawn(&mut a, "husk", Vec3::new(0.0, 0.0, -20.0), 0.0));
    }
    // Stacked at first; apart within a second.
    a.ticks(60);
    let check = |a: &Arena, tol: f32| {
        let mut worst = f32::INFINITY;
        for (i, x) in ids.iter().enumerate() {
            for y in &ids[i + 1..] {
                let (ex, ey) = (&a.world.enemies[x], &a.world.enemies[y]);
                let d = Vec3::new(ex.position.x - ey.position.x, 0.0, ex.position.z - ey.position.z).length();
                worst = worst.min(d / (ex.radius() + ey.radius()));
            }
        }
        assert!(worst >= tol, "enemies overlap: {worst}");
        worst
    };
    check(&a, 0.85);
    // Crowding the player: still apart.
    let mut worst: f32 = 1.0;
    for _ in 0..(6 * 60) {
        a.tick();
        worst = worst.min(check(&a, 0.8));
    }
    println!("closest pair while swarming: {:.2} x (r1 + r2)", worst);
}

#[test]
fn dead_and_untargetable_players_are_dropped() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let q = a.player(1, Vec3::new(0.0, 0.0, 30.0), 0.0);
    let id = spawn(&mut a, "husk", Vec3::new(0.0, 0.0, -10.0), 0.0);
    a.ticks(30);
    assert_eq!(a.world.enemies[&id].target, Some(p));
    // p dies: the husk switches to q right away and never hits a corpse.
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
    assert!(!shared::sim::enemy::is_targetable(&a.world.players[&p]));
    a.tick();
    assert_eq!(a.world.enemies[&id].target, Some(q));
    // q too: nobody left.
    a.world.kill_player(&t, q, Attack::Dev);
    a.tick();
    assert_eq!(a.world.enemies[&id].target, None);
    a.ticks(5);
    assert_eq!(a.world.enemies[&id].state, AiState::Idle);
}

#[test]
fn hurting_an_unaware_enemy_alerts_it() {
    // The player is behind the husk, out of sight (it faces away... sight is 360°, so use a
    // wall with a window the player shoots through? simpler: shoot from beyond sight range).
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let id = spawn(&mut a, "husk", Vec3::new(0.0, 0.0, -80.0), 0.0);
    a.ticks(30);
    assert_eq!(a.world.enemies[&id].state, AiState::Idle, "80 m is out of sight range");
    let t = a.tuning.clone();
    let info = DamageInfo::new(5.0, Owner::Player(p), Attack::Quill).at(Vec3::new(0.0, 1.0, -80.0), Vec3::NEG_Z);
    a.world.apply_damage(&t, HitTarget::Entity(id), info);
    a.ticks(5);
    let e = &a.world.enemies[&id];
    assert_eq!(e.target, Some(p));
    assert!(matches!(e.state, AiState::Alert | AiState::Chase));
}

#[test]
fn all_enemy_types_are_deterministic() {
    let run = || {
        let mut a = Arena::new(lava_level());
        let p = a.player(0, Vec3::new(-8.0, 0.0, 20.0), 0.0);
        god(&mut a, p);
        for (i, name) in ["husk", "spitter", "gnat", "bastion", "husk", "gnat"].iter().enumerate() {
            let y = if *name == "gnat" { 6.0 } else { 0.0 };
            let id = spawn(&mut a, name, Vec3::new(-15.0 + 5.0 * i as f32, y, -20.0), 0.0);
            a.world.enemies.get_mut(&id).unwrap().aware = true;
        }
        a.ticks(900);
        let v: Vec<_> = a
            .world
            .enemies
            .values()
            .map(|e| (e.position, e.velocity, e.state, e.health))
            .collect();
        (v, a.events.len())
    };
    assert_eq!(run(), run());
}

#[test]
fn frozen_world_stops_enemies() {
    // Results screen: `SimWorld::frozen` — nobody attacks, nobody thinks.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let ids: Vec<NetId> = ["husk", "spitter", "bastion"]
        .iter()
        .enumerate()
        .map(|(i, n)| spawn(&mut a, n, Vec3::new(-4.0 + 4.0 * i as f32, 0.0, -6.0), 0.0))
        .collect();
    a.ticks(50);
    assert!(ids.iter().any(|id| a.world.enemies[id].target == Some(p)));
    a.world.frozen = true;
    let ev = a.ticks(300);
    assert_eq!(damage_to(&ev, HitTarget::Player(p)), 0.0, "no damage while frozen");
    assert!(!ev.iter().any(|e| matches!(
        e,
        SimEvent::EnemyAttack { .. } | SimEvent::EnemyFired { .. }
    )));
    for id in &ids {
        let e = &a.world.enemies[id];
        assert_eq!(e.state, AiState::Idle);
        assert_eq!(e.target, None);
        assert!(e.on_ground, "still standing on the floor");
    }
}
