//! Authoritative game server. Used in-process for single-player and listen servers
//! ([`ServerHandle::spawn`]), and by the headless `velocide-server` binary for dedicated
//! servers. See `docs/network.md` for the protocol.
//!
//! The server owns the true [`SimWorld`]. Clients only send inputs; every fixed tick the
//! server applies queued inputs (one movement step each), runs the simulation, records
//! hitboxes for lag compensation, and at `snapshot_rate` sends each client a delta-compressed
//! snapshot plus its own authoritative state for reconciliation.
//!
//! ```no_run
//! use server::{Server, ServerConfig};
//! let mut server = Server::new(ServerConfig::default())?;
//! let mut last = std::time::Instant::now();
//! loop {
//!     let now = std::time::Instant::now();
//!     server.update(now - last);
//!     last = now;
//!     std::thread::sleep(server.suggested_sleep());
//! }
//! # #[allow(unreachable_code)] Ok::<(), anyhow::Error>(())
//! ```

pub mod combat;
mod commands;
pub mod input_queue;
pub mod lagcomp;
pub mod netsim;
pub mod transport;

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use engine::hotreload::FileWatcher;
use engine::time::FixedTimestep;
use glam::Vec3;
use renet::{ClientId, RenetServer, ServerEvent};
use shared::input::PlayerInput;
use shared::protocol::{
    self, CHANNEL_RELIABLE, CHANNEL_UNRELIABLE, ClientMessage, ClientUnreliable, ConnectRequest,
    GameEvent, MAX_INPUTS_PER_PACKET, NetFrame, OwnState, PROTOCOL_VERSION, PlayerInfo,
    ServerMessage, ServerUnreliable, SnapshotMessage, Welcome,
};
use shared::sim::{EnemyRegistry, Hitbox, PlayerId, SimEvent, SimWorld, TickInputs};
use shared::tuning::Tuning;

pub use combat::FxLog;

pub use input_queue::InputQueue;
pub use lagcomp::{LagCompHistory, Rewound};
pub use netsim::{NetSim, NetSimConfig};
pub use transport::{
    NetcodeServerTransport, RateMeter, ServerTransport, TransportStats, UdpLink, connection_config,
};

/// Snapshots older than this many ticks are not used as delta baselines.
pub const MAX_BASELINE_AGE_TICKS: u32 = 120;
/// Sent frames remembered per client (delta baselines).
const SENT_FRAMES_KEPT: usize = 64;
/// Lag compensation history length (ms). Rewinds are clamped by `NetTuning::lag_comp_max_ms`.
pub const LAG_COMP_HISTORY_MS: f32 = 1000.0;
/// A client sending this many undecodable messages is kicked.
const MAX_DECODE_ERRORS: u32 = 200;
/// Per update, at most this many messages are processed per client and channel (the rest is
/// drained and dropped).
const MAX_RELIABLE_PER_UPDATE: usize = 32;
const MAX_UNRELIABLE_PER_UPDATE: usize = 128;
/// Chat rate limit per client: burst size and sustained messages per second.
const CHAT_BURST: f32 = 5.0;
const CHAT_PER_SECOND: f32 = 1.0;
/// Delay between sending `Rejected` and disconnecting, so the message arrives.
const KICK_DELAY_SECS: f64 = 0.5;
/// Simulation events kept for [`Server::drain_sim_events`] when nobody drains them.
const MAX_EVENT_LOG: usize = 8192;

/// Server settings.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// UDP address to listen on. Use port 0 for an ephemeral port (single-player).
    pub bind: SocketAddr,
    pub max_players: usize,
    /// Level name (`assets/levels/<name>.ron` or a built-in level).
    pub level: String,
    /// Simulation ticks per second.
    pub tick_rate: u32,
    /// Snapshots per second; `None` uses `tuning.net.snapshot_rate` (hot-reloadable).
    pub snapshot_rate: Option<f32>,
    /// Network simulator applied to all of the server's traffic.
    pub netsim: Option<NetSimConfig>,
    /// Tuning file; `None` = `assets/tuning.ron`.
    pub tuning_path: Option<PathBuf>,
    /// Watch the tuning file and push changes to clients.
    pub watch_tuning: bool,
    /// Delta-compress snapshots (disable only to measure the savings).
    pub delta_compression: bool,
    pub server_name: String,
    /// Allow dev chat commands (`/spawn husk 3`, `/god`, `/give`...). See `docs/combat.md`.
    pub dev_commands: bool,
    /// Enemy definitions directory; `None` = `assets/enemies` (built-ins as fallback).
    pub enemies_dir: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], shared::DEFAULT_PORT)),
            max_players: 8,
            level: shared::sim::BUILTIN_LEVEL.to_string(),
            tick_rate: shared::sim::DEFAULT_TICK_RATE,
            snapshot_rate: None,
            netsim: None,
            tuning_path: None,
            watch_tuning: true,
            delta_compression: true,
            server_name: format!("{} server", shared::GAME_NAME),
            dev_commands: true,
            enemies_dir: None,
        }
    }
}

