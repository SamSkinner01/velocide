//! Client networking core: connection, prediction, reconciliation, interpolation, clock sync,
//! network stats and the session layer. Rendering/UI code builds on [`NetClient`] (or on a
//! [`Session`], which also owns an in-process server for single-player and hosting).
//!
//! # Using it from a game loop
//!
//! ```no_run
//! use std::time::{Duration, Instant};
//! use client::net::{NetEvent, Session};
//! use engine::time::FixedTimestep;
//! use shared::input::PlayerInput;
//!
//! let mut session = Session::single_player("proving_ground", "Shrike")?;
//! let mut timestep = FixedTimestep::new(60.0);
//! let mut last = Instant::now();
//! loop {
//!     let now = Instant::now();
//!     let frame_dt = now - last;
//!     last = now;
//!     let net = session.client_mut();
//!
//!     // 1. Network: receive events/snapshots, reconcile, advance interpolation.
//!     net.update(frame_dt);
//!     for event in net.drain_events() {
//!         match event {
//!             NetEvent::Spawned { yaw, .. } => { /* set camera yaw */ let _ = yaw; }
//!             NetEvent::Disconnected { reason } => anyhow::bail!(reason),
//!             _ => {}
//!         }
//!     }
//!
//!     // 2. Fixed ticks: sample input, predict, send.
//!     timestep.set_hz(net.tick_rate() as f64);
//!     for _ in 0..timestep.advance(frame_dt.as_secs_f64()) {
//!         let input = PlayerInput { forward: 127, ..Default::default() }; // from the keyboard
//!         net.tick(input); // fills seq + view_tick, predicts, sends (no-op until in game)
//!     }
//!
//!     // 3. Render: local player predicted & smoothed, everyone else interpolated.
//!     let _me = net.local_render_position(timestep.alpha());
//!     for _entity in net.remote_entities() { /* draw at entity.position / yaw */ }
//!     let _overlay = net.stats(); // ping, loss, bandwidth, corrections, ghost position
//!     # break;
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```

pub mod clock;
pub mod interpolation;
pub mod lan;
/// Lobby state, level catalog and host commands on `NetClient` (docs/lobby.md).
mod lobby;
pub mod prediction;
pub mod session;
pub mod transport;

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use glam::Vec3;
use renet::RenetClient;
use shared::input::PlayerInput;
use shared::lobby::{LevelInfo, LobbyPhase, LobbyState};
use shared::movement::{MoveEvents, MoveState};
use shared::protocol::{
    self, CHANNEL_RELIABLE, CHANNEL_UNRELIABLE, ClientMessage, ClientUnreliable, ConnectRequest,
    EntityState, FxEvent, GameEvent, INPUT_REDUNDANCY, InputPacket, NetFrame, NetId,
    PROTOCOL_VERSION, PlayerInfo, ServerMessage, ServerUnreliable, SnapshotMessage, Welcome, kind,
};
use shared::sim::{PlayerCombat, PlayerId, SimWorld, tick_dt};
use shared::tuning::Tuning;

pub use clock::ClockSync;
pub use interpolation::{SampleInfo, SnapshotBuffer};
pub use prediction::{Correction, Prediction, PredictionStats};
pub use server::{NetSimConfig, TransportStats};
pub use session::{JoinTarget, Session, SessionKind, SessionSpec, parse_netsim, resolve_address};
pub use transport::{ClientTransport, NetcodeClientTransport};

/// Render clock errors beyond this (ticks) snap instead of time-stretching.
const RENDER_RESYNC_TICKS: f64 = 30.0;

/// Connection options.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    pub name: String,
    /// Network simulator for this client's traffic (both directions).
    pub netsim: Option<NetSimConfig>,
    /// Protocol version announced to the server (override only to test rejection).
    pub protocol_version: u32,
    /// Listen-server host: the secret the session gave its in-process server; presented after
    /// the welcome to become the lobby host (`LobbyRequest::ClaimHost`).
    pub host_token: Option<u64>,
}

impl ClientConfig {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            netsim: None,
            protocol_version: PROTOCOL_VERSION,
            host_token: None,
        }
    }

    pub fn with_netsim(mut self, netsim: Option<NetSimConfig>) -> Self {
        self.netsim = netsim;
        self
    }
}

/// Connection lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    /// Transport handshake in progress.
    Connecting,
    /// Connected; waiting for welcome, level, tuning and the first snapshot.
    Loading,
    /// Predicting and sending inputs.
    InGame,
    Disconnected,
}

/// Things the game should react to ([`NetClient::drain_events`]).
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    /// Welcome received.
    Connected {
        player_id: PlayerId,
    },
    /// Server refused us (a `Disconnected` follows).
    Rejected {
        reason: String,
    },
    /// A (new) level was received; rebuild level meshes.
    LevelLoaded {
        name: String,
    },
    /// Server tuning received or changed.
    TuningChanged,
    /// The local player got a body (first time, or after a respawn). Adopt `yaw` as the view
    /// angle.
    Spawned {
        position: Vec3,
        yaw: f32,
    },
    PlayerJoined(PlayerInfo),
    PlayerLeft {
        id: PlayerId,
        name: String,
    },
    PlayerUpdated(PlayerInfo),
    Chat {
        from: Option<PlayerId>,
        name: String,
        text: String,
    },
    Game(GameEvent),
    /// A lobby state arrived (`NetClient::lobby`): phase, settings, players, pings. Sent on
    /// every change and once per second. When the phase leaves the match (`Lobby` /
    /// `Countdown`) the level has already been unloaded (`world()` is `None`).
    LobbyChanged {
        phase: LobbyPhase,
        previous: Option<LobbyPhase>,
    },
    /// The server refused a lobby request ("only the host can...").
    Notice {
        text: String,
    },
    Disconnected {
        reason: String,
    },
}

/// Numbers for the debug overlay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetStats {
    /// Round trip time (ms, from renet's packet acks).
    pub rtt_ms: f32,
    /// Fraction of our packets not acknowledged (0..1).
    pub packet_loss: f32,
    pub bytes_in_per_sec: f32,
    pub bytes_out_per_sec: f32,
    /// Snapshots received over the last second.
    pub snapshot_rate: f32,
    pub snapshots_received: u64,
    pub full_snapshots_received: u64,
    /// Snapshots that could not be decoded (baseline missing, malformed).
    pub snapshots_dropped: u64,
    pub decode_errors: u64,
    pub corrections: u64,
    pub correction_snaps: u64,
    pub last_correction_m: f32,
    pub avg_correction_m: f32,
    pub max_correction_m: f32,
    /// Largest jump of the predicted position caused by a correction (m), i.e. the visual
    /// error that smoothing hid.
    pub max_correction_shift_m: f32,
    /// Current visual smoothing offset (m).
    pub render_offset_m: f32,
    pub interp_delay_ms: f32,
    pub jitter_ms: f32,
    /// Remote entities are being extrapolated (snapshots late).
    pub extrapolating: bool,
    /// Estimated current server tick.
    pub server_tick: f64,
    /// Tick remote entities are displayed at.
    pub render_tick: f64,
    pub unacked_inputs: usize,
    /// Server-authoritative position of the local player (latest snapshot; draw as a ghost).
    pub server_position: Option<Vec3>,
    pub predicted_position: Option<Vec3>,
    pub transport: TransportStats,
}

/// Connection to a server.
pub struct NetClient {
    renet: RenetClient,
    transport: Box<dyn ClientTransport>,
    state: ConnectionState,
    disconnect_reason: Option<String>,
    rejected: Option<String>,
    welcome: Option<Welcome>,
    world: Option<SimWorld>,
    tuning: Tuning,
    players: BTreeMap<PlayerId, PlayerInfo>,
    events: VecDeque<NetEvent>,
    frames: SnapshotBuffer,
    latest_snapshot_tick: Option<u32>,
    /// Tick gaps between recent consecutive newest snapshots (to measure the snapshot
    /// interval, which the server may change when its tuning is reloaded).
    snapshot_gaps: VecDeque<u32>,
    last_ack_sent: Option<u32>,
    prediction: Option<Prediction>,
    next_seq: u32,
    clock: ClockSync,
    render_tick: Option<f64>,
    interp_delay_ticks: Option<f64>,
    remote: Vec<EntityState>,
    sample_info: SampleInfo,
    /// Local time of the last input packet / renet flush.
    last_input_time: f64,
    last_flush_time: f64,
    snapshot_arrivals: VecDeque<f64>,
    time: f64,
    stats: NetStats,
    /// Style meter / rank / freshness from the newest `OwnState` (style HUD).
    own_style: Option<shared::sim::StyleView>,
    /// Latest lobby state / level catalog from the server (docs/lobby.md).
    lobby: Option<LobbyState>,
    levels: Vec<LevelInfo>,
    host_token: Option<u64>,
    /// Latest authoritative combat state of the local player (`OwnState::combat`).
    own_combat: Option<OwnCombat>,
    /// Cosmetic combat events received and not yet drained (de-duplicated by `seq`).
    fx: Vec<FxEvent>,
    fx_seen: std::collections::BTreeSet<u32>,
}

/// The local player's authoritative combat state from the newest snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnCombat {
    /// Snapshot tick.
    pub tick: u32,
    /// The state is after this input (`SnapshotMessage::last_input_seq`).
    pub seq: u32,
    pub combat: PlayerCombat,
}

impl OwnCombat {
    /// Is input `seq` simulated while dead? Inputs up to the server's respawn tick are (one
    /// input per server tick after `self.seq`); used to predict the corpse with
    /// `shared::sim::neutral_input` and to stop predicted firing.
    pub fn dead_at(&self, seq: u32) -> bool {
        self.combat.dead && seq <= self.seq.wrapping_add(self.combat.respawn_in as u32)
    }
}

impl NetClient {
    /// Starts connecting to a server over UDP (non-blocking; call [`NetClient::update`]).
    pub fn connect(addr: SocketAddr, config: ClientConfig) -> anyhow::Result<Self> {
        let request = ConnectRequest {
            protocol_version: config.protocol_version,
            name: protocol::sanitize_name(&config.name),
        };
        let transport = NetcodeClientTransport::new(
            addr,
            random_client_id(),
            request.to_user_data(),
            config.netsim,
        )?;
        let mut client = Self::with_transport(Box::new(transport));
        client.host_token = config.host_token;
        Ok(client)
    }

    /// Uses a custom transport (Steam networking, relays...).
    pub fn with_transport(transport: Box<dyn ClientTransport>) -> Self {
        Self {
            renet: RenetClient::new(server::connection_config()),
            transport,
            state: ConnectionState::Connecting,
            disconnect_reason: None,
            rejected: None,
            welcome: None,
            world: None,
            tuning: Tuning::default(),
            players: BTreeMap::new(),
            events: VecDeque::new(),
            frames: SnapshotBuffer::new(),
            latest_snapshot_tick: None,
            snapshot_gaps: VecDeque::new(),
            last_ack_sent: None,
            prediction: None,
            next_seq: 1,
            clock: ClockSync::new(shared::sim::DEFAULT_TICK_RATE),
            render_tick: None,
            interp_delay_ticks: None,
            remote: Vec::new(),
            sample_info: SampleInfo::default(),
            last_input_time: f64::NEG_INFINITY,
            last_flush_time: f64::NEG_INFINITY,
            snapshot_arrivals: VecDeque::new(),
            time: 0.0,
            stats: NetStats::default(),
            own_style: None,
            lobby: None,
            levels: Vec::new(),
            host_token: None,
            own_combat: None,
            fx: Vec::new(),
            fx_seen: std::collections::BTreeSet::new(),
        }
    }

    // ---- combat -------------------------------------------------------------------------------

    /// The local player's latest authoritative combat state (health, hard damage, weapons,
    /// death), `None` before the first snapshot.
    pub fn own_combat(&self) -> Option<&OwnCombat> {
        self.own_combat.as_ref()
    }

    /// Cosmetic combat events (`Fx`) received since the last call, each exactly once (repeats
    /// are de-duplicated by `seq`), in arrival order. Play them when `render_tick` reaches
    /// their `tick`.
    pub fn drain_fx(&mut self) -> Vec<FxEvent> {
        std::mem::take(&mut self.fx)
    }

    fn receive_fx(&mut self, fx: Vec<FxEvent>) {
        for e in fx {
            if self.fx_seen.insert(e.seq) {
                self.fx.push(e);
            }
        }
        // Keep the de-duplication window bounded (repeats come one snapshot later at most).
        while self.fx_seen.len() > 4096 {
            self.fx_seen.pop_first();
        }
        // Nobody drained for a long time (headless tools): don't grow without bound.
        if self.fx.len() > 4096 {
            let excess = self.fx.len() - 4096;
            self.fx.drain(..excess);
        }
    }

    // ---- state ------------------------------------------------------------------------------

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn is_in_game(&self) -> bool {
        self.state == ConnectionState::InGame
    }

    pub fn is_disconnected(&self) -> bool {
        self.state == ConnectionState::Disconnected
    }

    /// Why we were disconnected (or rejected).
    pub fn disconnect_reason(&self) -> Option<&str> {
        self.disconnect_reason.as_deref()
    }

    /// The local player's style meter, rank and freshness (newest snapshot).
    pub fn own_style(&self) -> Option<&shared::sim::StyleView> {
        self.own_style.as_ref()
    }

    pub fn local_player_id(&self) -> Option<PlayerId> {
        self.welcome.as_ref().map(|w| w.player_id)
    }

    pub fn welcome(&self) -> Option<&Welcome> {
        self.welcome.as_ref()
    }

    /// Server tick rate (prediction must run at exactly this rate).
    pub fn tick_rate(&self) -> u32 {
        self.welcome
            .as_ref()
            .map_or(shared::sim::DEFAULT_TICK_RATE, |w| w.tick_rate)
    }

    /// The server's tuning (use it for everything client-side, e.g. camera feel).
    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// Client copy of the level and its collision (doors follow the server).
    pub fn world(&self) -> Option<&SimWorld> {
        self.world.as_ref()
    }

    pub fn players(&self) -> &BTreeMap<PlayerId, PlayerInfo> {
        &self.players
    }

    pub fn player_name(&self, id: PlayerId) -> Option<&str> {
        self.players.get(&id).map(|p| p.name.as_str())
    }

    /// Next queued event.
    pub fn poll_event(&mut self) -> Option<NetEvent> {
        self.events.pop_front()
    }

    pub fn drain_events(&mut self) -> Vec<NetEvent> {
        self.events.drain(..).collect()
    }

    pub fn stats(&self) -> &NetStats {
        &self.stats
    }

    pub fn transport_stats(&self) -> TransportStats {
        self.transport.stats()
    }

    // ---- local player -------------------------------------------------------------------------

    pub fn prediction(&self) -> Option<&Prediction> {
        self.prediction.as_ref()
    }

    /// Latest predicted state of the local player.
    pub fn local_state(&self) -> Option<&MoveState> {
        self.prediction.as_ref().map(Prediction::state)
    }

    /// Where to draw the local player / camera: predicted, interpolated between the last two
    /// ticks with the fixed-timestep `alpha`, plus correction smoothing.
    pub fn local_render_position(&self, alpha: f32) -> Option<Vec3> {
        self.prediction.as_ref().map(|p| p.render_position(alpha))
    }

    /// Latest authoritative state of the local player.
    pub fn server_state(&self) -> Option<&MoveState> {
        self.prediction.as_ref().map(Prediction::server_state)
    }

    // ---- remote entities ----------------------------------------------------------------------

    /// Everything except the local player, interpolated for this frame.
    pub fn remote_entities(&self) -> &[EntityState] {
        &self.remote
    }

    pub fn remote_entity(&self, id: NetId) -> Option<&EntityState> {
        self.remote.iter().find(|e| e.id == id)
    }

    /// Remote players, interpolated for this frame.
    pub fn remote_players(&self) -> impl Iterator<Item = &EntityState> {
        self.remote.iter().filter(|e| e.kind == kind::PLAYER)
    }

    /// The (fractional) server tick remote entities are displayed at.
    pub fn render_tick(&self) -> f64 {
        self.render_tick.unwrap_or(0.0)
    }

    /// Estimated current server tick.
    pub fn server_tick_estimate(&self) -> Option<f64> {
        self.clock.server_tick()
    }

    pub fn interp_delay_ms(&self) -> f32 {
        self.interp_delay_ticks.unwrap_or(0.0) as f32 * 1000.0 / self.tick_rate() as f32
    }

    /// Decoded snapshot frames (interpolation buffer / delta baselines).
    pub fn frames(&self) -> &SnapshotBuffer {
        &self.frames
    }

    /// Samples remote entities at an arbitrary render tick (debug tools, replays).
    pub fn sample_at(&self, render_tick: f64) -> Vec<EntityState> {
        let mut out = Vec::new();
        self.frames.sample(
            render_tick,
            self.max_extrapolation_ticks(),
            tick_dt(self.tick_rate()),
            self.local_net_id(),
            &mut out,
        );
        out
    }

    // ---- sending ------------------------------------------------------------------------------

    /// One fixed tick of the local player: assigns `seq` and the view time (for lag
    /// compensation), predicts it immediately and sends it (with redundancy). Returns the
    /// predicted movement events, or `None` while not in game.
    pub fn tick(&mut self, mut input: PlayerInput) -> Option<MoveEvents> {
        if self.state != ConnectionState::InGame {
            return None;
        }
        let tick_rate = self.tick_rate();
        let world = self.world.as_ref()?;
        let prediction = self.prediction.as_mut()?;
        input.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        protocol::sanitize_input(&mut input);
        let view = self.render_tick.unwrap_or(0.0).max(0.0);
        input.view_tick = view.floor().min(u32::MAX as f64) as u32;
        input.view_frac = ((view - view.floor()) * 256.0).floor().clamp(0.0, 255.0) as u8;
        // A dead body is simulated with a neutral input on the server: predict (and send)
        // exactly that until the predicted respawn tick.
        if self.own_combat.is_some_and(|c| c.dead_at(input.seq)) {
            input = shared::sim::neutral_input(&input);
        }
        let events = prediction.apply(
            input,
            &world.move_env(),
            &self.tuning.movement,
            tick_dt(tick_rate),
        );
        self.send_inputs();
        self.last_input_time = self.time;
        Some(events)
    }

    fn send_inputs(&mut self) {
        let inputs: Vec<PlayerInput> = match &self.prediction {
            Some(p) => {
                let n = p.unacked_len();
                p.unacked()
                    .skip(n.saturating_sub(INPUT_REDUNDANCY))
                    .copied()
                    .collect()
            }
            None => Vec::new(),
        };
        let packet = ClientUnreliable::Input(InputPacket {
            ack_tick: self.latest_snapshot_tick,
            inputs,
        });
        self.renet
            .send_message(CHANNEL_UNRELIABLE, protocol::encode(&packet));
        self.last_ack_sent = self.latest_snapshot_tick;
        self.flush();
    }

    pub fn send_chat(&mut self, text: &str) {
        if let Some(text) = protocol::sanitize_chat(text) {
            self.send_reliable(&ClientMessage::Chat { text });
        }
    }

    pub fn set_ready(&mut self, ready: bool) {
        self.send_reliable(&ClientMessage::SetReady { ready });
    }

    fn send_reliable(&mut self, msg: &ClientMessage) {
        if self.state == ConnectionState::Disconnected {
            return;
        }
        self.renet
            .send_message(CHANNEL_RELIABLE, protocol::encode(msg));
    }

    /// Sends raw bytes on a channel (protocol fuzz tests only).
    #[doc(hidden)]
    pub fn send_raw(&mut self, reliable: bool, bytes: Vec<u8>) {
        let channel = if reliable {
            CHANNEL_RELIABLE
        } else {
            CHANNEL_UNRELIABLE
        };
        self.renet.send_message(channel, bytes);
        self.flush();
    }

    /// Leaves the server (the disconnect packet is sent immediately).
    pub fn disconnect(&mut self) {
        if self.state == ConnectionState::Disconnected {
            return;
        }
        self.renet.disconnect();
        self.transport.disconnect();
        self.on_disconnected("disconnected by client".to_string());
    }

    fn flush(&mut self) {
        if self.state == ConnectionState::Disconnected {
            return;
        }
        self.last_flush_time = self.time;
        if let Err(reason) = self.transport.send_packets(&mut self.renet) {
            self.on_disconnected(reason);
        }
    }

    // ---- receiving ----------------------------------------------------------------------------

    /// Pumps the connection; call once per rendered frame with the real frame time.
    pub fn update(&mut self, dt: Duration) {
        if self.state == ConnectionState::Disconnected {
            return;
        }
        let secs = dt.as_secs_f64();
        self.time += secs;
        self.clock.advance(secs);
        self.renet.update(dt);
        if let Err(reason) = self.transport.update(dt, &mut self.renet) {
            self.on_disconnected(reason);
            return;
        }
        if let Some(reason) = self.renet.disconnect_reason() {
            self.on_disconnected(reason.to_string());
            return;
        }
        if self.state == ConnectionState::Connecting && self.transport.is_connected() {
            self.state = ConnectionState::Loading;
        }

        self.receive();
        if self.state == ConnectionState::Disconnected {
            return;
        }
        self.clock.set_rtt(self.renet.rtt());
        self.update_interpolation(secs);
        if let Some(p) = &mut self.prediction {
            p.decay_smoothing(secs as f32, &self.tuning.net);
        }
        let tick_secs = tick_dt(self.tick_rate()) as f64;
        // Acknowledge snapshots when no inputs are flowing (loading, menus, paused input):
        // input packets normally carry the ack.
        if self.time - self.last_input_time > 3.0 * tick_secs
            && self.latest_snapshot_tick.is_some()
            && self.latest_snapshot_tick != self.last_ack_sent
        {
            self.send_inputs();
        }
        // Flush at most once per tick: each renet flush also sends an ack packet.
        if self.time - self.last_flush_time >= tick_secs * 0.999 {
            self.flush();
        }
        self.update_stats();
    }

    fn on_disconnected(&mut self, reason: String) {
        if self.state == ConnectionState::Disconnected {
            return;
        }
        let reason = self.rejected.clone().unwrap_or(reason);
        log::info!("disconnected: {reason}");
        self.state = ConnectionState::Disconnected;
        self.disconnect_reason = Some(reason.clone());
        self.events.push_back(NetEvent::Disconnected { reason });
    }

    fn receive(&mut self) {
        while let Some(bytes) = self.renet.receive_message(CHANNEL_RELIABLE) {
            match protocol::decode_server_message::<ServerMessage>(&bytes) {
                Ok(msg) => self.handle_message(msg),
                Err(err) => {
                    self.stats.decode_errors += 1;
                    log::warn!("bad server message: {err}");
                }
            }
        }
        while let Some(bytes) = self.renet.receive_message(CHANNEL_UNRELIABLE) {
            match protocol::decode_server_message::<ServerUnreliable>(&bytes) {
                Ok(ServerUnreliable::Snapshot(snapshot)) => self.handle_snapshot(snapshot),
                Err(err) => {
                    self.stats.decode_errors += 1;
                    log::warn!("bad snapshot: {err}");
                }
            }
        }
    }

    fn handle_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Welcome(welcome) => {
                if welcome.protocol_version != PROTOCOL_VERSION {
                    self.rejected = Some(format!(
                        "server protocol {} != client {}",
                        welcome.protocol_version, PROTOCOL_VERSION
                    ));
                    self.disconnect();
                    return;
                }
                self.clock.set_tick_rate(welcome.tick_rate);
                let player_id = welcome.player_id;
                self.welcome = Some(welcome);
                self.events.push_back(NetEvent::Connected { player_id });
                self.claim_host();
            }
            ServerMessage::Rejected { reason } => {
                self.rejected = Some(reason.clone());
                self.events.push_back(NetEvent::Rejected { reason });
            }
            ServerMessage::LoadLevel { name, level } => {
                // A hostile or broken server could send a level that takes forever or all the
                // memory to build (100 km brushes, millions of them, NaN coordinates).
                let checked = if name.len() > shared::level::MAX_LEVEL_STRING_BYTES {
                    Err("level name too long".to_string())
                } else {
                    level.validate()
                };
                if let Err(err) = checked {
                    log::warn!("server sent an invalid level: {err}");
                    self.rejected = Some(format!("The server sent an invalid level ({err})"));
                    self.disconnect();
                    return;
                }
                self.world = Some(SimWorld::new(&name, Arc::new(*level), self.tick_rate()));
                // A new level means a new body; the next snapshot re-creates the prediction.
                self.prediction = None;
                self.own_combat = None;
                if self.state == ConnectionState::InGame {
                    self.state = ConnectionState::Loading;
                }
                self.events.push_back(NetEvent::LevelLoaded { name });
            }
            ServerMessage::Tuning(mut tuning) => {
                // Never trust the numbers blindly (u32::MAX pellets, 1e30 m ranges...). A
                // well-behaved server sanitizes too, so prediction still matches it.
                if tuning.sanitize() {
                    log::warn!("server tuning had out-of-range values (clamped)");
                }
                self.tuning = *tuning;
                self.events.push_back(NetEvent::TuningChanged);
            }
            ServerMessage::PlayerList(list) => {
                self.players.clear();
                for info in list {
                    self.players.insert(info.id, info.clone());
                    self.events.push_back(NetEvent::PlayerJoined(info));
                }
            }
            ServerMessage::PlayerJoined(info) => {
                self.players.insert(info.id, info.clone());
                self.events.push_back(NetEvent::PlayerJoined(info));
            }
            ServerMessage::PlayerLeft { id } => {
                let name = self.players.remove(&id).map(|p| p.name).unwrap_or_default();
                self.events.push_back(NetEvent::PlayerLeft { id, name });
            }
            ServerMessage::PlayerUpdated(info) => {
                self.players.insert(info.id, info.clone());
                self.events.push_back(NetEvent::PlayerUpdated(info));
            }
            ServerMessage::Chat { from, text } => {
                let name = match from {
                    Some(id) => self.player_name(id).unwrap_or("?").to_string(),
                    None => "server".to_string(),
                };
                self.events.push_back(NetEvent::Chat { from, name, text });
            }
            ServerMessage::Game(event) => {
                if let GameEvent::Respawned {
                    player,
                    position,
                    yaw,
                } = &event
                    && Some(*player) == self.local_player_id()
                {
                    self.events.push_back(NetEvent::Spawned {
                        position: *position,
                        yaw: *yaw,
                    });
                }
                self.events.push_back(NetEvent::Game(event));
            }
            ServerMessage::Lobby(update) => self.on_lobby_update(update),
        }
    }

    fn handle_snapshot(&mut self, mut snap: SnapshotMessage) {
        if self.frames.contains(snap.tick) {
            return; // duplicate
        }
        if self.frames.is_stale(snap.tick) {
            // Far older than the newest frame: useless, and keeping it would let late or
            // replayed snapshots grow the buffer without bound.
            self.stats.snapshots_dropped += 1;
            return;
        }
        // Effects don't depend on the entity baseline: keep them even if the delta is lost.
        let fx = std::mem::take(&mut snap.fx);
        self.receive_fx(fx);
        let empty = NetFrame::default();
        let baseline = match snap.baseline_tick {
            None => &empty,
            Some(tick) => match self.frames.get(tick) {
                Some(frame) => &**frame,
                None => {
                    self.stats.snapshots_dropped += 1;
                    return;
                }
            },
        };
        let frame = match protocol::decode_delta(baseline, &snap.entities) {
            Ok(frame) => Arc::new(frame),
            Err(err) => {
                self.stats.snapshots_dropped += 1;
                log::warn!("bad snapshot {}: {err}", snap.tick);
                return;
            }
        };
        if !self.frames.insert(snap.tick, frame.clone()) {
            self.stats.snapshots_dropped += 1;
            return;
        }
        self.stats.snapshots_received += 1;
        if snap.baseline_tick.is_none() {
            self.stats.full_snapshots_received += 1;
        }
        self.snapshot_arrivals.push_back(self.time);
        self.clock.on_snapshot(snap.tick);

        if self.latest_snapshot_tick.is_some_and(|t| snap.tick <= t) {
            return; // older than what we have: only useful for interpolation
        }
        if let Some(prev) = self.latest_snapshot_tick {
            self.snapshot_gaps.push_back(snap.tick - prev);
            while self.snapshot_gaps.len() > 16 {
                self.snapshot_gaps.pop_front();
            }
        }
        self.latest_snapshot_tick = Some(snap.tick);

        if let Some(world) = &mut self.world {
            world.apply_replicated_level_state(&frame);
        }
        let Some(own) = snap.own else {
            return;
        };
        self.own_style = Some(own.style);
        self.own_combat = Some(OwnCombat {
            tick: snap.tick,
            seq: snap.last_input_seq,
            combat: own.combat,
        });
        let (Some(world), Some(_)) = (&self.world, &self.welcome) else {
            return; // level not loaded yet; a later snapshot starts prediction
        };
        let dt = tick_dt(self.tick_rate());
        match &mut self.prediction {
            Some(p) => {
                p.reconcile(
                    snap.last_input_seq,
                    &own.movement,
                    &world.move_env(),
                    &self.tuning.movement,
                    &self.tuning.net,
                    dt,
                );
            }
            None => {
                self.prediction = Some(Prediction::new(snap.last_input_seq, own.movement));
                self.next_seq = self.next_seq.max(snap.last_input_seq.wrapping_add(1));
                self.state = ConnectionState::InGame;
                let yaw = self
                    .local_net_id()
                    .and_then(|id| frame.get(id))
                    .map_or(0.0, |e| e.yaw);
                self.events.push_back(NetEvent::Spawned {
                    position: own.movement.position,
                    yaw,
                });
            }
        }
    }

    fn local_net_id(&self) -> Option<NetId> {
        self.local_player_id().map(NetId::player)
    }

    /// Ticks between snapshots: the smallest recent gap (lost snapshots only make gaps
    /// larger), falling back to the rate announced in the welcome.
    pub fn snapshot_interval_ticks(&self) -> f64 {
        match self.snapshot_gaps.iter().min() {
            Some(&gap) if self.snapshot_gaps.len() >= 4 => gap.max(1) as f64,
            _ => {
                let rate = self
                    .welcome
                    .as_ref()
                    .map_or(self.tuning.net.snapshot_rate, |w| w.snapshot_rate)
                    .max(1.0) as f64;
                (self.tick_rate() as f64 / rate).max(1.0)
            }
        }
    }

    fn max_extrapolation_ticks(&self) -> f64 {
        self.tuning.net.max_extrapolation_ms.max(0.0) as f64 / 1000.0 * self.tick_rate() as f64
    }

    fn update_interpolation(&mut self, secs: f64) {
        let Some(latest) = self.clock.latest_tick() else {
            return;
        };
        let tick_rate = self.tick_rate() as f64;
        let interval = self.snapshot_interval_ticks();
        let net = &self.tuning.net;
        let max_ticks =
            (net.interp_delay_max_ms.max(0.0) as f64 / 1000.0 * tick_rate).max(interval);
        let target_delay = (interval
            + net.interp_delay_ms.max(0.0) as f64 / 1000.0 * tick_rate
            + net.interp_jitter_factor.max(0.0) as f64 * self.clock.jitter_ticks())
        .min(max_ticks);
        let delay = match self.interp_delay_ticks {
            None => target_delay,
            // Adapt gently (time constant ~0.5 s).
            Some(d) => d + (target_delay - d) * (secs * 2.0).min(1.0),
        };
        self.interp_delay_ticks = Some(delay);

        let target = latest - delay;
        let render = match self.render_tick {
            Some(r) if (target - r).abs() < RENDER_RESYNC_TICKS => {
                // Time-stretch (+-10%) towards the target instead of jumping.
                let rate = 1.0 + ((target - r) * 0.05).clamp(-0.1, 0.1);
                r + secs * tick_rate * rate
            }
            _ => target,
        };
        self.render_tick = Some(render);

        let local = self.local_net_id();
        self.sample_info = self.frames.sample(
            render,
            self.max_extrapolation_ticks(),
            tick_dt(self.tick_rate()),
            local,
            &mut self.remote,
        );
    }

    fn update_stats(&mut self) {
        while self
            .snapshot_arrivals
            .front()
            .is_some_and(|t| self.time - t > 1.0)
        {
            self.snapshot_arrivals.pop_front();
        }
        let tick_rate = self.tick_rate() as f32;
        let info = self.renet.network_info();
        let transport = self.transport.stats();
        let s = &mut self.stats;
        s.rtt_ms = (info.rtt * 1000.0) as f32;
        s.packet_loss = info.packet_loss as f32;
        s.bytes_in_per_sec = transport.bytes_received_per_sec as f32;
        s.bytes_out_per_sec = transport.bytes_sent_per_sec as f32;
        s.transport = transport;
        s.snapshot_rate = self.snapshot_arrivals.len() as f32 / self.time.clamp(1e-3, 1.0) as f32;
        s.interp_delay_ms = self.interp_delay_ticks.unwrap_or(0.0) as f32 * 1000.0 / tick_rate;
        s.jitter_ms = self.clock.jitter_ticks() as f32 * 1000.0 / tick_rate;
        s.extrapolating = self.sample_info.extrapolated_ticks > 0.0;
        s.server_tick = self.clock.server_tick().unwrap_or(0.0);
        s.render_tick = self.render_tick.unwrap_or(0.0);
        if let Some(p) = &self.prediction {
            let ps = p.stats();
            s.corrections = ps.corrections;
            s.correction_snaps = ps.snaps;
            s.last_correction_m = ps.last_error;
            s.avg_correction_m = ps.average_error();
            s.max_correction_m = ps.max_error;
            s.max_correction_shift_m = ps.max_shift;
            s.render_offset_m = p.render_offset().length();
            s.unacked_inputs = p.unacked_len();
            s.server_position = Some(p.server_state().position);
            s.predicted_position = Some(p.state().position);
        }
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        if self.state != ConnectionState::Disconnected {
            self.transport.disconnect();
        }
    }
}

/// Random 64-bit id for netcode's unsecure mode (must be unique per connection; from the OS
/// random source, so other clients can't predict it).
fn random_client_id() -> u64 {
    u64::from_le_bytes(renet_netcode::generate_random_bytes::<8>())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport that never delivers anything: messages are fed to the client directly.
    struct NoTransport;

    impl ClientTransport for NoTransport {
        fn update(&mut self, _: Duration, _: &mut RenetClient) -> Result<(), String> {
            Ok(())
        }
        fn send_packets(&mut self, _: &mut RenetClient) -> Result<(), String> {
            Ok(())
        }
        fn disconnect(&mut self) {}
        fn is_connected(&self) -> bool {
            true
        }
        fn server_addr(&self) -> Option<SocketAddr> {
            None
        }
        fn stats(&self) -> TransportStats {
            TransportStats::default()
        }
    }

    fn client() -> NetClient {
        NetClient::with_transport(Box::new(NoTransport))
    }

    fn proving_ground() -> Box<shared::level::LevelDef> {
        Box::new(shared::sim::builtin_level(shared::sim::BUILTIN_LEVEL).unwrap())
    }

    #[test]
    fn oversized_levels_from_the_server_are_refused() {
        let mut c = client();
        let mut level = proving_ground();
        // 100 km extents: building its collision used to take minutes and gigabytes.
        level.brushes[0].max = Vec3::splat(1.0e5);
        let started = std::time::Instant::now();
        c.handle_message(ServerMessage::LoadLevel {
            name: "evil".into(),
            level,
        });
        assert!(started.elapsed().as_secs_f32() < 1.0);
        assert!(c.world().is_none());
        assert!(c.is_disconnected());
        let reason = c.disconnect_reason().unwrap();
        assert!(
            reason.contains("invalid level") && reason.contains("brush 0"),
            "{reason}"
        );

        let mut c = client();
        c.handle_message(ServerMessage::LoadLevel {
            name: "x".repeat(10_000),
            level: proving_ground(),
        });
        assert!(c.is_disconnected() && c.world().is_none());

        let mut c = client();
        c.handle_message(ServerMessage::LoadLevel {
            name: shared::sim::BUILTIN_LEVEL.into(),
            level: proving_ground(),
        });
        assert!(c.world().is_some() && !c.is_disconnected());
    }

    #[test]
    fn late_snapshots_cannot_grow_the_frame_buffer() {
        let mut c = client();
        let snapshot = |tick: u32| SnapshotMessage {
            tick,
            baseline_tick: None,
            last_input_seq: 0,
            own: None,
            entities: protocol::encode_full(&NetFrame::default()),
            fx: Vec::new(),
        };
        c.handle_snapshot(snapshot(10_000));
        for tick in (1..10_000).rev() {
            c.handle_snapshot(snapshot(tick));
        }
        let kept = interpolation::FRAME_RETENTION_TICKS as usize + 1;
        assert_eq!(c.frames.len(), kept);
        assert_eq!(c.stats.snapshots_received, kept as u64);
        assert_eq!(c.stats.snapshots_dropped, 10_000 - kept as u64);
        assert_eq!(c.latest_snapshot_tick, Some(10_000));
    }

    #[test]
    fn server_tuning_is_sanitized() {
        let mut c = client();
        let mut t = Tuning::default();
        t.combat.bellows.pellets = u32::MAX;
        t.combat.hitscan_range = 1.0e30;
        t.movement.gravity = f32::NAN;
        c.handle_message(ServerMessage::Tuning(Box::new(t)));
        let t = c.tuning();
        assert_eq!(t.combat.bellows.pellets, 64);
        assert!(t.combat.hitscan_range <= 1.0e4);
        assert_eq!(t.movement.gravity, Tuning::default().movement.gravity);
        assert!(c.drain_events().contains(&NetEvent::TuningChanged));
    }
}
