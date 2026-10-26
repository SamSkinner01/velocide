//! Play of the game over the real network stack (docs/potg.md): a server and real `NetClient`s
//! on UDP loopback with a virtual clock. A deathmatch's best burst of kills is announced to
//! everyone once it closes (`HighlightMark`) and at the frag limit every client gets
//! `PlayOfTheGame` before the results (`ModeEvent::Ended`); a co-op match sends neither.

use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient, NetEvent};
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{Server, ServerConfig};
use shared::input::PlayerInput;
use shared::movement::MoveState;
use shared::protocol::GameEvent;
use shared::sim::{
    Attack, DamageFlags, DamageInfo, Highlight, HitTarget, ModeEvent, ModeRules, Owner, PlayerId,
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
}

impl TestClient {
    fn id(&self) -> PlayerId {
        self.net.local_player_id().expect("connected")
    }

    fn marks(&self) -> Vec<Highlight> {
        self.events
            .iter()
            .filter_map(|e| match e {
                NetEvent::Game(GameEvent::HighlightMark(h)) => Some(h.clone()),
                _ => None,
            })
            .collect()
    }

    fn potg(&self) -> Option<Highlight> {
        self.events.iter().find_map(|e| match e {
            NetEvent::Game(GameEvent::PlayOfTheGame(h)) => Some(h.clone()),
            _ => None,
        })
    }

    /// Index of the first event matching `f`.
    fn index_of(&self, f: impl Fn(&NetEvent) -> bool) -> Option<usize> {
        self.events.iter().position(f)
    }

