//! Lobby and match flow on the server (docs/lobby.md).
//!
//! The server is always in one [`LobbyPhase`] (`shared::lobby`):
//!
//! - **Lobby**: the host edits [`MatchSettings`], players ready up and chat. With the
//!   **warm-up** ([`LobbyConfig::warmup`], on by default) the server runs the hub level
//!   ([`WARMUP_LEVEL`], `shared::sim::warmup`): everyone has a body, runs around and shoots,
//!   deaths respawn in a second, nothing is scored, and players hurt each other only when the
//!   selected next mode is PvP. Without it (or if the hub can't be loaded) players have no body
//!   and nothing is simulated (the tick counter stands still).
//! - **Countdown**: [`shared::lobby::COUNTDOWN_SECS`] seconds (the warm-up keeps running),
//!   then [`Server::start_match`].
//! - **InMatch**: a fresh `SimWorld` for the chosen level (doors, encounters... reset), every
//!   connected player and bot gets a body, `LoadLevel` goes to everyone. Players joining now
//!   go straight in (join-in-progress).
//! - **Results**: [`Server::end_match`] (called by the game-mode rules when a match is won /
//!   the exit is reached). The level keeps running behind the results screen until the host
//!   continues ([`LobbyRequest::ReturnToLobby`]) or [`RESULTS_TIMEOUT_SECS`] pass; then
//!   everyone is back in a fresh warm-up.
//!
//! **Host**: on a listen server the player whose client presents the session's secret
//! ([`LobbyConfig::host_token`], `LobbyRequest::ClaimHost`); on a dedicated server the
//! longest-connected human, re-assigned when they leave. Only the host may change settings,
//! start, cancel, return to the lobby or kick; every request is validated, clamped and
//! rate-limited, and rejected ones are answered with a `LobbyUpdate::Notice` to the sender.
//!
//! Servers without a lobby (single-player, `LobbyConfig::enabled == false`) start in
//! `InMatch` with `ServerConfig::level`; for them "return to lobby" restarts the level.
//!
//! The lobby state (phase, settings, players with ready / ping / host) is broadcast whenever it
//! changes and once per second with fresh pings (renet RTT per connection).

use std::collections::BTreeMap;
use std::sync::Arc;

use renet::ClientId;
use shared::cosmetics::Cosmetics;
use shared::level::{LevelDef, LevelKind};
use shared::lobby::{
    COUNTDOWN_SECS, GameMode, LevelInfo, LobbyPhase, LobbyPlayer, LobbyRequest, LobbyState,
    LobbyUpdate, MatchSettings,
};
use shared::protocol::{self, CHANNEL_RELIABLE, ServerMessage};
pub use shared::sim::warmup::WARMUP_LEVEL;
use shared::sim::{PlayerId, SimWorld};

use crate::{LAG_COMP_HISTORY_MS, LagCompHistory, Server, ServerConfig};

/// Pings in the lobby state are refreshed this often (seconds).
const PING_INTERVAL_SECS: f64 = 1.0;
/// The lobby state is compared with the last broadcast at most this often (seconds).
const STATE_CHECK_SECS: f64 = 1.0 / 30.0;
/// The results screen returns to the lobby by itself after this long (seconds).
pub const RESULTS_TIMEOUT_SECS: f64 = 30.0;
/// Lobby requests (incl. ready toggles) per client: burst size and sustained rate. Spam beyond
/// that is dropped silently.
const REQUEST_BURST: f32 = 10.0;
const REQUESTS_PER_SECOND: f32 = 4.0;
/// Extra netcode slots beyond the player limit, so a full server can still answer
/// "server is full" instead of netcode silently denying the connection.
pub const SPARE_CONNECTION_SLOTS: usize = 2;

