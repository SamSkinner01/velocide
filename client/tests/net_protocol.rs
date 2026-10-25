//! Connection lifecycle, validation of hostile data, tuning hot reload and sessions.

mod common;

use std::time::{Duration, Instant};

use client::net::{ClientConfig, ConnectionState, NetClient, NetEvent, Session, SessionKind};
use common::*;
use engine::time::FixedTimestep;
use shared::input::{Buttons, PlayerInput};
use shared::protocol::{
    self, ClientMessage, ClientUnreliable, InputPacket, MAX_WEAPON_SLOTS, PROTOCOL_VERSION,
};

#[test]
fn version_mismatch_is_rejected_with_a_reason() {
    let mut h = Harness::new(server_config());
    let mut config = ClientConfig::new("Old");
    config.protocol_version = PROTOCOL_VERSION + 1;
    let c = h.add_client_with(config, script_idle);
    let ok = h.run_until(5.0, |h| h.clients[c].net.is_disconnected());
    assert!(ok, "client should be disconnected");
    let reason = h.clients[c].net.disconnect_reason().unwrap().to_string();
    assert!(reason.contains("version"), "reason: {reason}");
    assert!(
        h.clients[c]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Rejected { .. }))
    );
    assert!(h.server.world().players.is_empty());
}

#[test]
fn server_full_is_refused() {
    let mut config = server_config();
    config.max_players = 1;
    let mut h = Harness::new(config);
    let a = h.add_client("First", None, script_idle);
    h.wait_in_game();
    let b = h.add_client("Second", None, script_idle);
    let ok = h.run_until(5.0, |h| h.clients[b].net.is_disconnected());
    assert!(ok, "second client should be refused");
    println!("refused: {:?}", h.clients[b].net.disconnect_reason());
    assert!(h.clients[a].net.is_in_game());
    assert_eq!(h.server.world().players.len(), 1);
}

#[test]
fn chat_joins_and_leaves_are_delivered() {
    let mut h = Harness::new(server_config());
    let a = h.add_client("Alpha", None, script_idle);
    let b = h.add_client("Bravo", None, script_idle);
    h.wait_in_game();
    let id_a = h.clients[a].id();
    h.clients[a].net.send_chat("hello\u{7}world  ");
    h.clients[a].net.set_ready(true);
    let ok = h.run_until(2.0, |h| {
        h.clients[b].events.iter().any(|e| {
            matches!(e, NetEvent::Chat { from: Some(_), name, text } if name == "Alpha" && text == "hello world")
        })
    });
    assert!(ok, "chat not received: {:?}", h.clients[b].events);
    assert!(h.run_until(2.0, |h| h.clients[b].net.players()[&id_a].ready));

    // Chat spam is rate limited (burst of 5, then 1 per second).
    for i in 0..20 {
        h.clients[a].net.send_chat(&format!("spam {i}"));
    }
    h.run_for(1.0, |_| {});
    let spam = h.clients[b]
        .events
        .iter()
        .filter(|e| matches!(e, NetEvent::Chat { text, .. } if text.starts_with("spam")))
        .count();
    assert!((4..=6).contains(&spam), "{spam} spam messages got through");
    assert_eq!(h.clients[b].net.player_name(id_a), Some("Alpha"));

    h.clients[a].net.disconnect();
    let ok = h.run_until(3.0, |h| {
        h.clients[b]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::PlayerLeft { id, .. } if *id == id_a))
    });
    assert!(ok, "leave not received");
    assert!(!h.server.world().players.contains_key(&id_a));
    assert!(!h.clients[b].net.players().contains_key(&id_a));
}

