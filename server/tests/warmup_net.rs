//! The lobby's warm-up and cosmetics over the real network stack (docs/lobby.md): a lobby
//! server runs the hub (`vestry`) with everyone in it, deaths come back in a second and score
//! nothing, PvP damage follows the next match's mode, a match replaces the hub and the return
//! to the lobby brings it back; `--no-warmup` keeps the old empty lobby; looks are sanitised,
//! shared with everyone and rate-limited.

use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient, NetEvent};
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{LobbyConfig, Server, ServerConfig, WARMUP_LEVEL};
use shared::cosmetics::Cosmetics;
use shared::input::PlayerInput;
use shared::level::LevelKind;
use shared::lobby::{GameMode, LobbyPhase, MatchSettings};
use shared::protocol::GameEvent;
use shared::sim::{Attack, DamageInfo, HitTarget, ModeEvent, Owner, PlayerId};

const STEP: Duration = Duration::from_millis(2);

fn config(warmup: bool) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: "cistern_nine".into(),
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        watch_tuning: false,
        lobby: LobbyConfig {
            warmup,
            ..LobbyConfig::enabled()
        },
        ..Default::default()
    }
}

struct TestClient {
    net: NetClient,
    timestep: FixedTimestep,
    events: Vec<NetEvent>,
}

impl TestClient {
    fn id(&self) -> PlayerId {
        self.net.local_player_id().expect("connected")
    }
}

struct Net {
    server: Server,
    clients: Vec<TestClient>,
    time: f64,
}

impl Net {
    fn new(config: ServerConfig) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        Self {
            server: Server::new(config).expect("server starts"),
            clients: Vec::new(),
            time: 0.0,
        }
    }

    fn add(&mut self, config: ClientConfig) -> usize {
        let net = NetClient::connect(self.server.local_addr(), config).expect("client starts");
        self.clients.push(TestClient {
            net,
            timestep: FixedTimestep::new(60.0),
            events: Vec::new(),
        });
        self.clients.len() - 1
    }

    fn step(&mut self) {
        self.server.update(STEP);
        for c in &mut self.clients {
            c.net.update(STEP);
            c.events.extend(c.net.drain_events());
            c.timestep.set_hz(c.net.tick_rate() as f64);
            for _ in 0..c.timestep.advance(STEP.as_secs_f64()) {
                if c.net.is_in_game() {
                    c.net.tick(PlayerInput::default());
                }
            }
        }
        self.time += STEP.as_secs_f64();
    }

    fn run(&mut self, secs: f64) {
        let end = self.time + secs;
        while self.time < end {
            self.step();
        }
    }

    fn run_until(&mut self, secs: f64, mut done: impl FnMut(&Net) -> bool) -> bool {
        let end = self.time + secs;
        while self.time < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn wait(&mut self, secs: f64, what: &str, done: impl FnMut(&Net) -> bool) {
        assert!(self.run_until(secs, done), "timed out waiting for: {what}");
    }

    /// Every client is in the warm-up hub with a body.
    fn all_in_warmup(&self) -> bool {
        self.clients.iter().all(|c| {
            c.net.in_warmup()
                && c.net.is_in_game()
                && c.net
                    .world()
                    .is_some_and(|w| w.level.kind == LevelKind::Hub)
        })
    }
}

