//! The CUSTOMISE screen (docs/ui.md "Cosmetics"): how your Shrike looks — mask, crest, cloak,
//! lantern emblem, coat colour, mask / trim colour and wick flame — with a turning 3D preview.
//!
//! [`CustomizeMenu`] is a panel like the settings screen: the main menu shows it as a page,
//! the lobby screen and the warm-up's lobby overlay as a sub-screen. It edits
//! `Settings::cosmetics` live (saved when it closes); whoever owns a network session sends
//! the change to the server with [`LookSync`] (at most every half second).
//!
//! The preview Shrike is drawn into the **viewmodel pass** (view space, fresh depth) so it
//! stands in front of whatever the screen shows behind it (menu backdrop, lobby, the warm-up
//! world), in the right half of the screen. Turn it by dragging with the mouse, with the
//! shoulder buttons / PageUp / PageDown, or let it turn by itself.
//!
//! Every row is a `choice` (left / right on the keyboard or d-pad, click either half with the
//! mouse); colour rows show a swatch of the colour.

use engine::gfx::{DrawItem, UiDrawList};
use engine::ui::draw::{self, fade};
use engine::ui::{Rect, TextStyle, Ui, UiDrawExt, UiKey};
use glam::{Mat4, Quat, Vec3, Vec4, vec2};
use shared::cosmetics::{Cosmetics, Slot};

use crate::models::{ModelLibrary, PlayerAnim, Tint};
use crate::net::NetClient;

/// Seconds between two cosmetics updates sent to a server.
pub const SEND_INTERVAL: f64 = 0.5;
/// Preview turn per shoulder-button press (radians) and the idle spin (radians / s).
const TURN_STEP: f32 = 0.6;
const IDLE_SPIN: f32 = 0.45;
/// The idle spin resumes this long after the player last turned the preview (seconds).
const IDLE_AFTER: f64 = 2.5;

/// What the panel did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustomizeResponse {
    /// The look changed (apply it, send it).
    pub changed: bool,
    /// DONE / Escape: close (and save the settings).
    pub back: bool,
}

/// The customise panel and its preview's turntable.
#[derive(Clone, Debug)]
pub struct CustomizeMenu {
    /// Preview yaw (radians, 0 = facing the camera).
    spin: f32,
    /// Mouse x while dragging the preview.
    drag_x: Option<f32>,
    /// When the player last turned the preview (no idle spin for a while).
    turned_at: f64,
    /// Seconds the panel has been open (animation clock).
    time: f64,
    /// Focus the first row once.
    focus: bool,
}

impl Default for CustomizeMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomizeMenu {
    pub fn new() -> Self {
        Self {
            spin: -0.45,
            drag_x: None,
            turned_at: f64::NEG_INFINITY,
            time: 0.0,
            focus: true,
        }
    }

    /// The panel's rectangle on a canvas (left side; the preview stands on the right).
    pub fn panel_rect(canvas: Rect) -> Rect {
        let w = 250.0f32.min(canvas.width() * 0.5 - 8.0);
        Rect::new(
            vec2(canvas.min.x + 16.0, canvas.min.y + 40.0),
            vec2(canvas.min.x + 16.0 + w, canvas.max.y - 30.0),
        )
    }

