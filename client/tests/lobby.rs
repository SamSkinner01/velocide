//! Lobby and match flow end to end (docs/lobby.md): a real server and real clients over UDP
//! loopback on the shared virtual clock (see `common`). Host rules, settings validation, the
//! warm-up around every match, ready-up → countdown → match on every client,
//! join-in-progress, host migration, return to the lobby (level state reset; with and without
//! the warm-up), results, autostart, pings, disconnects and spam.

mod common;

use client::net::{ClientConfig, NetEvent, NetSimConfig};
use common::*;
use server::{LobbyConfig, ServerConfig};
use shared::level::LevelKind;
use shared::lobby::{GameMode, LobbyPhase, LobbyRequest, MAX_FRAG_LIMIT, MatchSettings};
use shared::sim::PlayerId;

const TOKEN: u64 = 0x5EC2_E7E5;

/// A lobby server with the warm-up (like every real one); `token` = listen server (the client
/// with the token hosts; dev commands on for it, like `ServerConfig::host`), `None` = dedicated
/// rules (no `--dev-commands`).
fn lobby_config(token: Option<u64>) -> ServerConfig {
    let mut config = server_config();
    config.lobby = LobbyConfig {
        enabled: true,
        host_token: token,
        ..LobbyConfig::enabled()
    };
    config.dev_commands = token.is_some();
    config
}

/// A lobby server without a world in its lobby (`--no-warmup`).
fn empty_lobby_config(token: Option<u64>) -> ServerConfig {
    let mut config = lobby_config(token);
    config.lobby.warmup = false;
    config
}

fn host_client(name: &str) -> ClientConfig {
    let mut c = ClientConfig::new(name);
    c.host_token = Some(TOKEN);
    c
}

fn wait(h: &mut Harness, secs: f64, what: &str, done: impl FnMut(&Harness) -> bool) {
    let ok = h.run_until(secs, done);
    assert!(ok, "timed out waiting for: {what}");
}

/// Waits until every client has a lobby state listing `players` players.
fn wait_lobby(h: &mut Harness, players: usize) {
    wait(h, 5.0, "everyone in the lobby", |h| {
        h.clients.iter().all(|c| {
            c.net
                .lobby()
                .is_some_and(|l| l.players.len() == players && l.host.is_some())
        })
    });
}

