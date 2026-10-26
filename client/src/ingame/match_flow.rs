//! Match flow on the game screen (docs/lobby.md): reacting to lobby phase changes (the server
//! went back to its lobby → hand the session to the lobby screen; the match ended → results
//! screen), the results screen, and the scoreboard rows with everyone's ping.
//!
//! **Results screen**: shown while the server is in `LobbyPhase::Results` (after
//! `Server::end_match`). Today it is the scoreboard plus the match summary; the game-mode /
//! style code adds grades, times and per-player stats here (`results_ui`) and fills the score
//! cells in [`InGame::score_rows`]. The host continues to the lobby (single-player: restart or
//! main menu); everyone else waits, the server returns by itself after a timeout.

use engine::audio::Sfx;
use engine::gfx::UiDrawList;
use engine::render::Renderer;
use engine::ui::{Rect, TextStyle, Ui, UiDrawExt};
use glam::vec2;
use shared::lobby::{GameMode, LobbyPhase};

use super::remote_players::player_color;
use super::scoreboard::{self, ScoreColumn, ScoreRow, ScoreTable};
use super::{InGame, hud_mode_name};
use crate::app::Common;
use crate::frame::Transition;
use crate::net::{NetClient, SessionKind};

/// Results-screen state.
#[derive(Clone, Debug, Default)]
pub struct MatchUi {
    /// When the results screen opened (game time).
    results_since: Option<f64>,
}

impl InGame {
    /// Builds the level the session already received (the lobby screen consumed its
    /// `LevelLoaded` event before handing the session over).
    pub(super) fn load_received_level(&mut self, common: &mut Common, renderer: &mut Renderer) {
        let Some(world) = self.net().and_then(NetClient::world) else {
            return;
        };
        let (name, level) = (world.level_name.clone(), world.level.clone());
        self.world.load(renderer, &name, &level);
        common
            .audio
            .set_music(Common::game_music(self.world.kind()));
        common.audio.set_music_intensity(0.2);
    }

    /// `NetEvent::LobbyChanged`: leave for the lobby, or open / close the results screen.
    pub(super) fn on_lobby_changed(
        &mut self,
        phase: LobbyPhase,
        previous: Option<LobbyPhase>,
        common: &mut Common,
        now: f64,
    ) -> Option<Transition> {
        match phase {
            LobbyPhase::Lobby | LobbyPhase::Countdown { .. } => Some(Transition::ReturnToLobby),
            LobbyPhase::Results => {
                if previous != Some(LobbyPhase::Results) {
                    self.match_ui.results_since = Some(now);
                    self.resume();
                    self.chat.close();
                    self.hud
                        .banner("MATCH OVER", common.theme.crimson_bright, now);
                    common.audio.play_2d(Sfx::UiSelect);
                }
                None
            }
            LobbyPhase::InMatch => {
                self.match_ui.results_since = None;
                // Game modes: a new match (single-player restart) closes the old results.
                self.modes.results = None;
                None
            }
        }
    }

    /// The results reveal starts over from `now` (after the play of the game, `replay.rs`).
    pub(super) fn restart_results_reveal(&mut self, now: f64) {
        if self.match_ui.results_since.is_some() {
            self.match_ui.results_since = Some(now);
        }
        if let Some(screen) = self.modes.results.take() {
            let screen =
                crate::menus::results::ResultsScreen::new(screen.results, screen.local, now);
            self.modes.results = Some(screen);
        }
    }

    /// The running session (tools / tests send server commands through it).
    pub fn session(&self) -> Option<&crate::net::Session> {
        self.session.as_ref()
    }

    /// The results screen is up (server in `LobbyPhase::Results`).
    pub fn results_open(&self) -> bool {
        self.net().and_then(NetClient::lobby_phase) == Some(LobbyPhase::Results)
    }

    /// Mode of the running match (from the lobby state; the level kind without one).
    pub fn match_mode(&self) -> GameMode {
        self.net()
            .and_then(NetClient::lobby)
            .map_or(GameMode::for_level_kind(self.world.kind()), |l| {
                l.settings.mode
            })
    }

