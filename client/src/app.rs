//! The top-level game: implements [`engine::app::Game`], owns everything that lives longer
//! than a screen (settings, audio, UI state, the egui debug overlay) and runs the screen state
//! machine:
//!
//! ```text
//!   MainMenu ──Start(mode)──▶ InGame (connecting → loading → playing; pause/chat/scoreboard)
//!      ▲                         │
//!      └──MainMenu{message}──────┘   (leave, disconnect, connection error)
//! ```
//!
//! A lobby screen (ready-up, mode / level selection) slots in between: `Transition::Start`
//! would go to a `Screen::Lobby` that owns the `Session` and hands it to `InGame` when the
//! match starts (see docs/client.md).
//!
//! Each frame: global hotkeys (F11 fullscreen, F3 debug overlay) → build `UiInput` → the
//! screen fills the [`FrameBuffers`] and returns a [`ScreenOutput`] → settings changes are
//! applied → cursor grab follows the screen's request → audio update → debug overlay →
//! render → screen transition.

use std::path::PathBuf;
use std::sync::Mutex;

use engine::app::{EngineContext, Game};
use engine::audio::{Audio, MusicTrack, Sfx};
use engine::gfx::UiDrawList;
use engine::ui::{Id, Theme, UiState};
use winit::event::WindowEvent;

use crate::cli::{LaunchOptions, StartMode};
use crate::controls::Action;
use crate::frame::{FrameBuffers, FrameIo, ScreenOutput, Transition};
use crate::ingame::InGame;
use crate::menus::main_menu::MainMenu;
use crate::settings::Settings;
use crate::ui_input::UiInputMapper;

/// Everything the app hands to [`run`]'s `Game::init` (engine's `init` takes no arguments).
pub struct Launch {
    pub options: LaunchOptions,
    pub settings: Settings,
    pub settings_path: PathBuf,
}

static LAUNCH: Mutex<Option<Launch>> = Mutex::new(None);

/// Opens the window and runs the game.
pub fn run(launch: Launch) -> anyhow::Result<()> {
    let (w, h) = launch.options.window_size.unwrap_or((1280, 720));
    let config = engine::app::AppConfig {
        title: shared::GAME_NAME.to_string(),
        width: w,
        height: h,
        retro: launch.settings.retro.clone(),
    };
    if let Ok(mut slot) = LAUNCH.lock() {
        *slot = Some(launch);
    }
    engine::app::run::<VelocideApp>(config)
}

/// Frame-time statistics for the HUD and the debug overlay.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    /// Smoothed frames per second.
    pub fps: f32,
    /// Smoothed frame time (ms).
    pub frame_ms: f32,
    /// Worst frame time over the last second (ms).
    pub worst_ms: f32,
    worst_window: f32,
    window_time: f32,
}

impl FrameStats {
    pub fn update(&mut self, dt: f32) {
        let ms = dt * 1000.0;
        self.frame_ms = if self.frame_ms == 0.0 {
            ms
        } else {
            self.frame_ms + (ms - self.frame_ms) * 0.05
        };
        self.fps = if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        };
        self.worst_window = self.worst_window.max(ms);
        self.window_time += dt;
        if self.window_time >= 1.0 {
            self.worst_ms = self.worst_window;
            self.worst_window = 0.0;
            self.window_time = 0.0;
        }
    }
}

/// App-wide state shared by every screen.
pub struct Common {
    pub settings: Settings,
    pub settings_path: PathBuf,
    /// Set by screens when they changed settings: the app applies them (renderer, audio,
    /// fullscreen) after the screen's frame.
    pub settings_changed: bool,
    /// Set by screens when settings should be written to disk (leaving a settings page).
    pub settings_save: bool,
    pub audio: Audio,
    pub theme: Theme,
    pub ui_state: UiState,
    pub launch: LaunchOptions,
    /// `--name` for this run only (cleared when the player edits their name in a menu).
    pub name_override: Option<String>,
    pub stats: FrameStats,
    /// Debug overlay visible (F3).
    pub debug_overlay: bool,
}

impl Common {
    /// App state for a run (also used by headless tools and tests with `Audio::disabled()`).
    pub fn new(
        settings: Settings,
        settings_path: PathBuf,
        audio: Audio,
        launch: LaunchOptions,
    ) -> Self {
        Self {
            name_override: launch.name.clone(),
            debug_overlay: launch.debug_overlay,
            settings,
            settings_path,
            settings_changed: false,
            settings_save: false,
            audio,
            theme: Theme::default(),
            ui_state: UiState::new(),
            launch,
            stats: FrameStats::default(),
        }
    }

    /// Player name for new sessions.
    pub fn player_name(&self) -> String {
        self.name_override
            .clone()
            .unwrap_or_else(|| self.settings.player_name.clone())
    }

    pub fn ui_sound(&mut self, sfx: Sfx) {
        self.audio.play_2d(sfx);
    }