/// Lobby options of a [`ServerConfig`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LobbyConfig {
    /// Start in the lobby (listen and dedicated servers). When false (single-player, tests)
    /// `ServerConfig::level` runs immediately.
    pub enabled: bool,
    /// Start the countdown by itself whenever every connected player is ready (dedicated
    /// servers without a host at the keyboard), and leave the results screen after the timeout.
    pub autostart: bool,
    /// Listen servers: the secret the host's own client presents (`LobbyRequest::ClaimHost`)
    /// to become host. `None` = dedicated rules (first player to join is host, migrates).
    pub host_token: Option<u64>,
    /// Run the warm-up hub ([`WARMUP_LEVEL`]) in the lobby phases: everyone gets a body and can
    /// run around until the match starts. Without it (or if the hub can't be loaded) the lobby
    /// has no world (`velocide-server --no-warmup`).
    pub warmup: bool,
    /// Initial mode, limits and friendly fire. `level` and `max_players` are taken from
    /// `ServerConfig::level` / `ServerConfig::max_players`. If the mode doesn't suit the level,
    /// the level's natural mode is used.
    pub settings: MatchSettings,
}

impl LobbyConfig {
    /// A lobby with default settings and the warm-up.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            warmup: true,
            ..Default::default()
        }
    }
}

/// The look of a server bot: varied, the same for a slot every time.
pub fn bot_cosmetics(id: PlayerId) -> Cosmetics {
    Cosmetics::from_seed(0xB07 + id.0 as u32)
}

/// Lobby control for whoever owns the server (`ServerCommand::Lobby`): no host check.
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyCommand {
    SetSettings(MatchSettings),
    /// Countdown (when all are ready, or `force`).
    StartMatch {
        force: bool,
    },
    /// Results phase.
    EndMatch,
    ReturnToLobby,
}

/// Match-flow state of a [`Server`].
pub(crate) struct MatchFlow {
    pub(crate) phase: LobbyPhase,
    pub(crate) settings: MatchSettings,
    pub(crate) host: Option<PlayerId>,
    pub(crate) levels: Vec<LevelInfo>,
    /// The warm-up hub, loaded once at startup (`None`: no warm-up).
    warmup_level: Option<Arc<LevelDef>>,
    /// The world is the warm-up (Lobby / Countdown phases with `warmup_level`).
    warmup_on: bool,
    enabled: bool,
    autostart: bool,
    host_token: Option<u64>,
    countdown_end: Option<f64>,
    /// The countdown was started by autostart (cancelled when someone isn't ready any more).
    countdown_auto: bool,
    results_end: Option<f64>,
    join_order: BTreeMap<PlayerId, u64>,
    next_join: u64,
    pings: BTreeMap<PlayerId, u16>,
    next_ping_time: f64,
    next_state_check: f64,
    /// Request rate limit per player: (tokens, last refill time).
    limits: BTreeMap<PlayerId, (f32, f64)>,
    last_sent: Option<LobbyState>,
}

