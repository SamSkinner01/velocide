//! Main menu: SINGLE PLAYER (level select), HOST GAME (port, level, max players), JOIN GAME
//! (address, name), SETTINGS, QUIT. A level renders behind it; messages (disconnect reasons,
//! connection errors) appear in a modal box.
//!
//! Adding a page: add a [`Page`] variant, a menu item in `root_items`, and a `fn xxx_page`
//! drawing a panel that returns a [`Transition`] when it starts something.

use engine::audio::Sfx;
use engine::render::Renderer;
use engine::ui::{Rect, TextFilter, TextStyle, Ui, UiDrawExt};
use glam::{Vec4, vec2};
use shared::level::LevelDef;

use super::backdrop::Backdrop;
use super::settings_menu::{self, SettingsMenu};
use super::style;
use crate::app::Common;
use crate::cli::StartMode;
use crate::frame::{FrameBuffers, FrameIo, SceneParams, ScreenOutput, Transition};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Root,
    SinglePlayer,
    Host,
    Join,
    Settings,
}

const ROOT_ITEMS: [(&str, Page); 4] = [
    ("SINGLE PLAYER", Page::SinglePlayer),
    ("HOST GAME", Page::Host),
    ("JOIN GAME", Page::Join),
    ("SETTINGS", Page::Settings),
];

pub struct MainMenu {
    backdrop: Option<Backdrop>,
    page: Page,
    /// Root item to focus when coming back from a page.
    return_focus: Option<&'static str>,
    message: Option<String>,
    levels: Vec<String>,
    level: usize,
    port: String,
    max_players: f32,
    address: String,
    settings: SettingsMenu,
}

/// Available levels: `assets/levels/*.ron` plus the built-in level.
pub fn level_list() -> Vec<String> {
    let mut levels = LevelDef::list();
    if !levels.iter().any(|l| l == shared::sim::BUILTIN_LEVEL) {
        levels.push(shared::sim::BUILTIN_LEVEL.to_string());
    }
    levels
}

/// "movement_test" -> "MOVEMENT TEST".
pub fn level_display_name(name: &str) -> String {
    name.replace(['_', '-'], " ").to_uppercase()
}

impl MainMenu {
    pub fn new(renderer: &mut Renderer, common: &Common) -> Self {
        let mut menu = Self::empty();
        menu.backdrop = Backdrop::new(renderer);
        let s = &common.settings;
        menu.level = menu
            .levels
            .iter()
            .position(|l| *l == s.last_level)
            .unwrap_or(0);
        menu.port = s.host_port.to_string();
        menu.max_players = s.host_max_players as f32;
        menu.address = s.last_join_address.clone();
        menu
    }

    /// A menu without GPU resources (placeholder while switching screens).
    pub fn empty() -> Self {
        Self {
            backdrop: None,
            page: Page::Root,
            return_focus: None,
            message: None,
            levels: level_list(),
            level: 0,
            port: shared::DEFAULT_PORT.to_string(),
            max_players: 4.0,
            address: String::new(),
            settings: SettingsMenu::new(),
        }
    }

    pub fn show_message(&mut self, message: Option<String>) {
        if message.is_some() {
            self.message = message;
        }
        self.page = Page::Root;
    }