impl ServerConfig {
    /// In-process server for single-player: loopback, ephemeral port, one player.
    pub fn single_player(level: &str) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            max_players: 1,
            level: level.to_string(),
            ..Default::default()
        }
    }

    /// Listen server on all interfaces.
    pub fn host(port: u16, level: &str, max_players: usize) -> Self {
        Self {
            bind: SocketAddr::from(([0, 0, 0, 0], port)),
            max_players,
            level: level.to_string(),
            ..Default::default()
        }
    }
}

/// Counters for monitoring and tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerStats {
    pub tick: u32,
    pub clients: usize,
    pub players: usize,
    pub snapshots_sent: u64,
    pub full_snapshots_sent: u64,
    /// Encoded snapshot message bytes (payload only, without transport overhead).
    pub snapshot_bytes: u64,
    /// Of which the delta-encoded entity blocks.
    pub entity_bytes: u64,
    pub inputs_received: u64,
    pub inputs_processed: u64,
    /// Redundant copies (expected: every input is sent several times).
    pub inputs_redundant: u64,
    /// Inputs that never arrived (loss burst longer than the redundancy).
    pub inputs_skipped: u64,
    pub inputs_overflow: u64,
    /// Ticks where a player had no input to simulate.
    pub input_stalls: u64,
    pub decode_errors: u64,
    pub transport: TransportStats,
    /// Average simulation time per tick (microseconds).
    pub tick_time_us: f32,
    /// Cosmetic effect events put into snapshots (all clients, repeats included).
    pub fx_sent: u64,
    /// Reliable combat `GameEvent`s sent (all clients).
    pub combat_events_sent: u64,
    /// Live simulation objects.
    pub enemies: usize,
    pub projectiles: usize,
}

/// Commands for a server running on its own thread ([`ServerHandle::send`]).
#[derive(Clone, Debug)]
pub enum ServerCommand {
    SetTuning(Box<Tuning>),
    ApplyImpulse {
        player: PlayerId,
        impulse: Vec3,
    },
    Kick {
        player: PlayerId,
        reason: String,
    },
    AddBot {
        name: String,
    },
    Chat {
        text: String,
    },
    /// Freeze / resume the simulation (single-player pause menu). The connection stays alive.
    SetPaused(bool),
    /// Put every player back at the level start (single-player instant restart).
    RestartLevel,
    /// Spawns `count` enemies of definition `name` around `position`.
    SpawnEnemy {
        name: String,
        position: Vec3,
        yaw: f32,
        count: u32,
    },
    /// Kills every enemy (through the damage pipeline).
    KillAllEnemies,
    /// PvP damage between players on/off.
    SetFriendlyFire(bool),
}

struct ClientConn {
    id: ClientId,
    player: Option<PlayerId>,
    name: String,
    ready: bool,
    inputs: InputQueue,
    ack_tick: Option<u32>,
    sent_frames: VecDeque<(u32, Arc<NetFrame>)>,
    kick_at: Option<f64>,
    decode_errors: u32,
    /// Chat rate limit: token bucket refilled at `CHAT_PER_SECOND`.
    chat_tokens: f32,
    chat_refill_time: f64,
}

impl ClientConn {
    fn new(id: ClientId) -> Self {
        Self {
            id,
            player: None,
            name: String::new(),
            ready: false,
            inputs: InputQueue::new(),
            ack_tick: None,
            sent_frames: VecDeque::new(),
            kick_at: None,
            decode_errors: 0,
            chat_tokens: CHAT_BURST,
            chat_refill_time: 0.0,
        }
    }

    fn info(&self) -> Option<PlayerInfo> {
        Some(PlayerInfo {
            id: self.player?,
            name: self.name.clone(),
            ready: self.ready,
            bot: false,
        })
    }
}

/// The authoritative server.
pub struct Server {
    config: ServerConfig,
    renet: RenetServer,
    transport: Box<dyn ServerTransport>,
    world: SimWorld,
    tuning: Tuning,
    tuning_path: PathBuf,
    watcher: Option<FileWatcher>,
    timestep: FixedTimestep,
    clients: BTreeMap<ClientId, ClientConn>,
    bots: BTreeMap<PlayerId, String>,
    lagcomp: LagCompHistory,
    snapshot_interval: u32,
    time: f64,
    stats: ServerStats,
    tick_time_accum: Duration,
    tick_time_count: u32,
    hitbox_scratch: Vec<Hitbox>,
    empty_frame: NetFrame,
    /// Simulation frozen (single-player pause): no ticks run, no snapshots are sent, but the
    /// transport keeps flushing so the connection stays alive.
    paused: bool,
    /// Cosmetic combat events waiting to go out in snapshots.
    fx: FxLog,
    /// Simulation events since the last `drain_sim_events` (bounded).
    event_log: Vec<shared::sim::SimEvent>,
    /// Tick of the last and second-to-last snapshot (FX batching).
    last_snapshot_tick: u32,
    prev_snapshot_tick: u32,
}

