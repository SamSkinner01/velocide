//! Settings screen (used by the main menu and the pause menu): tabs for Video/Retro, Audio,
//! Controls (sensitivity, invert, key rebinding), Gameplay (also the update check and the
//! version) and Accessibility (colour vision, shape cues, with a live palette preview).
//! Changes apply live (the caller sets `Common::settings_changed` when
//! [`SettingsResponse::changed`]) and are saved when the player leaves the screen.

use engine::color_vision::ColorVisionMode;
use engine::input::{Binding, InputState, Key};
use engine::ui::draw::{self, fade};
use engine::ui::hud::{self, BarStyle};
use engine::ui::{Rect, TextFilter, TextStyle, Ui, UiDrawExt};
use glam::{Vec2, Vec4, vec2};

use crate::controls::{Action, Bindings, is_gamepad_binding};
use crate::ingame::cues;
use crate::palette::{Palette, marker_shape};
use crate::settings::{CROSSHAIRS, CueMode, RESOLUTIONS, Settings};

pub const TABS: [&str; 5] = [
    "VIDEO/RETRO",
    "AUDIO",
    "CONTROLS",
    "GAMEPLAY",
    "ACCESSIBILITY",
];

/// Index of the ACCESSIBILITY tab in [`TABS`].
pub const ACCESSIBILITY_TAB: usize = 4;

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
                3 => gameplay_tab(ui, s, &mut r),
                _ => accessibility_tab(ui, s, &mut r),
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
    ui.heading("ONLINE");
    r.changed |= ui
        .text_field(
            "ONLINE SERVICE",
            &mut s.online_service,
            64,
            TextFilter::Address,
        )
        .changed;
    let built_in = rendezvous::BUILT_IN_SERVICE;
    ui.label_wrapped(&if built_in.is_empty() {
        "Hosting and joining by code go through this service (HOST:PORT). This build has \
         none built in."
            .to_string()
    } else {
        format!(
            "Hosting and joining by code go through this service. Leave it empty for the \
             built-in one ({built_in})."
        )
    });
    ui.heading("UPDATES");
    r.changed |= ui.toggle("CHECK FOR UPDATES", &mut s.check_for_updates);
    ui.enabled_scope(false, |ui| {
        ui.value_row("VERSION", &shared::build_info::label());
        ui.value_row(
            "NETCODE",
            &format!("PROTOCOL {}", shared::protocol::PROTOCOL_VERSION),
        );
    });
    ui.label_wrapped(
        "Everyone in a game needs the same version. Release builds look for a new one at \
         launch and offer it on the main menu.",
    );
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
        3 => {
            s.crosshair = d.crosshair;
            s.view_bob = d.view_bob;
            s.show_viewmodel = d.show_viewmodel;
            s.show_timer = d.show_timer;
            s.show_speed = d.show_speed;
            s.show_fps = d.show_fps;
            s.show_ping = d.show_ping;
            s.check_for_updates = d.check_for_updates;
            s.online_service = d.online_service;
        }
        _ => s.accessibility = d.accessibility,
    }
}

fn accessibility_tab(ui: &mut Ui, s: &mut Settings, r: &mut SettingsResponse) {
    let a = &mut s.accessibility;
    ui.heading("COLOUR VISION");
    let modes = ColorVisionMode::ALL.map(ColorVisionMode::label);
    let mut mode = a.color_vision.mode.index();
    if ui.choice("COLOUR MODE", &mut mode, &modes) {
        a.color_vision.mode = ColorVisionMode::from_index(mode);
        r.changed = true;
    }
    ui.label_wrapped(a.color_vision.mode.description());
    let active = a.color_vision.mode != ColorVisionMode::Off;
    ui.enabled_scope(active, |ui| {
        r.changed |= ui.slider(
            "FILTER STRENGTH",
            &mut a.color_vision.strength,
            0.0,
            1.0,
            0.05,
            "{%}%",
        );
    });
    let mut cues = a.shape_cues.index();
    if ui.choice("SHAPE CUES", &mut cues, &CueMode::LABELS) {
        a.shape_cues = CueMode::ALL[cues];
        r.changed = true;
    }
    ui.heading("PREVIEW");
    palette_preview(
        ui,
        Palette::for_mode(a.color_vision.mode),
        a.shape_cues_on(),
    );
    ui.separator();
    ui.label_wrapped(
        "The filter recolours the 3D view for your eyes (0% keeps only the palette swap). The \
         HUD and menus switch to a matching palette: player colours, enemy wind-up vs parry \
         window, hard damage, heals and style ranks. Shape cues add player marker shapes, a \
         parry icon over enemies and +/- health popups (AUTO: on with any colour mode).",
    );
}