    /// Draws the panel (editing `look`) and the preview: `team` is the identity colour the
    /// preview's eyes and trim show, `viewmodel_fov` the vertical FOV of the frame's viewmodel
    /// pass (where the preview goes, into `viewmodel`).
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        look: &mut Cosmetics,
        team: Vec4,
        lib: Option<&ModelLibrary>,
        viewmodel: &mut Vec<DrawItem>,
        viewmodel_fov: f32,
        dt: f32,
    ) -> CustomizeResponse {
        self.time += dt as f64;
        let mut resp = CustomizeResponse::default();
        let canvas = ui.canvas();
        let panel = Self::panel_rect(canvas);
        let t = ui.theme();
        crate::menus::style::left_shade(ui.painter(), t, canvas, panel.max.x + 40.0);
        let focus = std::mem::take(&mut self.focus);
        ui.panel(panel, "CUSTOMISE", |ui| {
            if focus {
                ui.request_focus(Slot::Mask.title());
            }
            ui.default_focus();
            for slot in Slot::ALL {
                let names = slot.names();
                let mut index = look.get(slot) as usize;
                let row = ui.available_rect();
                let changed = ui.choice(slot.title(), &mut index, &names);
                if let Some(rgb) = slot.color(index as u8) {
                    // A swatch of the colour at the right end of the label.
                    let x = row.min.x + (row.width() * ui.theme().label_fraction).round() - 12.0;
                    let swatch = Rect::from_min_size(vec2(x, row.min.y + 3.0), vec2(8.0, 8.0));
                    let c = Vec4::new(rgb[0], rgb[1], rgb[2], 1.0);
                    let ink = ui.theme().ink;
                    draw::fill(ui.painter(), swatch.expand(1.0), ink);
                    draw::fill(ui.painter(), swatch, c);
                }
                if changed {
                    look.set(slot, index as u8);
                    resp.changed = true;
                }
            }
            ui.spacing(3.0);
            ui.separator();
            ui.row(2, |ui| {
                if ui.button("RANDOMISE") {
                    let seed = (self.time * 1000.0) as u32 ^ (look.mask as u32 * 977);
                    *look = Cosmetics::from_seed(seed);
                    resp.changed = true;
                }
                if ui.button("CLASSIC") {
                    *look = Cosmetics::default();
                    resp.changed = true;
                }
            });
            if ui.button("DONE") {
                resp.back = true;
            }
            ui.spacing(3.0);
            let dim = TextStyle::new(ui.theme().ash);
            ui.text("YOUR SLOT COLOUR SHOWS ON THE", &dim);
            ui.text("EYES AND TRIM; THE REST IS YOURS.", &dim);
        });
        if ui.back_pressed() {
            resp.back = true;
        }

        // Turntable: drag on the right half, shoulder buttons, or the idle spin.
        let stage = Rect::new(vec2(panel.max.x + 8.0, canvas.min.y), canvas.max);
        let id = ui.id("##preview");
        let r = ui.interact(id, stage, false);
        let mouse_x = ui.input().mouse_pos.x;
        if r.held {
            if let Some(x) = self.drag_x {
                self.spin += (mouse_x - x) * 0.02;
            }
            self.drag_x = Some(mouse_x);
            self.turned_at = self.time;
        } else {
            self.drag_x = None;
        }
        let turns = ui.input().key_count(UiKey::PageDown) as f32
            - ui.input().key_count(UiKey::PageUp) as f32;
        if turns != 0.0 {
            self.spin += turns * TURN_STEP;
            self.turned_at = self.time;
        }
        if self.time - self.turned_at > IDLE_AFTER {
            self.spin += IDLE_SPIN * dt;
        }
        self.spin = self.spin.rem_euclid(std::f32::consts::TAU);
        let hint = TextStyle::new(ui.theme().ash)
            .centered()
            .shadow(ui.theme().shadow);
        ui.painter().text(
            vec2(stage.center().x, canvas.max.y - 18.0),
            "DRAG / PGUP PGDN / SHOULDERS TURN",
            &hint,
        );

        if let Some(lib) = lib {
            let center = vec2((stage.center().x - canvas.center().x) / canvas.width(), 0.0);
            draw_preview(
                lib,
                look,
                team,
                self.spin,
                self.time,
                center.x,
                viewmodel_fov,
                viewmodel,
            );
        }
        resp
    }
}

