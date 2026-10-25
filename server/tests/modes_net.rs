//! Style and game modes over the real network stack: a server and real `NetClient`s on UDP
//! loopback with a virtual clock. Style events reach only their owner (plus the replicated
//! meter in `OwnState`), everyone sees ranks on the scoreboard, co-op downed / revive works with
//! a client holding USE, a deathmatch frag limit and the campaign exit end the match with
//! results and move the server to the lobby's Results phase.

use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient, NetEvent};
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput};
use shared::lobby::LobbyPhase;
use shared::movement::MoveState;
use shared::protocol::{GameEvent, NetId, player_flags};
use shared::sim::{
    Attack, DamageInfo, EndReason, Grade, HitTarget, LifeState, MatchResults, ModeEvent, ModeRules,
    Owner, PlayerId, StyleBonus, StyleEvent,
};

const STEP: Duration = Duration::from_millis(2);

fn config(level: &str) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: level.to_string(),
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        enemies_dir: Some(PathBuf::from("/nonexistent/velocide-test/enemies")),
        watch_tuning: false,
        ..Default::default()
    }
}

struct TestClient {
    net: NetClient,
    timestep: FixedTimestep,
    events: Vec<NetEvent>,
    /// Buttons held every tick.
    buttons: Buttons,
}

impl TestClient {
    fn id(&self) -> PlayerId {
        self.net.local_player_id().expect("connected")
    }

    fn style_events(&self) -> Vec<StyleEvent> {
        self.events
            .iter()
            .filter_map(|e| match e {
                NetEvent::Game(GameEvent::Style(s)) => Some(*s),
                _ => None,
            })
            .collect()
    }

    fn mode_events(&self) -> Vec<ModeEvent> {
        self.events
            .iter()
            .filter_map(|e| match e {
                NetEvent::Game(GameEvent::Mode(m)) => Some(m.clone()),
                _ => None,
            })
            .collect()
    }

