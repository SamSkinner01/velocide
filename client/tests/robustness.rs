//! Server hardening end to end (docs/network.md, "Robustness"): who may use dev chat
//! commands, that they are rate limited like chat, and that a client can't claim an older
//! view (more lag-compensation rewind) than its connection explains. A real server and real
//! clients over UDP loopback on the shared virtual clock (see `common`).

mod common;

use client::net::NetEvent;
use common::*;
use server::{LobbyConfig, ServerConfig};
use shared::input::PlayerInput;
use shared::protocol::{self, ClientUnreliable, InputPacket};
use shared::tuning::Tuning;

/// Dedicated-server rules: a lobby without a host token (the first player to join hosts).
fn dedicated(dev_commands: bool) -> ServerConfig {
    ServerConfig {
        lobby: LobbyConfig::enabled(),
        dev_commands,
        ..server_config()
    }
}

/// Server chat lines (dev command replies, announcements) a client received.
fn server_lines(c: &TestClient) -> Vec<String> {
    c.events
        .iter()
        .filter_map(|e| match e {
            NetEvent::Chat {
                from: None, text, ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn wait(h: &mut Harness, secs: f64, what: &str, done: impl FnMut(&Harness) -> bool) {
    assert!(h.run_until(secs, done), "timed out waiting for: {what}");
}

/// A host and a guest (joined in that order) in a running match.
fn host_and_guest(config: ServerConfig) -> (Harness, usize, usize) {
    let mut h = Harness::new(config);
    let host = h.add_client("Host", None, script_idle);
    wait(&mut h, 5.0, "a host", |h| h.server.lobby_host().is_some());
    let guest = h.add_client("Guest", None, script_idle);
    wait(&mut h, 5.0, "both in the lobby", |h| {
        h.clients
            .iter()
            .all(|c| c.net.lobby().is_some_and(|l| l.players.len() == 2))
    });
    h.server.start_match().unwrap();
    h.wait_in_game();
    assert_eq!(h.server.lobby_host(), Some(h.clients[host].id()));
    (h, host, guest)
}

/// Sends `/god` from `client` and waits for the reply.
fn god(h: &mut Harness, client: usize) -> String {
    let before = server_lines(&h.clients[client]).len();
    h.clients[client].net.send_chat("/god");
    wait(h, 3.0, "a reply", |h| server_lines(&h.clients[client]).len() > before);
    server_lines(&h.clients[client]).pop().unwrap()
}

fn has_god(h: &Harness, client: usize) -> bool {
    h.server.world().players[&h.clients[client].id()].combat.god
}

#[test]
fn dedicated_servers_need_dev_commands_opted_in_and_then_only_the_host_has_them() {
    // Default dedicated server: nobody, not even the host; `/...` is not broadcast either.
    let (mut h, host, guest) = host_and_guest(dedicated(false));
    let reply = god(&mut h, host);
    assert!(reply.contains("disabled"), "{reply}");
    assert!(!has_god(&h, host));
    h.run_for(0.3, |_| {});
    assert!(
        !h.clients[guest]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Chat { from: Some(_), .. })),
        "a refused command must not be broadcast as chat"
    );

    // `--dev-commands`: the lobby host may, guests may not.
    let (mut h, host, guest) = host_and_guest(dedicated(true));
    let reply = god(&mut h, guest);
    assert!(reply.contains("host only"), "{reply}");
    assert!(!has_god(&h, guest));
    let reply = god(&mut h, host);
    assert_eq!(reply, "god on");
    assert!(has_god(&h, host));
}

/// A client without network delay whose inputs all claim a view `claim_ms` old (its own
/// inputs would claim ~100 ms). Returns the server's RTT-based limit for it and the largest
/// view lag (rewind) the server granted, both in ms.
fn claimed_view_lag(tuning: Tuning, claim_ms: f32) -> (f32, f32) {
    let mut h = Harness::new(server_config());
    h.server.set_tuning(tuning);
    let a = h.add_client("Liar", None, script_idle);
    h.clients[a].scripted = false; // inputs are crafted below
    h.wait_in_game();
    h.run_for(1.0, |_| {}); // RTT samples
    let id = h.clients[a].id();
    let claim_ticks = (claim_ms / 1000.0 * 60.0).round() as u32;
    let mut seq = h.server.world().players[&id].last_input_seq;
    let mut granted = 0.0f32;
    for _ in 0..120 {
        let tick = h.server.tick();
        seq += 1;
        let input = PlayerInput {
            seq,
            view_tick: tick.saturating_sub(claim_ticks),
            ..Default::default()
        };
        let packet = ClientUnreliable::Input(InputPacket {
            ack_tick: None,
            inputs: vec![input],
        });
        h.clients[a].net.send_raw(false, protocol::encode(&packet));
        h.run_until(0.1, |h| h.server.tick() > tick);
        granted = granted.max(h.server.world().players[&id].view_lag_ticks);
    }
    let p = &h.server.world().players[&id];
    assert!(p.inputs_processed >= 100, "crafted inputs processed");
    (p.view_lag_limit_ms.expect("server sets a limit"), granted * 1000.0 / 60.0)
}

#[test]
fn a_low_ping_client_cannot_claim_the_maximum_rewind() {
    // Default tuning: lag_comp_max_ms 350. A LAN client claiming a 350 ms old view (the most
    // the server would rewind, and the longest parry delay) gets what its connection
    // explains: RTT + snapshot interval + max interpolation delay (250) + 50 ms margin.
    let (limit, granted) = claimed_view_lag(Tuning::default(), 350.0);
    println!("claimed 350 ms: limit {limit:.0} ms, granted {granted:.0} ms");
    assert!(limit < 345.0, "limit {limit}");
    // (The limit follows the RTT estimate, which moves by a millisecond or two.)
    assert!(granted < 345.0, "granted {granted} ms of a 350 ms claim");
    assert!((granted - limit).abs() < 5.0, "clamped to the limit: {granted} vs {limit}");

    // With a generous lag_comp_max_ms the RTT bound is what stops a 900 ms claim.
    let mut tuning = Tuning::default();
    tuning.net.lag_comp_max_ms = 1000.0;
    let (limit, granted) = claimed_view_lag(tuning, 900.0);
    println!("claimed 900 ms: limit {limit:.0} ms, granted {granted:.0} ms");
    assert!(granted < limit + 5.0 && granted < 400.0, "granted {granted} ms");
}

#[test]
fn look_alike_and_reserved_names_are_neutralised() {
    let mut h = Harness::new(server_config());
    let a = h.add_client("Bob", None, script_idle);
    h.wait_in_game();
    let b = h.add_client("Bob\u{200B}", None, script_idle); // zero-width space
    let c = h.add_client("\u{202E}SERVER", None, script_idle); // RTL override + reserved
    h.wait_in_game();
    let name = |i: usize| h.server.world().players[&h.clients[i].id()].name.clone();
    assert_eq!(name(a), "Bob");
    assert_eq!(name(b), "Bob (2)", "invisible characters must not make a new name");
    assert_eq!(name(c), "Shrike", "nobody chats as 'server'");
}

#[test]
fn dev_commands_cost_chat_tokens() {
    // Single-player rules (no lobby, dev commands on).
    let mut h = Harness::new(ServerConfig {
        dev_commands: true,
        ..server_config()
    });
    let a = h.add_client("Spammer", None, script_idle);
    h.wait_in_game();
    for _ in 0..20 {
        h.clients[a].net.send_chat("/notarget");
    }
    h.run_for(0.5, |_| {});
    let ran = server_lines(&h.clients[a])
        .iter()
        .filter(|t| t.starts_with("notarget"))
        .count();
    // Chat's token bucket: a burst of 5 (they used to bypass it: 20 replies).
    assert!((5..=6).contains(&ran), "{ran} commands ran");
    // The bucket is shared: chat right after the burst is dropped too.
    h.clients[a].net.send_chat("hello");
    h.run_for(0.3, |_| {});
    assert!(
        !h.clients[a]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Chat { from: Some(_), .. })),
        "chat after a command burst is rate limited"
    );
}