    /// Writes settings now (errors are logged, never fatal).
    pub fn save_settings(&mut self) {
        self.settings_save = false;
        if let Err(e) = self.settings.save(&self.settings_path) {
            log::warn!(
                "couldn't save settings to {}: {e}",
                self.settings_path.display()
            );
        } else {
            log::info!("settings saved to {}", self.settings_path.display());
        }
    }

    /// Music for a running game.
    pub fn game_music(kind: shared::level::LevelKind) -> MusicTrack {
        match kind {
            shared::level::LevelKind::Arena => MusicTrack::Arena,
            shared::level::LevelKind::Campaign => MusicTrack::Campaign,
        }
    }
}

enum Screen {
    Menu(Box<MainMenu>),
    Game(Box<InGame>),
}

pub struct VelocideApp {
    common: Common,
    screen: Screen,
    buffers: FrameBuffers,
    ui_mapper: UiInputMapper,
    last_focus: Option<Id>,
    capture_taken: bool,
    #[cfg(feature = "debug-ui")]
    debug_ui: engine::debug_ui::DebugUi,
}

impl VelocideApp {
    fn start_game(&mut self, ctx: &mut EngineContext, mode: &StartMode) {
        match InGame::start(mode, &mut self.common, &mut ctx.renderer) {
            Ok(game) => {
                self.leave_screen(ctx);
                self.screen = Screen::Game(Box::new(game));
                self.common.ui_state.reset_interaction();
            }
            Err(e) => {
                log::error!("couldn't start the game: {e:#}");
                let msg = format!("{e:#}");
                self.go_to_menu(ctx, Some(msg));
            }
        }
    }

    fn go_to_menu(&mut self, ctx: &mut EngineContext, message: Option<String>) {
        match &mut self.screen {
            Screen::Menu(menu) => menu.show_message(message),
            Screen::Game(_) => {
                self.leave_screen(ctx);
                let mut menu = MainMenu::new(&mut ctx.renderer, &self.common);
                menu.show_message(message);
                self.screen = Screen::Menu(Box::new(menu));
            }
        }
        self.common.ui_state.reset_interaction();
        self.common.audio.set_music(MusicTrack::Menu);
        self.common.audio.set_music_intensity(0.0);
    }

    /// Releases the current screen's resources (sessions are shut down cleanly).
    fn leave_screen(&mut self, ctx: &mut EngineContext) {
        let placeholder = Screen::Menu(Box::new(MainMenu::empty()));
        match std::mem::replace(&mut self.screen, placeholder) {
            Screen::Menu(menu) => menu.free(&mut ctx.renderer),
            Screen::Game(game) => game.shutdown(&mut ctx.renderer, &mut self.common),
        }
    }

    fn apply_settings(&mut self, ctx: &mut EngineContext) {
        let s = &self.common.settings;
        ctx.renderer.apply_settings(&s.retro);
        let (m, sfx, music) = s.mixer_volumes();
        self.common.audio.set_volumes(m, sfx, music);
        if s.fullscreen != ctx.is_fullscreen() {
            ctx.set_fullscreen(s.fullscreen);
        }
        if let Screen::Game(game) = &mut self.screen {
            game.apply_settings(&self.common.settings);
        }
    }

    /// Global hotkeys, unless a text field or key capture owns the keyboard.
    fn hotkeys(&mut self, ctx: &mut EngineContext) {
        if self.common.ui_state.wants_text_input() || self.common.ui_state.is_capturing() {
            return;
        }
        let b = &self.common.settings.bindings;
        if b.pressed(&ctx.input, Action::Fullscreen) {
            self.common.settings.fullscreen = !ctx.is_fullscreen();
            ctx.set_fullscreen(self.common.settings.fullscreen);
            self.common.settings_save = true;
        }
        if b.pressed(&ctx.input, Action::DebugOverlay) {
            self.common.debug_overlay = !self.common.debug_overlay;
        }
    }

    fn handle_test_timers(&mut self, ctx: &mut EngineContext) {
        if let Some(path) = &self.common.launch.capture
            && !self.capture_taken
            && ctx.time >= self.common.launch.capture_after as f64
        {
            self.capture_taken = true;
            if let Some(dir) = path.parent()
                && !dir.as_os_str().is_empty()
            {
                let _ = std::fs::create_dir_all(dir);
            }
            ctx.renderer.capture_png(path);
        }
        if let Some(q) = self.common.launch.quit_after
            && ctx.time >= q as f64
        {
            log::info!("--quit-after {q} reached");
            ctx.quit();
        }
    }
}