    /// Scoreboard columns and one row per player (ping from the server's lobby state, our own
    /// RTT for us). Game modes fill the score cells and sort here.
    pub fn score_rows(&self) -> (Vec<ScoreColumn>, Vec<ScoreRow>) {
        let columns = scoreboard::columns_for(self.match_mode());
        let Some(net) = self.net() else {
            return (columns, Vec::new());
        };
        let local = net.local_player_id();
        let single = self.kind == SessionKind::SinglePlayer;
        let lobby = net.lobby().filter(|l| l.lobby_enabled);
        let ids: Vec<_> = net.players().keys().copied().collect();
        let mut rows: Vec<ScoreRow> = net
            .players()
            .values()
            .map(|p| {
                let mut row = ScoreRow::new(&p.name, player_color(p.id), columns.len());
                row.slot = Some(p.id.0 as usize);
                row.is_local = Some(p.id) == local;
                row.is_host = lobby.is_some_and(|l| l.is_host(p.id));
                row.ping_ms = if single || p.bot {
                    None
                } else {
                    net.player_ping(p.id)
                };
                row.status = if p.bot {
                    "BOT"
                } else if row.is_host {
                    "HOST"
                } else {
                    "ALIVE"
                }
                .to_string();
                // Game-mode stats: kills, deaths, frags, style rank / points (mode_hud.rs).
                self.fill_score_row(&columns, &mut row, p.id);
                row
            })
            .collect();
        self.sort_score_rows(&ids, &mut rows);
        (columns, rows)
    }

    fn score_title(&self) -> String {
        let mode = if self.kind == SessionKind::SinglePlayer {
            hud_mode_name(self.kind)
        } else {
            self.match_mode().title()
        };
        format!("{}  //  {mode}", self.world.name().to_uppercase())
    }

    /// The Tab scoreboard.
    pub(super) fn draw_scoreboard(&self, common: &Common, list: &mut UiDrawList, canvas: Rect) {
        let (columns, rows) = self.score_rows();
        let title = self.score_title();
        scoreboard::draw(
            list,
            &common.theme,
            canvas,
            &ScoreTable {
                title: &title,
                columns: &columns,
                rows: &rows,
                footer: None,
            },
        );
    }

    /// The results choices in one row at the bottom of the screen (under the game-mode results):
    /// RESTART / MAIN MENU (single player), CONTINUE TO LOBBY / LEAVE GAME (host), WAITING
    /// FOR THE HOST + LEAVE GAME (others), and the automatic return countdown.
    fn results_button_row(&mut self, ui: &mut Ui) -> (Option<Sfx>, Option<Transition>) {
        let net = self.net();
        let lobby = net.and_then(NetClient::lobby).cloned();
        let has_lobby = lobby.as_ref().is_some_and(|l| l.lobby_enabled);
        let host = net.is_some_and(NetClient::is_host);
        let timeout = lobby
            .as_ref()
            .and_then(|l| l.results_timeout)
            .filter(|_| has_lobby);
        let canvas = ui.canvas();
        let labels: [&str; 2] = if !has_lobby {
            ["RESTART", "MAIN MENU"]
        } else if host {
            ["CONTINUE TO LOBBY", "LEAVE GAME"]
        } else {
            ["", "LEAVE GAME"]
        };
        let n = labels.iter().filter(|l| !l.is_empty()).count();
        let w = 130.0 * n as f32 + 6.0 * (n as f32 - 1.0);
        let y = canvas.max.y - 26.0;
        let mut status = Vec::new();
        if has_lobby && !host {
            status.push("WAITING FOR THE HOST".to_string());
        }
        if let Some(secs) = timeout {
            status.push(format!("BACK TO THE LOBBY IN {secs} S"));
        }
        if !status.is_empty() {
            let style = TextStyle::new(ui.theme().bone_dim).centered();
            ui.painter().text(
                vec2(canvas.center().x, y - 13.0),
                &status.join("   //   "),
                &style,
            );
        }
        let rect = Rect::new(
            vec2(canvas.center().x - w * 0.5, y),
            vec2(canvas.center().x + w * 0.5, y + 16.0),
        );
        let mut first = false;
        let mut second = false;
        ui.area(rect, |ui| {
            ui.default_focus();
            ui.row(n, |ui| {
                if !labels[0].is_empty() {
                    first = ui.button(labels[0]);
                }
                second = ui.button(labels[1]);
            });
        });
        if first {
            if let Some(s) = &mut self.session {
                s.client_mut().return_to_lobby();
            }
            return (Some(Sfx::UiSelect), None);
        }
        if second {
            return (
                Some(Sfx::UiBack),
                Some(Transition::MainMenu { message: None }),
            );
        }
        (None, None)
    }