#[test]
fn hostile_client_data_is_contained() {
    let mut h = Harness::new(server_config());
    let honest = h.add_client("Honest", None, script_runner);
    let evil = h.add_client("Evil", None, script_idle);
    h.clients[evil].scripted = false;
    h.wait_in_game();
    let evil_id = h.clients[evil].id();

    let mut seed = 0x1234_5678u32;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    // Garbage on both channels.
    for i in 0..30 {
        let len = (rand() % 200) as usize + i;
        let junk: Vec<u8> = (0..len).map(|_| rand() as u8).collect();
        h.clients[evil].net.send_raw(i % 3 == 0, junk);
        h.step();
    }
    // A well-formed packet with absurd contents.
    let inputs: Vec<PlayerInput> = (1..=100)
        .map(|seq| PlayerInput {
            seq,
            forward: i8::MIN,
            right: i8::MIN,
            yaw: f32::NAN,
            pitch: f32::INFINITY,
            buttons: Buttons(0xFFFF),
            weapon: 250,
            weapon_mode: 200,
            view_tick: u32::MAX,
            view_frac: 255,
        })
        .chain([PlayerInput {
            seq: u32::MAX,
            ..Default::default()
        }])
        .collect();
    let packet = ClientUnreliable::Input(InputPacket {
        ack_tick: Some(u32::MAX),
        inputs,
    });
    h.clients[evil]
        .net
        .send_raw(false, protocol::encode(&packet));
    // Oversized chat (dropped) and a control-character chat (sanitised).
    let huge = ClientMessage::Chat {
        text: "x".repeat(100_000),
    };
    h.clients[evil].net.send_raw(true, protocol::encode(&huge));
    // Raw UDP garbage straight at the server socket.
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    for len in [0usize, 1, 20, 500, 1400] {
        let junk: Vec<u8> = (0..len).map(|_| rand() as u8).collect();
        let _ = sock.send_to(&junk, h.server.local_addr());
    }

    h.run_for(2.0, |_| {});

    let stats = h.server.stats();
    println!("decode errors: {}", stats.decode_errors);
    assert!(stats.decode_errors >= 20, "{stats:?}");
    assert!(
        h.clients[honest].net.is_in_game(),
        "honest client unaffected"
    );
    assert!(h.clients[honest].net.stats().corrections == 0);
    let p = &h.server.world().players[&evil_id];
    assert!(p.movement.position.is_finite());
    assert!(p.yaw.is_finite() && p.pitch.is_finite());
    assert!(p.last_input.weapon < MAX_WEAPON_SLOTS);
    assert_eq!(p.last_input.buttons.0 & !protocol::VALID_BUTTONS, 0);
    // Only the last 16 of the 100 inputs were accepted, the bogus seq was ignored.
    assert!(p.inputs_processed <= 16, "{}", p.inputs_processed);
    assert!(p.last_input_seq <= 100);
    // The oversized chat never reached anyone.
    assert!(
        !h.clients[honest]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Chat { text, .. } if text.len() > 1000))
    );
}

