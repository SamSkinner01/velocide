//! Arena encounters end to end on `crucible_of_veins`: a real server and two real clients over
//! UDP loopback (virtual clock). Player A walks into arena 1's trigger while B lags behind in
//! the start hall: the entry door locks, B is pulled to the rally point (and its client snaps),
//! the waves spawn in order with co-op scaling (2 players: 1.5x enemies, 1.5x health), killing
//! everything opens the exit door, the encounter never restarts, and `restart_level` resets
//! it. Also: the encounter entity carries "enemies left" to the clients.

use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient, NetEvent};
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{Server, ServerConfig};
use shared::input::PlayerInput;
use shared::protocol::{GameEvent, NetId, door_flags, encounter_flags, kind};
use shared::sim::encounter::EncounterPhase;
use shared::sim::{PlayerId, SimEvent};

const STEP: Duration = Duration::from_millis(2);
const LEVEL: &str = "crucible_of_veins";

fn config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: LEVEL.to_string(),
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        // Built-in enemy definitions (the shipped RON files are compiled in).
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

    fn game_events(&self) -> impl Iterator<Item = &GameEvent> {
        self.events.iter().filter_map(|e| match e {
            NetEvent::Game(g) => Some(g),
            _ => None,
        })
    }
}

struct Net {
    server: Server,
    clients: Vec<TestClient>,
    sim_events: Vec<SimEvent>,
}

