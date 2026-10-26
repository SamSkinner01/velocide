//! The lobby screen (docs/lobby.md): where hosted and joined games gather before a match.
//!
//! ```text
//!  MainMenu ─HOST / JOIN─▶ LobbyScreen ──match starts──▶ InGame ──server back in lobby──┐
//!     ▲                       ▲  │                                                     │
//!     └────── LEAVE ──────────┘  └◀──────────────────────────── Handoff ───────────────┘
//! ```
//!
//! The screen owns the network [`Session`] while the server is in its lobby (or counting
//! down) and hands it to the game screen ([`Handoff`]) as soon as a level arrives; the game
//! hands it back when the server returns to the lobby. Joining a match in progress goes
//! straight through to the game, as does a server without a lobby.
//!
//! Layout: title + connection info ("HOSTING ON PORT 47420 // FRIENDS JOIN WITH ..."), the
//! player list (colour, host crown, name, ping, ready), the chat, and the match panel: mode,
//! level and host settings (editable by the host, read-only for everyone else), READY,
//! START / FORCE START / CANCEL for the host, LEAVE. A countdown banner shows before the match.
//! The host's edits are sent to the server (validated there) at most every 0.15 s.

use std::time::Duration;

use engine::audio::Sfx;
use engine::render::Renderer;
use engine::ui::draw::{self, fade};
use engine::ui::hud;
use engine::ui::{Rect, TextFilter, TextStyle, Ui, UiDrawExt};
use glam::{Vec4, vec2};
use server::{ServerCommand, ServerConfig};
use shared::lobby::{GameMode, LevelInfo, LobbyPhase, LobbyState, MatchSettings, time_limit_text};
use shared::protocol::MAX_CHAT_CHARS;
use shared::sim::PlayerId;

use super::backdrop::Backdrop;
use super::settings_menu;
use super::style;
use crate::app::Common;
use crate::cli::StartMode;
use crate::frame::{FrameBuffers, FrameIo, SceneParams, ScreenOutput, Transition};
use crate::ingame::chat::Chat;
use crate::ingame::cues;
use crate::ingame::remote_players::player_color;
use crate::ingame::scoreboard::crown;
use crate::net::{
    ClientConfig, ConnectionState, JoinTarget, NetClient, NetEvent, Session, SessionKind,
    SessionSpec, lan, resolve_address,
};

/// The host's settings edits are sent at most this often (seconds).
const SETTINGS_SEND_INTERVAL: f64 = 0.15;
/// A sent edit the server didn't adopt within this long is dropped (it was refused).
const SETTINGS_CONFIRM_TIMEOUT: f64 = 1.0;
/// Notices stay on screen this long (seconds).
const NOTICE_SECS: f64 = 6.0;
/// `--bot` / `--ready` clients ready up this long after entering the lobby.
const AUTO_READY_DELAY: f64 = 1.0;

const FRAG_PRESETS: [u32; 11] = [0, 5, 10, 15, 20, 25, 30, 40, 50, 75, 100];
const TIME_PRESETS: [u32; 9] = [0, 120, 180, 300, 600, 900, 1200, 1800, 3600];

/// A running session moving between the lobby and game screens.
pub struct Handoff {
    pub session: Session,
    /// "HOSTING ON PORT 47420" / "192.168.1.20:47420".
    pub target: String,
    /// The chat log travels along.
    pub chat: Chat,
}

/// The host's pending settings edit.
struct HostDraft {
    settings: MatchSettings,
    /// Changed locally, not sent yet.
    dirty: bool,
    /// Sent at this time, waiting for the server's state to adopt it.
    sent_at: Option<f64>,
}

/// Buttons pressed this frame (executed after the UI is built).
enum Action {
    Ready(bool),
    Start { force: bool },
    Cancel,
    Kick(PlayerId),
    Chat(String),
    Leave,
}

pub struct LobbyScreen {
    session: Option<Session>,
    kind: SessionKind,
    target: String,
    /// "FRIENDS JOIN WITH 192.168.1.20:47420" (host only).
    join_hint: Option<String>,
    backdrop: Option<Backdrop>,
    chat: Chat,
    chat_draft: String,
    draft: Option<HostDraft>,
    last_settings_send: f64,
    notice: Option<(String, f64)>,
    confirm_leave: bool,
    /// The leave dialog just opened: focus STAY once.
    focus_stay: bool,
    focus_ready: bool,
    focus_chat: bool,
    auto_ready: bool,
    auto_ready_done: bool,
    /// When we (re-)entered the lobby (first lobby state / return from a match).
    entered_at: Option<f64>,
    pending_bots: u32,
    /// Countdown number on screen and when it appeared (banner animation).
    countdown: Option<(u8, f64)>,
    time: f64,
}