impl Game for VelocideApp {
    fn init(ctx: &mut EngineContext) -> anyhow::Result<Self> {
        let launch = LAUNCH
            .lock()
            .ok()
            .and_then(|mut l| l.take())
            .unwrap_or_else(|| {
                let path = Settings::default_path();
                Launch {
                    options: LaunchOptions::default(),
                    settings: Settings::load(&path),
                    settings_path: path,
                }
            });
        let audio = if launch.options.no_audio {
            Audio::disabled()
        } else {
            Audio::new()
        };
        let mut common = Common::new(launch.settings, launch.settings_path, audio, launch.options);
        if let Some(fs) = common.launch.fullscreen {
            common.settings.fullscreen = fs;
        }
        if common.settings.fullscreen {
            ctx.set_fullscreen(true);
        }
        let (m, sfx, music) = common.settings.mixer_volumes();
        common.audio.set_volumes(m, sfx, music);
        common.audio.set_music(MusicTrack::Menu);
        log::info!(
            "renderer: {} ({}x{} internal)",
            ctx.renderer.adapter_name(),
            ctx.renderer.internal_size().0,
            ctx.renderer.internal_size().1
        );

        #[cfg(feature = "debug-ui")]
        let debug_ui = engine::debug_ui::DebugUi::new(ctx);
        let menu = MainMenu::new(&mut ctx.renderer, &common);
        let start = common.launch.start.clone();
        let mut app = VelocideApp {
            common,
            screen: Screen::Menu(Box::new(menu)),
            buffers: FrameBuffers::default(),
            ui_mapper: UiInputMapper::new(),
            last_focus: None,
            capture_taken: false,
            #[cfg(feature = "debug-ui")]
            debug_ui,
        };
        if start != StartMode::Menu {
            app.start_game(ctx, &start);
        }
        Ok(app)
    }

    fn window_event(&mut self, ctx: &mut EngineContext, event: &WindowEvent) -> bool {
        #[cfg(feature = "debug-ui")]
        {
            // egui only gets input while the overlay is open and the cursor is free (menus,
            // pause): with a locked cursor every click belongs to the game.
            if self.common.debug_overlay && !ctx.cursor_grabbed() {
                return self.debug_ui.on_window_event(&ctx.window, event);
            }
        }
        #[cfg(not(feature = "debug-ui"))]
        let _ = (ctx, event);
        false
    }

    fn frame(&mut self, ctx: &mut EngineContext) {
        let dt = ctx.frame_dt;
        self.common.stats.update(dt);
        self.hotkeys(ctx);

        let window_size = ctx.renderer.window_size();
        let ui_input = self.ui_mapper.build(&ctx.input, window_size, dt, ctx.time);
        self.buffers
            .clear(UiDrawList::canvas_for_window(window_size.0, window_size.1));

        let output: ScreenOutput = {
            let mut io = FrameIo {
                renderer: &mut ctx.renderer,
                input: &ctx.input,
                ui_input: &ui_input,
                dt,
                time: ctx.time,
                window_size,
            };
            match &mut self.screen {
                Screen::Menu(menu) => menu.frame(&mut self.common, &mut io, &mut self.buffers),
                Screen::Game(game) => game.frame(&mut self.common, &mut io, &mut self.buffers),
            }
        };

        // UI navigation sound.
        let focus = self.common.ui_state.focused();
        if focus != self.last_focus && focus.is_some() && self.last_focus.is_some() {
            self.common.ui_sound(Sfx::UiMove);
        }
        self.last_focus = focus;

        if self.common.settings_changed {
            self.common.settings_changed = false;
            self.apply_settings(ctx);
        }
        if self.common.settings_save {
            self.common.save_settings();
        }
        if output.grab_cursor != ctx.cursor_grabbed() && ctx.input.focused() {
            ctx.set_cursor_grab(output.grab_cursor);
        }
        self.common.audio.update(dt);
        self.handle_test_timers(ctx);

        #[cfg(feature = "debug-ui")]
        let overlay: Option<&mut dyn engine::render::OverlayPainter> = if self.common.debug_overlay
        {
            let common = &mut self.common;
            let screen = &mut self.screen;
            let renderer = &ctx.renderer;
            self.debug_ui.run(&ctx.window, |egui_ctx| match screen {
                Screen::Game(game) => game.debug_window(egui_ctx, common, renderer),
                Screen::Menu(_) => {
                    crate::ingame::debug::menu_debug_window(egui_ctx, common, renderer)
                }
            });
            Some(&mut self.debug_ui)
        } else {
            None
        };
        #[cfg(not(feature = "debug-ui"))]
        let overlay: Option<&mut dyn engine::render::OverlayPainter> = None;

        let frame = self.buffers.desc(&output.scene);
        if let Err(e) = ctx.renderer.render(&frame, overlay) {
            log::error!("render failed: {e:#}");
        }

        match output.transition {
            None => {}
            Some(Transition::Start(mode)) => self.start_game(ctx, &mode),
            Some(Transition::MainMenu { message }) => self.go_to_menu(ctx, message),
            Some(Transition::Quit) => ctx.quit(),
        }
    }

    fn exit(&mut self, ctx: &mut EngineContext) {
        self.leave_screen(ctx);
        if self.common.settings_save {
            self.common.save_settings();
        }
        log::info!("bye");
    }
}
