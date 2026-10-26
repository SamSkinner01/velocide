//! The lobby (docs/lobby.md): where hosted and joined games gather before a match.
//!
//! ```text
//!  MainMenu ─HOST / JOIN─▶ LobbyScreen ──a level arrives──▶ InGame ──server back in a lobby──┐
//!     ▲                       ▲  │         (warm-up hub or match)   without a warm-up world   │
//!     └────── LEAVE ──────────┘  └◀──────────────────────────── Handoff ──────────────────────┘
//! ```
//!
//! Lobby servers run the **warm-up** (`LobbyState::warmup`): the hub level arrives with the
//! first messages, so [`LobbyScreen`] only shows while connecting and hands the session to the
//! game screen at once; there the lobby is an **overlay** over the warm-up world
//! (`ingame::warmup`), built from the same [`LobbyPanel`]. A lobby without a world (an old
//! server, `--no-warmup`, a missing hub) keeps the menu-style [`LobbyScreen`], as before.
//!
//! [`LobbyPanel`] is everything the lobby shows and remembers between frames: title +
//! connection info ("HOSTING ON PORT 47420 // FRIENDS JOIN WITH ..."), the player list
//! (identity colour, look swatch, host crown, name, ping, ready), the chat, and the match panel:
//! mode, level and host settings (editable by the host, read-only for everyone else), READY,
//! START / FORCE START / CANCEL for the host, CUSTOMISE, LEAVE. The host's edits are sent to the
//! server (validated there) at most every 0.15 s. The CUSTOMISE sub-screen
//! (`menus::customize`) replaces it while open; look changes go to the server (debounced).
//! Ready also has its own key (`Action::Ready`, F1 / d-pad down), on both screens.

use std::time::Duration;

use engine::audio::{Audio, Sfx};
use engine::render::Renderer;
use engine::ui::draw::{self, fade};
use engine::ui::hud;
use engine::ui::{Rect, TextFilter, TextStyle, Ui, UiDrawExt};
use glam::{Vec4, vec2};
use server::{ServerCommand, ServerConfig};
use shared::cosmetics::Cosmetics;
use shared::lobby::{GameMode, LevelInfo, LobbyPhase, LobbyState, MatchSettings, time_limit_text};
use shared::protocol::MAX_CHAT_CHARS;
use shared::sim::PlayerId;

use super::backdrop::Backdrop;
use super::customize::{self, CustomizeMenu, CustomizeResponse, LookSync};
use super::settings_menu;
use super::style;
use crate::app::Common;
use crate::cli::StartMode;
use crate::controls::Action;
use crate::frame::{FrameBuffers, FrameIo, SceneParams, ScreenOutput, Transition};
use crate::ingame::chat::Chat;
use crate::ingame::cues;
use crate::ingame::remote_players::player_color;
use crate::ingame::scoreboard::crown;
use crate::models::ModelLibrary;
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

/// Buttons pressed this frame (executed after the UI is built, [`LobbyPanel::run`]).
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyAction {
    Ready(bool),
    Start {
        force: bool,
    },
    Cancel,
    Kick(PlayerId),
    Chat(String),
    /// Open the CUSTOMISE sub-screen.
    Customize,
    /// Overlay: back to the warm-up.
    Close,
    Leave,
}

/// How the lobby is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelMode {
    /// The menu-style lobby screen (no world).
    Screen,
    /// Over the warm-up world (the game screen's lobby overlay).
    Overlay,
}

/// What [`LobbyPanel::ui`] produced this frame.
#[derive(Debug, Default)]
pub struct PanelOutput {
    pub actions: Vec<LobbyAction>,
    pub sound: Option<Sfx>,
}

/// The lobby UI and its state: shared by the lobby screen and the warm-up overlay.
pub struct LobbyPanel {
    kind: SessionKind,
    target: String,
    /// "FRIENDS JOIN WITH 192.168.1.20:47420" (host only).
    join_hint: Option<String>,
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
    /// Countdown number on screen and when it appeared (banner animation).
    countdown: Option<(u8, f64)>,
    customize: Option<CustomizeMenu>,
    look_sync: LookSync,
    time: f64,
}

