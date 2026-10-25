//! Settings screen (used by the main menu and the pause menu): tabs for Video/Retro, Audio,
//! Controls (sensitivity, invert, key rebinding) and Gameplay. Changes apply live (the caller
//! sets `Common::settings_changed` when [`SettingsResponse::changed`]) and are saved when the
//! player leaves the screen.

use engine::input::{Binding, InputState, Key};
use engine::ui::{Rect, TextFilter, TextStyle, Ui, UiDrawExt};
use glam::vec2;

use crate::controls::{Action, Bindings, is_gamepad_binding};
use crate::settings::{CROSSHAIRS, RESOLUTIONS, Settings};

pub const TABS: [&str; 4] = ["VIDEO/RETRO", "AUDIO", "CONTROLS", "GAMEPLAY"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingsResponse {
    /// Something changed this frame: apply it.
    pub changed: bool,
    /// The player left the screen (BACK / Escape): save and close.
    pub back: bool,
    /// The player name was edited (drop any `--name` override).
    pub name_changed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsMenu {
    pub tab: usize,
    /// Action whose key bind widget is capturing.
    capture: Option<Action>,
}

/// Panel rectangle for the settings screen on a canvas.
pub fn panel_rect(canvas: Rect) -> Rect {
    let size = vec2(460.0, 330.0).min(canvas.size() - vec2(16.0, 16.0));
    Rect::from_center_size(canvas.center(), size)
}

impl SettingsMenu {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws the settings panel in `rect`.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        s: &mut Settings,
        input: &InputState,
        rect: Rect,
    ) -> SettingsResponse {
        let mut r = SettingsResponse::default();
        ui.panel(rect, "SETTINGS", |ui| {
            ui.default_focus();
            if ui.tabs("tabs", &mut self.tab, &TABS) {
                self.capture = None;
            }
            ui.spacing(2.0);
            let list_h = ui.available_rect().height() - 22.0;
            let tab = self.tab;
            ui.scroll_area("page", list_h, |ui| match tab {
                0 => video_tab(ui, s, &mut r),
                1 => audio_tab(ui, s, &mut r),
                2 => self.controls_tab(ui, s, input, &mut r),
                _ => gameplay_tab(ui, s, &mut r),
            });
            ui.row(2, |ui| {
                if ui.button("DEFAULTS##tab") {
                    reset_tab(tab, s);
                    r.changed = true;
                }
                if ui.button("BACK") {
                    r.back = true;
                }
            });
        });
        if ui.back_pressed() {
            r.back = true;
        }
        if r.back {
            self.capture = None;
        }
        r
    }

    fn controls_tab(
        &mut self,
        ui: &mut Ui,
        s: &mut Settings,
        input: &InputState,
        r: &mut SettingsResponse,
    ) {
        ui.heading("MOUSE");
        r.changed |= ui.slider(
            "SENSITIVITY",
            &mut s.mouse_sensitivity,
            0.1,
            10.0,
            0.05,
            "{}",
        );
        r.changed |= ui.toggle("INVERT LOOK", &mut s.invert_y);
        ui.heading("GAMEPAD");
        r.changed |= ui.slider(
            "LOOK SPEED",
            &mut s.gamepad_look_sensitivity,
            60.0,
            600.0,
            10.0,
            "{} DEG/S",
        );
        r.changed |= ui.toggle("INVERT LOOK##pad", &mut s.gamepad_invert_y);
        ui.heading("BINDINGS");
        let hint = TextStyle::new(ui.theme().ash);
        ui.text("ENTER/CLICK: REBIND  BACKSPACE: CLEAR  ESC: CANCEL", &hint);
        for action in Action::ALL {
            let label = format!("{}##bind{:?}", action.label(), action);
            let name = if self.capture == Some(action) {
                "PRESS A KEY...".to_string()
            } else {
                s.bindings.display(action)
            };
            let resp = ui.keybind(&label, &name);
            if resp.started {
                self.capture = Some(action);
            }
            if resp.cancelled {
                self.capture = None;
            }
            if resp.capturing
                && let Some(b) = input.last_pressed_binding()
                && b != Binding::Key(Key::Escape)
            {
                s.bindings.bind(action, b);
                ui.finish_capture();
                self.capture = None;
                r.changed = true;
            }
            if resp.cleared {
                s.bindings.clear(action, false);
                r.changed = true;
            }
        }
        ui.spacing(2.0);
        if ui.button("RESET ALL BINDINGS") {
            s.bindings = Bindings::default();
            r.changed = true;
        }
        let pads = Action::ALL
            .iter()
            .filter(|&&a| s.bindings.get(a).iter().any(is_gamepad_binding))
            .count();
        ui.label_wrapped(&format!(
            "Gamepad: left stick moves, right stick looks, {pads} actions bound. Binding a \
             gamepad button replaces the gamepad binding only."
        ));
    }
}

fn video_tab(ui: &mut Ui, s: &mut Settings, r: &mut SettingsResponse) {
    ui.heading("DISPLAY");
    let mut res = RESOLUTIONS
        .iter()
        .position(|(h, _)| *h == s.retro.internal_height)
        .unwrap_or(3);
    let names: Vec<&str> = RESOLUTIONS.iter().map(|(_, n)| *n).collect();
    if ui.choice("RESOLUTION SCALE", &mut res, &names) {
        s.retro.internal_height = RESOLUTIONS[res].0;
        r.changed = true;
    }
    r.changed |= ui.toggle("FULLSCREEN", &mut s.fullscreen);
    r.changed |= ui.toggle("VSYNC", &mut s.retro.vsync);
    r.changed |= ui.slider("FIELD OF VIEW", &mut s.fov, 70.0, 130.0, 1.0, "{}");
    r.changed |= ui.slider("WEAPON FOV", &mut s.viewmodel_fov, 45.0, 80.0, 1.0, "{}");
    ui.heading("RETRO EFFECTS");
    r.changed |= ui.toggle("VERTEX SNAPPING", &mut s.retro.vertex_snap);
    let snap = s.retro.vertex_snap;
    ui.enabled_scope(snap, |ui| {
        r.changed |= ui.slider(
            "SNAP STRENGTH",
            &mut s.retro.vertex_snap_strength,
            0.5,
            4.0,
            0.25,
            "{}x",
        );
    });
    r.changed |= ui.toggle("AFFINE TEXTURES", &mut s.retro.affine_textures);
    r.changed |= ui.toggle("DITHERING", &mut s.retro.dithering);
    let mut bits = s.retro.color_bits as f32;
    if ui.slider("COLOUR DEPTH", &mut bits, 3.0, 8.0, 1.0, "{} BIT") {
        s.retro.color_bits = bits as u32;
        r.changed = true;
    }
    r.changed |= ui.toggle("DISTANCE FOG", &mut s.retro.fog);
    r.changed |= ui.toggle("VERTEX LIGHTING", &mut s.retro.vertex_lighting);
    ui.heading("FEEDBACK");
    r.changed |= ui.toggle("SCREEN SHAKE", &mut s.retro.screen_shake);
    r.changed |= ui.toggle("SCREEN FLASHES", &mut s.retro.screen_flashes);
    r.changed |= ui.toggle("HITSTOP", &mut s.retro.hitstop);
}

fn audio_tab(ui: &mut Ui, s: &mut Settings, r: &mut SettingsResponse) {
    ui.heading("VOLUME");
    r.changed |= ui.slider("MASTER", &mut s.master_volume, 0.0, 1.0, 0.05, "{%}%");
    r.changed |= ui.slider("EFFECTS", &mut s.sfx_volume, 0.0, 1.0, 0.05, "{%}%");
    r.changed |= ui.slider("MUSIC", &mut s.music_volume, 0.0, 1.0, 0.05, "{%}%");
    ui.separator();
    ui.label_wrapped(
        "Music stems swell as the fight heats up and fade between encounters. Parries, \
         ricochets and rank-ups always cut through the mix.",
    );
}

fn gameplay_tab(ui: &mut Ui, s: &mut Settings, r: &mut SettingsResponse) {
    ui.heading("IDENTITY");
    let resp = ui.text_field("PLAYER NAME", &mut s.player_name, 24, TextFilter::Name);
    if resp.changed {
        r.changed = true;
        r.name_changed = true;
    }
    ui.heading("VIEW");
    r.changed |= ui.choice("CROSSHAIR", &mut s.crosshair, &CROSSHAIRS);
    r.changed |= ui.slider("VIEW BOB", &mut s.view_bob, 0.0, 1.0, 0.1, "{%}%");
    r.changed |= ui.toggle("SHOW WEAPON", &mut s.show_viewmodel);
    ui.heading("HUD");
    r.changed |= ui.toggle("LEVEL TIMER", &mut s.show_timer);
    r.changed |= ui.toggle("SPEEDOMETER", &mut s.show_speed);
    r.changed |= ui.toggle("FPS COUNTER", &mut s.show_fps);
    r.changed |= ui.toggle("PING DISPLAY", &mut s.show_ping);
    ui.enabled_scope(false, |ui| {
        ui.value_row(
            "NETCODE",
            &format!("PROTOCOL {}", shared::protocol::PROTOCOL_VERSION),
        );
    });
}

fn reset_tab(tab: usize, s: &mut Settings) {
    let d = Settings::default();
    match tab {
        0 => {
            s.retro = d.retro;
            s.fov = d.fov;
            s.viewmodel_fov = d.viewmodel_fov;
        }
        1 => {
            s.master_volume = d.master_volume;
            s.sfx_volume = d.sfx_volume;
            s.music_volume = d.music_volume;
        }
        2 => {
            s.mouse_sensitivity = d.mouse_sensitivity;
            s.invert_y = d.invert_y;
            s.gamepad_look_sensitivity = d.gamepad_look_sensitivity;
            s.gamepad_invert_y = d.gamepad_invert_y;
            s.bindings = d.bindings;
        }
        _ => {
            s.crosshair = d.crosshair;
            s.view_bob = d.view_bob;
            s.show_viewmodel = d.show_viewmodel;
            s.show_timer = d.show_timer;
            s.show_speed = d.show_speed;
            s.show_fps = d.show_fps;
            s.show_ping = d.show_ping;
        }
    }
}

/// Dims the whole canvas (behind a modal panel).
pub fn dim(ui: &mut Ui, alpha: f32) {
    let c = ui.canvas();
    let ink = ui.theme().ink;
    ui.painter()
        .fill(c, glam::Vec4::new(ink.x, ink.y, ink.z, alpha));
}