impl Server {
    /// Loads the level and tuning and starts listening on `config.bind` (UDP).
    pub fn new(config: ServerConfig) -> anyhow::Result<Server> {
        let transport =
            NetcodeServerTransport::new(config.bind, config.max_players, config.netsim)?;
        Self::with_transport(config, Box::new(transport))
    }

    /// Like [`Server::new`] with a custom transport (Steam networking, relays, tests).
    pub fn with_transport(
        config: ServerConfig,
        transport: Box<dyn ServerTransport>,
    ) -> anyhow::Result<Server> {
        let level = shared::sim::load_level(&config.level)?;
        let tuning_path = config
            .tuning_path
            .clone()
            .unwrap_or_else(Tuning::default_path);
        let tuning = Tuning::load_or_default(&tuning_path);
        let watcher = if config.watch_tuning {
            match FileWatcher::new(&[tuning_path.as_path()]) {
                Ok(w) => Some(w),
                Err(err) => {
                    log::warn!("not watching {}: {err}", tuning_path.display());
                    None
                }
            }
        } else {
            None
        };
        let tick_rate = config.tick_rate.clamp(1, 1000);
        let mut world = SimWorld::new(&config.level, Arc::new(level), tick_rate);
        world.enemy_defs = match &config.enemies_dir {
            Some(dir) => EnemyRegistry::load_dir(dir),
            None => EnemyRegistry::load_default(),
        };
        // Enemy navigation: build the nav grid at level load, not on the first enemy tick.
        world.build_nav();
        let mut server = Server {
            renet: RenetServer::new(connection_config()),
            transport,
            world,
            tuning,
            tuning_path,
            watcher,
            timestep: FixedTimestep::new(tick_rate as f64),
            clients: BTreeMap::new(),
            bots: BTreeMap::new(),
            lagcomp: LagCompHistory::new(tick_rate, LAG_COMP_HISTORY_MS),
            snapshot_interval: 2,
            time: 0.0,
            stats: ServerStats::default(),
            tick_time_accum: Duration::ZERO,
            tick_time_count: 0,
            hitbox_scratch: Vec::new(),
            empty_frame: NetFrame::default(),
            paused: false,
            fx: FxLog::default(),
            event_log: Vec::new(),
            last_snapshot_tick: 0,
            prev_snapshot_tick: 0,
            config,
        };
        server.config.tick_rate = tick_rate;
        server.update_snapshot_interval();
        log::info!(
            "server listening on {:?}: level '{}', {} Hz tick, {:.1} Hz snapshots, max {} players{}",
            server.local_addr(),
            server.config.level,
            tick_rate,
            server.snapshot_rate(),
            server.config.max_players,
            server
                .config
                .netsim
                .filter(NetSimConfig::is_active)
                .map(|n| format!(", netsim {n}"))
                .unwrap_or_default()
        );
        Ok(server)
    }

    /// The UDP address clients connect to (with the real port when bound to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.transport.local_addr().unwrap_or(self.config.bind)
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn tick(&self) -> u32 {
        self.world.tick
    }

    pub fn tick_rate(&self) -> u32 {
        self.config.tick_rate
    }

    /// Effective snapshot rate (Hz).
    pub fn snapshot_rate(&self) -> f32 {
        self.config.tick_rate as f32 / self.snapshot_interval as f32
    }

    pub fn world(&self) -> &SimWorld {
        &self.world
    }

    /// Direct access for admin tools and tests. Changing player state is fine (clients are
    /// corrected through reconciliation).
    pub fn world_mut(&mut self) -> &mut SimWorld {
        &mut self.world
    }

    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    pub fn lag_comp(&self) -> &LagCompHistory {
        &self.lagcomp
    }

    pub fn stats(&self) -> ServerStats {
        let mut s = self.stats.clone();
        s.tick = self.world.tick;
        s.clients = self.clients.len();
        s.players = self.world.players.len();
        s.transport = self.transport.stats();
        s.enemies = self.world.enemies.len();
        s.projectiles = self.world.projectiles.len();
        s
    }

    /// Spawns `count` enemies of definition `name` spread around `position` (on the ground
    /// below it when there is one). Returns their ids (empty if the name is unknown).
    pub fn spawn_enemies(
        &mut self,
        name: &str,
        position: Vec3,
        yaw: f32,
        count: u32,
    ) -> Vec<shared::protocol::NetId> {
        let mut ids = Vec::new();
        let n = count.clamp(1, 64);
        for i in 0..n {
            // A small ring so they don't spawn inside each other.
            let offset = if n == 1 {
                Vec3::ZERO
            } else {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                Vec3::new(a.cos(), 0.0, a.sin()) * (0.6 + 0.25 * n as f32).min(6.0)
            };
            let mut pos = position + offset;
            if let Some(hit) = self.world.collision.world.cast_ray(
                pos + Vec3::Y * 1.5,
                Vec3::NEG_Y,
                20.0,
                engine::physics::MASK_MOVEMENT,
            ) {
                pos = hit.point + Vec3::Y * 0.05;
            }
            match self.world.spawn_enemy(name, pos, yaw) {
                Some(id) => ids.push(id),
                None => break,
            }
        }
        ids
    }