fn notices(c: &TestClient) -> Vec<String> {
    c.events
        .iter()
        .filter_map(|e| match e {
            NetEvent::Notice { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn wait_notice(h: &mut Harness, client: usize, contains: &str) {
    let needle = contains.to_lowercase();
    wait(h, 3.0, &format!("notice '{contains}'"), |h| {
        notices(&h.clients[client])
            .iter()
            .any(|n| n.to_lowercase().contains(&needle))
    });
}

fn all_in_game(h: &Harness) -> bool {
    h.clients.iter().all(|c| c.net.is_in_game())
}

/// Every client plays the match (not the warm-up hub).
fn all_in_match(h: &Harness) -> bool {
    h.clients.iter().all(|c| {
        c.net.lobby_phase() == Some(LobbyPhase::InMatch)
            && c.net.is_in_game()
            && c.net
                .world()
                .is_some_and(|w| w.level.kind != LevelKind::Hub)
    })
}

/// Every client is in the warm-up hub with a body.
fn all_in_warmup(h: &Harness) -> bool {
    h.clients.iter().all(|c| {
        c.net.in_warmup()
            && c.net.is_in_game()
            && c.net
                .world()
                .is_some_and(|w| w.level.kind == LevelKind::Hub)
    })
}

#[test]
fn host_controls_the_lobby_and_the_match_starts_everywhere() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_runner);
    let a = h.add_client("Alpha", None, script_strafer);
    let b = h.add_client("Bravo", None, script_idle);
    wait_lobby(&mut h, 3);

    let host_id = h.clients[host].id();
    assert!(h.clients[host].net.is_host());
    assert!(!h.clients[a].net.is_host() && !h.clients[b].net.is_host());
    assert_eq!(h.server.lobby_host(), Some(host_id));
    assert_eq!(h.server.lobby_phase(), LobbyPhase::Lobby);
    // The lobby is the warm-up: everyone has a body in the hub.
    wait(&mut h, 5.0, "everyone in the warm-up", all_in_warmup);
    assert_eq!(h.server.world().players.len(), 3, "bodies in the warm-up");
    for c in &h.clients {
        assert!(c.net.in_lobby() && c.net.lobby().unwrap().warmup);
        assert!(
            c.net.levels().iter().any(|l| l.name == "cistern_nine"),
            "the server's level catalog arrives"
        );
    }

    // A guest can neither change settings nor start.
    let before = h.server.match_settings().clone();
    let mut dm = MatchSettings::new(GameMode::Deathmatch, "cistern_nine");
    h.clients[a].net.set_match_settings(dm.clone());
    h.clients[a].net.start_match(true);
    wait_notice(&mut h, a, "only the host");
    h.run_for(0.5, |_| {});
    assert_eq!(h.server.match_settings(), &before);
    assert_eq!(h.server.lobby_phase(), LobbyPhase::Lobby);

    // The host's settings are clamped and reach everyone.
    dm.frag_limit = 100_000;
    dm.time_limit_secs = 300;
    dm.max_players = 6;
    h.clients[host].net.set_match_settings(dm);
    wait(&mut h, 3.0, "settings everywhere", |h| {
        h.clients.iter().all(|c| {
            c.net
                .lobby()
                .is_some_and(|l| l.settings.level == "cistern_nine")
        })
    });
    let s = &h.clients[b].net.lobby().unwrap().settings;
    assert_eq!(
        (s.mode, s.frag_limit, s.time_limit_secs, s.max_players),
        (GameMode::Deathmatch, MAX_FRAG_LIMIT, 300, 6)
    );

    // START needs everyone ready (FORCE doesn't).
    h.clients[host].net.start_match(false);
    wait_notice(&mut h, host, "not everyone is ready");
    for c in &mut h.clients {
        c.net.set_ready(true);
    }
    wait(&mut h, 3.0, "all ready", |h| {
        h.clients[host].net.lobby().is_some_and(|l| l.all_ready())
    });
    h.clients[host].net.start_match(false);
    wait(&mut h, 2.0, "countdown everywhere", |h| {
        h.clients
            .iter()
            .all(|c| matches!(c.net.lobby_phase(), Some(LobbyPhase::Countdown { .. })))
    });
    let countdown_start = h.time;
    assert!(
        h.server.world().in_warmup(),
        "the warm-up runs through the countdown"
    );
    wait(&mut h, 6.0, "match on every client", all_in_match);
    let waited = h.time - countdown_start;
    assert!(
        (2.5..4.5).contains(&waited),
        "the 3 s countdown took {waited:.2} s"
    );
    assert_eq!(h.server.lobby_phase(), LobbyPhase::InMatch);
    assert_eq!(h.server.world().players.len(), 3);
    assert_eq!(h.server.world().match_settings.mode, GameMode::Deathmatch);
    assert_eq!(h.server.world().match_settings.frag_limit, MAX_FRAG_LIMIT);
    for c in &h.clients {
        assert_eq!(c.net.world().unwrap().level_name, "cistern_nine");
        assert_eq!(c.net.lobby_phase(), Some(LobbyPhase::InMatch));
    }
    // Everyone plays: predicted movement is exact after the level change.
    let spawn = h.clients[host].net.local_state().unwrap().position;
    h.run_for(2.0, |_| {});
    let moved = h.clients[host]
        .net
        .local_state()
        .unwrap()
        .position
        .distance(spawn);
    assert!(moved > 3.0, "host runs ({moved:.1} m)");
    for c in &h.clients {
        assert_eq!(c.net.stats().corrections, 0, "{}", c.id());
    }
}

#[test]
fn invalid_settings_are_rejected() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_idle);
    wait_lobby(&mut h, 1);
    let before = h.server.match_settings().clone();

    let cases = [
        (GameMode::Deathmatch, "crucible_of_veins", "arena"),
        (GameMode::Campaign, "cistern_nine", "campaign"),
        (GameMode::Campaign, "no_such_level", "doesn't exist"),
    ];
    for (mode, level, expect) in cases {
        h.clients[host]
            .net
            .set_match_settings(MatchSettings::new(mode, level));
        wait_notice(&mut h, host, expect);
        assert_eq!(h.server.match_settings(), &before, "{mode:?} on {level}");
    }
    // Sandbox levels work in both modes.
    for mode in GameMode::ALL {
        assert!(
            h.server
                .set_match_settings(MatchSettings::new(mode, "movement_test"))
                .is_ok()
        );
    }

    // Co-op is for up to 4: with 5 present the host can't switch to it.
    assert!(
        h.server
            .set_match_settings(MatchSettings::new(GameMode::Deathmatch, "cistern_nine"))
            .is_ok()
    );
    for i in 0..4 {
        h.server.add_bot(&format!("Bot{i}")).unwrap();
    }
    let err = h
        .server
        .set_match_settings(MatchSettings::new(GameMode::Campaign, "crucible_of_veins"))
        .unwrap_err();
    assert!(err.contains("5 players"), "{err}");
    assert_eq!(h.server.match_settings().mode, GameMode::Deathmatch);
    // Settings are locked outside the lobby.
    h.server.begin_countdown(true).unwrap();
    assert!(
        h.server
            .set_match_settings(MatchSettings::new(GameMode::Deathmatch, "proving_ground"))
            .is_err()
    );
}