#[test]
fn tuning_changes_are_hot_reloaded_and_pushed() {
    let dir = std::env::temp_dir().join(format!("velocide-tuning-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tuning.ron");
    std::fs::write(&path, "(movement: (jump_speed: 15.0))").unwrap();

    let mut config = server_config();
    config.tuning_path = Some(path.clone());
    config.watch_tuning = true;
    let mut h = Harness::new(config);
    let c = h.add_client("Tuner", None, script_runner);
    h.wait_in_game();
    assert_eq!(h.clients[c].net.tuning().movement.jump_speed, 15.0);

    // File watcher events arrive asynchronously: poll in real time.
    let wait_for = |h: &mut Harness, want: f32| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            h.step();
            std::thread::sleep(Duration::from_millis(1));
            if h.clients[c].net.tuning().movement.jump_speed == want {
                return true;
            }
        }
        false
    };
    std::fs::write(
        &path,
        "(movement: (jump_speed: 21.5), net: (interp_delay_ms: 40.0))",
    )
    .unwrap();
    assert!(wait_for(&mut h, 21.5), "tuning change not pushed");
    assert_eq!(h.clients[c].net.tuning().net.interp_delay_ms, 40.0);
    assert!(
        h.clients[c]
            .events
            .iter()
            .filter(|e| matches!(e, NetEvent::TuningChanged))
            .count()
            >= 2
    );

    // A broken file keeps the previous values.
    std::fs::write(&path, "(movement: (jump_speed: oops").unwrap();
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(300) {
        h.step();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(h.server.tuning().movement.jump_speed, 21.5);
    assert_eq!(h.clients[c].net.tuning().movement.jump_speed, 21.5);

    // Prediction keeps matching the server under the new tuning.
    h.run_for(2.0, |_| {});
    assert_eq!(h.clients[c].net.stats().corrections, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Drives a session in real time until `done` or the timeout.
fn drive(
    session: &mut Session,
    timestep: &mut FixedTimestep,
    secs: f64,
    mut done: impl FnMut(&NetClient) -> bool,
) -> bool {
    let start = Instant::now();
    let mut last = Instant::now();
    while start.elapsed().as_secs_f64() < secs {
        let now = Instant::now();
        let dt = now - last;
        last = now;
        let net = session.client_mut();
        net.update(dt);
        for _ in 0..timestep.advance(dt.as_secs_f64()) {
            net.tick(PlayerInput {
                forward: 127,
                ..Default::default()
            });
        }
        if done(net) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn single_player_session_runs_an_in_process_server() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut session = Session::single_player(shared::sim::BUILTIN_LEVEL, "Solo").unwrap();
    assert_eq!(session.kind(), SessionKind::SinglePlayer);
    let addr = session.server().unwrap().local_addr();
    assert!(addr.ip().is_loopback() && addr.port() != 0, "{addr}");
    let mut ts = FixedTimestep::new(60.0);
    assert!(
        drive(&mut session, &mut ts, 10.0, |n| n.is_in_game()),
        "not in game: {:?}",
        session.client().disconnect_reason()
    );
    let start = session.client().local_state().unwrap().position;
    drive(&mut session, &mut ts, 1.0, |_| false);
    let moved = session
        .client()
        .local_state()
        .unwrap()
        .position
        .distance(start);
    assert!(moved > 5.0, "moved {moved}");
    let stats = session.server().unwrap().stats();
    assert_eq!(stats.players, 1);
    assert!(stats.inputs_processed > 30);
    session.shutdown();
}

#[test]
fn listen_server_accepts_a_second_player() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut host = Session::host(0, shared::sim::BUILTIN_LEVEL, 4, "Host").unwrap();
    assert_eq!(host.kind(), SessionKind::Host);
    let addr = host.server().unwrap().connect_addr();
    let mut guest = Session::join(addr, "Guest").unwrap();
    let mut ts_host = FixedTimestep::new(60.0);
    let mut ts_guest = FixedTimestep::new(60.0);
    // Listen servers open in the lobby: once both are there, the host force-starts.
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        drive(&mut host, &mut ts_host, 0.01, |_| false);
        drive(&mut guest, &mut ts_guest, 0.01, |_| false);
        if host.client().is_host() && host.client().lobby().is_some_and(|l| l.players.len() == 2) {
            break;
        }
    }
    assert!(
        host.client().is_host(),
        "the listen server's own player hosts"
    );
    assert!(!guest.client().is_host());
    host.client_mut().start_match(true);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        drive(&mut host, &mut ts_host, 0.01, |_| false);
        drive(&mut guest, &mut ts_guest, 0.01, |_| false);
        if host.client().is_in_game()
            && guest.client().is_in_game()
            && host.client().players().len() == 2
        {
            break;
        }
    }
    assert_eq!(guest.client().state(), ConnectionState::InGame);
    assert_eq!(host.client().players().len(), 2);
    assert_ne!(
        host.client().local_player_id(),
        guest.client().local_player_id()
    );
    // The host eventually sees the guest as an interpolated remote player.
    let guest_id = guest.client().local_player_id().unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5)
        && host
            .client()
            .remote_entity(shared::protocol::NetId::player(guest_id))
            .is_none()
    {
        drive(&mut host, &mut ts_host, 0.01, |_| false);
        drive(&mut guest, &mut ts_guest, 0.01, |_| false);
    }
    assert!(
        host.client()
            .remote_entity(shared::protocol::NetId::player(guest_id))
            .is_some()
    );
    guest.shutdown();
    host.shutdown();
}