impl LobbyPanel {
    /// `in_lobby`: the session is already in the lobby (the auto-ready clock starts now).
    pub fn new(
        kind: SessionKind,
        target: String,
        join_hint: Option<String>,
        auto_ready: bool,
        in_lobby: bool,
    ) -> Self {
        Self {
            kind,
            target,
            join_hint,
            chat_draft: String::new(),
            draft: None,
            last_settings_send: f64::NEG_INFINITY,
            notice: None,
            confirm_leave: false,
            focus_stay: false,
            focus_ready: false,
            focus_chat: false,
            auto_ready,
            auto_ready_done: false,
            entered_at: in_lobby.then_some(0.0),
            countdown: None,
            customize: None,
            look_sync: LookSync::default(),
            time: 0.0,
        }
    }

    /// For a session: its kind, and the LAN join hint when it hosts.
    pub fn for_session(session: &Session, target: String, auto_ready: bool) -> Self {
        Self::new(
            session.kind(),
            target,
            session.hosted_port().map(lan::join_hint),
            auto_ready,
            session.client().in_lobby(),
        )
    }

    /// Advances the panel's clock (every frame, open or not).
    pub fn tick(&mut self, dt: f32) {
        self.time += dt as f64;
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// The countdown on screen: its number and when it appeared (panel clock).
    pub fn countdown(&self) -> Option<(u8, f64)> {
        self.countdown
    }

    /// The CUSTOMISE sub-screen is open.
    pub fn is_customizing(&self) -> bool {
        self.customize.is_some()
    }

    /// Opens the CUSTOMISE sub-screen.
    pub fn open_customize(&mut self) {
        self.customize = Some(CustomizeMenu::new());
    }

    /// Closes sub-screens and dialogs (the overlay closed, a match started).
    pub fn close_dialogs(&mut self) {
        self.customize = None;
        self.confirm_leave = false;
    }

    /// Focus READY next frame.
    pub fn focus_ready(&mut self) {
        self.focus_ready = true;
    }

    /// A server notice ("only the host can..."), shown for a few seconds.
    pub fn notice(&mut self, text: &str) {
        self.notice = Some((text.to_uppercase(), self.time));
    }

    /// `NetEvent::LobbyChanged`: entering the lobby restarts the auto-ready clock and the host
    /// draft; countdown numbers pop in with a tick.
    pub fn on_lobby_changed(
        &mut self,
        phase: LobbyPhase,
        previous: Option<LobbyPhase>,
        audio: &mut Audio,
    ) {
        if phase.level_running() {
            self.countdown = None;
            return;
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
                audio.play_2d(Sfx::UiMove);
            }
        } else if self.countdown.take().is_some() {
            self.notice("COUNTDOWN CANCELLED");
        }
    }

    /// Per frame: automatic ready-up (`--ready` / `--bot`), the host's settings edits, and our
    /// look (sent to the server when it changed).
    pub fn update(&mut self, net: &mut NetClient, look: Cosmetics) {
        self.auto_ready(net);
        self.sync_draft(net);
        self.look_sync.update(net, look, self.time);
    }

    fn auto_ready(&mut self, net: &mut NetClient) {
        if !self.auto_ready || self.auto_ready_done {
            return;
        }
        let Some(entered) = self.entered_at else {
            return;
        };
        if self.time - entered < AUTO_READY_DELAY {
            return;
        }
        if net.lobby_phase() == Some(LobbyPhase::Lobby) {
            net.set_ready(true);
            self.auto_ready_done = true;
        }
    }