impl LobbyScreen {
    /// Hosts (listen server in the lobby) or joins a server. Errors (port in use, bad
    /// address) are returned for the main menu to show.
    pub fn start(
        mode: &StartMode,
        common: &mut Common,
        renderer: &mut Renderer,
    ) -> anyhow::Result<LobbyScreen> {
        let name = common.player_name();
        let client = ClientConfig::new(&name).with_netsim(common.launch.netsim);
        let s = &common.settings;
        let (spec, target) = match mode {
            StartMode::Host { port, level } => {
                let port = port.unwrap_or(s.host_port);
                let level = level.clone().unwrap_or_else(|| s.last_level.clone());
                let max = common
                    .launch
                    .max_players
                    .unwrap_or(s.host_max_players)
                    .max(1 + common.launch.add_bots)
                    .clamp(1, shared::lobby::MAX_PLAYERS as u32);
                let mut server = ServerConfig::host(port, &level, max as usize);
                server.lobby.autostart = common.launch.autostart;
                server.lobby.settings.mode = common.launch.mode.unwrap_or(s.host_mode);
                server.lobby.settings.difficulty = common.launch.difficulty.unwrap_or(s.difficulty);
                (
                    SessionSpec::Host {
                        server: Box::new(server),
                    },
                    format!("HOSTING ON PORT {port}"),
                )
            }
            StartMode::Join { address } => {
                let addr = resolve_address(address)
                    .map_err(|e| anyhow::anyhow!("can't join '{address}': {e}"))?;
                (
                    SessionSpec::Join {
                        target: JoinTarget::Address(addr),
                    },
                    addr.to_string(),
                )
            }
            StartMode::Menu | StartMode::SinglePlayer { .. } => {
                anyhow::bail!("single-player has no lobby")
            }
        };
        let session = Session::start(spec, client)?;
        // Port 0 binds an ephemeral port: show the real one.
        let target = match session.hosted_port() {
            Some(port) => format!("HOSTING ON PORT {port}"),
            None => target,
        };
        log::info!("session started: {target} as '{name}'");
        let mut lobby = Self::from_handoff(
            Handoff {
                session,
                target,
                chat: Chat::new(),
            },
            common,
            renderer,
        );
        lobby.pending_bots = common.launch.add_bots;
        Ok(lobby)
    }

    /// The lobby around a running session (new, or back from a match).
    pub fn from_handoff(handoff: Handoff, common: &Common, renderer: &mut Renderer) -> Self {
        let Handoff {
            session,
            target,
            chat,
        } = handoff;
        let join_hint = session.hosted_port().map(lan::join_hint);
        let entered_at = session.client().in_lobby().then_some(0.0);
        Self {
            kind: session.kind(),
            session: Some(session),
            target,
            join_hint,
            backdrop: Backdrop::new(renderer),
            chat,
            chat_draft: String::new(),
            draft: None,
            last_settings_send: f64::NEG_INFINITY,
            notice: None,
            confirm_leave: false,
            focus_stay: false,
            focus_ready: false,
            focus_chat: false,
            auto_ready: common.launch.ready || common.launch.bot,
            auto_ready_done: false,
            entered_at,
            pending_bots: 0,
            countdown: None,
            time: 0.0,
        }
    }