#[test]
fn players_join_a_match_in_progress() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_runner);
    wait_lobby(&mut h, 1);
    h.clients[host]
        .net
        .set_match_settings(MatchSettings::new(GameMode::Campaign, "movement_test"));
    h.run_for(0.3, |_| {});
    h.clients[host].net.start_match(true);
    wait(&mut h, 6.0, "host in the match", all_in_match);

    let late = h.add_client("Late", None, script_strafer);
    wait(&mut h, 5.0, "late joiner in the match", all_in_match);
    assert_eq!(h.clients[late].net.lobby_phase(), Some(LobbyPhase::InMatch));
    assert_eq!(
        h.clients[late].net.world().unwrap().level_name,
        "movement_test"
    );
    assert_eq!(h.server.world().players.len(), 2);
    assert!(!h.clients[late].net.is_host());
    // Nobody went through a countdown or lobby on the way in.
    assert!(!h.clients[late].events.iter().any(|e| matches!(
        e,
        NetEvent::LobbyChanged {
            phase: LobbyPhase::Lobby | LobbyPhase::Countdown { .. },
            ..
        }
    )));
}

#[test]
fn dedicated_host_migrates_when_the_host_leaves() {
    let mut h = Harness::new(lobby_config(None));
    let first = h.add_client("First", None, script_idle);
    wait_lobby(&mut h, 1);
    let second = h.add_client("Second", None, script_idle);
    wait_lobby(&mut h, 2);
    let third = h.add_client("Third", None, script_idle);
    wait_lobby(&mut h, 3);
    assert!(h.clients[first].net.is_host(), "first to join hosts");
    let second_id = h.clients[second].id();

    h.clients[first].net.disconnect();
    wait(&mut h, 3.0, "host passes to the next player", |h| {
        h.clients[second].net.is_host()
            && h.clients[third]
                .net
                .lobby()
                .is_some_and(|l| l.host == Some(second_id) && l.players.len() == 2)
    });
    assert!(h.clients[third].events.iter().any(|e| matches!(
        e,
        NetEvent::Chat { from: None, text, .. } if text.contains("Second is now the host")
    )));
    // The new host's commands work, the others' still don't.
    h.clients[second]
        .net
        .set_match_settings(MatchSettings::new(GameMode::Deathmatch, "cistern_nine"));
    h.clients[third]
        .net
        .set_match_settings(MatchSettings::new(GameMode::Campaign, "movement_test"));
    wait_notice(&mut h, third, "only the host");
    h.run_for(0.5, |_| {});
    assert_eq!(h.server.match_settings().level, "cistern_nine");

    // A listen server never hands the host role to a guest.
    let mut l = Harness::new(lobby_config(Some(TOKEN)));
    let host = l.add_client_with(host_client("Host"), script_idle);
    let guest = l.add_client("Guest", None, script_idle);
    wait_lobby(&mut l, 2);
    l.clients[host].net.disconnect();
    l.run_for(1.0, |_| {});
    assert!(!l.clients[guest].net.is_host());
    assert_eq!(l.server.lobby_host(), None);
    // ...and a wrong token claims nothing.
    l.clients[guest]
        .net
        .send_lobby(LobbyRequest::ClaimHost { token: TOKEN + 1 });
    l.run_for(0.5, |_| {});
    assert_eq!(l.server.lobby_host(), None);
}

