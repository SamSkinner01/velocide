//! Combat replication: trajectory-replicated projectiles/thorns reproduce the server position,
//! a crowded frame stays small in deltas, own state / events survive bincode, and
//! `assets/tuning.ron` spells out every combat value equal to the defaults.

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;
use std::path::Path;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::protocol::{
    self, EntityState, Fx, FxEvent, GameEvent, NetFrame, OwnState, QPos, ServerUnreliable,
    SnapshotMessage, encode_delta, encode_full, kind, projectile_flags,
};
use shared::sim::projectiles::replicated_position;
use shared::sim::{Attack, Owner, PlayerCombat, ProjectileKind, ThornState};
use shared::tuning::{CombatTuning, Tuning};
use weapon_support::*;

#[test]
fn tuning_file_has_every_combat_value_and_matches_the_defaults() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
    let text = std::fs::read_to_string(&path).expect("assets/tuning.ron exists");
    let tuning: Tuning = ron::from_str(&text).expect("assets/tuning.ron parses");
    assert_eq!(tuning.combat, CombatTuning::default());
    let defaults = ron::to_string(&CombatTuning::default()).unwrap();
    let fields: Vec<&str> = defaults
        .split([',', '('])
        .filter_map(|kv| kv.split_once(':').map(|(k, _)| k.trim()))
        .filter(|k| !k.is_empty())
        .collect();
    assert!(fields.len() > 100, "{fields:?}");
    for field in fields {
        assert!(
            text.contains(&format!("{field}:")),
            "assets/tuning.ron is missing combat `{field}`"
        );
    }
}

fn frame_state(a: &Arena, id: shared::protocol::NetId) -> EntityState {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).expect("replicated")
}

#[test]
fn trajectories_reproduce_server_positions() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.ticks(3);
    // A Gravewell canister (gravity) and a thorn, lobbed / thrown up and forward.
    a.select(p, 3, 1);
    a.held.get_mut(&p).unwrap().pitch = 0.4;
    a.set_buttons(p, Buttons::ALT_FIRE | Buttons::THROW);
    a.tick();
    a.set_buttons(p, Buttons::NONE);
    let well = *a.world.projectiles.keys().next().unwrap();
    let thorn = *a.world.thorns.keys().next().unwrap();
    let well_entity = frame_state(&a, well);
    let thorn_entity = frame_state(&a, thorn);
    assert_eq!(well_entity.kind, kind::GRAVEWELL);
    assert_eq!(thorn_entity.kind, kind::THORN);
    let mut max_err: f32 = 0.0;
    for _ in 0..25 {
        a.tick();
        let now = a.world.tick as f64;
        if let Some(f) = a.world.projectiles.get(&well)
            && f.stuck.is_none()
        {
            let predicted = replicated_position(&well_entity, now, 60, &a.tuning.combat);
            max_err = max_err.max(predicted.distance(f.position));
            // The entity doesn't change while it flies: no bandwidth.
            assert_eq!(frame_state(&a, well), well_entity);
        }
        if let Some(th) = a.world.thorns.get(&thorn)
            && th.state == ThornState::Flying
        {
            assert_eq!(frame_state(&a, thorn), thorn_entity);
            let predicted = replicated_position(&thorn_entity, now, 60, &a.tuning.combat);
            max_err = max_err.max(predicted.distance(th.position));
        }
    }
    assert!(max_err < 0.02, "trajectory error {max_err} m");
    // Bombs bounce: a new trajectory segment per bounce, each reproduced as well.
    let t = a.tuning.clone();
    let bomb = a
        .world
        .spawn_projectile(&t, ProjectileKind::Bomb, Owner::Player(p), Vec3::new(0.0, 1.2, -10.0), Vec3::new(3.0, -3.0, 0.0))
        .unwrap();
    let mut entity = frame_state(&a, bomb);
    let mut segments = 1;
    let mut max_err: f32 = 0.0;
    while let Some(b) = a.world.projectiles.get(&bomb) {
        let predicted = replicated_position(&entity, a.world.tick as f64, 60, &t.combat);
        max_err = max_err.max(predicted.distance(b.position));
        a.tick();
        if a.world.projectiles.contains_key(&bomb) {
            let now = frame_state(&a, bomb);
            if now != entity {
                segments += 1;
                entity = now;
            }
        }
    }
    assert!(max_err < 0.02, "bomb trajectory error {max_err} m");
    assert_eq!(segments, 3, "launch + two bounces");
}