    pub fn net(&self) -> Option<&NetClient> {
        self.session.as_ref().map(Session::client)
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn session_kind(&self) -> SessionKind {
        self.kind
    }

    /// The lobby state has arrived (the lobby UI is up, not the connecting panel).
    pub fn is_ready(&self) -> bool {
        self.net().and_then(NetClient::lobby).is_some()
    }

    /// Hands the session to the game screen (frees the backdrop).
    pub fn into_handoff(mut self, renderer: &mut Renderer) -> Option<Handoff> {
        if let Some(b) = self.backdrop.take() {
            b.free(renderer);
        }
        Some(Handoff {
            session: self.session.take()?,
            target: std::mem::take(&mut self.target),
            chat: std::mem::take(&mut self.chat),
        })
    }

    /// Leaves: disconnects (stopping our listen server) and frees GPU resources.
    pub fn shutdown(mut self, renderer: &mut Renderer) {
        if let Some(b) = self.backdrop.take() {
            b.free(renderer);
        }
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
    }

    // ---- frame ----------------------------------------------------------------------------------

    pub fn frame(
        &mut self,
        common: &mut Common,
        io: &mut FrameIo,
        out: &mut FrameBuffers,
    ) -> ScreenOutput {
        self.time += io.dt as f64;
        let mut transition = None;

        // 1. Network. Stop taking events once the match starts: the game screen gets the rest.
        if let Some(s) = &mut self.session {
            s.client_mut().update(Duration::from_secs_f32(io.dt));
        }
        while transition.is_none()
            && let Some(ev) = self
                .session
                .as_mut()
                .and_then(|s| s.client_mut().poll_event())
        {
            transition = self.on_event(ev, common);
        }
        // A server without a lobby (or an old one) just puts us in the game.
        if transition.is_none()
            && self
                .net()
                .is_some_and(|n| n.lobby().is_none() && n.is_in_game())
        {
            transition = Some(Transition::EnterMatch);
        }

        // 2. Automation (--ready / --bot) and the host's settings edits.
        self.auto_ready();
        self.sync_draft();

        // 3. Backdrop and UI.
        let mut scene = match &self.backdrop {
            Some(b) => b.frame(io.time, common.models.as_ref(), out),
            None => SceneParams::default(),
        };
        scene.screen_flash = Vec4::new(0.05, 0.0, 0.01, 0.35);
        let mut actions = Vec::new();
        let mut sound = None;
        self.draw(common, io, out, &mut actions, &mut sound);
        for action in actions {
            if let Some(t) = self.run(action, common) {
                transition = Some(t);
            }
        }
        if let Some(s) = sound {
            common.ui_sound(s);
        }
        ScreenOutput {
            scene,
            grab_cursor: false,
            transition,
        }
    }

    fn on_event(&mut self, ev: NetEvent, common: &mut Common) -> Option<Transition> {
        let t = &common.theme;
        match ev {
            NetEvent::Connected { player_id } => {
                log::info!("connected as {player_id}");
                for i in 0..std::mem::take(&mut self.pending_bots) {
                    if let Some(s) = &self.session {
                        s.command(ServerCommand::AddBot {
                            name: format!("BOT {}", i + 1),
                        });
                    }
                }
            }
            // A level arrives only when a match starts (or is running): hand over now, so the
            // game screen sees the spawn that follows.
            NetEvent::LevelLoaded { .. } => return Some(Transition::EnterMatch),
            NetEvent::LobbyChanged { phase, previous } => {
                if phase.level_running() {
                    return Some(Transition::EnterMatch);
                }
                if previous.is_none_or(LobbyPhase::level_running) {
                    // Entered the lobby (first state, or back from a match).
                    self.entered_at = Some(self.time);
                    self.auto_ready_done = false;
                    self.draft = None;
                }
                if let LobbyPhase::Countdown { secs } = phase {
                    if self.countdown.is_none_or(|(s, _)| s != secs) {
                        self.countdown = Some((secs, self.time));
                        common.audio.play_2d(Sfx::UiMove);
                    }
                } else if self.countdown.take().is_some() {
                    self.notice = Some(("COUNTDOWN CANCELLED".into(), self.time));
                }
            }
            NetEvent::Chat { from, name, text } => {
                match from {
                    Some(id) => self.chat.push(&name, player_color(id), &text, self.time),
                    None => self.chat.system(&text, t.bone_dim, self.time),
                }
                common.audio.play_2d(Sfx::ChatMessage);
            }
            NetEvent::Notice { text } => {
                self.notice = Some((text.to_uppercase(), self.time));
                common.audio.play_2d(Sfx::UiBack);
            }
            NetEvent::Rejected { reason } => log::warn!("rejected: {reason}"),
            NetEvent::Disconnected { reason } => {
                let what = if self.entered_at.is_some() {
                    "Disconnected"
                } else {
                    "Couldn't connect"
                };
                return Some(Transition::MainMenu {
                    message: Some(format!("{what}: {reason}")),
                });
            }
            NetEvent::TuningChanged
            | NetEvent::Spawned { .. }
            | NetEvent::PlayerJoined(_)
            | NetEvent::PlayerLeft { .. }
            | NetEvent::PlayerUpdated(_)
            | NetEvent::Game(_) => {}
        }
        None
    }

    fn auto_ready(&mut self) {
        if !self.auto_ready || self.auto_ready_done {
            return;
        }
        let Some(entered) = self.entered_at else {
            return;
        };
        if self.time - entered < AUTO_READY_DELAY {
            return;
        }
        if let Some(s) = &mut self.session
            && s.client().lobby_phase() == Some(LobbyPhase::Lobby)
        {
            s.client_mut().set_ready(true);
            self.auto_ready_done = true;
        }
    }

    /// Keeps the host's draft in step with the server and sends local edits (rate-limited).
    fn sync_draft(&mut self) {
        let now = self.time;
        let Some(session) = &mut self.session else {
            return;
        };
        let net = session.client_mut();
        let server_settings = match net.lobby() {
            Some(l) if net.is_host() && l.phase == LobbyPhase::Lobby && l.lobby_enabled => {
                l.settings.clone()
            }
            _ => {
                self.draft = None;
                return;
            }
        };
        let draft = self.draft.get_or_insert_with(|| HostDraft {
            settings: server_settings.clone(),
            dirty: false,
            sent_at: None,
        });
        if draft.dirty {
            if now - self.last_settings_send >= SETTINGS_SEND_INTERVAL {
                net.set_match_settings(draft.settings.clone());
                draft.dirty = false;
                draft.sent_at = Some(now);
                self.last_settings_send = now;
            }
        } else if let Some(sent) = draft.sent_at {
            if draft.settings == server_settings || now - sent > SETTINGS_CONFIRM_TIMEOUT {
                draft.sent_at = None;
                draft.settings = server_settings;
            }
        } else {
            draft.settings = server_settings;
        }
    }

    fn run(&mut self, action: Action, common: &mut Common) -> Option<Transition> {
        let session = self.session.as_mut()?;
        let net = session.client_mut();
        match action {
            Action::Ready(r) => net.set_ready(r),
            Action::Start { force } => net.start_match(force),
            Action::Cancel => net.cancel_countdown(),
            Action::Kick(id) => net.kick(id),
            Action::Chat(text) => net.send_chat(&text),
            Action::Leave => {
                common.audio.play_2d(Sfx::UiBack);
                return Some(Transition::MainMenu { message: None });
            }
        }
        None
    }

    // ---- drawing --------------------------------------------------------------------------------

    fn draw(
        &mut self,
        common: &mut Common,
        io: &FrameIo,
        out: &mut FrameBuffers,
        actions: &mut Vec<Action>,
        sound: &mut Option<Sfx>,
    ) {
        let typing = common.ui_state.wants_text_input();
        let chat_key = !typing
            && !common.ui_state.is_capturing()
            && io.input.key_pressed(engine::input::Key::KeyT);
        let theme = &common.theme;
        let player_name = common.player_name();
        let mut ui = Ui::new(&mut out.ui, &mut common.ui_state, io.ui_input, theme);
        let canvas = ui.canvas();
        settings_menu::dim(&mut ui, 0.3);
        style::left_shade(ui.painter(), theme, canvas, canvas.width() * 0.55);

        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        let Some(lobby) = net.lobby().cloned() else {
            if self.connecting_ui(&mut ui, net) {
                actions.push(Action::Leave);
            }
            return;
        };
        let levels = net.levels().to_vec();
        let local = net.local_player_id();
        let my_rtt = net.stats().rtt_ms;
        let is_host = net.is_host();
        let me_ready = local
            .and_then(|id| lobby.player(id))
            .is_some_and(|p| p.ready);

        // Header.
        let y = style::title(ui.painter(), theme, vec2(16.0, 10.0), 3.0, &self.target);
        let hint = self
            .join_hint
            .as_deref()
            .map(|h| format!("{h}  (UDP)"))
            .unwrap_or_else(|| {
                if is_host {
                    "YOU ARE THE HOST".into()
                } else {
                    String::new()
                }
            });
        ui.painter().text(
            vec2(canvas.max.x - 12.0, 44.0),
            &hint,
            &TextStyle::new(theme.bone).right().shadow(theme.shadow),
        );
        ui.painter().text(
            vec2(canvas.max.x - 12.0, 10.0),
            &format!("PLAYING AS {}", player_name.to_uppercase()),
            &TextStyle::new(theme.bone_dim).right().shadow(theme.shadow),
        );
        ui.painter().text(
            vec2(canvas.max.x - 12.0, 22.0),
            "LOBBY",
            &TextStyle::new(theme.crimson_bright)
                .scale(2.0)
                .right()
                .shadow(theme.shadow),
        );

        let top = (y + 6.0).max(64.0);
        let gap = 12.0;
        let col_w = ((canvas.width() - 32.0 - gap) * 0.5).floor();
        let left = Rect::new(vec2(16.0, top), vec2(16.0 + col_w, canvas.max.y - 26.0));
        let right = Rect::new(
            vec2(left.max.x + gap, top),
            vec2(canvas.max.x - 16.0, canvas.max.y - 26.0),
        );
        let modal = self.confirm_leave;

        // Match panel first: it owns the default focus (READY) and comes first in key order.
        ui.enabled_scope(!modal, |ui| {
            self.match_panel(ui, right, &lobby, &levels, is_host, me_ready, actions);
        });
        let players_h = 24.0 + 13.0 + lobby.settings.max_players.max(1) as f32 * 12.0 + 10.0;
        let players_rect = Rect::new(left.min, vec2(left.max.x, left.min.y + players_h));
        ui.enabled_scope(!modal, |ui| {
            Self::players_panel(ui, players_rect, &lobby, local, my_rtt, is_host, actions);
        });
        let chat_rect = Rect::new(vec2(left.min.x, players_rect.max.y + 6.0), left.max);
        if chat_key && !modal {
            self.focus_chat = true;
        }
        ui.enabled_scope(!modal, |ui| self.chat_panel(ui, chat_rect, actions));

        // Countdown: a band across the lower screen (clear of the host's CANCEL button) with
        // the number popping in.
        if let (LobbyPhase::Countdown { secs }, Some((_, since))) = (lobby.phase, self.countdown) {
            let c = vec2(canvas.center().x, canvas.max.y - 80.0);
            let band = Rect::new(
                vec2(canvas.min.x, c.y - 52.0),
                vec2(canvas.max.x, c.y + 44.0),
            );
            let p = ui.painter();
            draw::fill(p, band, fade(theme.ink, 0.82));
            draw::hline(p, band.min.x, band.max.x, band.min.y, theme.crimson);
            draw::hline(p, band.min.x, band.max.x, band.max.y - 1.0, theme.crimson);
            p.text(
                vec2(c.x, band.min.y + 8.0),
                "MATCH STARTING",
                &TextStyle::new(theme.bone)
                    .scale(2.0)
                    .centered()
                    .shadow(theme.crimson_dark),
            );
            hud::banner(
                p,
                theme,
                c + vec2(0.0, 4.0),
                &format!("{secs}"),
                (self.time - since).min(0.9),
                1.0,
                theme.crimson_bright,
            );
            p.text(
                vec2(c.x, band.max.y - 14.0),
                &lobby.settings.summary(&levels),
                &TextStyle::new(theme.bone_dim).centered(),
            );
        }

        // Leave confirmation.
        if modal {
            settings_menu::dim(&mut ui, 0.45);
            let w = 240.0;
            let mut close = false;
            let focus_stay = std::mem::take(&mut self.focus_stay);
            ui.panel_auto(
                canvas.center() - vec2(w * 0.5, 40.0),
                w,
                "LEAVE LOBBY?",
                |ui| {
                    let text = if self.kind == SessionKind::Host {
                        "YOU ARE HOSTING: EVERYONE WILL BE DISCONNECTED."
                    } else {
                        "YOU CAN REJOIN WHILE THE SERVER RUNS."
                    };
                    ui.label_wrapped(text);
                    ui.spacing(2.0);
                    ui.row(2, |ui| {
                        if focus_stay {
                            ui.request_focus("STAY");
                        }
                        if ui.button("LEAVE") {
                            actions.push(Action::Leave);
                        }
                        close |= ui.button("STAY");
                    });
                },
            );
            if close || ui.back_pressed() {
                self.confirm_leave = false;
                self.focus_ready = true;
                *sound = Some(Sfx::UiBack);
            }
        } else if ui.back_pressed() {
            if typing {
                self.focus_ready = true;
            } else {
                self.confirm_leave = true;
                self.focus_stay = true;
                *sound = Some(Sfx::UiSelect);
            }
        }

        // Notice and hints.
        if let Some((text, since)) = &self.notice {
            let age = self.time - since;
            if age < NOTICE_SECS {
                let a = ((NOTICE_SECS - age) / 0.5).clamp(0.0, 1.0) as f32;
                let style =
                    TextStyle::new(fade(theme.crimson_bright, a)).shadow(fade(theme.ink, a));
                ui.painter()
                    .text(vec2(16.0, canvas.max.y - 22.0), text, &style);
            }
        }
        style::hints(
            ui.painter(),
            theme,
            canvas,
            "ENTER SELECT   T CHAT   ESC LEAVE",
            &style::build_tag(),
        );
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, t)| self.time - t >= NOTICE_SECS)
        {
            self.notice = None;
        }
    }

    /// Mode, level and host settings; READY; START / FORCE / CANCEL; LEAVE.
    #[allow(clippy::too_many_arguments)]
    fn match_panel(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        lobby: &LobbyState,
        levels: &[LevelInfo],
        is_host: bool,
        me_ready: bool,
        actions: &mut Vec<Action>,
    ) {
        let focus_ready = std::mem::take(&mut self.focus_ready);
        let draft = self.draft.as_mut().filter(|_| is_host);
        let title = if is_host { "MATCH // HOST" } else { "MATCH" };
        ui.panel(rect, title, |ui| {
            let t = ui.theme();
            match draft {
                Some(d) => {
                    if host_settings(ui, &mut d.settings, levels) {
                        d.dirty = true;
                    }
                }
                None => {
                    let locked = is_host && !matches!(lobby.phase, LobbyPhase::Lobby);
                    ui.enabled_scope(!locked, |ui| settings_rows(ui, &lobby.settings, levels));
                }
            }
            ui.spacing(2.0);
            ui.separator();
            if focus_ready {
                ui.request_focus("READY");
            }
            ui.default_focus();
            let mut ready = me_ready;
            if ui.toggle("READY", &mut ready) {
                actions.push(Action::Ready(ready));
            }
            let countdown = matches!(lobby.phase, LobbyPhase::Countdown { .. });
            let ready_n = lobby.ready_count();
            let total = lobby.players.len();
            if is_host && lobby.lobby_enabled {
                if countdown {
                    if ui.button("CANCEL START") {
                        actions.push(Action::Cancel);
                    }
                } else {
                    let all = lobby.all_ready();
                    ui.row(2, |ui| {
                        ui.enabled_scope(all, |ui| {
                            if ui.button("START MATCH") {
                                actions.push(Action::Start { force: false });
                            }
                        });
                        if ui.button("FORCE START") {
                            actions.push(Action::Start { force: true });
                        }
                    });
                }
            }
            if ui.button("LEAVE LOBBY") {
                actions.push(Action::Leave);
            }
            ui.spacing(2.0);
            let status = if let LobbyPhase::Countdown { secs } = lobby.phase {
                format!("MATCH STARTS IN {secs}")
            } else if lobby.autostart {
                format!("AUTOSTART WHEN ALL ARE READY  ({ready_n}/{total})")
            } else if lobby.all_ready() {
                if is_host {
                    "EVERYONE IS READY".to_string()
                } else {
                    "WAITING FOR THE HOST TO START".to_string()
                }
            } else {
                format!("WAITING FOR PLAYERS  ({ready_n}/{total} READY)")
            };
            let color = if lobby.all_ready() || countdown {
                t.bone
            } else {
                t.bone_dim
            };
            ui.text(&status, &TextStyle::new(color));
        });
    }

    /// Player list: colour, crown, name, ping, ready; open slots; host kick buttons (mouse).
    fn players_panel(
        ui: &mut Ui,
        rect: Rect,
        lobby: &LobbyState,
        local: Option<PlayerId>,
        my_rtt: f32,
        is_host: bool,
        actions: &mut Vec<Action>,
    ) {
        let title = format!(
            "PLAYERS  {}/{}",
            lobby.players.len(),
            lobby.settings.max_players
        );
        ui.panel(rect, &title, |ui| {
            let t = ui.theme();
            let head = TextStyle::new(t.ash);
            let r = ui.allocate(9.0);
            let x_status = r.max.x - 2.0;
            let x_ping = x_status - 56.0 - if is_host { 12.0 } else { 0.0 };
            let x_status_head = if is_host { x_status - 12.0 } else { x_status };
            ui.painter()
                .text(vec2(r.min.x + 8.0, r.min.y), "NAME", &head);
            ui.painter()
                .text(vec2(x_ping, r.min.y), "PING", &head.right());
            ui.painter()
                .text(vec2(x_status_head, r.min.y), "STATUS", &head.right());
            draw::hline(ui.painter(), r.min.x, r.max.x, r.max.y + 1.0, t.panel_edge);
            ui.spacing(1.0);
            let slots = (lobby.settings.max_players as usize).max(lobby.players.len());
            for i in 0..slots {
                let row = ui.allocate(9.0);
                let Some(p) = lobby.players.get(i) else {
                    ui.painter().text(
                        vec2(row.min.x + 8.0, row.min.y),
                        "- OPEN -",
                        &TextStyle::new(fade(t.ash, 0.6)),
                    );
                    continue;
                };
                let is_local = Some(p.id) == local;
                if is_local {
                    draw::fill(
                        ui.painter(),
                        Rect::new(row.min - vec2(3.0, 2.0), row.max + vec2(3.0, 1.0)),
                        fade(t.crimson, 0.35),
                    );
                }
                let color = shared_color(p.color);
                cues::player_swatch(
                    ui.painter(),
                    row.min,
                    vec2(4.0, 7.0),
                    p.color as usize,
                    color,
                    fade(t.ink, 0.9),
                );
                let mut x = row.min.x + 8.0;
                if p.is_host {
                    crown(ui.painter(), vec2(x, row.min.y), t.rank_color(1));
                    x += 12.0;
                }
                let name = engine::ui::text::truncate(&p.name, x_ping - 40.0 - x, 1.0);
                ui.painter()
                    .text(vec2(x, row.min.y), &name, &TextStyle::new(t.bone));
                let ping = if is_local { my_rtt } else { p.ping_ms as f32 };
                let (ping_text, ping_color) = if p.bot {
                    ("-".to_string(), t.ash)
                } else {
                    (format!("{}", ping.round() as i32), hud::ping_color(t, ping))
                };
                ui.painter().text(
                    vec2(x_ping, row.min.y),
                    &ping_text,
                    &TextStyle::new(ping_color).right(),
                );
                let (status, status_color) = if p.bot {
                    ("BOT", t.bone_dim)
                } else if p.ready {
                    ("READY", t.stamina)
                } else {
                    ("...", t.ash)
                };
                // The host gets a kick column: a small mouse-only X per other player.
                let x_st = if is_host { x_status - 12.0 } else { x_status };
                if is_host && !p.is_host {
                    let kick = Rect::new(
                        vec2(x_status - 8.0, row.min.y - 1.0),
                        vec2(x_status + 1.0, row.max.y + 1.0),
                    );
                    let id = ui.id(&format!("kick##{}", p.id.0));
                    let resp = ui.interact(id, kick, false);
                    let c = if resp.hovered {
                        t.crimson_bright
                    } else {
                        fade(t.ash, 0.7)
                    };
                    ui.painter()
                        .text(vec2(kick.min.x + 2.0, row.min.y), "X", &TextStyle::new(c));
                    if resp.clicked {
                        actions.push(Action::Kick(p.id));
                    }
                }
                ui.painter().text(
                    vec2(x_st, row.min.y),
                    status,
                    &TextStyle::new(status_color).right(),
                );
            }
        });
    }

    fn chat_panel(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let focus = std::mem::take(&mut self.focus_chat);
        ui.panel(rect, "CHAT", |ui| {
            let h = (ui.available_rect().height() - 20.0).max(20.0);
            ui.scroll_area_sticky("log", h, |ui| {
                for line in &self.chat.lines {
                    ui.chat_line(&line.name, line.name_color, &line.text);
                }
            });
            ui.row_weights(&[3.0, 1.0], |ui| {
                if focus {
                    ui.request_focus("##lobbychat");
                }
                let r = ui.text_field(
                    "##lobbychat",
                    &mut self.chat_draft,
                    MAX_CHAT_CHARS,
                    TextFilter::Any,
                );
                let send = ui.button("SEND");
                if r.submitted || send {
                    let text = self.chat_draft.trim().to_string();
                    if !text.is_empty() {
                        actions.push(Action::Chat(text));
                    }
                    self.chat_draft.clear();
                }
            });
        });
    }

    /// Before the lobby state arrives: connecting / entering panel with CANCEL.
    fn connecting_ui(&self, ui: &mut Ui, net: &NetClient) -> bool {
        let state = match net.state() {
            ConnectionState::Connecting => "CONNECTING",
            ConnectionState::Loading | ConnectionState::InGame => "ENTERING LOBBY",
            ConnectionState::Disconnected => "DISCONNECTED",
        };
        let dots = ".".repeat(((self.time * 3.0) as usize) % 4);
        let canvas = ui.canvas();
        let title = if self.kind == SessionKind::Host {
            "HOSTING"
        } else {
            "JOINING"
        };
        let mut cancel = false;
        let time = self.time;
        ui.panel_auto(canvas.center() - vec2(140.0, 50.0), 280.0, title, |ui| {
            ui.default_focus();
            ui.label(&self.target);
            ui.label_colored(&format!("{state}{dots}"), ui.theme().bone);
            ui.text(&format!("{time:.1} S"), &TextStyle::new(ui.theme().ash));
            ui.spacing(4.0);
            cancel = ui.button("CANCEL");
        });
        cancel || ui.back_pressed()
    }
}