#[test]
fn the_warmup_comes_back_after_every_match() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_runner);
    let guest = h.add_client("Guest", None, script_strafer);
    wait_lobby(&mut h, 2);
    wait(&mut h, 5.0, "warm-up", all_in_warmup);
    // Running around the hub: prediction stays exact there too.
    let start = h.clients[host].net.local_state().unwrap().position;
    h.run_for(2.0, |_| {});
    let moved = h.clients[host]
        .net
        .local_state()
        .unwrap()
        .position
        .distance(start);
    assert!(moved > 3.0, "the host runs in the warm-up ({moved:.1} m)");
    for c in &mut h.clients {
        c.net.set_ready(true);
    }
    wait(&mut h, 2.0, "ready", |h| {
        h.clients[host].net.lobby().is_some_and(|l| l.all_ready())
    });
    h.clients[host].net.start_match(false);
    wait(&mut h, 6.0, "first match", all_in_match);
    h.run_for(1.0, |_| {});
    h.server.world_mut().collision.set_door_open(1, false);

    // Back in the lobby = back in a fresh warm-up, un-readied, still on the game screen.
    h.clients[host].net.return_to_lobby();
    wait(&mut h, 4.0, "everyone back in the warm-up", all_in_warmup);
    assert_eq!(h.server.lobby_phase(), LobbyPhase::Lobby);
    assert_eq!(h.server.world().players.len(), 2);
    wait(&mut h, 2.0, "ready reset", |h| {
        h.clients.iter().all(|c| {
            c.net
                .lobby()
                .is_some_and(|l| l.ready_count() == 0 && l.players.len() == 2)
        })
    });
    // Second match: a fresh level (the door is open again), inputs flow, prediction exact.
    h.clients[host].net.start_match(true);
    wait(&mut h, 6.0, "second match", all_in_match);
    assert!(h.server.world().collision.is_door_open(1));
    let start = h.clients[guest].net.local_state().unwrap().position;
    h.run_for(2.0, |_| {});
    let moved = h.clients[guest]
        .net
        .local_state()
        .unwrap()
        .position
        .distance(start);
    assert!(
        moved > 3.0,
        "guest moves in the second match ({moved:.1} m)"
    );
    for c in &h.clients {
        let s = c.net.stats();
        assert!(
            s.corrections <= 2,
            "{}: {} corrections",
            c.id(),
            s.corrections
        );
    }
}

#[test]
fn return_to_an_empty_lobby_resets_the_level_and_ready_state() {
    let mut h = Harness::new(empty_lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_runner);
    let guest = h.add_client("Guest", None, script_strafer);
    wait_lobby(&mut h, 2);
    for c in &mut h.clients {
        c.net.set_ready(true);
    }
    wait(&mut h, 2.0, "ready", |h| {
        h.clients[host].net.lobby().is_some_and(|l| l.all_ready())
    });
    h.clients[host].net.start_match(false);
    wait(&mut h, 6.0, "first match", all_in_match);
    h.run_for(1.0, |_| {});

    // Change level state (the proving ground's door starts open) and try a guest return.
    h.server.world_mut().collision.set_door_open(1, false);
    h.clients[guest].net.return_to_lobby();
    wait_notice(&mut h, guest, "only the host");
    assert_eq!(h.server.lobby_phase(), LobbyPhase::InMatch);

    h.clients[host].net.return_to_lobby();
    wait(&mut h, 3.0, "everyone back in the lobby", |h| {
        h.clients
            .iter()
            .all(|c| c.net.in_lobby() && !c.net.is_in_game() && c.net.world().is_none())
    });
    assert_eq!(h.server.lobby_phase(), LobbyPhase::Lobby);
    assert!(h.server.world().players.is_empty());
    assert!(
        h.server.world().collision.is_door_open(1),
        "a fresh level: the door is open again"
    );
    wait(&mut h, 2.0, "ready reset", |h| {
        h.clients.iter().all(|c| {
            c.net
                .lobby()
                .is_some_and(|l| l.ready_count() == 0 && l.players.len() == 2)
        })
    });
    assert!(h.clients[host].net.remote_entities().is_empty());

    // Second match: inputs flow again and prediction stays exact.
    h.clients[host].net.start_match(true);
    wait(&mut h, 6.0, "second match", all_in_match);
    let start = h.clients[guest].net.local_state().unwrap().position;
    h.run_for(2.0, |_| {});
    let moved = h.clients[guest]
        .net
        .local_state()
        .unwrap()
        .position
        .distance(start);
    assert!(
        moved > 3.0,
        "guest moves in the second match ({moved:.1} m)"
    );
    for c in &h.clients {
        let s = c.net.stats();
        assert!(
            s.corrections <= 1,
            "{}: {} corrections",
            c.id(),
            s.corrections
        );
    }
    assert_eq!(h.server.world().players.len(), 2);
}

