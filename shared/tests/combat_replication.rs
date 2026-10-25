//! Combat replication: trajectory-replicated projectiles/prisms reproduce the server position,
//! a crowded frame stays small in deltas, own state / events survive bincode, and
//! `assets/tuning.ron` spells out every combat value equal to the defaults.

mod combat_support;

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
use shared::sim::{Attack, Owner, PlayerCombat, ProjectileKind};
use shared::tuning::{CombatTuning, Tuning};

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
    // A flak grenade (gravity) and a prism, thrown up and forward.
    a.select(p, 1, 1);
    a.held.get_mut(&p).unwrap().pitch = 0.4;
    a.set_buttons(p, Buttons::ALT_FIRE | Buttons::THROW);
    a.tick();
    a.set_buttons(p, Buttons::NONE);
    let flak = *a.world.projectiles.keys().next().unwrap();
    let prism = *a.world.prisms.keys().next().unwrap();
    let flak_entity = frame_state(&a, flak);
    let prism_entity = frame_state(&a, prism);
    assert_eq!(flak_entity.kind, kind::FLAK);
    assert_eq!(prism_entity.kind, kind::PRISM);
    let mut max_err: f32 = 0.0;
    for _ in 0..25 {
        a.tick();
        let now = a.world.tick as f64;
        if let Some(f) = a.world.projectiles.get(&flak) {
            let predicted = replicated_position(&flak_entity, now, 60, &a.tuning.combat);
            max_err = max_err.max(predicted.distance(f.position));
            // The entity doesn't change while it flies: no bandwidth.
            assert_eq!(frame_state(&a, flak), flak_entity);
        }
        if let Some(pr) = a.world.prisms.get(&prism)
            && !pr.resting
            && frame_state(&a, prism) == prism_entity
        {
            let predicted = replicated_position(&prism_entity, now, 60, &a.tuning.combat);
            max_err = max_err.max(predicted.distance(pr.position));
        }
    }
    assert!(max_err < 0.02, "trajectory error {max_err} m");
}

#[test]
fn parried_and_stuck_projectiles_replicate_their_state() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let husk = a.husk(Vec3::new(0.0, 0.0, -8.0), PI);
    a.ticks(5);
    a.select(p, 3, 0);
    let head = a.husk_head(husk);
    a.aim(p, head);
    a.tap(p, Buttons::ALT_FIRE, 30);
    let sticky = *a.world.projectiles.keys().next().unwrap();
    let e = frame_state(&a, sticky);
    assert!(e.flags & projectile_flags::STUCK != 0);
    let pos = replicated_position(&e, a.world.tick as f64 + 30.0, 60, &a.tuning.combat);
    assert!(
        pos.distance(a.world.projectiles[&sticky].position) < 0.01,
        "stuck: no trajectory"
    );
    assert_eq!(e.owner, 0);
}

#[test]
fn crowded_frame_is_cheap_in_deltas() {
    // 8 players, 50 husks, 150 projectiles in flight.
    let mut a = Arena::coop();
    for i in 0..8u8 {
        let p = a.player(i, Vec3::new(i as f32 * 2.0 - 8.0, 0.0, 20.0), 0.0);
        a.world.players.get_mut(&p).unwrap().notarget = true;
    }
    for i in 0..50 {
        let x = (i % 10) as f32 * 3.0 - 15.0;
        let z = -10.0 - (i / 10) as f32 * 3.0;
        a.husk(Vec3::new(x, 0.0, z), 0.0);
    }
    a.ticks(5);
    let t = a.tuning.clone();
    for i in 0..150 {
        let a_ = i as f32 * 0.04;
        a.world.spawn_projectile(
            &t,
            if i % 2 == 0 {
                ProjectileKind::Rivet
            } else {
                ProjectileKind::Rocket
            },
            Owner::Player(shared::sim::PlayerId((i % 8) as u8)),
            Vec3::new(0.0, 30.0 + (i as f32) * 0.1, 0.0),
            Vec3::new(a_.cos() * 5.0, 3.0, a_.sin() * 5.0),
        );
    }
    let mut prev = NetFrame::new();
    a.world.replicate(&a.tuning, &mut prev);
    let full = encode_full(&prev).len();
    // Players walking around, husks idle, projectiles flying: two ticks later (one snapshot).
    for p in a.held.values_mut() {
        p.forward = 127;
    }
    a.ticks(2);
    let mut next = NetFrame::new();
    a.world.replicate(&a.tuning, &mut next);
    let mut delta = Vec::new();
    encode_delta(&prev, &next, &mut delta);
    println!(
        "8 players + 50 husks + 150 projectiles: full {full} B, delta {} B ({} entities)",
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
                attack: Attack::Pellet,
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
    let h = a.husk(Vec3::new(0.0, 0.0, -10.0), PI);
    a.ticks(5);
    let head = a.husk_head(h);
    a.aim(p, head);
    let ev = a.tap(p, Buttons::FIRE, 1);
    let kill = kills(&ev)[0];
    let msg = protocol::ServerMessage::Game(GameEvent::Killed(kill));
    let back: protocol::ServerMessage =
        protocol::decode_server_message(&protocol::encode(&msg)).unwrap();
    assert_eq!(back, msg);
}