impl MatchFlow {
    pub(crate) fn new(config: &ServerConfig, level: &LevelDef) -> MatchFlow {
        let lobby = &config.lobby;
        let mut levels = if lobby.enabled {
            shared::lobby::scan_levels()
        } else {
            Vec::new()
        };
        if !levels.iter().any(|l| l.name == config.level) {
            levels.push(LevelInfo::from_def(&config.level, level));
        }
        let mut settings = lobby.settings.clone();
        settings.level = config.level.clone();
        let info = LevelInfo::from_def(&config.level, level);
        // Without a lobby (single-player, tests) the level's kind decides the mode, so arenas
        // keep PvP rules, unless a mode no level kind implies was asked for (solo grind); with
        // one, the configured mode wins if the level allows it.
        let explicit = !matches!(settings.mode, GameMode::Campaign | GameMode::Deathmatch);
        if (!lobby.enabled && !explicit) || !settings.mode.accepts(&info) {
            settings.mode = GameMode::for_level_kind(info.kind);
        }
        settings.max_players = config.max_players.clamp(1, u8::MAX as usize) as u8;
        if lobby.enabled {
            settings.clamp();
        }
        let warmup_level = (lobby.enabled && lobby.warmup)
            .then(|| match shared::sim::load_level(WARMUP_LEVEL) {
                Ok(def) if def.kind == LevelKind::Hub => Some(Arc::new(def)),
                Ok(_) => {
                    log::warn!("'{WARMUP_LEVEL}' is not a hub level: lobby without warm-up");
                    None
                }
                Err(e) => {
                    log::warn!("no warm-up hub ({e:#}): lobby without a world");
                    None
                }
            })
            .flatten();
        MatchFlow {
            phase: if lobby.enabled {
                LobbyPhase::Lobby
            } else {
                LobbyPhase::InMatch
            },
            settings,
            host: None,
            levels,
            warmup_level,
            warmup_on: false,
            enabled: lobby.enabled,
            autostart: lobby.autostart,
            host_token: lobby.host_token,
            countdown_end: None,
            countdown_auto: false,
            results_end: None,
            join_order: BTreeMap::new(),
            next_join: 0,
            pings: BTreeMap::new(),
            next_ping_time: 0.0,
            next_state_check: 0.0,
            limits: BTreeMap::new(),
            last_sent: None,
        }
    }
}

impl Server {
    // ---- public API -----------------------------------------------------------------------------

    /// Current phase of the match flow.
    pub fn lobby_phase(&self) -> LobbyPhase {
        self.flow.phase
    }

    /// Settings of the next (in the lobby) or running match.
    pub fn match_settings(&self) -> &MatchSettings {
        &self.flow.settings
    }

    /// The player allowed to change settings and start the match.
    pub fn lobby_host(&self) -> Option<PlayerId> {
        self.flow.host
    }

    /// Levels this server can run (sent to clients for the lobby's level choice).
    pub fn levels(&self) -> &[LevelInfo] {
        &self.flow.levels
    }

    /// A level is loaded and players have bodies: a match, its results, or the warm-up.
    pub fn world_running(&self) -> bool {
        self.flow.phase.level_running() || self.flow.warmup_on
    }

    /// The players are in the warm-up hub (Lobby / Countdown with the warm-up world).
    pub fn in_warmup(&self) -> bool {
        self.flow.warmup_on && self.flow.phase.in_lobby()
    }

    /// The lobby state as it is broadcast.
    pub fn lobby_state(&self) -> LobbyState {
        let mut players: Vec<LobbyPlayer> = self
            .clients
            .values()
            .filter_map(|c| {
                let id = c.player?;
                Some(LobbyPlayer {
                    id,
                    name: c.name.clone(),
                    ready: c.ready,
                    color: id.0 % shared::lobby::MAX_PLAYERS,
                    ping_ms: self.flow.pings.get(&id).copied().unwrap_or(0),
                    is_host: self.flow.host == Some(id),
                    bot: false,
                    cosmetics: c.cosmetics,
                })
            })
            .collect();
        players.extend(self.bots.iter().map(|(id, name)| LobbyPlayer {
            id: *id,
            name: name.clone(),
            ready: true,
            color: id.0 % shared::lobby::MAX_PLAYERS,
            ping_ms: 0,
            is_host: false,
            bot: true,
            cosmetics: bot_cosmetics(*id),
        }));
        players.sort_by_key(|p| p.id);
        LobbyState {
            phase: self.flow.phase,
            settings: self.flow.settings.clone(),
            players,
            host: self.flow.host,
            lobby_enabled: self.flow.enabled,
            autostart: self.flow.autostart,
            results_timeout: self
                .flow
                .results_end
                .filter(|_| self.flow.phase == LobbyPhase::Results)
                .map(|t| (t - self.time).max(0.0).ceil() as u16),
            warmup: self.in_warmup(),
        }
    }