    /// Keeps the host's draft in step with the server and sends local edits (rate-limited).
    fn sync_draft(&mut self, net: &mut NetClient) {
        let now = self.time;
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

    /// Toggles our ready state (the Ready key, the READY toggle).
    pub fn toggle_ready(net: &mut NetClient) {
        let ready = net
            .local_player_id()
            .and_then(|id| net.lobby()?.player(id))
            .is_some_and(|p| p.ready);
        if net.lobby_phase().is_some_and(LobbyPhase::in_lobby) {
            net.set_ready(!ready);
        }
    }

    /// Executes one action. Returns a screen transition (LEAVE).
    pub fn run(
        &mut self,
        action: LobbyAction,
        net: &mut NetClient,
        audio: &mut Audio,
    ) -> Option<Transition> {
        match action {
            LobbyAction::Ready(r) => net.set_ready(r),
            LobbyAction::Start { force } => net.start_match(force),
            LobbyAction::Cancel => net.cancel_countdown(),
            LobbyAction::Kick(id) => net.kick(id),
            LobbyAction::Chat(text) => net.send_chat(&text),
            LobbyAction::Customize => {
                self.open_customize();
                audio.play_2d(Sfx::UiSelect);
            }
            // The overlay's owner closes it.
            LobbyAction::Close => {}
            LobbyAction::Leave => {
                audio.play_2d(Sfx::UiBack);
                return Some(Transition::MainMenu { message: None });
            }
        }
        None
    }

    /// The CUSTOMISE sub-screen (call instead of [`LobbyPanel::ui`] while
    /// [`LobbyPanel::is_customizing`]). Closing it focuses READY again; the caller saves the
    /// settings when `back` is set.
    #[allow(clippy::too_many_arguments)]
    pub fn customize_ui(
        &mut self,
        ui: &mut Ui,
        look: &mut Cosmetics,
        team: Vec4,
        lib: Option<&ModelLibrary>,
        viewmodel: &mut Vec<engine::gfx::DrawItem>,
        viewmodel_fov: f32,
        dt: f32,
    ) -> CustomizeResponse {
        let Some(menu) = &mut self.customize else {
            return CustomizeResponse::default();
        };
        let resp = menu.ui(ui, look, team, lib, viewmodel, viewmodel_fov, dt);
        if resp.back {
            self.customize = None;
            self.focus_ready = true;
        }
        resp
    }

    // ---- drawing --------------------------------------------------------------------------------

    /// Draws the lobby (connecting panel until the first lobby state). `chat` is the log;
    /// `player_name` who we are.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        net: &NetClient,
        chat: &Chat,
        player_name: &str,
        mode: PanelMode,
        chat_key: bool,
    ) -> PanelOutput {
        let mut out = PanelOutput::default();
        let typing = ui.state().wants_text_input();
        let theme = ui.theme();
        let canvas = ui.canvas();
        match mode {
            PanelMode::Screen => {
                settings_menu::dim(ui, 0.3);
                style::left_shade(ui.painter(), theme, canvas, canvas.width() * 0.55);
            }
            PanelMode::Overlay => settings_menu::dim(ui, 0.5),
        }

        let Some(lobby) = net.lobby().cloned() else {
            if self.connecting_ui(ui, net) {
                out.actions.push(LobbyAction::Leave);
            }
            return out;
        };
        let levels = net.levels().to_vec();
        let local = net.local_player_id();
        let my_rtt = net.stats().rtt_ms;
        let is_host = net.is_host();
        let me_ready = local
            .and_then(|id| lobby.player(id))
            .is_some_and(|p| p.ready);

        // Header.
        let y = match mode {
            PanelMode::Screen => {
                style::title(ui.painter(), theme, vec2(16.0, 10.0), 3.0, &self.target)
            }
            PanelMode::Overlay => {
                let p = ui.painter();
                p.text(
                    vec2(16.0, 12.0),
                    "WARM-UP LOBBY",
                    &TextStyle::new(theme.bone)
                        .scale(2.0)
                        .shadow(theme.crimson_dark),
                );
                p.text(
                    vec2(16.0, 32.0),
                    &self.target,
                    &TextStyle::new(theme.bone_dim).shadow(theme.shadow),
                );
                44.0
            }
        };
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
        let actions = &mut out.actions;

        // Match panel first: it owns the default focus (READY) and comes first in key order.
        ui.enabled_scope(!modal, |ui| {
            self.match_panel(ui, right, &lobby, &levels, is_host, me_ready, mode, actions);
        });
        let players_h = 24.0 + 13.0 + lobby.settings.max_players.max(1) as f32 * 12.0 + 10.0;
        let players_rect = Rect::new(left.min, vec2(left.max.x, left.min.y + players_h));
        ui.enabled_scope(!modal, |ui| {
            Self::players_panel(ui, players_rect, &lobby, local, my_rtt, is_host, actions);
        });
        let chat_rect = Rect::new(vec2(left.min.x, players_rect.max.y + 6.0), left.max);
        if chat_key && !typing && !modal {
            self.focus_chat = true;
        }
        ui.enabled_scope(!modal, |ui| self.chat_panel(ui, chat_rect, chat, actions));

        // Countdown (the warm-up draws its own banner over the world).
        if mode == PanelMode::Screen
            && let (LobbyPhase::Countdown { secs }, Some((_, since))) =
                (lobby.phase, self.countdown)
        {
            countdown_band(
                ui.painter(),
                theme,
                canvas,
                secs,
                self.time - since,
                &lobby.settings.summary(&levels),
            );
        }

        // Leave confirmation.
        if modal {
            settings_menu::dim(ui, 0.45);
            let w = 240.0;
            let mut close = false;
            let focus_stay = std::mem::take(&mut self.focus_stay);
            let host = self.kind == SessionKind::Host;
            ui.panel_auto(
                canvas.center() - vec2(w * 0.5, 40.0),
                w,
                "LEAVE LOBBY?",
                |ui| {
                    let text = if host {
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
                            out.actions.push(LobbyAction::Leave);
                        }
                        close |= ui.button("STAY");
                    });
                },
            );
            if close || ui.back_pressed() {
                self.confirm_leave = false;
                self.focus_ready = true;
                out.sound = Some(Sfx::UiBack);
            }
        } else if ui.back_pressed() {
            if typing {
                self.focus_ready = true;
            } else if mode == PanelMode::Overlay {
                out.actions.push(LobbyAction::Close);
                out.sound = Some(Sfx::UiBack);
            } else {
                self.confirm_leave = true;
                self.focus_stay = true;
                out.sound = Some(Sfx::UiSelect);
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
        let hints = match mode {
            PanelMode::Screen => "ENTER SELECT   T CHAT   ESC LEAVE".to_string(),
            PanelMode::Overlay => {
                "ENTER SELECT   T CHAT   TAB / ESC BACK TO THE WARM-UP".to_string()
            }
        };
        style::hints(ui.painter(), theme, canvas, &hints, &style::build_tag());
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, t)| self.time - t >= NOTICE_SECS)
        {
            self.notice = None;
        }
        out
    }

    /// Mode, level and host settings; READY; START / FORCE / CANCEL; CUSTOMISE; LEAVE (and
    /// back to the warm-up on the overlay).
    #[allow(clippy::too_many_arguments)]
    fn match_panel(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        lobby: &LobbyState,
        levels: &[LevelInfo],
        is_host: bool,
        me_ready: bool,
        mode: PanelMode,
        actions: &mut Vec<LobbyAction>,
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
                actions.push(LobbyAction::Ready(ready));
            }
            let countdown = matches!(lobby.phase, LobbyPhase::Countdown { .. });
            let ready_n = lobby.ready_count();
            let total = lobby.players.len();
            if is_host && lobby.lobby_enabled {
                if countdown {
                    if ui.button("CANCEL START") {
                        actions.push(LobbyAction::Cancel);
                    }
                } else {
                    let all = lobby.all_ready();
                    ui.row(2, |ui| {
                        ui.enabled_scope(all, |ui| {
                            if ui.button("START MATCH") {
                                actions.push(LobbyAction::Start { force: false });
                            }
                        });
                        if ui.button("FORCE START") {
                            actions.push(LobbyAction::Start { force: true });
                        }
                    });
                }
            }
            player_buttons(ui, actions);
            match mode {
                PanelMode::Screen => {
                    if ui.button("LEAVE LOBBY") {
                        actions.push(LobbyAction::Leave);
                    }
                }
                PanelMode::Overlay => ui.row(2, |ui| {
                    if ui.button("WARM UP") {
                        actions.push(LobbyAction::Close);
                    }
                    if ui.button("LEAVE LOBBY") {
                        actions.push(LobbyAction::Leave);
                    }
                }),
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

    /// Player list: identity colour, look, crown, name, ping, ready; open slots; host kick
    /// buttons (mouse).
    fn players_panel(
        ui: &mut Ui,
        rect: Rect,
        lobby: &LobbyState,
        local: Option<PlayerId>,
        my_rtt: f32,
        is_host: bool,
        actions: &mut Vec<LobbyAction>,
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
                .text(vec2(r.min.x + 16.0, r.min.y), "NAME", &head);
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
                        vec2(row.min.x + 16.0, row.min.y),
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
                // Identity colour (marker shape with shape cues), then their look.
                let color = shared_color(p.color);
                cues::player_swatch(
                    ui.painter(),
                    row.min,
                    vec2(4.0, 7.0),
                    p.color as usize,
                    color,
                    fade(t.ink, 0.9),
                );
                customize::look_swatch(
                    ui.painter(),
                    row.min + vec2(8.0, 0.0),
                    7.0,
                    &p.cosmetics,
                    fade(t.ink, 0.9),
                );
                let mut x = row.min.x + 16.0;
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
                        actions.push(LobbyAction::Kick(p.id));
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

    fn chat_panel(&mut self, ui: &mut Ui, rect: Rect, chat: &Chat, actions: &mut Vec<LobbyAction>) {
        let focus = std::mem::take(&mut self.focus_chat);
        ui.panel(rect, "CHAT", |ui| {
            let h = (ui.available_rect().height() - 20.0).max(20.0);
            ui.scroll_area_sticky("log", h, |ui| {
                for line in &chat.lines {
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
                        actions.push(LobbyAction::Chat(text));
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

/// The player's own buttons in the match panel: CUSTOMISE (the arsenal's LOADOUT goes next to
/// it).
fn player_buttons(ui: &mut Ui, actions: &mut Vec<LobbyAction>) {
    if ui.button("CUSTOMISE") {
        actions.push(LobbyAction::Customize);
    }
}

/// The countdown band across the lower screen: MATCH STARTING, the number popping in, the
/// match summary. `age` = seconds since this number appeared.
pub fn countdown_band(
    p: &mut engine::gfx::UiDrawList,
    theme: &engine::ui::Theme,
    canvas: Rect,
    secs: u8,
    age: f64,
    summary: &str,
) {
    let c = vec2(canvas.center().x, canvas.max.y - 80.0);
    let band = Rect::new(
        vec2(canvas.min.x, c.y - 52.0),
        vec2(canvas.max.x, c.y + 44.0),
    );
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
        age.min(0.9),
        1.0,
        theme.crimson_bright,
    );
    p.text(
        vec2(c.x, band.max.y - 14.0),
        summary,
        &TextStyle::new(theme.bone_dim).centered(),
    );
}

/// The menu-style lobby screen: while connecting, and on servers whose lobby has no world.
pub struct LobbyScreen {
    session: Option<Session>,
    backdrop: Option<Backdrop>,
    chat: Chat,
    pending_bots: u32,
    panel: LobbyPanel,
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
        let client = ClientConfig::new(&name)
            .with_netsim(common.launch.netsim)
            .with_cosmetics(common.settings.cosmetics);
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
        let auto_ready = common.launch.ready || common.launch.bot;
        let panel = LobbyPanel::for_session(&session, target, auto_ready);
        Self {
            session: Some(session),
            backdrop: Backdrop::new(renderer),
            chat,
            pending_bots: 0,
            panel,
        }
    }

    pub fn net(&self) -> Option<&NetClient> {
        self.session.as_ref().map(Session::client)
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    pub fn session_kind(&self) -> SessionKind {
        self.panel.kind
    }

    /// The lobby state has arrived (the lobby UI is up, not the connecting panel).
    pub fn is_ready(&self) -> bool {
        self.net().and_then(NetClient::lobby).is_some()
    }

    /// The CUSTOMISE sub-screen is open (tools).
    pub fn is_customizing(&self) -> bool {
        self.panel.is_customizing()
    }

    /// Opens the CUSTOMISE sub-screen (tools).
    pub fn open_customize(&mut self) {
        self.panel.open_customize();
    }

    /// Hands the session to the game screen (frees the backdrop).
    pub fn into_handoff(mut self, renderer: &mut Renderer) -> Option<Handoff> {
        if let Some(b) = self.backdrop.take() {
            b.free(renderer);
        }
        Some(Handoff {
            session: self.session.take()?,
            target: std::mem::take(&mut self.panel.target),
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
        self.panel.tick(io.dt);
        let mut transition = None;

        // 1. Network. Stop taking events once a level arrives: the game screen gets the rest.
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

        // 2. Automation (--ready / --bot), the host's settings edits, our look; the Ready key.
        let typing = common.ui_state.wants_text_input() || common.ui_state.is_capturing();
        if let Some(s) = &mut self.session {
            let net = s.client_mut();
            self.panel.update(net, common.settings.cosmetics);
            if !typing
                && !self.panel.is_customizing()
                && common.settings.bindings.pressed(io.input, Action::Ready)
            {
                LobbyPanel::toggle_ready(net);
            }
        }

        // 3. Backdrop and UI.
        let mut scene = match &self.backdrop {
            Some(b) => b.frame(io.time, common.models.as_ref(), out),
            None => SceneParams::default(),
        };
        scene.screen_flash = Vec4::new(0.05, 0.0, 0.01, 0.35);
        let chat_key = !typing && io.input.key_pressed(engine::input::Key::KeyT);
        let player_name = common.player_name();
        let mut output = PanelOutput::default();
        if self.panel.is_customizing() {
            let team = self
                .net()
                .and_then(NetClient::local_player_id)
                .map_or(crate::models::DEFAULT_TEAM_COLOR, player_color);
            // A lamp by the preview (in front of the camera, to its right).
            let cam = scene.camera;
            out.lights.push(engine::gfx::PointLight {
                position: cam.position + cam.rotation * glam::Vec3::new(1.2, 1.0, -1.0),
                color: glam::Vec3::new(1.3, 1.15, 1.0),
                radius: 9.0,
            });
            let FrameBuffers {
                ui: list,
                viewmodel,
                ..
            } = out;
            let mut ui = Ui::new(list, &mut common.ui_state, io.ui_input, &common.theme);
            let resp = self.panel.customize_ui(
                &mut ui,
                &mut common.settings.cosmetics,
                team,
                common.models.as_ref(),
                viewmodel,
                scene.viewmodel_fov,
                io.dt,
            );
            drop(ui);
            if resp.changed {
                common.ui_sound(Sfx::UiMove);
            }
            if resp.back {
                common.settings_save = true;
                common.ui_sound(Sfx::UiBack);
            }
        } else if let Some(net) = self.session.as_ref().map(Session::client) {
            let mut ui = Ui::new(
                &mut out.ui,
                &mut common.ui_state,
                io.ui_input,
                &common.theme,
            );
            output = self.panel.ui(
                &mut ui,
                net,
                &self.chat,
                &player_name,
                PanelMode::Screen,
                chat_key,
            );
        }
        for action in output.actions {
            if let Some(s) = &mut self.session
                && let Some(t) = self.panel.run(action, s.client_mut(), &mut common.audio)
            {
                transition = Some(t);
            }
        }
        if let Some(s) = output.sound {
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
        let time = self.panel.time();
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
            // A level arrives when a match starts (or is running), and on servers with a
            // warm-up right away: hand over now, so the game screen sees the spawn that follows.
            NetEvent::LevelLoaded { .. } => return Some(Transition::EnterMatch),
            NetEvent::LobbyChanged { phase, previous } => {
                if self
                    .net()
                    .and_then(NetClient::lobby)
                    .is_some_and(LobbyState::world_running)
                {
                    return Some(Transition::EnterMatch);
                }
                self.panel
                    .on_lobby_changed(phase, previous, &mut common.audio);
            }
            NetEvent::Chat { from, name, text } => {
                match from {
                    Some(id) => self.chat.push(&name, player_color(id), &text, time),
                    None => self.chat.system(&text, t.bone_dim, time),
                }
                common.audio.play_2d(Sfx::ChatMessage);
            }
            NetEvent::Notice { text } => {
                self.panel.notice(&text);
                common.audio.play_2d(Sfx::UiBack);
            }
            NetEvent::Rejected { reason } => log::warn!("rejected: {reason}"),
            NetEvent::Disconnected { reason } => {
                let what = if self.panel.entered_at.is_some() {
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