impl Net {
    fn new() -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        Self {
            server: Server::new(config()).expect("server starts"),
            clients: Vec::new(),
            sim_events: Vec::new(),
        }
    }

    fn add(&mut self, name: &str) {
        let net = NetClient::connect(self.server.local_addr(), ClientConfig::new(name))
            .expect("client starts");
        self.clients.push(TestClient {
            net,
            timestep: FixedTimestep::new(60.0),
            events: Vec::new(),
        });
    }

    fn step(&mut self) {
        self.server.update(STEP);
        self.sim_events.extend(self.server.drain_sim_events());
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
    }

    fn run(&mut self, secs: f64) {
        for _ in 0..(secs / STEP.as_secs_f64()).round() as u32 {
            self.step();
        }
    }

    fn run_until(&mut self, secs: f64, mut done: impl FnMut(&Net) -> bool) -> bool {
        for _ in 0..(secs / STEP.as_secs_f64()).round() as u32 {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    fn place(&mut self, p: PlayerId, pos: Vec3) {
        let w = self.server.world_mut();
        w.players.get_mut(&p).unwrap().movement = shared::movement::MoveState::spawn(pos);
    }

    fn phase(&self, i: usize) -> EncounterPhase {
        self.server
            .world()
            .encounters
            .states
            .get(i)
            .map_or(EncounterPhase::Idle, |s| s.phase)
    }

    fn door_open(&self, id: u32) -> bool {
        self.server.world().collision.is_door_open(id)
    }

    fn living_enemies(&self) -> Vec<(f32, f32)> {
        self.server
            .world()
            .enemies
            .values()
            .filter(|e| e.is_alive())
            .map(|e| (e.health, e.max_health))
            .collect()
    }

    fn count(&self, f: impl Fn(&SimEvent) -> bool) -> usize {
        self.sim_events.iter().filter(|e| f(e)).count()
    }
}

#[test]
fn arena_one_locks_rallies_scales_waves_and_clears_for_good() {
    let mut n = Net::new();
    n.add("A");
    n.add("B");
    assert!(n.run_until(10.0, |n| n.clients.iter().all(|c| c.net.is_in_game())));
    let (a, b) = (n.clients[0].id(), n.clients[1].id());
    for p in [a, b] {
        n.server.world_mut().players.get_mut(&p).unwrap().combat.god = true;
    }
    let level = n.server.world().level.clone();
    let enc = &level.encounters[0];
    let rally = enc.rally.unwrap().position;
    assert_eq!(enc.lock_doors, vec![1]);
    assert_eq!(enc.unlock_doors, vec![2]);
    assert!(
        n.door_open(1) && !n.door_open(2),
        "level start: entry open, exit closed"
    );
    n.run(0.5);
    assert_eq!(
        n.phase(0),
        EncounterPhase::Idle,
        "nobody in the trigger yet"
    );

    // A runs in, B is still in the start hall.
    n.place(a, Vec3::new(0.0, 0.0, -45.0));
    n.place(b, Vec3::new(0.0, 0.0, 20.0));
    assert!(n.run_until(0.2, |n| n.phase(0) == EncounterPhase::Active));
    assert!(!n.door_open(1), "entry door locked");
    let bpos = n.server.world().players[&b].movement.position;
    assert!(
        bpos.distance(rally) < 0.5,
        "B pulled to the rally point, at {bpos}"
    );
    assert!(enc.arena.unwrap().contains(bpos));
    assert_eq!(
        n.count(|e| matches!(e, SimEvent::Respawned { player, .. } if *player == b)),
        1,
        "B got a teleport event"
    );
    assert_eq!(
        n.count(|e| matches!(e, SimEvent::Respawned { player, .. } if *player == a)),
        0,
        "A was inside already"
    );

    // Wave 1 after 0.5 s: 4 authored gnawers x 1.5 = 6, 67.5 HP each.
    assert!(n.run_until(1.0, |n| !n.living_enemies().is_empty()));
    n.run(0.1);
    let wave1 = n.living_enemies();
    assert_eq!(wave1.len(), 6, "2 players: 1.5x the 4 authored gnawers");
    assert!(
        wave1.iter().all(|(_, max)| *max == 67.5),
        "health x1.5: {wave1:?}"
    );
    // Every enemy stands somewhere sensible (inside the arena, not in the slag).
    for e in n.server.world().enemies.values() {
        assert!(
            enc.arena.unwrap().contains(e.position),
            "enemy outside the arena at {}",
            e.position
        );
        assert!(!level.kill_volumes.iter().any(|v| v.contains(e.position)));
        assert!(e.aware && e.encounter == Some((0, 0)));
    }
    // The clients know how many are left.
    n.run(0.2);
    for c in &n.clients {
        let e = c
            .net
            .remote_entity(NetId::encounter(0))
            .expect("encounter entity replicated");
        assert_eq!(e.kind, kind::ENCOUNTER);
        assert!(e.flags & encounter_flags::ACTIVE != 0);
        assert_eq!(
            (e.state, e.health, e.aux & 0xFFFF),
            (1, 6, 2),
            "wave 1/2, 6 left"
        );
    }

    // Kill wave 1 through the damage pipeline: wave 2 comes 1.5 s later with 9 gnawers.
    n.server.kill_all_enemies();
    n.run(0.5);
    assert_eq!(n.phase(0), EncounterPhase::Active);
    assert!(n.living_enemies().is_empty(), "between waves");
    assert!(n.run_until(2.0, |n| !n.living_enemies().is_empty()));
    n.run(0.1);
    assert_eq!(n.living_enemies().len(), 9, "6 authored x 1.5");
    assert!(!n.door_open(2), "exit still closed during the fight");

    // Kill wave 2: cleared, the exit opens (and the entry too).
    n.server.kill_all_enemies();
    assert!(n.run_until(1.0, |n| n.phase(0) == EncounterPhase::Cleared));
    assert!(n.door_open(2), "exit door open");
    assert!(n.door_open(1), "entry door open again");
    n.run(0.3);

    // Events in order, on the server and on both clients.
    let order: Vec<String> = n
        .sim_events
        .iter()
        .filter_map(|e| match e {
            SimEvent::EncounterStarted { index, waves } => Some(format!("start {index} {waves}")),
            SimEvent::WaveStarted {
                index,
                wave,
                waves,
                enemies,
            } => Some(format!("wave {index} {wave}/{waves} {enemies}")),
            SimEvent::EncounterCleared { index } => Some(format!("clear {index}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        ["start 0 2", "wave 0 1/2 6", "wave 0 2/2 9", "clear 0"]
    );
    for c in &n.clients {
        let got: Vec<&GameEvent> = c
            .game_events()
            .filter(|g| {
                matches!(
                    g,
                    GameEvent::EncounterStarted { .. }
                        | GameEvent::WaveStarted { .. }
                        | GameEvent::EncounterCleared { .. }
                )
            })
            .collect();
        assert_eq!(got.len(), 4, "client got {got:?}");
        assert!(matches!(
            got[0],
            GameEvent::EncounterStarted { index: 0, waves: 2 }
        ));
        assert!(matches!(
            got[2],
            GameEvent::WaveStarted {
                wave: 2,
                waves: 2,
                enemies: 9,
                ..
            }
        ));
        assert!(matches!(got[3], GameEvent::EncounterCleared { index: 0 }));
        let door = c.net.remote_entity(NetId::door(2)).expect("door 2");
        assert!(
            door.flags & door_flags::OPEN != 0,
            "client sees the exit open"
        );
        let e = c.net.remote_entity(NetId::encounter(0)).unwrap();
        assert!(e.flags & encounter_flags::CLEARED != 0);
    }
    // B's client snapped to the rally point (teleport = Spawned).
    assert!(
        n.clients[1].events.iter().any(
            |e| matches!(e, NetEvent::Spawned { position, .. } if position.distance(rally) < 0.1)
        ),
        "B's client got the teleport"
    );
    // Spawn effects for every enemy.
    assert_eq!(n.count(|e| matches!(e, SimEvent::EnemySpawned { .. })), 15);

    // Cleared for good: walking through the trigger again does nothing.
    n.place(a, Vec3::new(0.0, 0.0, -20.0));
    n.run(0.2);
    n.place(a, Vec3::new(0.0, 0.0, -45.0));
    n.run(1.5);
    assert_eq!(n.phase(0), EncounterPhase::Cleared);
    assert!(n.living_enemies().is_empty());
    assert!(n.door_open(1) && n.door_open(2));

    // Restart: back to idle, doors as at the level start, and it can be fought again.
    n.server.restart_level();
    n.run(0.2);
    assert_eq!(n.phase(0), EncounterPhase::Idle);
    assert!(n.door_open(1) && !n.door_open(2));
    n.place(a, Vec3::new(0.0, 0.0, -45.0));
    assert!(n.run_until(1.5, |n| n.living_enemies().len() == 6));
    assert!(!n.door_open(1));
}

#[test]
fn checkpoint_reset_hooks_and_single_player_counts() {
    // One player: no scaling. reset_uncleared() (team wiped) puts the arena back to idle.
    let mut n = Net::new();
    n.add("Solo");
    assert!(n.run_until(10.0, |n| n.clients.iter().all(|c| c.net.is_in_game())));
    let a = n.clients[0].id();
    n.server.world_mut().players.get_mut(&a).unwrap().combat.god = true;
    // Arena 2 (Weeping Cloister): gnawers + censers on the mezzanine.
    n.place(a, Vec3::new(0.0, 0.0, -105.0));
    assert!(n.run_until(1.5, |n| n.living_enemies().len() == 6));
    assert_eq!(n.phase(1), EncounterPhase::Active);
    assert!(
        n.living_enemies()
            .iter()
            .all(|(_, max)| *max == 45.0 || *max == 80.0)
    );
    assert!(!n.door_open(3));
    // Some time in the fight, then the modes agent's checkpoint reset.
    n.run(2.0);
    n.server.world_mut().reset_uncleared();
    // (The modes put the team back at the checkpoint, before the trigger.)
    n.place(a, Vec3::new(0.0, 0.0, -93.5));
    n.run(0.1);
    assert_eq!(n.phase(1), EncounterPhase::Idle);
    assert!(
        n.server.world().enemies.is_empty(),
        "the wave's enemies are gone"
    );
    assert!(n.door_open(3), "entry door back open");
    assert!(!n.door_open(4), "exit door back closed");
    // Walking in again restarts it.
    n.place(a, Vec3::new(0.0, 0.0, -105.0));
    assert!(n.run_until(1.5, |n| n.living_enemies().len() == 6));
    // reset_encounter on a single encounter (cleared or not) works the same.
    n.server.world_mut().reset_encounter(1);
    n.place(a, Vec3::new(0.0, 0.0, -93.5));
    n.run(0.3);
    assert_eq!(n.phase(1), EncounterPhase::Idle);
}

#[test]
fn deathmatch_arena_has_no_encounters_but_dev_spawns_work() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut server = Server::new(ServerConfig {
        level: "cistern_nine".into(),
        ..config()
    })
    .expect("server");
    let bot = server.add_bot("bot").unwrap();
    server.world_mut().players.get_mut(&bot).unwrap().combat.god = true;
    let ids = server.spawn_enemies("censer", Vec3::new(10.0, 0.0, 10.0), 0.0, 2);
    assert_eq!(ids.len(), 2);
    for _ in 0..120 {
        server.update(Duration::from_secs_f64(1.0 / 60.0));
    }
    assert!(server.world().encounters.states.is_empty());
    assert!(server.world().ai.nav.is_some(), "nav grid built at load");
    let _ = server.drain_sim_events();
}