    /// Changes the settings of the next match (Lobby phase only). Values are clamped; the
    /// level must exist and suit the mode, and the player limit must fit everyone present.
    pub fn set_match_settings(&mut self, mut settings: MatchSettings) -> Result<(), String> {
        if !self.flow.enabled {
            return Err("This server has no lobby".into());
        }
        if self.flow.phase != LobbyPhase::Lobby {
            return Err("Settings can only be changed in the lobby".into());
        }
        settings.clamp();
        settings.validate(&self.flow.levels)?;
        let present = self.member_count();
        if present > settings.max_players as usize {
            return Err(format!(
                "{present} players are here: {} allows {}",
                settings.mode.title(),
                settings.max_players
            ));
        }
        if settings != self.flow.settings {
            log::info!("match settings: {}", settings.summary(&self.flow.levels));
            // The warm-up follows the next mode: PvP damage between players or not, and that
            // mode's balance profile.
            self.world.set_warmup_next(&settings);
            self.flow.settings = settings;
            self.update_effective_tuning();
        }
        Ok(())
    }

    /// Starts the countdown (host START): requires everyone to be ready unless `force`.
    pub fn begin_countdown(&mut self, force: bool) -> Result<(), String> {
        if !self.flow.enabled {
            return Err("This server has no lobby".into());
        }
        if self.flow.phase != LobbyPhase::Lobby {
            return Err("The match is already starting".into());
        }
        if self.human_count() == 0 {
            return Err("Nobody is here".into());
        }
        if !force && !self.all_ready() {
            return Err("Not everyone is ready (FORCE START starts anyway)".into());
        }
        self.flow.settings.validate(&self.flow.levels)?;
        self.start_countdown(false);
        Ok(())
    }

    /// Loads the chosen level now and puts every connected player (and bot) into it.
    pub fn start_match(&mut self) -> Result<(), String> {
        let mut settings = self.flow.settings.clone();
        if self.flow.enabled {
            settings.clamp();
            settings.validate(&self.flow.levels)?;
        }
        let level = shared::sim::load_level(&settings.level)
            .map_err(|e| format!("Couldn't load {}: {e:#}", settings.level))?;
        let name = settings.level.clone();
        self.load_match_world(&name, Arc::new(level), settings);
        Ok(())
    }

    /// The match is over: Results phase (clients show the results screen). The game-mode rules
    /// call this when a deathmatch is won or the campaign exit is reached.
    pub fn end_match(&mut self) {
        if self.flow.phase != LobbyPhase::InMatch {
            return;
        }
        log::info!("match over");
        // Game modes: make sure the sim's match is over too (results, frozen world; modes.rs).
        self.end_sim_match();
        self.flow.phase = LobbyPhase::Results;
        // Enemies stop attacking on the results screen (docs/enemies.md).
        self.world.frozen = true;
        // Lobby servers return by themselves; single-player waits for the player.
        self.flow.results_end = self
            .flow
            .enabled
            .then_some(self.time + RESULTS_TIMEOUT_SECS);
        self.broadcast_lobby_state();
    }

    /// Ends the match (or the results screen) and brings everyone back to the lobby, everyone
    /// un-readied: into a fresh warm-up (everyone spawned in the hub), or without one a fresh,
    /// empty copy of the level. Without a lobby: restarts the level.
    pub fn return_to_lobby(&mut self) {
        if !self.flow.enabled {
            let name = self.world.level_name.clone();
            let level = self.world.level.clone();
            let settings = self.flow.settings.clone();
            self.load_match_world(&name, level, settings);
            return;
        }
        match self.flow.phase {
            LobbyPhase::Lobby => return,
            LobbyPhase::Countdown { .. } => {
                self.cancel_countdown();
                return;
            }
            LobbyPhase::InMatch | LobbyPhase::Results => {}
        }
        log::info!("returning to the lobby");
        if !self.enter_warmup() {
            let world = SimWorld::new(
                &self.world.level_name,
                self.world.level.clone(),
                self.config.tick_rate,
            );
            let settings = self.flow.settings.clone();
            self.replace_world(world, settings);
        }
        self.flow.phase = LobbyPhase::Lobby;
        self.flow.results_end = None;
        self.flow.countdown_end = None;
        let mut updated = Vec::new();
        for conn in self.clients.values_mut() {
            if conn.ready {
                conn.ready = false;
                updated.extend(conn.info());
            }
        }
        for info in updated {
            self.broadcast(&ServerMessage::PlayerUpdated(info));
        }
        self.broadcast_lobby_state();
    }