#[test]
fn the_lobby_runs_the_warmup_around_every_match() {
    let mut n = Net::new(config(true));
    let a = n.add(ClientConfig::new("Alpha"));
    let b = n.add(ClientConfig::new("Bravo"));
    n.wait(8.0, "both in the warm-up", Net::all_in_warmup);
    let (pa, pb) = (n.clients[a].id(), n.clients[b].id());
    assert!(n.server.in_warmup() && n.server.world_running());
    assert_eq!(n.server.lobby_phase(), LobbyPhase::Lobby);
    assert_eq!(n.server.world().level_name, WARMUP_LEVEL);
    assert!(n.server.world().in_warmup());
    assert_eq!(n.server.world().players.len(), 2, "everyone has a body");
    assert!(n.server.lobby_state().warmup);
    // The hub is never a match level, the host can't pick it.
    let err = n
        .server
        .set_match_settings(MatchSettings::new(GameMode::Campaign, WARMUP_LEVEL))
        .unwrap_err();
    assert!(err.contains("warm-up"), "{err}");
    // A bot gets a body too.
    let bot = n.server.add_bot("Dummy").unwrap();
    assert!(n.server.world().players.contains_key(&bot));

    // Co-op next: knockback only. Deathmatch next: players hurt each other.
    let t = n.server.tuning().clone();
    let hit = |n: &mut Net| {
        let w = n.server.world_mut();
        let at = w.players[&pb].movement.position + Vec3::Y;
        let info = DamageInfo::new(30.0, Owner::Player(pa), Attack::Quill).at(at, Vec3::NEG_Z);
        w.apply_damage(&t, HitTarget::Player(pb), info);
        w.players[&pb].combat.damage
    };
    n.run(2.0); // spawn protection runs out
    n.server
        .set_match_settings(MatchSettings::new(GameMode::Campaign, "crucible_of_veins"))
        .unwrap();
    assert!(!n.server.world().rules.friendly_fire);
    assert_eq!(hit(&mut n), 0.0, "co-op next: no damage between players");
    n.server
        .set_match_settings(MatchSettings::new(GameMode::Deathmatch, "cistern_nine"))
        .unwrap();
    assert!(n.server.world().rules.friendly_fire);
    assert!(
        hit(&mut n) > 0.0,
        "deathmatch next: players hurt each other"
    );
    // ...with the deathmatch balance, on the clients too (their prediction must match).
    let pvp = shared::tuning::Tuning::default().for_mode(GameMode::Deathmatch);
    assert_eq!(n.server.tuning(), &pvp, "the warm-up runs the next mode's balance");
    n.wait(2.0, "PvP balance on the clients", |n| {
        n.clients.iter().all(|c| c.net.tuning() == &pvp)
    });

    // A death comes back within about a second, and nothing is scored.
    n.server.world_mut().kill_player(&t, pa, Attack::World);
    assert!(n.server.world().players[&pa].combat.dead);
    n.run(1.3);
    assert!(
        !n.server.world().players[&pa].combat.dead,
        "back after ~1 s"
    );
    assert!(
        !n.clients.iter().any(|c| c.events.iter().any(|e| matches!(
            e,
            NetEvent::Game(GameEvent::Mode(
                ModeEvent::Scoreboard(_) | ModeEvent::Ended(_)
            ))
        ))),
        "no scoreboard or results in the warm-up"
    );

    // Countdown: the warm-up keeps running; then the match replaces it.
    let tick = n.server.tick();
    n.server.begin_countdown(true).unwrap();
    n.run(0.5);
    assert!(
        n.server.tick() > tick,
        "the warm-up ticks during the countdown"
    );
    assert!(n.server.world().in_warmup());
    n.wait(6.0, "the match on every client", |n| {
        n.clients.iter().all(|c| {
            c.net.lobby_phase() == Some(LobbyPhase::InMatch)
                && c.net.is_in_game()
                && c.net
                    .world()
                    .is_some_and(|w| w.level_name == "cistern_nine")
        })
    });
    assert!(!n.server.in_warmup() && !n.server.world().in_warmup());
    assert_eq!(n.server.world().players.len(), 3);

    // Results, then everyone is back in a fresh warm-up, un-readied.
    n.server.end_match();
    n.wait(2.0, "results", |n| {
        n.clients
            .iter()
            .all(|c| c.net.lobby_phase() == Some(LobbyPhase::Results))
    });
    n.server.return_to_lobby();
    n.wait(6.0, "back in the warm-up", Net::all_in_warmup);
    assert_eq!(n.server.world().level_name, WARMUP_LEVEL);
    assert_eq!(n.server.world().players.len(), 3);
    assert!(
        n.server
            .lobby_state()
            .players
            .iter()
            .all(|p| p.bot || !p.ready)
    );

    // A late joiner lands in the warm-up too.
    let c = n.add(ClientConfig::new("Charlie"));
    n.wait(6.0, "the late joiner in the warm-up", Net::all_in_warmup);
    assert!(n.clients[c].net.is_in_game());
    // The last human leaving mid-match returns an empty server to its (warm-up) lobby.
    n.server.start_match().unwrap();
    n.run(1.0);
    for c in &mut n.clients {
        c.net.disconnect();
    }
    n.wait(4.0, "empty server back in the lobby", |n| {
        n.server.lobby_phase() == LobbyPhase::Lobby
    });
    assert!(n.server.in_warmup());
}