    /// Connected players (and bots).
    pub fn player_infos(&self) -> Vec<PlayerInfo> {
        let mut infos: Vec<PlayerInfo> =
            self.clients.values().filter_map(ClientConn::info).collect();
        infos.extend(self.bots.iter().map(|(id, name)| PlayerInfo {
            id: *id,
            name: name.clone(),
            ready: true,
            bot: true,
        }));
        infos.sort_by_key(|i| i.id);
        infos
    }

    /// Replaces the tuning and pushes it to every client.
    pub fn set_tuning(&mut self, tuning: Tuning) {
        if tuning == self.tuning {
            return;
        }
        self.tuning = tuning;
        self.update_snapshot_interval();
        let msg = ServerMessage::Tuning(Box::new(self.tuning.clone()));
        self.broadcast(&msg);
        log::info!("tuning updated and sent to {} clients", self.clients.len());
    }

    /// External velocity change (explosion knockback...).
    pub fn apply_impulse(&mut self, player: PlayerId, impulse: Vec3) {
        self.world.apply_impulse(player, impulse);
    }

    /// Adds a server-side player without a connection (it stands still). Useful for tests
    /// and as a placeholder for AI players.
    pub fn add_bot(&mut self, name: &str) -> Option<PlayerId> {
        let id = self.free_player_id()?;
        let name = self.unique_name(&protocol::sanitize_name(name));
        self.world.add_player(id, &name);
        self.bots.insert(id, name.clone());
        self.broadcast(&ServerMessage::PlayerJoined(PlayerInfo {
            id,
            name,
            ready: true,
            bot: true,
        }));
        Some(id)
    }

    pub fn kick(&mut self, player: PlayerId, reason: &str) {
        if self.bots.remove(&player).is_some() {
            self.world.remove_player(player);
            self.broadcast(&ServerMessage::PlayerLeft { id: player });
            return;
        }
        let Some(id) = self
            .clients
            .values()
            .find(|c| c.player == Some(player))
            .map(|c| c.id)
        else {
            return;
        };
        self.reject(id, reason);
    }

    /// Sends a server chat message to everyone.
    pub fn say(&mut self, text: &str) {
        if let Some(text) = protocol::sanitize_chat(text) {
            self.broadcast(&ServerMessage::Chat { from: None, text });
        }
    }

    pub fn handle_command(&mut self, cmd: ServerCommand) {
        match cmd {
            ServerCommand::SetTuning(t) => self.set_tuning(*t),
            ServerCommand::ApplyImpulse { player, impulse } => self.apply_impulse(player, impulse),
            ServerCommand::Kick { player, reason } => self.kick(player, &reason),
            ServerCommand::AddBot { name } => {
                self.add_bot(&name);
            }
            ServerCommand::Chat { text } => self.say(&text),
            ServerCommand::SetPaused(paused) => self.set_paused(paused),
            ServerCommand::RestartLevel => self.restart_level(),
            ServerCommand::SpawnEnemy {
                name,
                position,
                yaw,
                count,
            } => {
                self.spawn_enemies(&name, position, yaw, count);
            }
            ServerCommand::KillAllEnemies => self.kill_all_enemies(),
            ServerCommand::SetFriendlyFire(on) => self.world.rules.friendly_fire = on,
        }
    }