    fn results(&self) -> Option<MatchResults> {
        self.mode_events().into_iter().find_map(|e| match e {
            ModeEvent::Ended(r) => Some(*r),
            _ => None,
        })
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

    fn add(&mut self, name: &str) -> usize {
        let net = NetClient::connect(self.server.local_addr(), ClientConfig::new(name))
            .expect("client starts");
        self.clients.push(TestClient {
            net,
            timestep: FixedTimestep::new(60.0),
            events: Vec::new(),
            buttons: Buttons::NONE,
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
                    c.net.tick(PlayerInput {
                        buttons: c.buttons,
                        ..Default::default()
                    });
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

    fn wait_in_game(&mut self) {
        let ok = self.run_until(10.0, |n| n.clients.iter().all(|c| c.net.is_in_game()));
        assert!(ok, "clients did not get in game");
        self.run(0.3);
    }

    fn place(&mut self, p: PlayerId, pos: Vec3) {
        let w = self.server.world_mut();
        w.players.get_mut(&p).unwrap().movement = MoveState::spawn(pos);
    }

    /// `by` deals `amount` to `victim` through the damage pipeline (server-side).
    fn hit(&mut self, victim: PlayerId, by: PlayerId, amount: f32) {
        let t = self.server.tuning().clone();
        let w = self.server.world_mut();
        let at = w.players[&victim].movement.position + Vec3::Y;
        let info = DamageInfo::new(amount, Owner::Player(by), Attack::Slug).at(at, Vec3::NEG_Z);
        w.apply_damage(&t, HitTarget::Player(victim), info);
    }
}

#[test]
fn style_goes_to_the_owner_and_ranks_to_everyone() {
    // The built-in arena: deathmatch rules without a lobby.
    let mut n = Net::new(config(shared::sim::BUILTIN_LEVEL));
    let a = n.add("Alpha");
    let b = n.add("Bravo");
    n.wait_in_game();
    let (pa, pb) = (n.clients[a].id(), n.clients[b].id());
    n.place(pa, Vec3::new(-10.0, 0.0, 0.0));
    n.place(pb, Vec3::new(10.0, 0.0, 0.0));
    n.run(1.8); // spawn protection runs out
    n.hit(pb, pa, 500.0);
    n.run(0.5);
    // The killer's feed: FRAG (100 points: CRUDE) and the rank-up, only for them.
    let mine = n.clients[a].style_events();
    assert!(mine.iter().any(|e| matches!(e,
        StyleEvent::Bonus { bonus: StyleBonus::Frag, points, .. } if *points == 100.0)));
    assert!(mine.iter().any(|e| matches!(
        e,
        StyleEvent::Rank {
            rank: 1,
            up: true,
            ..
        }
    )));
    assert!(n.clients[b].style_events().iter().all(|e| e.player() == pb));
    // The meter rides in the owner's own state.
    let view = n.clients[a].net.own_style().copied().expect("own style");
    assert_eq!(view.rank, 1);
    // Everyone's scoreboard shows the frag and the rank.
    for c in &n.clients {
        let board = c
            .mode_events()
            .into_iter()
            .rev()
            .find_map(|e| match e {
                ModeEvent::Scoreboard(b) => Some(b),
                _ => None,
            })
            .expect("scoreboard");
        let row = board.rows.iter().find(|r| r.player == pa).unwrap();
        assert_eq!(row.frags, 1);
        assert_eq!(row.rank, 1);
        assert_eq!(row.style, 100);
        let victim = board.rows.iter().find(|r| r.player == pb).unwrap();
        assert_eq!(victim.deaths, 1);
    }
}

#[test]
fn frag_limit_ends_the_match_into_the_results_phase() {
    let mut n = Net::new(config(shared::sim::BUILTIN_LEVEL));
    let a = n.add("Alpha");
    let b = n.add("Bravo");
    n.wait_in_game();
    n.server
        .world_mut()
        .set_mode_rules(ModeRules::deathmatch(2, 0.0));
    let (pa, pb) = (n.clients[a].id(), n.clients[b].id());
    n.run(0.2);
    for _ in 0..2 {
        n.run(3.8); // respawn (2 s) + spawn protection (1.5 s)
        n.hit(pb, pa, 500.0);
    }
    let ok = n.run_until(2.0, |n| n.clients.iter().all(|c| c.results().is_some()));
    assert!(ok, "results reached everyone");
    let r = n.clients[b].results().unwrap();
    assert_eq!(r.reason, EndReason::FragLimit);
    assert_eq!(r.winner, Some(pa));
    assert_eq!(r.players[0].stats.frags, 2);
    assert_eq!(n.server.lobby_phase(), LobbyPhase::Results);
    let ok = n.run_until(1.0, |n| {
        n.clients
            .iter()
            .all(|c| c.net.lobby_phase() == Some(LobbyPhase::Results))
    });
    assert!(ok, "clients see the Results phase");
}

#[test]
fn ending_the_match_from_the_lobby_flow_builds_results_too() {
    let mut n = Net::new(config("crucible_of_veins"));
    let a = n.add("Alpha");
    n.wait_in_game();
    n.server.end_match();
    let ok = n.run_until(1.0, |n| n.clients[a].results().is_some());
    assert!(ok);
    let r = n.clients[a].results().unwrap();
    assert_eq!(r.reason, EndReason::Host);
    assert!(r.players[0].grades.is_some());
}

#[test]
fn coop_downed_teammate_is_revived_by_a_client_holding_use() {
    let mut n = Net::new(config("crucible_of_veins"));
    let a = n.add("Alpha");
    let b = n.add("Bravo");
    n.wait_in_game();
    let (pa, pb) = (n.clients[a].id(), n.clients[b].id());
    let spot = n.server.world().players[&pb].movement.position;
    let t = n.server.tuning().clone();
    n.server.world_mut().kill_player(&t, pb, Attack::Dev);
    n.run(0.5);
    assert_eq!(n.server.world().mode.life(pb), LifeState::Downed);
    // A sees B downed (DOWNED, not DEAD) and got the event.
    let e = n.clients[a]
        .net
        .remote_entity(NetId::player(pb))
        .expect("B visible");
    assert!(e.flags & player_flags::DOWNED != 0);
    assert!(e.flags & player_flags::DEAD == 0);
    assert!(n.clients[a].mode_events().iter().any(|e| matches!(e,
        ModeEvent::Downed { player, .. } if *player == pb)));
    // A walks up and holds USE.
    n.place(pa, spot + Vec3::new(1.5, 0.0, 0.0));
    n.run(0.3);
    n.clients[a].buttons = Buttons::USE;
    let ok = n.run_until(3.0, |n| {
        n.clients.iter().all(|c| {
            c.mode_events().iter().any(
                |e| matches!(e, ModeEvent::Revived { player, by } if *player == pb && *by == pa),
            )
        })
    });
    assert!(ok, "revived, both clients told");
    for c in &n.clients {
        assert!(c.mode_events().iter().any(|e| matches!(e,
            ModeEvent::Reviving { player, reviver: Some(r), .. } if *player == pb && *r == pa)));
    }
    let pl = &n.server.world().players[&pb];
    assert!(!pl.combat.dead);
    assert_eq!(pl.combat.health(&t.combat), 40.0);
}

#[test]
fn reaching_the_exit_sends_graded_results() {
    let level = shared::level::LevelDef::load_named("crucible_of_veins").expect("level");
    let exit = level.exit.expect("exit");
    let mut n = Net::new(config("crucible_of_veins"));
    let a = n.add("Solo");
    n.wait_in_game();
    let p = n.clients[a].id();
    let c = exit.center();
    n.place(p, Vec3::new(c.x, exit.min.y.max(c.y - 1.0), c.z));
    let ok = n.run_until(2.0, |n| n.clients[a].results().is_some());
    assert!(ok, "level complete");
    let r = n.clients[a].results().unwrap();
    assert_eq!(r.reason, EndReason::Exit);
    assert_eq!(r.total_enemies, 86);
    assert_eq!(r.par_secs, level.par_time);
    let me = &r.players[0];
    assert!(me.extracted);
    let g = me.grades.expect("grades");
    assert_eq!(g.time, Some(Grade::S), "a teleport is well under par");
    assert_eq!(g.kills, Some(Grade::D), "but nobody was killed");
    assert_eq!(n.server.lobby_phase(), LobbyPhase::Results);
}