    pub fn handle_lobby_command(&mut self, cmd: LobbyCommand) {
        let result = match cmd {
            LobbyCommand::SetSettings(s) => self.set_match_settings(s),
            LobbyCommand::StartMatch { force } => self.begin_countdown(force),
            LobbyCommand::EndMatch => {
                self.end_match();
                Ok(())
            }
            LobbyCommand::ReturnToLobby => {
                self.return_to_lobby();
                Ok(())
            }
        };
        if let Err(e) = result {
            log::warn!("lobby command refused: {e}");
        }
    }

    // ---- hooks called from lib.rs ---------------------------------------------------------------

    /// Applies the initial settings to the world (end of `Server::with_transport`); lobby
    /// servers with a warm-up start in the hub.
    pub(crate) fn init_match_flow(&mut self) {
        self.world.apply_match_settings(self.flow.settings.clone());
        if self.flow.enabled && self.enter_warmup() {
            log::info!("warm-up: '{WARMUP_LEVEL}'");
        }
        if self.flow.enabled {
            log::info!(
                "lobby open: {} ({} levels){}",
                self.flow.settings.summary(&self.flow.levels),
                self.flow.levels.len(),
                if self.flow.autostart {
                    ", autostart"
                } else {
                    ""
                }
            );
        }
    }

    /// Most players the server takes right now.
    pub(crate) fn player_limit(&self) -> usize {
        if self.flow.enabled {
            self.flow.settings.max_players.max(1) as usize
        } else {
            self.config.max_players.max(1)
        }
    }

    /// A player was accepted (after `Welcome` / `LoadLevel` / `PlayerList`).
    pub(crate) fn lobby_on_join(&mut self, client: ClientId, player: PlayerId) {
        self.flow.join_order.insert(player, self.flow.next_join);
        self.flow.next_join += 1;
        self.flow.limits.remove(&player);
        self.flow.pings.remove(&player);
        let levels = ServerMessage::Lobby(LobbyUpdate::Levels(self.flow.levels.clone()));
        self.send(client, &levels);
        self.update_host();
    }

    /// `ClientMessage::SetReady` (rate-limited).
    pub(crate) fn lobby_set_ready(&mut self, client: ClientId, player: PlayerId, ready: bool) {
        if !self.lobby_rate_ok(player) {
            return;
        }
        let Some(conn) = self.clients.get_mut(&client) else {
            return;
        };
        if conn.ready != ready {
            conn.ready = ready;
            if let Some(info) = conn.info() {
                self.broadcast(&ServerMessage::PlayerUpdated(info));
            }
        }
    }

    /// `ClientMessage::SetCosmetics` (sanitised, rate-limited with the lobby requests): stored
    /// and shared with everyone (`PlayerUpdated`, lobby state).
    pub(crate) fn lobby_set_cosmetics(
        &mut self,
        client: ClientId,
        player: PlayerId,
        cosmetics: Cosmetics,
    ) {
        if !self.lobby_rate_ok(player) {
            return;
        }
        let cosmetics = cosmetics.sanitized();
        let Some(conn) = self.clients.get_mut(&client) else {
            return;
        };
        if conn.cosmetics != cosmetics {
            conn.cosmetics = cosmetics;
            if let Some(info) = conn.info() {
                self.broadcast(&ServerMessage::PlayerUpdated(info));
            }
        }
    }