#[test]
fn stuck_and_deflected_projectiles_replicate_their_state() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.ticks(5);
    // A Gravewell lobbed at the floor sticks there while it pulls.
    a.select(p, 3, 1);
    a.held.get_mut(&p).unwrap().pitch = -0.8;
    a.tap(p, Buttons::ALT_FIRE, 30);
    let well = *a.world.projectiles.keys().next().unwrap();
    let e = frame_state(&a, well);
    assert!(e.flags & projectile_flags::STUCK != 0);
    let pos = replicated_position(&e, a.world.tick as f64 + 30.0, 60, &a.tuning.combat);
    assert!(
        pos.distance(a.world.projectiles[&well].position) < 0.01,
        "stuck: no trajectory"
    );
    assert_eq!(e.owner, 0);
    // An enemy projectile turned around by a Gust flies straight and is the player's now.
    let d = dummy(&mut a, Vec3::new(20.0, 0.0, -20.0), PI, 1000.0);
    let t = a.tuning.clone();
    let kind = ProjectileKind::ALL.into_iter().find(|k| k.parryable()).unwrap();
    let eye = a.eye(p);
    let id = a
        .world
        .spawn_enemy_projectile(&t, kind, d, Attack::Enemy { kind: 60, attack: 0 }, eye + Vec3::new(0.0, 0.0, -3.0), Vec3::new(0.0, 0.0, 15.0), 10.0)
        .unwrap();
    assert!(frame_state(&a, id).flags & projectile_flags::ENEMY != 0);
    a.select(p, 1, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    a.held.get_mut(&p).unwrap().yaw = 0.0;
    a.tap(p, Buttons::ALT_FIRE, 0);
    let e = frame_state(&a, id);
    assert!(e.flags & projectile_flags::PARRIED != 0);
    assert_eq!(e.owner, 0);
    let later = replicated_position(&e, a.world.tick as f64 + 10.0, 60, &t.combat);
    assert!((later.y - e.position.y).abs() < 1e-3, "no gravity once deflected");
}

#[test]
fn crowded_frame_is_cheap_in_deltas() {
    // 8 players, 50 enemies, 150 projectiles in flight.
    let mut a = Arena::coop();
    for i in 0..8u8 {
        let p = a.player(i, Vec3::new(i as f32 * 2.0 - 8.0, 0.0, 20.0), 0.0);
        a.world.players.get_mut(&p).unwrap().notarget = true;
    }
    for i in 0..50 {
        let x = (i % 10) as f32 * 3.0 - 15.0;
        let z = -10.0 - (i / 10) as f32 * 3.0;
        dummy(&mut a, Vec3::new(x, 0.0, z), 0.0, 100.0);
    }
    a.ticks(5);
    let t = a.tuning.clone();
    for i in 0..150 {
        let a_ = i as f32 * 0.04;
        a.world.spawn_projectile(
            &t,
            if i % 2 == 0 {
                ProjectileKind::Wisp
            } else {
                ProjectileKind::Bomb
            },
            Owner::Player(shared::sim::PlayerId((i % 8) as u8)),
            Vec3::new(0.0, 30.0 + (i as f32) * 0.1, 0.0),
            Vec3::new(a_.cos() * 5.0, 3.0, a_.sin() * 5.0),
        );
    }
    let mut prev = NetFrame::new();
    a.world.replicate(&a.tuning, &mut prev);
    let full = encode_full(&prev).len();
    // Players walking around, enemies idle, projectiles flying: two ticks later (one snapshot).
    for p in a.held.values_mut() {
        p.forward = 127;
    }
    a.ticks(2);
    let mut next = NetFrame::new();
    a.world.replicate(&a.tuning, &mut next);
    let mut delta = Vec::new();
    encode_delta(&prev, &next, &mut delta);
    println!(
        "8 players + 50 enemies + 150 projectiles: full {full} B, delta {} B ({} entities)",
        delta.len(),
        next.len()
    );
    assert_eq!(next.len(), 8 + 50 + 150);
    assert!(full < 6000, "full frame {full} B");
    assert!(
        delta.len() < 200,
        "delta {} B: flying projectiles must cost nothing",
        delta.len()
    );
}

#[test]
fn own_state_and_events_survive_bincode() {
    let mut c = PlayerCombat {
        damage: 37.5,
        hard: 12.25,
        ..Default::default()
    };
    c.weapons.primary_cooldown = [3, 0, 17, 50];
    c.weapons.charge = 22;
    let own = OwnState {
        movement: shared::movement::MoveState::spawn(Vec3::new(1.0, 2.0, 3.0)),
        combat: c,
        style: Default::default(),
    };
    let snap = ServerUnreliable::Snapshot(SnapshotMessage {
        tick: 99,
        baseline_tick: Some(97),
        last_input_seq: 1234,
        own: Some(own.clone()),
        entities: vec![0, 0],
        fx: vec![FxEvent {
            seq: 7,
            tick: 98,
            fx: Fx::Shot {
                shooter: shared::sim::PlayerId(2),
                attack: Attack::Blast,
                origin: QPos::from(Vec3::new(1.0, 1.6, 2.0)),
                ends: vec![QPos::from(Vec3::new(10.0, 1.0, -20.0)); 10],
            },
        }],
    });
    let bytes = protocol::encode(&snap);
    let back: ServerUnreliable = protocol::decode_server_message(&bytes).unwrap();
    assert_eq!(back, snap);
    let combat_bytes = protocol::encode(&c).len();
    println!(
        "own combat state: {combat_bytes} B; snapshot with a 10-pellet shot: {} B",
        bytes.len()
    );
    assert!(combat_bytes < 48);

    // A kill event round-trips through the reliable channel encoding.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let h = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), PI, 30.0);
    a.ticks(5);
    let head = dummy_head(&a, h);
    a.aim(p, head);
    let ev = a.tap(p, Buttons::FIRE, 1);
    let kill = kills(&ev)[0];
    let msg = protocol::ServerMessage::Game(GameEvent::Killed(kill));
    let back: protocol::ServerMessage =
        protocol::decode_server_message(&protocol::encode(&msg)).unwrap();
    assert_eq!(back, msg);
}
