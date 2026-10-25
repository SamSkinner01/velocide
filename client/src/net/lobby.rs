//! Lobby side of [`NetClient`]: the server's lobby state and level catalog, pings, and the
//! host's commands (docs/lobby.md).
//!
//! When the lobby phase leaves the match (`Lobby` / `Countdown`) the client unloads its level
//! copy and prediction (the server removed every body), so `is_in_game()` turns false and the
//! game screen hands the session back to the lobby screen. The next `LoadLevel` + snapshot
//! starts prediction again as on first connect.

use shared::lobby::{LevelInfo, LobbyPhase, LobbyRequest, LobbyState, LobbyUpdate, MatchSettings};
use shared::protocol::ClientMessage;
use shared::sim::PlayerId;

use super::{ConnectionState, NetClient, NetEvent, SnapshotBuffer};

impl NetClient {
    /// Latest lobby state (phase, settings, players with ping / ready / host).
    pub fn lobby(&self) -> Option<&LobbyState> {
        self.lobby.as_ref()
    }

    pub fn lobby_phase(&self) -> Option<LobbyPhase> {
        self.lobby.as_ref().map(|l| l.phase)
    }

    /// The server is in its lobby (or counting down): show the lobby screen.
    pub fn in_lobby(&self) -> bool {
        self.lobby_phase().is_some_and(LobbyPhase::in_lobby)
    }

    /// Levels the server can run.
    pub fn levels(&self) -> &[LevelInfo] {
        &self.levels
    }

    /// This client is the lobby host (may change settings, start, return to the lobby).
    pub fn is_host(&self) -> bool {
        match (self.lobby.as_ref(), self.local_player_id()) {
            (Some(l), Some(id)) => l.is_host(id),
            _ => false,
        }
    }

    /// Round-trip time of a player in ms: the server's measurement for everyone, our own
    /// measurement for us (fresher).
    pub fn player_ping(&self, id: PlayerId) -> Option<f32> {
        if Some(id) == self.local_player_id() {
            return Some(self.stats.rtt_ms);
        }
        let p = self.lobby.as_ref()?.player(id)?;
        (!p.bot).then_some(p.ping_ms as f32)
    }

    pub fn send_lobby(&mut self, request: LobbyRequest) {
        self.send_reliable(&ClientMessage::Lobby(request));
    }

    /// Host: new settings for the next match (the server validates and clamps them).
    pub fn set_match_settings(&mut self, settings: MatchSettings) {
        self.send_lobby(LobbyRequest::SetSettings(settings));
    }

    /// Host: start the countdown (all ready, or `force`).
    pub fn start_match(&mut self, force: bool) {
        self.send_lobby(LobbyRequest::StartMatch { force });
    }

    pub fn cancel_countdown(&mut self) {
        self.send_lobby(LobbyRequest::CancelCountdown);
    }

    /// Host: end the match / leave the results screen; everyone returns to the lobby.
    pub fn return_to_lobby(&mut self) {
        self.send_lobby(LobbyRequest::ReturnToLobby);
    }

    pub fn kick(&mut self, player: PlayerId) {
        self.send_lobby(LobbyRequest::Kick { player });
    }

    /// Presents the listen-server host token right after the welcome.
    pub(super) fn claim_host(&mut self) {
        if let Some(token) = self.host_token {
            self.send_lobby(LobbyRequest::ClaimHost { token });
        }
    }

    pub(super) fn on_lobby_update(&mut self, update: LobbyUpdate) {
        match update {
            LobbyUpdate::State(state) => {
                let previous = self.lobby_phase();
                let phase = state.phase;
                if !phase.level_running() {
                    self.unload_level();
                }
                self.lobby = Some(state);
                self.events
                    .push_back(NetEvent::LobbyChanged { phase, previous });
            }
            LobbyUpdate::Levels(levels) => self.levels = levels,
            LobbyUpdate::Notice(text) => self.events.push_back(NetEvent::Notice { text }),
        }
    }

    /// Back to the lobby: no level, no body, nothing to interpolate.
    fn unload_level(&mut self) {
        if self.world.is_none() && self.prediction.is_none() {
            return;
        }
        log::info!("level unloaded (lobby)");
        self.world = None;
        self.prediction = None;
        self.remote.clear();
        self.frames = SnapshotBuffer::new();
        self.render_tick = None;
        if self.state == ConnectionState::InGame {
            self.state = ConnectionState::Loading;
        }
    }
}