    pub fn free(self, renderer: &mut Renderer) {
        if let Some(b) = self.backdrop {
            b.free(renderer);
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    fn open(&mut self, page: Page) {
        if page == Page::SinglePlayer || page == Page::Host {
            self.levels = level_list();
            self.level = self.level.min(self.levels.len().saturating_sub(1));
        }
        self.page = page;
    }

    fn back(&mut self) {
        self.return_focus = ROOT_ITEMS
            .iter()
            .find(|(_, p)| *p == self.page)
            .map(|(l, _)| *l);
        self.page = Page::Root;
    }

    pub fn frame(
        &mut self,
        common: &mut Common,
        io: &mut FrameIo,
        out: &mut FrameBuffers,
    ) -> ScreenOutput {
        let mut scene = match &self.backdrop {
            Some(b) => b.frame(io.time, common.models.as_ref(), out),
            None => SceneParams::default(),
        };
        // Slightly darker / redder than in game so the UI pops.
        scene.screen_flash = Vec4::new(0.05, 0.0, 0.01, 0.3);

        let mut transition = None;
        let mut sound = None;
        let mut settings_resp = settings_menu::SettingsResponse::default();
        {
            let theme = &common.theme;
            let settings = &mut common.settings;
            let mut ui = Ui::new(&mut out.ui, &mut common.ui_state, io.ui_input, theme);
            let canvas = ui.canvas();

            if self.page == Page::Settings {
                settings_menu::dim(&mut ui, 0.45);
                settings_resp = self.settings.ui(
                    &mut ui,
                    settings,
                    io.input,
                    settings_menu::panel_rect(canvas),
                );
                if settings_resp.back {
                    sound = Some(Sfx::UiBack);
                    self.back();
                }
            } else {
                style::left_shade(ui.painter(), theme, canvas, 300.0);
                style::title(
                    ui.painter(),
                    theme,
                    vec2(32.0, 34.0),
                    5.0,
                    "CHROME. BLOOD. VELOCITY.",
                );
                let modal = self.message.is_some();
                let in_page = self.page != Page::Root;
                // Root column (greyed out while a page or message is open).
                ui.area(Rect::new(vec2(28.0, 138.0), vec2(290.0, 340.0)), |ui| {
                    ui.enabled_scope(!modal && !in_page, |ui| {
                        if let Some(label) = self.return_focus.take() {
                            ui.request_focus(label);
                        }
                        ui.default_focus();
                        for (label, page) in ROOT_ITEMS {
                            if ui.menu_item(label) {
                                sound = Some(Sfx::UiSelect);
                                self.open(page);
                            }
                        }
                        if ui.menu_item("QUIT") {
                            transition = Some(Transition::Quit);
                        }
                    });
                });
                // Who you are, top right.
                let name_style = TextStyle::new(theme.bone_dim).right().shadow(theme.shadow);
                ui.painter().text(
                    vec2(canvas.max.x - 12.0, 10.0),
                    &format!("PLAYING AS {}", settings.player_name.to_uppercase()),
                    &name_style,
                );

                let panel = Rect::new(
                    vec2((canvas.max.x - 316.0).max(300.0), 44.0),
                    vec2(canvas.max.x - 16.0, 316.0),
                );
                if !modal {
                    let mut name_changed = false;
                    let page_result = match self.page {
                        Page::Root | Page::Settings => None,
                        Page::SinglePlayer => self.single_player_page(&mut ui, panel, settings),
                        Page::Host => self.host_page(&mut ui, panel, settings, &mut name_changed),
                        Page::Join => self.join_page(&mut ui, panel, settings, &mut name_changed),
                    };
                    if name_changed {
                        settings_resp.name_changed = true;
                        settings_resp.changed = true;
                    }
                    match page_result {
                        Some(PageResult::Start(t)) => {
                            sound = Some(Sfx::UiSelect);
                            transition = Some(t);
                        }
                        Some(PageResult::Back) => {
                            sound = Some(Sfx::UiBack);
                            self.back();
                        }
                        Some(PageResult::Error(e)) => self.message = Some(e),
                        None => {}
                    }
                    if in_page && ui.back_pressed() {
                        sound = Some(Sfx::UiBack);
                        self.back();
                    }
                } else if let Some(msg) = self.message.clone() {
                    settings_menu::dim(&mut ui, 0.35);
                    let rect = Rect::from_center_size(canvas.center(), vec2(330.0, 0.0));
                    let mut close = false;
                    ui.panel_auto(rect.min - vec2(0.0, 50.0), 330.0, "NOTICE", |ui| {
                        ui.label_wrapped(&msg.to_uppercase());
                        ui.spacing(4.0);
                        ui.request_focus("OK");
                        close = ui.button("OK");
                    });
                    if close || ui.back_pressed() {
                        sound = Some(Sfx::UiBack);
                        self.message = None;
                        self.return_focus = Some("SINGLE PLAYER");
                    }
                }
                style::hints(
                    ui.painter(),
                    theme,
                    canvas,
                    "ARROWS/PAD SELECT   ENTER CONFIRM   ESC BACK   F11 FULLSCREEN",
                    &style::build_tag(),
                );
            }
        }
        if let Some(s) = sound {
            common.ui_sound(s);
        }
        if settings_resp.changed {
            common.settings_changed = true;
        }
        if settings_resp.name_changed {
            common.name_override = None;
        }
        if settings_resp.back {
            common.settings_save = true;
        }
        if matches!(transition, Some(Transition::Start(_))) {
            common.settings_save = true;
        }
        ScreenOutput {
            scene,
            grab_cursor: false,
            transition,
        }
    }

    fn level_name(&self) -> String {
        self.levels
            .get(self.level)
            .cloned()
            .unwrap_or_else(|| shared::sim::BUILTIN_LEVEL.to_string())
    }

    fn single_player_page(
        &mut self,
        ui: &mut Ui,
        panel: Rect,
        settings: &mut crate::settings::Settings,
    ) -> Option<PageResult> {
        let mut result = None;
        ui.panel(panel, "SINGLE PLAYER", |ui| {
            ui.default_focus();
            ui.heading("SELECT LEVEL");
            let h = ui.available_rect().height() - 40.0;
            let mut chosen = None;
            ui.scroll_area("levels", h, |ui| {
                for (i, name) in self.levels.iter().enumerate() {
                    let label = format!("{}##lvl{i}", level_display_name(name));
                    if ui.selectable(&label, i == self.level) {
                        if i == self.level {
                            chosen = Some(i); // second click / Enter on the selected row plays
                        }
                        self.level = i;
                    }
                }
            });
            ui.spacing(2.0);
            ui.row(2, |ui| {
                if ui.button("PLAY") {
                    chosen = Some(self.level);
                }
                if ui.button("BACK") {
                    result = Some(PageResult::Back);
                }
            });
            if chosen.is_some() {
                let level = self.level_name();
                settings.last_level = level.clone();
                result = Some(PageResult::Start(Transition::Start(
                    StartMode::SinglePlayer { level: Some(level) },
                )));
            }
        });
        result
    }

    fn host_page(
        &mut self,
        ui: &mut Ui,
        panel: Rect,
        settings: &mut crate::settings::Settings,
        name_changed: &mut bool,
    ) -> Option<PageResult> {
        let mut result = None;
        let names: Vec<String> = self.levels.iter().map(|l| level_display_name(l)).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        ui.panel(panel, "HOST GAME", |ui| {
            ui.default_focus();
            if ui
                .text_field(
                    "PLAYER NAME",
                    &mut settings.player_name,
                    24,
                    TextFilter::Name,
                )
                .changed
            {
                *name_changed = true;
            }
            ui.choice("LEVEL", &mut self.level, &names);
            ui.text_field("PORT", &mut self.port, 5, TextFilter::Digits);
            ui.slider("MAX PLAYERS", &mut self.max_players, 2.0, 8.0, 1.0, "{}");
            ui.spacing(2.0);
            let dim = TextStyle::new(ui.theme().bone_dim);
            ui.text("FRIENDS JOIN WITH YOUR IP AND THIS PORT (UDP).", &dim);
            ui.text("FORWARD THE PORT TO PLAY OVER THE INTERNET.", &dim);
            ui.spacing(4.0);
            ui.row(2, |ui| {
                if ui.button("HOST") {
                    result = Some(match self.port.trim().parse::<u16>() {
                        Ok(port) if port > 0 => {
                            let level = self.level_name();
                            settings.last_level = level.clone();
                            settings.host_port = port;
                            settings.host_max_players = self.max_players as u32;
                            PageResult::Start(Transition::Start(StartMode::Host {
                                port: Some(port),
                                level: Some(level),
                            }))
                        }
                        _ => PageResult::Error(format!("'{}' is not a valid port", self.port)),
                    });
                }
                if ui.button("BACK") {
                    result = Some(PageResult::Back);
                }
            });
        });
        result
    }

    fn join_page(
        &mut self,
        ui: &mut Ui,
        panel: Rect,
        settings: &mut crate::settings::Settings,
        name_changed: &mut bool,
    ) -> Option<PageResult> {
        let mut result = None;
        ui.panel(panel, "JOIN GAME", |ui| {
            ui.default_focus();
            if ui
                .text_field(
                    "PLAYER NAME",
                    &mut settings.player_name,
                    24,
                    TextFilter::Name,
                )
                .changed
            {
                *name_changed = true;
            }
            let submitted = ui
                .text_field("ADDRESS", &mut self.address, 64, TextFilter::Address)
                .submitted;
            let dim = TextStyle::new(ui.theme().bone_dim);
            ui.text(
                &format!("IP:PORT  (DEFAULT PORT {})", shared::DEFAULT_PORT),
                &dim,
            );
            ui.spacing(4.0);
            let mut connect = submitted;
            ui.row(2, |ui| {
                connect |= ui.button("CONNECT");
                if ui.button("BACK") {
                    result = Some(PageResult::Back);
                }
            });
            if connect {
                let address = self.address.trim().to_string();
                result = Some(if address.is_empty() {
                    PageResult::Error("Enter the host's address, e.g. 192.168.1.20:47420".into())
                } else {
                    settings.last_join_address = address.clone();
                    PageResult::Start(Transition::Start(StartMode::Join { address }))
                });
            }
        });
        result
    }
}

enum PageResult {
    Start(Transition),
    Back,
    Error(String),
}
