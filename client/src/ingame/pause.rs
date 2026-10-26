//! Pause menu (Esc / Start): RESUME, RESTART (single-player), RETURN TO LOBBY (multiplayer
//! host), LOADOUT (`menus::loadout`; changes apply at the next respawn), SETTINGS, LEAVE. In multiplayer the game keeps running behind it; in single-player
//! the in-process server is paused (`ServerCommand::SetPaused`).

use engine::audio::Sfx;
use engine::input::InputState;
use engine::ui::{Rect, TextStyle, Ui};
use glam::vec2;

use crate::menus::loadout::{self, LoadoutMenu};
use crate::menus::settings_menu::{self, SettingsMenu, SettingsResponse};
use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseAction {
    None,
    Resume,
    Restart,
    /// Host: end the match and bring everyone back to the lobby.
    ReturnToLobby,
    Leave,
}

#[derive(Default)]
pub struct PauseMenu {
    open: bool,
    settings: Option<SettingsMenu>,
    loadout: Option<LoadoutMenu>,
    /// Opened this frame (the Escape that opened it must not close it again).
    fresh: bool,
}

/// What the pause menu did this frame.
pub struct PauseOutput {
    pub action: PauseAction,
    pub settings: SettingsResponse,
    pub sound: Option<Sfx>,
}

impl PauseMenu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.fresh = true;
        self.settings = None;
        self.loadout = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.settings = None;
        self.loadout = None;
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        s: &mut Settings,
        input: &InputState,
        single_player: bool,
        lobby_host: bool,
        subtitle: &str,
    ) -> PauseOutput {
        let mut out = PauseOutput {
            action: PauseAction::None,
            settings: SettingsResponse::default(),
            sound: None,
        };
        if !self.open {
            return out;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let canvas = ui.canvas();
        let sub = self.settings.is_some() || self.loadout.is_some();
        settings_menu::dim(ui, if sub { 0.75 } else { 0.5 });
        if let Some(menu) = &mut self.settings {
            out.settings = menu.ui(ui, s, input, settings_menu::panel_rect(canvas));
            if out.settings.back {
                self.settings = None;
                out.sound = Some(Sfx::UiBack);
            }
            return out;
        }
        if let Some(menu) = &mut self.loadout {
            // The loadout lives in the settings file: report it like a settings change.
            let r = menu.ui(ui, &mut s.loadout, loadout::panel_rect(canvas));
            out.settings.changed = r.changed;
            if r.back {
                out.settings.back = true;
                self.loadout = None;
                out.sound = Some(Sfx::UiBack);
            }
            return out;
        }
        let w = 220.0;
        let rect = Rect::from_center_size(canvas.center(), vec2(w, 0.0));
        let title = if single_player { "PAUSED" } else { "MENU" };
        ui.panel_auto(rect.min - vec2(0.0, 70.0), w, title, |ui| {
            ui.default_focus();
            if ui.button("RESUME") {
                out.action = PauseAction::Resume;
            }
            if single_player && ui.button("RESTART LEVEL") {
                out.action = PauseAction::Restart;
            }
            if lobby_host && ui.button("RETURN TO LOBBY") {
                out.action = PauseAction::ReturnToLobby;
            }
            if ui.button("LOADOUT") {
                self.loadout =
                    Some(LoadoutMenu::new().with_note("CHANGES APPLY AT YOUR NEXT RESPAWN"));
                out.sound = Some(Sfx::UiSelect);
            }
            if ui.button("SETTINGS") {
                self.settings = Some(SettingsMenu::new());
                out.sound = Some(Sfx::UiSelect);
            }
            ui.spacing(4.0);
            if ui.button("LEAVE GAME") {
                out.action = PauseAction::Leave;
            }
            ui.spacing(2.0);
            let dim = TextStyle::new(ui.theme().ash);
            ui.text(subtitle, &dim);
            if !single_player {
                ui.text("THE GAME KEEPS RUNNING", &dim);
            }
            ui.text("ESC RESUME   F3 DEBUG OVERLAY", &dim);
        });
        if ui.back_pressed() && !fresh {
            out.action = PauseAction::Resume;
        }
        if out.action != PauseAction::None {
            out.sound = Some(if out.action == PauseAction::Resume {
                Sfx::UiBack
            } else {
                Sfx::UiSelect
            });
        }
        out
    }
}