    /// The results screen (inside the frame's interactive `Ui`). Returns a UI sound and a
    /// screen transition (leave).
    pub(super) fn results_ui(
        &mut self,
        ui: &mut Ui,
        now: f64,
    ) -> (Option<Sfx>, Option<Transition>) {
        // Game modes: the animated tally / standings (mode_hud.rs, menus/results.rs) with the
        // buttons in one row under it once the reveal is done.
        if let Some((sound, finished)) = self.draw_mode_results(ui, now) {
            if !finished {
                return (sound, None);
            }
            let (s, t) = self.results_button_row(ui);
            return (s.or(sound), t);
        }
        let (columns, rows) = self.score_rows();
        let title = format!("MATCH OVER  //  {}", self.score_title());
        let net = self.net();
        let lobby = net.and_then(NetClient::lobby).cloned();
        let summary = lobby
            .as_ref()
            .map(|l| {
                l.settings
                    .summary(net.map(NetClient::levels).unwrap_or_default())
            })
            .unwrap_or_default();
        let has_lobby = lobby.as_ref().is_some_and(|l| l.lobby_enabled);
        let host = net.is_some_and(NetClient::is_host);
        let timeout = lobby.as_ref().and_then(|l| l.results_timeout);
        let shown = self.match_ui.results_since.map_or(0.0, |t| now - t);

        crate::menus::settings_menu::dim(ui, (shown as f32 * 2.0).min(1.0) * 0.55);
        let canvas = ui.canvas();
        let theme = ui.theme();
        let table = ScoreTable {
            title: &title,
            columns: &columns,
            rows: &rows,
            footer: (!summary.is_empty()).then_some(summary.as_str()),
        };
        let size = scoreboard::table_size(&table, canvas);
        let rect = Rect::from_min_size(
            vec2(canvas.center().x - size.x * 0.5, canvas.min.y + 34.0),
            size,
        )
        .round();
        scoreboard::draw_in(ui.painter(), theme, rect, &table);

        let mut sound = None;
        let mut transition = None;
        let mut return_to_lobby = false;
        let w = 240.0;
        ui.panel_auto(
            vec2(canvas.center().x - w * 0.5, rect.max.y + 12.0),
            w,
            "RESULTS",
            |ui| {
                ui.default_focus();
                if !has_lobby {
                    if ui.button("RESTART") {
                        return_to_lobby = true;
                    }
                    if ui.button("MAIN MENU") {
                        transition = Some(Transition::MainMenu { message: None });
                    }
                } else if host {
                    if ui.button("CONTINUE TO LOBBY") {
                        return_to_lobby = true;
                    }
                    if ui.button("LEAVE GAME") {
                        transition = Some(Transition::MainMenu { message: None });
                    }
                } else {
                    ui.label("WAITING FOR THE HOST");
                    if ui.button("LEAVE GAME") {
                        transition = Some(Transition::MainMenu { message: None });
                    }
                }
                if let Some(secs) = timeout.filter(|_| has_lobby) {
                    ui.text(
                        &format!("BACK TO THE LOBBY IN {secs} S"),
                        &TextStyle::new(ui.theme().ash),
                    );
                }
            },
        );
        if return_to_lobby {
            sound = Some(Sfx::UiSelect);
            if let Some(s) = &mut self.session {
                s.client_mut().return_to_lobby();
            }
        }
        if transition.is_some() {
            sound = Some(Sfx::UiBack);
        }
        (sound, transition)
    }
}
