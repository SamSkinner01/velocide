//! Test harness: a real server and real clients talking over UDP loopback, driven on a shared
//! **virtual clock** (every component takes `dt`, including the network simulator), so a 10 s
//! session runs in about a second of wall time and is reproducible with fixed netsim seeds.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient, NetEvent, NetSimConfig};
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput};
use shared::sim::PlayerId;

pub const STEP: Duration = Duration::from_millis(2);

pub type Script = fn(u32) -> PlayerInput;

pub fn server_config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: shared::sim::BUILTIN_LEVEL.to_string(),
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        watch_tuning: false,
        ..Default::default()
    }
}

pub struct TestClient {
    pub net: NetClient,
    pub timestep: FixedTimestep,
    /// Fixed ticks run while in game (the script's clock).
    pub ticks: u32,
    pub script: Script,
    pub scripted: bool,
    pub events: Vec<NetEvent>,
    /// Largest change of the drawn local position caused by `NetClient::update` alone (no
    /// simulation tick in between): a visible pop if corrections weren't smoothed.
    pub max_update_jump: f32,
}

impl TestClient {
    pub fn id(&self) -> PlayerId {
        self.net.local_player_id().expect("connected")
    }
}

pub struct Harness {
    pub server: Server,
    pub clients: Vec<TestClient>,
    /// Virtual seconds.
    pub time: f64,
    /// Server-side positions of every player after each tick.
    pub trajectories: BTreeMap<PlayerId, Vec<(u32, Vec3)>>,
    last_tick: u32,
}

impl Harness {
    pub fn new(config: ServerConfig) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        Self {
            server: Server::new(config).expect("server starts"),
            clients: Vec::new(),
            time: 0.0,
            trajectories: BTreeMap::new(),
            last_tick: 0,
        }
    }

    pub fn add_client(
        &mut self,
        name: &str,
        netsim: Option<NetSimConfig>,
        script: Script,
    ) -> usize {
        self.add_client_with(ClientConfig::new(name).with_netsim(netsim), script)
    }

    pub fn add_client_with(&mut self, config: ClientConfig, script: Script) -> usize {
        let net = NetClient::connect(self.server.local_addr(), config).expect("client starts");
        self.clients.push(TestClient {
            net,
            timestep: FixedTimestep::new(60.0),
            ticks: 0,
            script,
            scripted: true,
            events: Vec::new(),
            max_update_jump: 0.0,
        });
        self.clients.len() - 1
    }

    /// Advances everything by one `STEP` of virtual time.
    pub fn step(&mut self) {
        self.server.update(STEP);
        let tick = self.server.tick();
        if tick != self.last_tick {
            self.last_tick = tick;
            for (id, p) in &self.server.world().players {
                self.trajectories
                    .entry(*id)
                    .or_default()
                    .push((tick, p.movement.position));
            }
        }
        for c in &mut self.clients {
            let alpha = c.timestep.alpha();
            let before = c.net.local_render_position(alpha);
            c.net.update(STEP);
            if let (Some(a), Some(b)) = (before, c.net.local_render_position(alpha)) {
                c.max_update_jump = c.max_update_jump.max(a.distance(b));
            }
            c.events.extend(c.net.drain_events());
            c.timestep.set_hz(c.net.tick_rate() as f64);
            let ticks = c.timestep.advance(STEP.as_secs_f64());
            for _ in 0..ticks {
                if c.scripted && c.net.is_in_game() {
                    let input = (c.script)(c.ticks);
                    c.net.tick(input);
                    c.ticks += 1;
                }
            }
        }
        self.time += STEP.as_secs_f64();
    }

    /// Steps until `done` returns true (true) or `timeout` virtual seconds pass (false).
    pub fn run_until(&mut self, timeout: f64, mut done: impl FnMut(&Harness) -> bool) -> bool {
        let end = self.time + timeout;
        while self.time < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    /// Steps for `secs` virtual seconds, calling `each` after every step.
    pub fn run_for(&mut self, secs: f64, mut each: impl FnMut(&mut Harness)) {
        let end = self.time + secs;
        while self.time < end {
            self.step();
            each(self);
        }
    }

    pub fn wait_in_game(&mut self) {
        let ok = self.run_until(10.0, |h| h.clients.iter().all(|c| c.net.is_in_game()));
        let states: Vec<_> = self
            .clients
            .iter()
            .map(|c| (c.net.state(), c.net.disconnect_reason().map(str::to_owned)))
            .collect();
        assert!(ok, "clients did not get in game: {states:?}");
    }

    /// Server position of `player` at a fractional tick (linear between recorded ticks).
    pub fn server_position_at(&self, player: PlayerId, tick: f64) -> Option<Vec3> {
        let traj = self.trajectories.get(&player)?;
        let i = traj.partition_point(|(t, _)| (*t as f64) <= tick);
        if i == 0 || i >= traj.len() {
            return None;
        }
        let (ta, a) = traj[i - 1];
        let (tb, b) = traj[i];
        let f = ((tick - ta as f64) / (tb - ta) as f64) as f32;
        Some(a.lerp(b, f))
    }
}

/// Runs in a circle (radius ~30 m), jumping now and then.
pub fn script_runner(tick: u32) -> PlayerInput {
    let mut input = PlayerInput {
        forward: 127,
        right: if (tick / 90).is_multiple_of(2) {
            40
        } else {
            -40
        },
        yaw: tick as f32 * 0.0085 + (tick as f32 * 0.05).sin() * 0.2,
        pitch: (tick as f32 * 0.02).sin() * 0.3,
        ..Default::default()
    };
    input.buttons.set(Buttons::JUMP, tick % 75 < 2);
    input
}

/// Strafes back and forth while turning the other way, with frequent jumps.
pub fn script_strafer(tick: u32) -> PlayerInput {
    let mut input = PlayerInput {
        forward: 90,
        right: if (tick / 120).is_multiple_of(2) {
            127
        } else {
            -127
        },
        yaw: -(tick as f32) * 0.012,
        pitch: -0.2,
        weapon: (tick / 200 % 4) as u8,
        ..Default::default()
    };
    input.buttons.set(Buttons::JUMP, tick.is_multiple_of(50));
    input.buttons.set(Buttons::FIRE, tick % 30 < 5);
    input
}

pub fn script_idle(_tick: u32) -> PlayerInput {
    PlayerInput::default()
}

/// Summary of a list of errors.
pub fn summarize(errors: &[f32]) -> (f32, f32, f32) {
    if errors.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted = errors.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mean = sorted.iter().sum::<f32>() / sorted.len() as f32;
    let p95 = sorted[(sorted.len() as f32 * 0.95) as usize];
    let max = *sorted.last().unwrap();
    (mean, p95, max)
}