    /// Freezes or resumes the simulation (single-player pause). While paused no ticks run and
    /// no snapshots are sent; the transport keeps flushing so the connection stays alive.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if !paused {
            // Inputs stopped while paused; don't count the gap as network jitter.
            for conn in self.clients.values_mut() {
                conn.inputs.reset_timing();
            }
        }
        log::info!("simulation {}", if paused { "paused" } else { "resumed" });
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Puts every player back at the level start and tells the clients (`Respawned`).
    pub fn restart_level(&mut self) {
        for event in self.world.restart_level() {
            if let SimEvent::Respawned {
                player,
                position,
                yaw,
            } = event
            {
                self.broadcast(&ServerMessage::Game(GameEvent::Respawned {
                    player,
                    position,
                    yaw,
                }));
            }
        }
    }

    /// Kills every living enemy through the damage pipeline.
    pub fn kill_all_enemies(&mut self) {
        let ids: Vec<_> = self
            .world
            .enemies
            .values()
            .filter(|e| e.is_alive())
            .map(|e| e.id)
            .collect();
        let tuning = self.tuning.clone();
        for id in ids {
            self.world.kill_enemy(&tuning, id);
        }
    }

    /// Advances by `real_dt`: receives packets, runs as many fixed ticks as are due (sending
    /// snapshots on snapshot ticks), sends packets. Returns the number of ticks run.
    pub fn update(&mut self, real_dt: Duration) -> u32 {
        self.time += real_dt.as_secs_f64();
        self.renet.update(real_dt);
        if let Err(err) = self.transport.update(real_dt, &mut self.renet) {
            log::warn!("transport: {err}");
        }
        self.handle_server_events();
        self.receive_messages();
        self.process_kicks();
        self.check_tuning_reload();

        let ticks = self.timestep.advance(real_dt.as_secs_f64());
        if !self.paused {
            for _ in 0..ticks {
                self.run_tick();
            }
        }
        // Flush renet once per tick at most: every flush also emits an ack packet, so flushing
        // on every (sub-millisecond) host loop iteration would flood clients with acks.
        if ticks > 0 {
            self.transport.send_packets(&mut self.renet);
        }
        ticks
    }

    /// How long a host loop may sleep before calling [`Server::update`] again.
    pub fn suggested_sleep(&self) -> Duration {
        if self.transport.has_pending() {
            return Duration::from_millis(1);
        }
        let remaining = (1.0 - self.timestep.alpha() as f64) * self.timestep.dt();
        Duration::from_secs_f64(remaining.clamp(0.0005, 0.004))
    }

    /// Disconnects everyone (call before dropping on shutdown).
    pub fn shutdown(&mut self) {
        self.say("Server shutting down");
        self.transport.send_packets(&mut self.renet);
        self.transport.disconnect_all(&mut self.renet);
    }

    // -----------------------------------------------------------------------------------------

    fn update_snapshot_interval(&mut self) {
        let rate = self
            .config
            .snapshot_rate
            .unwrap_or(self.tuning.net.snapshot_rate);
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            30.0
        };
        self.snapshot_interval = ((self.config.tick_rate as f32 / rate).round() as u32).max(1);
    }

    fn check_tuning_reload(&mut self) {
        let Some(watcher) = &self.watcher else {
            return;
        };
        if watcher.poll_changed().is_empty() {
            return;
        }
        match Tuning::load(&self.tuning_path) {
            Ok(t) => self.set_tuning(t),
            Err(err) => log::warn!(
                "keeping previous tuning, {} is invalid: {err}",
                self.tuning_path.display()
            ),
        }
    }

    fn send(&mut self, client: ClientId, msg: &ServerMessage) {
        self.renet
            .send_message(client, CHANNEL_RELIABLE, protocol::encode(msg));
    }

    /// Reliable message to every client that has joined the game. Returns the number of
    /// recipients.
    fn broadcast(&mut self, msg: &ServerMessage) -> usize {
        let bytes = protocol::encode(msg);
        let ids: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.player.is_some())
            .map(|c| c.id)
            .collect();
        for id in &ids {
            self.renet.send_message(*id, CHANNEL_RELIABLE, bytes.clone());
        }
        ids.len()
    }

    fn free_player_id(&self) -> Option<PlayerId> {
        if self.world.players.len() >= self.config.max_players.max(1) {
            return None;
        }
        (0..=u8::MAX)
            .map(PlayerId)
            .find(|id| !self.world.players.contains_key(id))
    }

    fn unique_name(&self, name: &str) -> String {
        let taken = |n: &str| {
            self.clients
                .values()
                .any(|c| c.player.is_some() && c.name == n)
                || self.bots.values().any(|b| b == n)
        };
        if !taken(name) {
            return name.to_string();
        }
        (2..)
            .map(|i| format!("{name} ({i})"))
            .find(|n| !taken(n))
            .unwrap_or_else(|| name.to_string())
    }

    fn reject(&mut self, client: ClientId, reason: &str) {
        log::info!("rejecting client {client}: {reason}");
        self.send(
            client,
            &ServerMessage::Rejected {
                reason: reason.to_string(),
            },
        );
        if let Some(c) = self.clients.get_mut(&client) {
            c.kick_at.get_or_insert(self.time + KICK_DELAY_SECS);
        }
    }

    fn process_kicks(&mut self) {
        let due: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.kick_at.is_some_and(|t| t <= self.time))
            .map(|c| c.id)
            .collect();
        for id in due {
            self.renet.disconnect(id);
        }
    }

    fn handle_server_events(&mut self) {
        while let Some(event) = self.renet.get_event() {
            match event {
                ServerEvent::ClientConnected { client_id } => self.on_connect(client_id),
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    self.on_disconnect(client_id, &reason.to_string())
                }
            }
        }
    }

    fn on_connect(&mut self, client_id: ClientId) {
        self.clients.insert(client_id, ClientConn::new(client_id));
        let request = self
            .transport
            .user_data(client_id)
            .and_then(|data| ConnectRequest::from_user_data(&data));
        let Some(request) = request else {
            self.reject(client_id, "not a VELOCIDE client");
            return;
        };
        if request.protocol_version != PROTOCOL_VERSION {
            self.reject(
                client_id,
                &format!(
                    "version mismatch: server protocol {PROTOCOL_VERSION}, client {}",
                    request.protocol_version
                ),
            );
            return;
        }
        let Some(player) = self.free_player_id() else {
            self.reject(client_id, "server is full");
            return;
        };
        let name = self.unique_name(&request.name);
        self.world.add_player(player, &name);
        if let Some(c) = self.clients.get_mut(&client_id) {
            c.player = Some(player);
            c.name = name.clone();
        }
        log::info!("client {client_id} joined as {player} '{name}'");

        let welcome = ServerMessage::Welcome(Welcome {
            protocol_version: PROTOCOL_VERSION,
            player_id: player,
            tick_rate: self.config.tick_rate,
            snapshot_rate: self.snapshot_rate(),
            server_tick: self.world.tick,
            max_players: self.config.max_players as u32,
        });
        self.send(client_id, &welcome);
        let level = ServerMessage::LoadLevel {
            name: self.world.level_name.clone(),
            level: Box::new((*self.world.level).clone()),
        };
        self.send(client_id, &level);
        self.send(
            client_id,
            &ServerMessage::Tuning(Box::new(self.tuning.clone())),
        );
        self.send(client_id, &ServerMessage::PlayerList(self.player_infos()));

        let info = PlayerInfo {
            id: player,
            name: name.clone(),
            ready: false,
            bot: false,
        };
        let joined = protocol::encode(&ServerMessage::PlayerJoined(info));
        let others: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.id != client_id && c.player.is_some())
            .map(|c| c.id)
            .collect();
        for id in others {
            self.renet
                .send_message(id, CHANNEL_RELIABLE, joined.clone());
        }
        self.say(&format!("{name} joined"));
    }

    fn on_disconnect(&mut self, client_id: ClientId, reason: &str) {
        let Some(conn) = self.clients.remove(&client_id) else {
            return;
        };
        let Some(player) = conn.player else {
            return;
        };
        log::info!("{player} '{}' left ({reason})", conn.name);
        self.world.remove_player(player);
        self.broadcast(&ServerMessage::PlayerLeft { id: player });
        self.say(&format!("{} left", conn.name));
    }

    fn receive_messages(&mut self) {
        let ids: Vec<ClientId> = self.clients.keys().copied().collect();
        let arrival_tick = self.time * self.config.tick_rate as f64;
        for id in ids {
            let mut reliable = Vec::new();
            while let Some(bytes) = self.renet.receive_message(id, CHANNEL_RELIABLE) {
                if reliable.len() < MAX_RELIABLE_PER_UPDATE {
                    reliable.push(bytes);
                }
            }
            let mut unreliable = Vec::new();
            while let Some(bytes) = self.renet.receive_message(id, CHANNEL_UNRELIABLE) {
                if unreliable.len() < MAX_UNRELIABLE_PER_UPDATE {
                    unreliable.push(bytes);
                }
            }
            for bytes in reliable {
                match protocol::decode_client_message::<ClientMessage>(&bytes) {
                    Ok(msg) => self.handle_client_message(id, msg),
                    Err(err) => self.decode_error(id, &err.to_string()),
                }
            }
            for bytes in unreliable {
                match protocol::decode_client_message::<ClientUnreliable>(&bytes) {
                    Ok(ClientUnreliable::Input(packet)) => {
                        self.handle_input_packet(id, packet.ack_tick, packet.inputs, arrival_tick)
                    }
                    Err(err) => self.decode_error(id, &err.to_string()),
                }
            }
        }
    }

    fn decode_error(&mut self, client: ClientId, err: &str) {
        self.stats.decode_errors += 1;
        let Some(c) = self.clients.get_mut(&client) else {
            return;
        };
        c.decode_errors += 1;
        log::debug!("client {client}: {err}");
        if c.decode_errors == MAX_DECODE_ERRORS {
            self.reject(client, "too many malformed messages");
        }
    }

    fn handle_client_message(&mut self, client: ClientId, msg: ClientMessage) {
        let now = self.time;
        let Some(conn) = self.clients.get_mut(&client) else {
            return;
        };
        let Some(player) = conn.player else {
            return;
        };
        match msg {
            ClientMessage::Chat { text } => {
                let elapsed = (now - conn.chat_refill_time).max(0.0) as f32;
                conn.chat_refill_time = now;
                conn.chat_tokens = (conn.chat_tokens + elapsed * CHAT_PER_SECOND).min(CHAT_BURST);
                if conn.chat_tokens < 1.0 {
                    return;
                }
                if self.config.dev_commands && text.trim_start().starts_with('/') {
                    // Dev commands are not chat (and not rate limited like it).
                    let reply = self.run_chat_command(player, text.trim());
                    if let Some(text) = protocol::sanitize_chat(&reply) {
                        self.send(client, &ServerMessage::Chat { from: None, text });
                    }
                    return;
                }
                conn.chat_tokens -= 1.0;
                if let Some(text) = protocol::sanitize_chat(&text) {
                    self.broadcast(&ServerMessage::Chat {
                        from: Some(player),
                        text,
                    });
                }
            }
            ClientMessage::SetReady { ready } => {
                if conn.ready != ready {
                    conn.ready = ready;
                    if let Some(info) = conn.info() {
                        self.broadcast(&ServerMessage::PlayerUpdated(info));
                    }
                }
            }
        }
    }

    fn handle_input_packet(
        &mut self,
        client: ClientId,
        ack_tick: Option<u32>,
        mut inputs: Vec<PlayerInput>,
        arrival_tick: f64,
    ) {
        let now = self.world.tick;
        let Some(conn) = self.clients.get_mut(&client) else {
            return;
        };
        if let Some(ack) = ack_tick
            && ack <= now
            && conn.ack_tick.is_none_or(|a| ack > a)
        {
            conn.ack_tick = Some(ack);
        }
        if conn.player.is_none() || conn.kick_at.is_some() {
            return;
        }
        if inputs.len() > MAX_INPUTS_PER_PACKET {
            inputs.drain(..inputs.len() - MAX_INPUTS_PER_PACKET);
        }
        for input in &mut inputs {
            protocol::sanitize_input(input);
        }
        let r = conn.inputs.receive(&inputs, arrival_tick);
        self.stats.inputs_received += r.accepted as u64;
        self.stats.inputs_redundant += (r.duplicate + r.stale) as u64;
        self.stats.inputs_overflow += r.overflow as u64;
    }

    fn run_tick(&mut self) {
        let started = Instant::now();

        // Inputs due this tick.
        let mut inputs = TickInputs::new();
        for conn in self.clients.values_mut() {
            let Some(player) = conn.player else {
                continue;
            };
            let mut list = Vec::new();
            let r = conn.inputs.take_for_tick(&mut list);
            self.stats.inputs_processed += r.processed as u64;
            self.stats.inputs_skipped += r.skipped as u64;
            if r.stalled {
                self.stats.input_stalls += 1;
            }
            if !list.is_empty() {
                inputs.insert(player, list);
            }
        }

        let events = self.world.step(&inputs, &self.tuning, Some(&self.lagcomp));

        let mut scratch = std::mem::take(&mut self.hitbox_scratch);
        self.world.hitboxes(&self.tuning, &mut scratch);
        self.lagcomp.record(self.world.tick, scratch.clone());
        self.hitbox_scratch = scratch;

        // Combat events: reliable reports to the players concerned, cosmetic FX batched into
        // the next snapshots.
        let routed = combat::route_events(&events, &self.world, &mut self.fx);
        for event in routed.broadcast {
            let is_combat = !matches!(event, GameEvent::Respawned { .. });
            let sent = self.broadcast(&ServerMessage::Game(event));
            if is_combat {
                self.stats.combat_events_sent += sent as u64;
            }
        }
        for (player, event) in routed.to_player {
            if let Some(client) = self.client_of(player) {
                self.send(client, &ServerMessage::Game(event));
                self.stats.combat_events_sent += 1;
            }
        }
        self.event_log.extend(events);
        if self.event_log.len() > MAX_EVENT_LOG {
            let excess = self.event_log.len() - MAX_EVENT_LOG / 2;
            self.event_log.drain(..excess);
        }

        if self.world.tick.is_multiple_of(self.snapshot_interval) {
            self.send_snapshots();
        }

        self.tick_time_accum += started.elapsed();
        self.tick_time_count += 1;
        if self.tick_time_count >= 60 {
            self.stats.tick_time_us =
                self.tick_time_accum.as_secs_f32() * 1e6 / self.tick_time_count as f32;
            self.tick_time_accum = Duration::ZERO;
            self.tick_time_count = 0;
        }
    }

    fn send_snapshots(&mut self) {
        let tick = self.world.tick;
        let mut frame = NetFrame::new();
        self.world.replicate(&self.tuning, &mut frame);
        let frame = Arc::new(frame);
        let (since, repeat_since) = (self.last_snapshot_tick, self.prev_snapshot_tick);

        for conn in self.clients.values_mut() {
            let Some(player) = conn.player else {
                continue;
            };
            if conn.kick_at.is_some() {
                continue;
            }
            let baseline = if self.config.delta_compression {
                conn.ack_tick.and_then(|ack| {
                    conn.sent_frames
                        .iter()
                        .find(|(t, _)| *t == ack && tick.wrapping_sub(*t) <= MAX_BASELINE_AGE_TICKS)
                        .map(|(t, f)| (*t, f.clone()))
                })
            } else {
                None
            };
            let mut entities = Vec::new();
            let base_frame = baseline
                .as_ref()
                .map(|(_, f)| &**f)
                .unwrap_or(&self.empty_frame);
            protocol::encode_delta(base_frame, &frame, &mut entities);

            let own_player = self.world.players.get(&player);
            let entity_bytes = entities.len() as u64;
            let fx = self.fx.collect(Some(player), since, repeat_since);
            self.stats.fx_sent += fx.len() as u64;
            let msg = ServerUnreliable::Snapshot(SnapshotMessage {
                tick,
                baseline_tick: baseline.as_ref().map(|(t, _)| *t),
                last_input_seq: own_player.map_or(0, |p| p.last_input_seq),
                own: own_player.map(|p| OwnState {
                    movement: p.movement,
                    combat: p.combat,
                }),
                entities,
                fx,
            });
            let bytes = protocol::encode(&msg);
            self.stats.snapshots_sent += 1;
            if baseline.is_none() {
                self.stats.full_snapshots_sent += 1;
            }
            self.stats.snapshot_bytes += bytes.len() as u64;
            self.stats.entity_bytes += entity_bytes;
            self.renet.send_message(conn.id, CHANNEL_UNRELIABLE, bytes);

            conn.sent_frames.push_back((tick, frame.clone()));
            while conn.sent_frames.len() > SENT_FRAMES_KEPT {
                conn.sent_frames.pop_front();
            }
        }
        // FX up to `since` have now been sent twice (if important) or once.
        self.fx.prune(since);
        self.prev_snapshot_tick = since;
        self.last_snapshot_tick = tick;
    }

    /// The connection of a player (bots have none).
    fn client_of(&self, player: PlayerId) -> Option<ClientId> {
        self.clients
            .values()
            .find(|c| c.player == Some(player) && c.kick_at.is_none())
            .map(|c| c.id)
    }

    /// Simulation events since the last call (tests, tools, a server-side style system).
    /// Bounded: if nobody drains, only the most recent events are kept.
    pub fn drain_sim_events(&mut self) -> Vec<shared::sim::SimEvent> {
        std::mem::take(&mut self.event_log)
    }

    /// FX waiting in the log (tests / debugging).
    pub fn fx_log(&self) -> &FxLog {
        &self.fx
    }
}