/// Player colour from the lobby's colour index.
fn shared_color(index: u8) -> Vec4 {
    player_color(PlayerId(index))
}

/// Read-only match settings (guests, and the host during the countdown).
fn settings_rows(ui: &mut Ui, s: &MatchSettings, levels: &[LevelInfo]) {
    ui.value_row("MODE", s.mode.title());
    let level = levels.iter().find(|l| l.name == s.level).map_or_else(
        || shared::lobby::level_display_name(&s.level),
        LevelInfo::display_title,
    );
    ui.value_row("LEVEL", &level);
    ui.value_row("MAX PLAYERS", &s.max_players.to_string());
    if s.mode.has_score_limits() {
        ui.value_row("FRAG LIMIT", &frag_text(s.frag_limit));
        ui.value_row("TIME LIMIT", &time_text(s.time_limit_secs));
    } else if s.mode.has_time_limit() {
        ui.value_row("TIME LIMIT", &time_text(s.time_limit_secs));
    }
    if s.mode.has_friendly_fire() {
        ui.value_row("FRIENDLY FIRE", if s.friendly_fire { "ON" } else { "OFF" });
    }
    if s.mode.has_difficulty() {
        ui.value_row("DIFFICULTY", s.difficulty.title());
    }
}

/// The host's editable settings. Returns true if anything changed.
fn host_settings(ui: &mut Ui, s: &mut MatchSettings, levels: &[LevelInfo]) -> bool {
    let mut changed = false;
    let mut mode = GameMode::ALL.iter().position(|m| *m == s.mode).unwrap_or(0);
    let names: Vec<&str> = GameMode::ALL.iter().map(|m| m.title()).collect();
    if ui.choice("MODE", &mut mode, &names) {
        s.mode = GameMode::ALL[mode];
        s.fit_to(levels);
        changed = true;
    }
    let options = levels_for(s.mode, levels);
    let titles: Vec<String> = options
        .iter()
        .map(|l| {
            if l.any_mode() {
                format!("{} (TEST)", l.display_title())
            } else {
                l.display_title()
            }
        })
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    let mut level = options.iter().position(|l| l.name == s.level).unwrap_or(0);
    if !titles.is_empty() && ui.choice("LEVEL", &mut level, &titles) {
        s.level = options[level].name.clone();
        changed = true;
    }
    let mut max = s.max_players as f32;
    if ui.slider(
        "MAX PLAYERS",
        &mut max,
        1.0,
        s.mode.max_players() as f32,
        1.0,
        "{}",
    ) {
        s.max_players = max.round() as u8;
        changed = true;
    }
    if s.mode.has_score_limits() {
        changed |= preset_choice(
            ui,
            "FRAG LIMIT",
            &mut s.frag_limit,
            &FRAG_PRESETS,
            frag_text,
        );
        changed |= preset_choice(
            ui,
            "TIME LIMIT",
            &mut s.time_limit_secs,
            &TIME_PRESETS,
            time_text,
        );
    } else if s.mode.has_time_limit() {
        // Gun game: no frag limit (the ladder ends it), the time limit ranks by tier.
        changed |= preset_choice(
            ui,
            "TIME LIMIT",
            &mut s.time_limit_secs,
            &TIME_PRESETS,
            time_text,
        );
    }
    if s.mode.has_friendly_fire() {
        changed |= ui.toggle("FRIENDLY FIRE", &mut s.friendly_fire);
    }
    if s.mode.has_difficulty() {
        use shared::lobby::Difficulty;
        let names = Difficulty::ALL.map(Difficulty::title);
        let mut index = s.difficulty.index();
        if ui.choice("DIFFICULTY", &mut index, &names) {
            s.difficulty = Difficulty::ALL[index.min(Difficulty::ALL.len() - 1)];
            changed = true;
        }
    }
    changed
}