    fn ended_at(&self) -> Option<usize> {
        self.index_of(|e| matches!(e, NetEvent::Game(GameEvent::Mode(ModeEvent::Ended(_)))))
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

    fn wait_in_game(&mut self) {
        let ok = self.run_until(10.0, |n| n.clients.iter().all(|c| c.net.is_in_game()));
        assert!(ok, "clients did not get in game");
        self.run(0.3);
    }

    fn place(&mut self, p: PlayerId, pos: Vec3) {
        let w = self.server.world_mut();
        w.players.get_mut(&p).unwrap().movement = MoveState::spawn(pos);
    }

    /// `by` headshots `victim` through the damage pipeline (server-side).
    fn frag(&mut self, victim: PlayerId, by: PlayerId) {
        let t = self.server.tuning().clone();
        let w = self.server.world_mut();
        let at = w.players[&victim].movement.position + Vec3::Y * 1.6;
        let info = DamageInfo::new(500.0, Owner::Player(by), Attack::Quill)
            .at(at, Vec3::NEG_Z)
            .flags(DamageFlags::HITSCAN | DamageFlags::HEADSHOT);
        w.apply_damage(&t, HitTarget::Player(victim), info);
    }
}

#[test]
fn deathmatch_marks_its_best_burst_and_sends_the_play_of_the_game_before_the_results() {
    // The built-in arena: deathmatch rules without a lobby.
    let mut n = Net::new(config(shared::sim::BUILTIN_LEVEL));
    let a = n.add("Alpha");
    let b = n.add("Bravo");
    let c = n.add("Charlie");
    n.wait_in_game();
    n.server
        .world_mut()
        .set_mode_rules(ModeRules::deathmatch(3, 0.0));
    let (pa, pb, pc) = (n.clients[a].id(), n.clients[b].id(), n.clients[c].id());
    n.place(pa, Vec3::new(0.0, 0.0, 0.0));
    n.place(pb, Vec3::new(0.0, 0.0, -12.0));
    n.place(pc, Vec3::new(8.0, 0.0, -10.0));
    n.run(1.8); // spawn protection runs out

    // Alpha: a double kill (headshots, 0.3 s apart).
    n.frag(pb, pa);
    n.run(0.3);
    n.frag(pc, pa);
    // Nothing is announced while the burst could still go on...
    n.run(1.5);
    assert!(n.clients.iter().all(|c| c.marks().is_empty()));
    // ...then everyone hears of it.
    let ok = n.run_until(3.0, |n| n.clients.iter().all(|c| !c.marks().is_empty()));
    assert!(ok, "the double kill was marked for everyone");
    let mark = n.clients[b].marks()[0].clone();
    assert_eq!(mark.player, pa);
    assert_eq!(mark.title, "HEADSHOT DOUBLE");
    assert!(mark.end_tick > mark.start_tick);
    assert!(n.clients[a].potg().is_none());

    // Charlie gets one back later: not better, not announced.
    n.run(2.0); // respawns
    n.frag(pa, pc);
    n.run(4.0);
    assert_eq!(n.clients[a].marks().len(), 1);

    // Alpha's third frag hits the limit: the play of the game, then the results.
    n.frag(pb, pa);
    let ok = n.run_until(2.0, |n| n.clients.iter().all(|c| c.ended_at().is_some()));
    assert!(ok, "the match ended for everyone");
    for client in &n.clients {
        let potg = client.potg().expect("play of the game");
        assert_eq!(potg, mark, "the double kill beats the single frags");
        let potg_at = client
            .index_of(|e| matches!(e, NetEvent::Game(GameEvent::PlayOfTheGame(_))))
            .unwrap();
        assert!(potg_at < client.ended_at().unwrap(), "before the results");
    }
}

#[test]
fn a_burst_still_open_at_the_frag_limit_can_be_the_play_of_the_game() {
    let mut n = Net::new(config(shared::sim::BUILTIN_LEVEL));
    let a = n.add("Alpha");
    let b = n.add("Bravo");
    let c = n.add("Charlie");
    n.wait_in_game();
    n.server
        .world_mut()
        .set_mode_rules(ModeRules::deathmatch(2, 0.0));
    let (pa, pb, pc) = (n.clients[a].id(), n.clients[b].id(), n.clients[c].id());
    n.place(pa, Vec3::new(0.0, 0.0, 0.0));
    n.place(pb, Vec3::new(0.0, 0.0, -50.0));
    n.place(pc, Vec3::new(10.0, 0.0, 0.0));
    n.run(1.8);
    // Bravo's two frags from ~50 m end the match at once: never marked, still the play of the
    // game.
    n.frag(pa, pb);
    n.run(0.2);
    n.frag(pc, pb);
    let ok = n.run_until(2.0, |n| n.clients.iter().all(|c| c.potg().is_some()));
    assert!(ok);
    for client in &n.clients {
        assert!(client.marks().is_empty());
        let potg = client.potg().unwrap();
        assert_eq!((potg.player, potg.title.as_str()), (pb, "LONGSHOT DOUBLE"));
    }
}

#[test]
fn coop_matches_have_no_play_of_the_game() {
    let mut n = Net::new(config("crucible_of_veins"));
    let a = n.add("Alpha");
    n.wait_in_game();
    let pa = n.clients[a].id();
    let feet = n.server.world().players[&pa].movement.position;
    // The player kills a Mould, then the match ends.
    let ids = n
        .server
        .spawn_enemies("gnawer", feet + Vec3::new(0.0, 0.0, -6.0), 0.0, 1);
    assert_eq!(ids.len(), 1, "a gnawer spawned");
    n.run(0.2);
    let t = n.server.tuning().clone();
    let w = n.server.world_mut();
    let info = DamageInfo::new(5000.0, Owner::Player(pa), Attack::Quill)
        .at(feet + Vec3::new(0.0, 1.0, -6.0), Vec3::NEG_Z)
        .flags(DamageFlags::HITSCAN);
    w.apply_damage(&t, HitTarget::Entity(ids[0]), info);
    n.run(4.0);
    n.server.end_match();
    let ok = n.run_until(2.0, |n| n.clients[a].ended_at().is_some());
    assert!(ok, "results");
    n.run(0.5);
    let client = &n.clients[a];
    assert!(
        client
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Game(GameEvent::Killed(_)))),
        "the kill happened"
    );
    assert!(client.marks().is_empty());
    assert!(client.potg().is_none());
}