/// A [`Server`] running on its own thread (single-player and listen servers).
pub struct ServerHandle {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    commands: Sender<ServerCommand>,
    stats: Arc<Mutex<ServerStats>>,
    thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// Creates the server (errors are returned here: bad level, port in use...) and runs it
    /// on a new thread until [`ServerHandle::stop`] or drop.
    pub fn spawn(config: ServerConfig) -> anyhow::Result<ServerHandle> {
        let server = Server::new(config)?;
        Self::spawn_server(server)
    }

    /// Runs an already constructed server on a new thread.
    pub fn spawn_server(server: Server) -> anyhow::Result<ServerHandle> {
        let addr = server.local_addr();
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(server.stats()));
        let (tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("velocide-server".into())
            .spawn({
                let stop = stop.clone();
                let stats = stats.clone();
                move || run_server_loop(server, &stop, &rx, &stats)
            })?;
        Ok(ServerHandle {
            addr,
            stop,
            commands: tx,
            stats,
            thread: Some(thread),
        })
    }

    /// Address to connect to. For servers bound to `0.0.0.0`/`[::]` this is the wildcard
    /// address with the real port; use [`ServerHandle::connect_addr`] for a local client.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Loopback address with the server's port (for the host's own client).
    pub fn connect_addr(&self) -> SocketAddr {
        loopback_for(self.addr)
    }

