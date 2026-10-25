//! Per-tick cost of the enemy AI on the campaign level: 4 players running around the Furnace
//! Cathedral (arena 5, the biggest) against ~50 aware enemies of all five types (every arena-5
//! spawn, a copy of each gnawer and a few herons), the whole simulation tick timed. Run with
//! `-- --nocapture` for the numbers (`docs/enemies.md`).

use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::LevelDef;
use shared::sim::{PlayerId, SimWorld, TickInputs};
use shared::tuning::Tuning;

#[test]
fn fifty_enemies_on_the_campaign_level() {
    let level = LevelDef::load_named("crucible_of_veins").expect("level");
    // No encounters: this measures a fixed crowd (the players stand in arena 5).
    let mut quiet = level.clone();
    quiet.encounters.clear();
    let mut world = SimWorld::new("crucible_of_veins", Arc::new(quiet), 60);
    let tuning = Tuning::default();
    let nav_ms = world.build_nav();
    let spots = [
        Vec3::new(91.0, 0.0, -268.0),
        Vec3::new(78.0, 0.0, -276.0),
        Vec3::new(104.0, 0.0, -276.0),
        Vec3::new(91.0, 0.0, -284.0),
    ];
    for (i, spot) in spots.iter().enumerate() {
        let p = world.add_player(PlayerId(i as u8), "p");
        p.movement = shared::movement::MoveState::spawn(*spot);
        p.combat.god = true;
    }
    // Every spawn of arena 5's four waves, plus a copy of each gnawer 1.5 m to the side and a
    // heron (the campaign has none yet) 4 m beside each gaoler.
    let enc = &level.encounters[4];
    let mut n = 0;
    let mut kinds = std::collections::BTreeSet::new();
    for wave in &enc.waves {
        for s in &wave.spawns {
            let mut spots = vec![(s.enemy.as_str(), s.position)];
            if s.enemy == "gnawer" {
                spots.push(("gnawer", s.position + Vec3::new(1.5, 0.0, 0.0)));
            }
            if s.enemy == "gaoler" {
                spots.push(("heron", s.position + Vec3::new(4.0, 0.0, 0.0)));
            }
            for (name, p) in spots {
                if let Some(id) = world.spawn_enemy(name, p, s.yaw.to_radians()) {
                    world.enemies.get_mut(&id).unwrap().aware = true;
                    kinds.insert(name);
                    n += 1;
                }
            }
        }
    }
    assert!(n >= 48, "{n} enemies");
    assert_eq!(kinds.len(), 5, "every type: {kinds:?}");

    let mut times = Vec::new();
    for tick in 0..900u32 {
        let mut inputs = TickInputs::new();
        let seq = tick + 1;
        for i in 0..4u8 {
            // Run in wide circles, jumping now and then (targets keep moving).
            let mut input = PlayerInput {
                seq,
                forward: 127,
                yaw: tick as f32 * 0.02 + i as f32 * 1.6,
                ..Default::default()
            };
            input.buttons.set(Buttons::JUMP, (tick + i as u32 * 17).is_multiple_of(90));
            inputs.insert(PlayerId(i), vec![input]);
        }
        let started = Instant::now();
        let events = world.step(&inputs, &tuning, None);
        if tick >= 120 {
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        for e in &events {
            if let shared::sim::SimEvent::Killed(k) = e {
                let name = match k.victim {
                    shared::sim::HitTarget::Entity(id) => {
                        world.enemies.get(&id).map(|e| e.def.name.clone())
                    }
                    _ => None,
                };
                println!("tick {tick}: {name:?} died ({:?}) at {}", k.attack, k.position);
            }
        }
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let p99 = times[(times.len() * 99) / 100];
    let max = times[times.len() - 1];
    let alive = world.enemies.values().filter(|e| e.is_alive()).count();
    let s = &world.ai.stats;
    println!(
        "{n} enemies ({alive} alive at the end), 4 players: tick avg {avg:.3} ms, p99 {p99:.3} ms, max {max:.3} ms \
         (nav build {nav_ms:.1} ms); A* {} ({} partial, {} failed, {} deferred), {} straight walks, {:.0} nodes/search",
        s.paths,
        s.partial,
        s.failed,
        s.deferred,
        s.direct,
        s.expanded as f64 / s.paths.max(1) as f64
    );
    // Nobody fell into the lava or out of the level.
    assert_eq!(alive, n, "enemies died on their own");
    // Generous bound (shared CI machines); the target in a dev build is < 2 ms.
    assert!(avg < 6.0, "average tick {avg:.2} ms");
}