#[test]
fn end_match_shows_results_then_returns_to_the_lobby() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_runner);
    let guest = h.add_client("Guest", None, script_idle);
    wait_lobby(&mut h, 2);
    h.clients[host].net.start_match(true);
    wait(&mut h, 6.0, "match", all_in_match);

    h.server.end_match();
    wait(&mut h, 2.0, "results everywhere", |h| {
        h.clients
            .iter()
            .all(|c| c.net.lobby_phase() == Some(LobbyPhase::Results))
    });
    let timeout = h.clients[guest].net.lobby().unwrap().results_timeout;
    assert!(timeout.is_some_and(|t| t > 20), "{timeout:?}");
    // The level keeps running behind the results screen.
    assert!(all_in_game(&h));
    h.clients[host].net.return_to_lobby();
    wait(&mut h, 2.0, "lobby after results", |h| {
        h.clients.iter().all(|c| c.net.in_lobby())
    });

    // Nobody continues: the results screen times out by itself.
    h.clients[host].net.start_match(true);
    wait(&mut h, 6.0, "second match", all_in_match);
    h.server.end_match();
    let ended = h.time;
    wait(
        &mut h,
        server::lobby::RESULTS_TIMEOUT_SECS + 2.0,
        "results timeout",
        |h| h.server.lobby_phase() == LobbyPhase::Lobby,
    );
    assert!(h.time - ended >= server::lobby::RESULTS_TIMEOUT_SECS - 0.1);
}

#[test]
fn autostart_starts_when_everyone_is_ready() {
    let mut config = lobby_config(None);
    config.lobby.autostart = true;
    let mut h = Harness::new(config);
    let a = h.add_client("A", None, script_idle);
    let b = h.add_client("B", None, script_idle);
    wait_lobby(&mut h, 2);
    assert!(h.clients[a].net.lobby().unwrap().autostart);
    h.clients[a].net.set_ready(true);
    h.run_for(1.0, |_| {});
    assert_eq!(h.server.lobby_phase(), LobbyPhase::Lobby, "B isn't ready");
    h.clients[b].net.set_ready(true);
    wait(&mut h, 2.0, "autostart countdown", |h| {
        matches!(h.server.lobby_phase(), LobbyPhase::Countdown { .. })
    });
    // Someone who isn't ready joins: the countdown waits for them.
    let c = h.add_client("C", None, script_idle);
    wait(&mut h, 3.0, "countdown cancelled", |h| {
        h.server.lobby_phase() == LobbyPhase::Lobby
    });
    wait_lobby(&mut h, 3);
    h.clients[c].net.set_ready(true);
    wait(&mut h, 6.0, "autostarted match", all_in_match);
    assert_eq!(h.server.world().players.len(), 3);
}