    pub fn send(&self, cmd: ServerCommand) {
        let _ = self.commands.send(cmd);
    }

    /// Latest stats (updated every server loop iteration).
    pub fn stats(&self) -> ServerStats {
        self.stats.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stops the server (disconnecting clients) and waits for its thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("server thread panicked");
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Maps a wildcard bind address to loopback (same port).
pub fn loopback_for(addr: SocketAddr) -> SocketAddr {
    let mut addr = addr;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr {
            SocketAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
            SocketAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
        });
    }
    addr
}

fn run_server_loop(
    mut server: Server,
    stop: &AtomicBool,
    commands: &Receiver<ServerCommand>,
    stats: &Mutex<ServerStats>,
) {
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        while let Ok(cmd) = commands.try_recv() {
            server.handle_command(cmd);
        }
        let now = Instant::now();
        // Clamp huge stalls (debugger, suspend) so we don't try to catch up forever.
        let dt = (now - last).min(Duration::from_millis(250));
        last = now;
        server.update(dt);
        if let Ok(mut s) = stats.lock() {
            *s = server.stats();
        }
        std::thread::sleep(server.suggested_sleep());
    }
    server.shutdown();
}

/// Resolves a tuning path argument relative to the assets dir if it is not absolute.
pub fn resolve_tuning_path(path: &Path) -> PathBuf {
    if path.is_absolute() || path.exists() {
        path.to_path_buf()
    } else {
        shared::assets_dir().join(path)
    }
}