    /// `ClientMessage::Lobby` from `player`.
    pub(crate) fn handle_lobby_request(
        &mut self,
        client: ClientId,
        player: PlayerId,
        request: LobbyRequest,
    ) {
        if !self.lobby_rate_ok(player) {
            return;
        }
        if let LobbyRequest::ClaimHost { token } = request {
            if self.flow.host_token == Some(token) {
                if self.flow.host != Some(player) {
                    log::info!("{player} claimed host");
                }
                self.flow.host = Some(player);
            } else {
                log::warn!("{player} presented a wrong host token");
            }
            return;
        }
        if self.flow.host != Some(player) {
            self.notice(client, "Only the host can do that");
            return;
        }
        let result = match request {
            LobbyRequest::ClaimHost { .. } => Ok(()),
            LobbyRequest::SetSettings(settings) => self.set_match_settings(settings),
            LobbyRequest::StartMatch { force } => self.begin_countdown(force),
            LobbyRequest::CancelCountdown => {
                self.cancel_countdown();
                Ok(())
            }
            LobbyRequest::ReturnToLobby => {
                if self.flow.phase.level_running() {
                    self.return_to_lobby();
                }
                Ok(())
            }
            LobbyRequest::Kick { player: target } => {
                if target == player {
                    Err("You can't kick yourself".into())
                } else {
                    self.kick(target, "kicked by the host");
                    Ok(())
                }
            }
        };
        if let Err(e) = result {
            self.notice(client, &e);
        }
    }

    /// Timers (countdown, results, autostart), host bookkeeping, pings and the state broadcast.
    /// Called once per `Server::update`.
    pub(crate) fn update_lobby(&mut self) {
        let now = self.time;
        self.update_host();
        let humans = self.human_count();
        match self.flow.phase {
            LobbyPhase::Lobby => {
                if self.flow.autostart && self.all_ready() {
                    self.start_countdown(true);
                }
            }
            LobbyPhase::Countdown { secs } => {
                if humans == 0 || (self.flow.countdown_auto && !self.all_ready()) {
                    self.cancel_countdown();
                } else {
                    let end = self.flow.countdown_end.unwrap_or(now);
                    let left = end - now;
                    if left <= 0.0 {
                        if let Err(e) = self.start_match() {
                            log::warn!("couldn't start the match: {e}");
                            // Still in the warm-up (if any): back to the lobby phase.
                            self.flow.phase = LobbyPhase::Lobby;
                            self.flow.countdown_end = None;
                            if let Some(host) = self.flow.host {
                                self.notice_player(host, &e);
                            }
                        }
                    } else {
                        let s = left.ceil().clamp(1.0, u8::MAX as f64) as u8;
                        if s != secs {
                            self.flow.phase = LobbyPhase::Countdown { secs: s };
                        }
                    }
                }
            }
            LobbyPhase::InMatch | LobbyPhase::Results => {
                if self.flow.enabled && humans == 0 {
                    // Everyone left: don't keep an empty match running.
                    self.return_to_lobby();
                } else if self.flow.phase == LobbyPhase::Results
                    && self.flow.results_end.is_some_and(|t| now >= t)
                {
                    self.return_to_lobby();
                }
            }
        }

        if now >= self.flow.next_ping_time {
            self.flow.next_ping_time = now + PING_INTERVAL_SECS;
            let pings: BTreeMap<PlayerId, u16> = self
                .clients
                .values()
                .filter_map(|c| {
                    let rtt_ms = self.renet.rtt(c.id) * 1000.0;
                    Some((c.player?, rtt_ms.round().clamp(0.0, u16::MAX as f64) as u16))
                })
                .collect();
            self.flow.pings = pings;
            self.flow.next_state_check = now;
        }
        if now >= self.flow.next_state_check {
            self.flow.next_state_check = now + STATE_CHECK_SECS;
            let state = self.lobby_state();
            if self.flow.last_sent.as_ref() != Some(&state) {
                self.send_lobby_state(state);
            }
        }
    }

    // ---- internals ------------------------------------------------------------------------------