#[test]
fn pings_reach_every_player() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_idle);
    let far = h.add_client(
        "Far",
        Some(NetSimConfig::new(60.0, 0.0, 0.0).with_seed(3)),
        script_idle,
    );
    wait_lobby(&mut h, 2);
    let far_id = h.clients[far].id();
    let host_id = h.clients[host].id();
    h.run_for(4.0, |_| {});
    let lobby = h.clients[host].net.lobby().unwrap();
    let far_ping = lobby.ping(far_id).unwrap();
    let host_ping = lobby.ping(host_id).unwrap();
    println!("pings: host {host_ping} ms, far {far_ping} ms");
    assert!(
        (90..250).contains(&far_ping),
        "120 ms RTT player shows {far_ping} ms"
    );
    assert!(host_ping < 40, "loopback player shows {host_ping} ms");
    // Everyone sees the same server-measured pings; our own comes from our own RTT.
    let seen_by_far = h.clients[far].net.lobby().unwrap().ping(far_id).unwrap();
    assert_eq!(seen_by_far, far_ping);
    let own = h.clients[far].net.player_ping(far_id).unwrap();
    assert!((90.0..250.0).contains(&own), "own rtt {own}");
    assert_eq!(
        h.clients[host].net.player_ping(far_id),
        Some(far_ping as f32)
    );
    // Pings keep flowing during the match (scoreboard).
    h.clients[host].net.start_match(true);
    wait(&mut h, 6.0, "match", all_in_match);
    h.run_for(2.0, |_| {});
    let p = h.clients[host].net.player_ping(far_id).unwrap();
    assert!((90.0..250.0).contains(&p), "in-match ping {p}");
}

#[test]
fn disconnects_during_countdown_and_match_are_handled() {
    let mut h = Harness::new(lobby_config(None));
    let a = h.add_client("A", None, script_runner);
    wait_lobby(&mut h, 1);
    let b = h.add_client("B", None, script_idle);
    wait_lobby(&mut h, 2);
    h.clients[a].net.start_match(true);
    wait(&mut h, 2.0, "countdown", |h| {
        matches!(h.server.lobby_phase(), LobbyPhase::Countdown { .. })
    });
    h.clients[b].net.disconnect();
    wait(&mut h, 6.0, "match with the one left", |h| {
        h.clients[a].net.lobby_phase() == Some(LobbyPhase::InMatch) && h.clients[a].net.is_in_game()
    });
    assert_eq!(h.server.world().players.len(), 1);

    // The last player leaves mid-match: the server goes back to an empty lobby.
    h.clients[a].net.disconnect();
    wait(&mut h, 3.0, "empty server back in the lobby", |h| {
        h.server.lobby_phase() == LobbyPhase::Lobby
    });
    assert!(h.server.world().players.is_empty());
    // A newcomer finds the lobby and becomes host.
    let c = h.add_client("C", None, script_idle);
    wait(&mut h, 3.0, "newcomer hosts", |h| {
        h.clients[c].net.is_host()
    });
    assert!(h.clients[c].net.in_lobby());
}

#[test]
fn full_servers_and_spam_are_handled() {
    let mut config = lobby_config(Some(TOKEN));
    config.max_players = 2;
    let mut h = Harness::new(config);
    let host = h.add_client_with(host_client("Host"), script_idle);
    let guest = h.add_client("Guest", None, script_idle);
    wait_lobby(&mut h, 2);
    let extra = h.add_client("Extra", None, script_idle);
    wait(&mut h, 3.0, "refused", |h| {
        h.clients[extra].net.is_disconnected()
    });
    let reason = h.clients[extra]
        .net
        .disconnect_reason()
        .unwrap()
        .to_string();
    assert!(reason.contains("full"), "{reason}");

    // Ready spam: at most the burst (10) gets through.
    let guest_id = h.clients[guest].id();
    for i in 0..60 {
        h.clients[guest].net.set_ready(i % 2 == 0);
    }
    h.run_for(1.0, |_| {});
    let updates = h.clients[host]
        .events
        .iter()
        .filter(|e| matches!(e, NetEvent::PlayerUpdated(p) if p.id == guest_id))
        .count();
    assert!(
        (5..=13).contains(&updates),
        "{updates} ready toggles got through"
    );
    // The host raises the limit: now a third player fits.
    let mut s = h.server.match_settings().clone();
    s.max_players = 3;
    h.clients[host].net.set_match_settings(s);
    wait(&mut h, 2.0, "limit raised", |h| {
        h.server.match_settings().max_players == 3
    });
    let third = h.add_client("Third", None, script_idle);
    wait(&mut h, 3.0, "third player joins", |h| {
        h.clients[third].net.lobby().is_some()
    });
    assert!(!h.clients[third].net.is_disconnected());
    let ids: Vec<PlayerId> = h.clients[host]
        .net
        .lobby()
        .unwrap()
        .players
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids.len(), 3);
}