/// Swatches of the palette's critical colours, the way the HUD will draw them.
fn palette_preview(ui: &mut Ui, p: &Palette, shapes: bool) {
    let theme = p.theme();
    let t = ui.theme().clone();
    let label_style = TextStyle::new(t.bone_dim);
    let row_h = 15.0;
    let row = |ui: &mut Ui, label: &str, draw: &mut dyn FnMut(&mut Ui, Rect)| {
        let rect = ui.allocate(row_h);
        let label_w = (rect.width() * t.label_fraction).round();
        let text_y = (rect.center().y - 3.5).round();
        ui.painter()
            .text(vec2(rect.min.x + 6.0, text_y), label, &label_style);
        let content = Rect::new(vec2(rect.min.x + label_w, rect.min.y), rect.max);
        draw(ui, content);
    };
    let chip = |ui: &mut Ui, at: Vec2, text: &str, color: Vec4| -> f32 {
        let w = engine::ui::text::line_width(text, 1.0) + 8.0;
        let r = Rect::from_min_size(at, vec2(w, 11.0));
        draw::fill(ui.painter(), r.expand(1.0), fade(t.ink, 0.9));
        draw::fill(ui.painter(), r, color);
        ui.painter().text(
            vec2(r.min.x + 4.0, r.min.y + 2.0),
            text,
            &TextStyle::new(t.ink),
        );
        w + 5.0
    };
    row(ui, "PLAYERS", &mut |ui, c| {
        for i in 0..p.team.len() {
            let at = vec2(c.min.x + i as f32 * 11.0, c.min.y + 4.0);
            let ink = fade(t.ink, 0.9);
            if shapes {
                cues::draw_marker(ui.painter(), at, marker_shape(i), 1.0, p.team[i], ink);
            } else {
                let r = Rect::from_min_size(at - vec2(0.0, 1.0), vec2(6.0, 7.0));
                draw::fill(ui.painter(), r.expand(1.0), ink);
                draw::fill(ui.painter(), r, p.team[i]);
            }
        }
    });
    row(ui, "ENEMY ATTACKS", &mut |ui, c| {
        let mut x = c.min.x;
        let y = c.min.y + 1.0;
        x += chip(ui, vec2(x, y), "WIND-UP", p.telegraph);
        x += chip(ui, vec2(x, y), "PARRY", p.parry_window);
        if shapes {
            let ink = t.ink;
            cues::draw_parry_icon(ui.painter(), vec2(x + 4.0, y + 5.0), 1.0, 1.0, p, ink, 0.0);
            x += 14.0;
        }
        chip(ui, vec2(x, y), "LASER", p.aim_laser);
    });
    row(ui, "HEALTH", &mut |ui, c| {
        let bar = Rect::from_min_size(vec2(c.min.x, c.min.y + 3.0), vec2(80.0, 7.0));
        let style = BarStyle::health(&theme);
        hud::segmented_bar(ui.painter(), bar, 55.0, 100.0, 25.0, 10, &style);
        let mut x = bar.max.x + 6.0;
        x += chip(ui, vec2(x, c.min.y + 1.0), "+12", p.heal);
        chip(ui, vec2(x, c.min.y + 1.0), "-8", p.damage);
    });
    row(ui, "STYLE RANKS", &mut |ui, c| {
        for (i, color) in p.ranks.iter().enumerate() {
            let r = Rect::from_min_size(
                vec2(c.min.x + i as f32 * 11.0, c.min.y + 3.0),
                vec2(8.0, 7.0),
            );
            draw::fill(ui.painter(), r.expand(1.0), fade(t.ink, 0.9));
            draw::fill(ui.painter(), r, *color);
        }
    });
}

/// Dims the whole canvas (behind a modal panel).
pub fn dim(ui: &mut Ui, alpha: f32) {
    let c = ui.canvas();
    let ink = ui.theme().ink;
    ui.painter()
        .fill(c, glam::Vec4::new(ink.x, ink.y, ink.z, alpha));
}