    fn start_countdown(&mut self, auto: bool) {
        self.flow.phase = LobbyPhase::Countdown {
            secs: COUNTDOWN_SECS,
        };
        self.flow.countdown_end = Some(self.time + COUNTDOWN_SECS as f64);
        self.flow.countdown_auto = auto;
        log::info!(
            "match starting in {COUNTDOWN_SECS} s{}",
            if auto { " (autostart)" } else { "" }
        );
        self.broadcast_lobby_state();
    }

    fn cancel_countdown(&mut self) {
        if let LobbyPhase::Countdown { .. } = self.flow.phase {
            self.flow.phase = LobbyPhase::Lobby;
            self.flow.countdown_end = None;
            log::info!("countdown cancelled");
            self.broadcast_lobby_state();
        }
    }

    /// Replaces the world with a fresh copy of `level`, gives everyone a body and tells the
    /// clients (`LoadLevel`, then the `InMatch` lobby state).
    fn load_match_world(&mut self, name: &str, level: Arc<LevelDef>, settings: MatchSettings) {
        let world = SimWorld::new(name, level, self.config.tick_rate);
        self.replace_world(world, settings);
        self.flow.warmup_on = false;
        let members = self.spawn_members();
        self.flow.phase = LobbyPhase::InMatch;
        self.flow.countdown_end = None;
        self.flow.results_end = None;
        log::info!(
            "match started: {} with {members} players",
            self.world.match_settings.summary(&self.flow.levels),
        );
        self.broadcast_load_level();
        self.broadcast_lobby_state();
    }

    /// Replaces the world with a fresh warm-up on the hub (everyone connected gets a body) and
    /// sends it (`LoadLevel`); the caller sets and broadcasts the phase. False (nothing changed)
    /// when there is no warm-up.
    fn enter_warmup(&mut self) -> bool {
        let Some(level) = self.flow.warmup_level.clone() else {
            return false;
        };
        let world = SimWorld::new(WARMUP_LEVEL, level, self.config.tick_rate);
        // The hub's own rules; `start_warmup` then follows the lobby's next mode.
        let hub = MatchSettings {
            friendly_fire: false,
            ..MatchSettings::new(GameMode::Campaign, WARMUP_LEVEL)
        };
        self.replace_world(world, hub);
        self.world.start_warmup(&self.flow.settings);
        // The balance of the selected mode (`update_effective_tuning` reads the lobby's).
        self.update_effective_tuning();
        self.flow.warmup_on = true;
        self.spawn_members();
        self.broadcast_load_level();
        true
    }

    /// Gives every connected player and bot a body in the (fresh) world, in id order. Returns
    /// how many.
    fn spawn_members(&mut self) -> usize {
        let mut members: Vec<(PlayerId, String)> = self
            .clients
            .values()
            .filter_map(|c| Some((c.player?, c.name.clone())))
            .chain(self.bots.iter().map(|(id, n)| (*id, n.clone())))
            .collect();
        members.sort_by_key(|(id, _)| *id);
        for (id, name) in &members {
            self.world.add_player(*id, name);
        }
        members.len()
    }

    fn broadcast_load_level(&mut self) {
        let msg = ServerMessage::LoadLevel {
            name: self.world.level_name.clone(),
            level: Box::new((*self.world.level).clone()),
        };
        self.broadcast(&msg);
    }