#[test]
fn servers_without_a_lobby_start_in_the_match() {
    // Single-player (and tests) skip the lobby entirely.
    let mut h = Harness::new(server_config());
    let a = h.add_client("Solo", None, script_runner);
    h.wait_in_game();
    wait(&mut h, 2.0, "lobby state", |h| {
        h.clients[a].net.lobby().is_some()
    });
    let lobby = h.clients[a].net.lobby().unwrap();
    assert_eq!(lobby.phase, LobbyPhase::InMatch);
    assert!(!lobby.lobby_enabled);
    h.run_for(1.0, |_| {});
    // "Return to lobby" restarts the level instead.
    h.server.end_match();
    wait(&mut h, 2.0, "results", |h| {
        h.clients[a].net.lobby_phase() == Some(LobbyPhase::Results)
    });
    // Single-player results wait for the player (no automatic return).
    assert_eq!(h.clients[a].net.lobby().unwrap().results_timeout, None);
    h.run_for(server::lobby::RESULTS_TIMEOUT_SECS + 1.0, |_| {});
    assert_eq!(h.server.lobby_phase(), LobbyPhase::Results);
    h.server.return_to_lobby();
    wait(&mut h, 3.0, "restarted", |h| {
        h.clients[a].net.lobby_phase() == Some(LobbyPhase::InMatch) && h.clients[a].net.is_in_game()
    });
    assert_eq!(h.server.world().players.len(), 1);
}

#[test]
fn combat_follows_the_match_settings() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_idle);
    wait_lobby(&mut h, 1);
    let defs: Vec<String> = h
        .server
        .world()
        .enemy_defs
        .names()
        .map(str::to_owned)
        .collect();
    assert!(!defs.is_empty());

    // The warm-up is a world: the host's dev commands work there.
    wait(&mut h, 5.0, "warm-up", all_in_warmup);
    h.clients[host].net.send_chat("/spawn gnawer");
    wait(&mut h, 2.0, "dev command in the warm-up", |h| {
        h.clients[host].events.iter().any(
            |e| matches!(e, NetEvent::Chat { from: None, text, .. } if text.contains("spawned 1")),
        )
    });
    // A lobby without a world has nothing to spawn into: dev commands refuse.
    let mut empty = Harness::new(empty_lobby_config(Some(TOKEN)));
    let other = empty.add_client_with(host_client("Host"), script_idle);
    wait_lobby(&mut empty, 1);
    assert!(
        empty
            .server
            .spawn_enemies("gnawer", glam::Vec3::ZERO, 0.0, 2)
            .is_empty()
    );
    empty.clients[other].net.send_chat("/spawn gnawer");
    wait(&mut empty, 2.0, "dev command refused", |h| {
        h.clients[other].events.iter().any(
            |e| matches!(e, NetEvent::Chat { from: None, text, .. } if text.contains("no match")),
        )
    });

    // Co-op with friendly fire on: the combat rules follow the lobby settings.
    let mut coop = MatchSettings::new(GameMode::Campaign, "movement_test");
    coop.friendly_fire = true;
    h.server.set_match_settings(coop).unwrap();
    h.server.start_match().unwrap();
    wait(&mut h, 5.0, "co-op match", all_in_match);
    let w = h.server.world();
    assert!(w.rules.friendly_fire && w.rules.coop_enemy_scaling);
    let carried: Vec<String> = w.enemy_defs.names().map(str::to_owned).collect();
    assert_eq!(
        carried, defs,
        "enemy definitions carry over to the match world"
    );
    let spawned = h
        .server
        .spawn_enemies("gnawer", glam::Vec3::new(0.0, 0.0, -10.0), 0.0, 2);
    assert_eq!(spawned.len(), 2);
    assert_eq!(h.server.world().enemies.len(), 2);

    // Back in the lobby the match's enemies are gone; deathmatch on the same sandbox level is
    // PvP.
    h.server.return_to_lobby();
    assert!(h.server.world().in_warmup());
    assert!(
        !h.server
            .world()
            .enemies
            .values()
            .any(|e| spawned.contains(&e.id))
    );
    h.server
        .set_match_settings(MatchSettings::new(GameMode::Deathmatch, "movement_test"))
        .unwrap();
    h.server.start_match().unwrap();
    wait(&mut h, 5.0, "deathmatch", all_in_match);
    let w = h.server.world();
    assert!(w.rules.friendly_fire && !w.rules.coop_enemy_scaling);
    assert!(w.enemies.is_empty());

    // Without a lobby the level kind decides, as before the lobby existed.
    let solo = Harness::new(server_config());
    assert_eq!(solo.server.match_settings().mode, GameMode::Deathmatch);
    assert!(
        solo.server.world().rules.friendly_fire,
        "proving ground is an arena"
    );
}