/// Draws a turning Shrike for a preview into the viewmodel pass (view space) on a dark card
/// that hides whatever is behind it: `x_frac` is where its centre sits across the screen
/// (-0.5 left edge .. 0.5 right edge), sized to about two thirds of the screen's height at
/// `fov_y` degrees.
#[allow(clippy::too_many_arguments)]
pub fn draw_preview(
    lib: &ModelLibrary,
    look: &Cosmetics,
    team: Vec4,
    spin: f32,
    time: f64,
    x_frac: f32,
    fov_y: f32,
    out: &mut Vec<DrawItem>,
) {
    let tan = (fov_y.clamp(30.0, 100.0).to_radians() * 0.5).tan();
    // 2.9 m of height on screen, 16:9.
    let distance = 2.9 / (2.0 * tan);
    let x = x_frac * 2.0 * distance * tan * 16.0 / 9.0;
    // The card: the ready board's slate stretched over the right half, a little behind.
    if let Some(slate) = lib.get(crate::models::ModelId::ReadyBoard).parts.get(1) {
        let back = distance + 1.4;
        let (half_h, half_w) = (back * tan, back * tan * 16.0 / 9.0);
        let center = Vec3::new(x_frac * 2.0 * half_w, 0.0, -back);
        let scale = Vec3::new(0.5 * 2.0 * half_w / 5.6, 2.04 * half_h / 2.5, 1.0);
        let transform = Mat4::from_translation(center)
            * Mat4::from_scale(scale)
            * Mat4::from_translation(-Vec3::new(0.0, 1.45, -0.045));
        out.push(DrawItem {
            mesh: slate.mesh,
            texture: slate.texture,
            transform,
            tint: Vec4::new(1.0, 0.85, 0.82, 1.0),
            unlit: true,
        });
    }
    let anim = PlayerAnim {
        on_ground: true,
        time,
        // Face the camera, turned by `spin`; the pose's aim follows the body.
        yaw: std::f32::consts::PI + spin,
        cosmetics: *look,
        weapon: 0,
        ..Default::default()
    };
    let root = Mat4::from_rotation_translation(
        Quat::from_rotation_y(anim.yaw),
        Vec3::new(x, -1.15, -distance),
    );
    lib.draw_shrike(&anim, root, &Tint::team(team), out);
}

/// Sends the local look to the server whenever it differs from what was sent (debounced: at
/// most every [`SEND_INTERVAL`] seconds, so clicking through options doesn't spam).
#[derive(Clone, Copy, Debug)]
pub struct LookSync {
    sent_at: f64,
}

impl Default for LookSync {
    fn default() -> Self {
        Self {
            sent_at: f64::NEG_INFINITY,
        }
    }
}

impl LookSync {
    pub fn update(&mut self, net: &mut NetClient, look: Cosmetics, now: f64) {
        if net.cosmetics() != look.sanitized() && now - self.sent_at >= SEND_INTERVAL {
            net.set_cosmetics(look);
            self.sent_at = now;
        }
    }
}

/// A small swatch of a look for lists (lobby, scoreboard): coat colour, mask colour and the
/// flame as a dot, `size` canvas units tall (about two thirds as wide). The identity colour
/// is drawn separately (`cues::player_swatch`).
pub fn look_swatch(
    list: &mut UiDrawList,
    pos: glam::Vec2,
    size: f32,
    look: &Cosmetics,
    outline: Vec4,
) {
    let rgb = |c: [f32; 3]| Vec4::new(c[0], c[1], c[2], 1.0);
    let w = (size * 0.7).round().max(4.0);
    let half = (w * 0.5).floor();
    let rect = Rect::from_min_size(pos, vec2(w, size));
    draw::fill(list, rect.expand(1.0), outline);
    draw::fill(
        list,
        Rect::from_min_size(pos, vec2(half, size)),
        rgb(look.primary_rgb()),
    );
    draw::fill(
        list,
        Rect::new(vec2(pos.x + half, pos.y), vec2(pos.x + w, pos.y + size)),
        rgb(look.secondary_rgb()),
    );
    let dot = Rect::from_center_size(rect.center(), vec2(2.0, 2.0) + (size * 0.15).floor());
    draw::fill(list, dot.expand(1.0), fade(outline, 0.9));
    draw::fill(list, dot, rgb(look.flame_rgb()));
}
