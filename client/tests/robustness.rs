//! Server hardening end to end (docs/network.md, "Robustness"): who may use dev chat
//! commands, and that they are rate limited like chat. A real server and real clients over
//! UDP loopback on the shared virtual clock (see `common`).

mod common;

use client::net::NetEvent;
use common::*;
use server::{LobbyConfig, ServerConfig};

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