/// Levels playable in `mode`: its own kind first, then sandbox levels.
fn levels_for(mode: GameMode, levels: &[LevelInfo]) -> Vec<&LevelInfo> {
    let mut out: Vec<&LevelInfo> = levels
        .iter()
        .filter(|l| mode.accepts(l) && !l.any_mode())
        .collect();
    out.extend(levels.iter().filter(|l| l.any_mode()));
    out
}

/// A choice between preset values (plus the current value if it isn't one of them).
fn preset_choice(
    ui: &mut Ui,
    label: &str,
    value: &mut u32,
    presets: &[u32],
    text: fn(u32) -> String,
) -> bool {
    let mut options = presets.to_vec();
    if !options.contains(value) {
        options.push(*value);
        options.sort_unstable();
    }
    let names: Vec<String> = options.iter().map(|v| text(*v)).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut index = options.iter().position(|v| v == value).unwrap_or(0);
    if ui.choice(label, &mut index, &names) {
        *value = options[index];
        return true;
    }
    false
}

fn frag_text(frags: u32) -> String {
    if frags == 0 {
        "NONE".into()
    } else {
        frags.to_string()
    }
}

fn time_text(secs: u32) -> String {
    if secs == 0 {
        "NONE".into()
    } else {
        time_limit_text(secs)
    }
}