    /// Swaps in a fresh world (match start / back to the lobby). Server-side configuration
    /// carries over (enemy definitions; the tick counter, so client snapshot ordering and
    /// clocks stay monotonic); the match settings set the combat rules; dev toggles (god,
    /// `/ff`) reset with the world. Old snapshots, hitboxes, queued inputs and pending FX
    /// don't apply to the new world.
    fn replace_world(&mut self, mut world: SimWorld, settings: MatchSettings) {
        world.tick = self.world.tick;
        world.enemy_defs = std::mem::take(&mut self.world.enemy_defs);
        // Loadouts chosen so far go along (docs/arsenal.md).
        world.loadout_requests = std::mem::take(&mut self.world.loadout_requests);
        // Enemy navigation: reuse the nav grid for the same level, else build it now rather
        // than on the first enemy tick (docs/enemies.md).
        match self.world.ai.nav.take() {
            Some(nav) if *world.level == *self.world.level => world.ai.nav = Some(nav),
            _ => {
                world.build_nav();
            }
        }
        world.apply_match_settings(settings);
        self.world = world;
        // The new mode's balance profile (docs/arsenal.md).
        self.update_effective_tuning();
        self.fx.prune(self.world.tick);
        self.lagcomp = LagCompHistory::new(self.config.tick_rate, LAG_COMP_HISTORY_MS);
        for conn in self.clients.values_mut() {
            conn.ack_tick = None;
            conn.sent_frames.clear();
            conn.inputs.discard_pending();
        }
    }

    fn broadcast_lobby_state(&mut self) {
        let state = self.lobby_state();
        self.send_lobby_state(state);
    }

    fn send_lobby_state(&mut self, state: LobbyState) {
        self.broadcast(&ServerMessage::Lobby(LobbyUpdate::State(state.clone())));
        self.flow.last_sent = Some(state);
    }

    fn notice(&mut self, client: ClientId, text: &str) {
        log::debug!("notice to client {client}: {text}");
        self.renet.send_message(
            client,
            CHANNEL_RELIABLE,
            protocol::encode(&ServerMessage::Lobby(LobbyUpdate::Notice(text.to_string()))),
        );
    }

    fn notice_player(&mut self, player: PlayerId, text: &str) {
        if let Some(id) = self
            .clients
            .values()
            .find(|c| c.player == Some(player))
            .map(|c| c.id)
        {
            self.notice(id, text);
        }
    }

    /// Connected humans with a player slot.
    fn human_count(&self) -> usize {
        self.clients.values().filter(|c| c.player.is_some()).count()
    }

    /// At least one human, and every human is ready (bots always are).
    fn all_ready(&self) -> bool {
        self.human_count() > 0
            && self
                .clients
                .values()
                .filter(|c| c.player.is_some())
                .all(|c| c.ready)
    }

    /// Humans + bots.
    fn member_count(&self) -> usize {
        self.human_count() + self.bots.len()
    }

    /// Keeps `host` valid: drops a host who left; on dedicated servers (no host token) the
    /// longest-connected human becomes host.
    fn update_host(&mut self) {
        let connected = |id: PlayerId| self.clients.values().any(|c| c.player == Some(id));
        self.flow.join_order.retain(|id, _| connected(*id));
        self.flow.limits.retain(|id, _| connected(*id));
        let old = self.flow.host;
        if old.is_some_and(|h| !connected(h)) {
            self.flow.host = None;
        }
        if self.flow.host.is_none() && self.flow.host_token.is_none() {
            self.flow.host = self
                .flow
                .join_order
                .iter()
                .min_by_key(|(_, order)| **order)
                .map(|(id, _)| *id);
        }
        if self.flow.host != old
            && let Some(new) = self.flow.host
        {
            let name = self
                .clients
                .values()
                .find(|c| c.player == Some(new))
                .map(|c| c.name.clone())
                .unwrap_or_default();
            log::info!("host is now {new} '{name}'");
            if old.is_some() {
                self.say(&format!("{name} is now the host"));
            }
        }
    }

    /// Token bucket per player for lobby requests and ready toggles.
    fn lobby_rate_ok(&mut self, player: PlayerId) -> bool {
        let now = self.time;
        let (tokens, last) = self
            .flow
            .limits
            .entry(player)
            .or_insert((REQUEST_BURST, now));
        *tokens =
            (*tokens + (now - *last).max(0.0) as f32 * REQUESTS_PER_SECOND).min(REQUEST_BURST);
        *last = now;
        if *tokens < 1.0 {
            return false;
        }
        *tokens -= 1.0;
        true
    }
}