#[test]
fn without_the_warmup_the_lobby_has_no_world() {
    let mut n = Net::new(config(false));
    let a = n.add(ClientConfig::new("Alpha"));
    n.wait(5.0, "lobby state", |n| n.clients[a].net.lobby().is_some());
    n.run(0.5);
    assert!(!n.server.in_warmup() && !n.server.world_running());
    assert!(n.server.world().players.is_empty());
    assert!(!n.clients[a].net.is_in_game() && n.clients[a].net.world().is_none());
    assert!(!n.clients[a].net.lobby().unwrap().warmup);
    // The match still starts and returns as before.
    n.server.begin_countdown(true).unwrap();
    n.wait(6.0, "match", |n| n.clients[a].net.is_in_game());
    n.server.return_to_lobby();
    n.wait(3.0, "back in the empty lobby", |n| {
        n.clients[a].net.in_lobby() && n.clients[a].net.world().is_none()
    });
}

#[test]
fn looks_are_sanitised_shared_and_rate_limited() {
    let mut n = Net::new(config(true));
    let look = Cosmetics {
        mask: 4,
        crest: 5,
        cloak: 2,
        emblem: 3,
        primary: 11,
        secondary: 7,
        flame: 3,
    };
    let a = n.add(ClientConfig::new("Alpha").with_cosmetics(look));
    let b = n.add(ClientConfig::new("Bravo"));
    n.wait(8.0, "both connected", |n| {
        n.clients
            .iter()
            .all(|c| c.net.lobby().is_some_and(|l| l.players.len() == 2))
    });
    let (pa, pb) = (n.clients[a].id(), n.clients[b].id());
    // Sent with the welcome: everyone sees it in the player list and the lobby state.
    n.wait(2.0, "look shared", |n| {
        n.clients[b]
            .net
            .players()
            .get(&pa)
            .is_some_and(|p| p.cosmetics == look)
            && n.clients[b]
                .net
                .lobby()
                .and_then(|l| l.player(pa))
                .is_some_and(|p| p.cosmetics == look)
    });
    assert_eq!(
        n.clients[a].net.players()[&pb].cosmetics,
        Cosmetics::default()
    );
    // Out-of-range values are wrapped into the catalogues before anyone sees them.
    let wild = Cosmetics {
        mask: 200,
        crest: 77,
        cloak: 9,
        emblem: 250,
        primary: 99,
        secondary: 255,
        flame: 44,
    };
    n.clients[b].net.set_cosmetics(wild);
    n.wait(2.0, "sanitised look", |n| {
        n.clients[a].net.players()[&pb].cosmetics == wild.sanitized()
    });
    assert_eq!(
        n.server.lobby_state().player(pb).unwrap().cosmetics,
        wild.sanitized()
    );
    // Spam: at most the burst gets through.
    let before = n.clients[a]
        .events
        .iter()
        .filter(|e| matches!(e, NetEvent::PlayerUpdated(p) if p.id == pb))
        .count();
    for i in 0..60u32 {
        n.clients[b].net.set_cosmetics(Cosmetics::from_seed(i));
    }
    n.run(1.0);
    let updates = n.clients[a]
        .events
        .iter()
        .filter(|e| matches!(e, NetEvent::PlayerUpdated(p) if p.id == pb))
        .count()
        - before;
    assert!((3..=13).contains(&updates), "{updates} looks got through");
    // Bots have a look of their own.
    let bot = n.server.add_bot("Dummy").unwrap();
    n.run(0.5);
    let bot_look = n.clients[a].net.players()[&bot].cosmetics;
    assert_eq!(bot_look, server::lobby::bot_cosmetics(bot));
}