/// The newest server chat line (dev command reply) a client received.
fn last_server_line(c: &TestClient) -> Option<String> {
    c.events.iter().rev().find_map(|e| match e {
        NetEvent::Chat {
            from: None, text, ..
        } => Some(text.clone()),
        _ => None,
    })
}

#[test]
fn dev_commands_are_host_only_and_teleports_work() {
    let mut h = Harness::new(lobby_config(Some(TOKEN)));
    let host = h.add_client_with(host_client("Host"), script_idle);
    let guest = h.add_client("Guest", None, script_idle);
    wait_lobby(&mut h, 2);
    h.server
        .set_match_settings(MatchSettings::new(GameMode::Campaign, "crucible_of_veins"))
        .unwrap();
    h.server.start_match().unwrap();
    wait(&mut h, 5.0, "co-op match", all_in_match);
    h.run_for(0.5, |_| {});
    let guest_id = h.clients[guest].id();
    let host_id = h.clients[host].id();

    // A guest can't cheat (no `/god` in someone's deathmatch).
    h.clients[guest].net.send_chat("/god");
    wait(&mut h, 2.0, "guest refused", |h| {
        last_server_line(&h.clients[guest]).is_some_and(|t| t.contains("host only"))
    });
    assert!(!h.server.world().players[&guest_id].combat.god);

    // The host can: `/goto checkpoint 2` puts them at its spawn and their client follows.
    let target = h.server.world().level.checkpoints[1].spawn.position;
    h.clients[host].net.send_chat("/goto checkpoint 2");
    wait(&mut h, 2.0, "host teleported", |h| {
        let s = h.server.world().players[&host_id].movement.position;
        let c = h.clients[host].net.local_state().map(|m| m.position);
        s.distance(target) < 1.0 && c.is_some_and(|c| c.distance(target) < 1.0)
    });
    assert!(
        last_server_line(&h.clients[host]).is_some_and(|t| t.starts_with("teleported")),
        "{:?}",
        last_server_line(&h.clients[host])
    );
    // `/tp x y z`, and a malformed one answers with the usage.
    h.clients[host].net.send_chat("/tp 0 0 -40");
    wait(&mut h, 2.0, "tp", |h| {
        let s = h.server.world().players[&host_id].movement.position;
        s.distance(glam::Vec3::new(0.0, 0.0, -40.0)) < 1.0
    });
    h.clients[host].net.send_chat("/tp 1 2");
    wait(&mut h, 2.0, "tp usage", |h| {
        last_server_line(&h.clients[host]).is_some_and(|t| t.contains("usage"))
    });
    // `/goto exit` lands in the exit volume (co-op: that starts the extraction countdown).
    h.clients[host].net.send_chat("/goto exit");
    let exit = h.server.world().level.exit.expect("campaign exit");
    wait(&mut h, 3.0, "at the exit", |h| {
        let p = h.server.world().players[&host_id].movement.position;
        exit.contains(p) || exit.contains(p + glam::Vec3::Y)
    });
}
