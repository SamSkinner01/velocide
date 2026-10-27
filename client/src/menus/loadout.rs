//! LOADOUT screen (main menu, lobby, the warm-up's overlay, pause menu): what you carry on
//! the slot keys 1–4, each weapon's two abilities, and named presets (docs/ui.md "Loadout
//! screen").
//!
//! One full-screen page:
//!
//! - left: the four **slot cards** (key, weapon, its two abilities; selecting one shows it on
//!   the right), the **knack** card (Q; it opens the knack picker), and the compact
//!   **presets** (PRESET choice, LOAD / SAVE / NEW / DELETE, NAME) with the gun game note;
//! - right, the **showcase**: the selected weapon turning on a card (drawn into the viewmodel
//!   pass like the CUSTOMISE preview; drag, PageUp / PageDown or the shoulders turn it), its
//!   name, class, role and numbers, CHANGE WEAPON; under it the **two ability boxes** (A and
//!   B): name, how it is fired, what it does and its numbers;
//! - CHANGE WEAPON opens the **weapon picker** (the weapons grouped by class; the one under
//!   the cursor / focus turns on the card with its role and numbers), an ability box opens
//!   the **ability picker** (the class's pool as four boxes explained the same way), the
//!   knack card the **knack picker** (the six knacks; the one under the cursor / focus
//!   explained in a box, PvE-only ones marked: in PvP they become Surge).
//!
//! Every number is shown for PvE and PvP from the effective tunings of the local
//! `assets/tuning.ron` (`Tuning::for_mode`); the words come from [`super::loadout_text`].
//!
//! The screen edits `Settings::loadout` directly (the caller saves the settings when the
//! player leaves and sends the loadout to the server: `NetClient::set_loadout`). Rules kept
//! while editing: no weapon twice (picking a weapon another slot holds swaps the two slots),
//! at least one weapon (the last one can't be emptied), the two abilities differ (picking the
//! other one swaps them); any knack. Gun game hands out its own weapons (`force_loadout`)
//! and keeps only the knack; the screen says so.
//!
//! Mouse, keyboard and gamepad: every control is focusable (Up / Down walk them; Left / Right
//! jump between the slot cards, the showcase and the boxes, and across the pickers' grids),
//! Enter / A activates, Escape / B closes a picker or leaves.

use engine::audio::Sfx;
use engine::gfx::{Camera, DrawItem, PointLight, UiDrawList};
use engine::ui::draw::{self, fade};
use engine::ui::text::{line_width, truncate, wrap_text};
use engine::ui::{Id, Rect, Response, TextFilter, TextStyle, Theme, Ui, UiDrawExt, UiKey};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4, vec2};
use shared::loadout::{
    Ability, Knack, LOADOUT_SLOTS, Loadout, WeaponBuild, WeaponClass, WeaponKind,
};
use shared::lobby::GameMode;
use shared::tuning::Tuning;

use super::loadout_text::{self, Stat};
use crate::controls::Action;
use crate::models::weapons::{HeldWeapon, WeaponLook as _, ability_color, weapon_pose};
use crate::models::{DEFAULT_TEAM_COLOR, ModelId, ModelLibrary, Tint};
use crate::settings::{LoadoutPreset, LoadoutSettings, MAX_PRESETS, PRESET_NAME_LEN, Settings};

/// Preview turn per shoulder-button press (radians) and the idle spin (radians / s).
const TURN_STEP: f32 = 0.6;
const IDLE_SPIN: f32 = 0.5;
/// The idle spin resumes this long after the player last turned the preview (seconds).
const IDLE_AFTER: f64 = 2.5;
/// The preview leans its top towards the camera by this much (radians).
const TILT: f32 = 0.32;

// Layout metrics (canvas units).
const TITLE_H: f32 = 15.0;
const PAD: f32 = 8.0;
const GAP: f32 = 8.0;
const FOOT_H: f32 = 16.0;
const SHOWCASE_H: f32 = 132.0;
const HEADING_H: f32 = 14.0;
const BOX_GAP: f32 = 6.0;
const BOX_PAD: f32 = 6.0;
const LINE_H: f32 = 11.0;
const CARD_H: f32 = 27.0;
const TILE_H: f32 = 24.0;
/// Lines of explanation an ability box, the weapon panel and the knack box have room for.
pub(crate) const BOX_LINES: usize = 5;
pub(crate) const ROLE_LINES: usize = 3;
pub(crate) const KNACK_LINES: usize = 6;

/// What the screen did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadoutResponse {
    /// The loadout or a preset changed: apply / send it.
    pub changed: bool,
    /// The player left the screen (BACK / Escape): save and close.
    pub back: bool,
    /// A UI sound to play.
    pub sound: Option<Sfx>,
}

/// Where the screen draws its turning weapon: the frame's **viewmodel pass** (view space,
/// fresh depth, so it stands in front of any world), like the CUSTOMISE preview. The screen
/// owns the list while it is open (it clears it: no held weapon in front of the menu) and
/// adds a lamp by the preview to the frame's lights.
pub struct Preview<'a> {
    pub lib: Option<&'a ModelLibrary>,
    pub viewmodel: &'a mut Vec<DrawItem>,
    pub lights: &'a mut Vec<PointLight>,
    /// The frame's camera (the lamp is placed in front of it).
    pub camera: Camera,
    /// Vertical FOV of the viewmodel pass (degrees).
    pub fov: f32,
}

/// Which picker covers the right side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Picker {
    None,
    /// The weapon of the selected slot.
    Weapon,
    /// Ability A (0) or B (1) of the selected slot.
    Ability(usize),
    /// The knack on Q.
    Knack,
}

/// One column of numbers: a label and the effective tuning it comes from.
struct Column {
    label: &'static str,
    tuning: Tuning,
    pvp: bool,
}

/// Where everything goes on a canvas (the screen fills it, up to 720 units wide).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub frame: Rect,
    /// Slot cards, the knack card and the presets.
    pub left: Rect,
    /// The showcase and the ability boxes, or a picker.
    pub right: Rect,
    /// The turning weapon.
    pub card: Rect,
    /// The weapon's name, role and numbers.
    pub info: Rect,
    /// Under the showcase: the abilities heading and boxes, or the weapon grid.
    pub lower: Rect,
    pub boxes: [Rect; 2],
    pub foot: Rect,
}

impl Layout {
    pub fn new(canvas: Rect) -> Layout {
        let width = (canvas.width() - 16.0).min(720.0);
        let frame =
            Rect::from_center_size(canvas.center(), vec2(width, canvas.height() - 16.0)).round();
        let inner = Rect::new(
            frame.min + vec2(PAD, TITLE_H + 6.0),
            frame.max - vec2(PAD, PAD),
        );
        let foot = Rect::new(vec2(inner.min.x, inner.max.y - FOOT_H), inner.max);
        let body = Rect::new(inner.min, vec2(inner.max.x, foot.min.y - 6.0));
        let left_w = (body.width() * 0.25).round().clamp(132.0, 176.0);
        let left = Rect::new(body.min, vec2(body.min.x + left_w, body.max.y));
        let right = Rect::new(vec2(left.max.x + GAP, body.min.y), body.max);
        let card = Rect::from_min_size(right.min, vec2((right.width() * 0.4).round(), SHOWCASE_H));
        let info = Rect::new(
            vec2(card.max.x + GAP, right.min.y),
            vec2(right.max.x, card.max.y),
        );
        let lower = Rect::new(vec2(right.min.x, card.max.y + 6.0), right.max);
        let top = lower.min.y + HEADING_H;
        let w = ((lower.width() - BOX_GAP) * 0.5).floor();
        let boxes = [
            Rect::new(vec2(lower.min.x, top), vec2(lower.min.x + w, lower.max.y)),
            Rect::new(vec2(lower.max.x - w, top), lower.max),
        ];
        Layout {
            frame,
            left,
            right,
            card,
            info,
            lower,
            boxes,
            foot,
        }
    }

    /// Slot card `i` (0..4); [`LOADOUT_SLOTS`] is the knack card under them.
    pub fn slot_card(&self, i: usize) -> Rect {
        let y = self.left.min.y + HEADING_H + i as f32 * (CARD_H + 3.0);
        Rect::from_min_size(vec2(self.left.min.x, y), vec2(self.left.width(), CARD_H))
    }

    /// The knack picker: tile `i` in a column on the left of the right side ...
    pub fn knack_tile(&self, i: usize) -> Rect {
        let w = (self.right.width() * 0.32).round();
        let y = self.right.min.y + HEADING_H + i as f32 * (CARD_H + 4.0);
        Rect::from_min_size(vec2(self.right.min.x, y), vec2(w, CARD_H))
    }

    /// ... and the box explaining the knack under the cursor / focus, beside them.
    /// (As tall as the column of tiles.)
    pub fn knack_detail(&self) -> Rect {
        let first = self.knack_tile(0);
        let last = self.knack_tile(Knack::COUNT - 1);
        Rect::new(
            vec2(first.max.x + BOX_GAP, first.min.y),
            vec2(self.right.max.x, last.max.y),
        )
    }

    /// The four boxes of the ability picker (2 × 2 over the right side, under a heading).
    pub fn grid(&self) -> [Rect; 4] {
        let r = self.right;
        let top = r.min.y + HEADING_H;
        let w = ((r.width() - BOX_GAP) * 0.5).floor();
        let h = ((r.max.y - top - BOX_GAP) * 0.5).floor();
        let cell = |col: f32, row: f32| {
            let x = if col == 0.0 { r.min.x } else { r.max.x - w };
            let y = if row == 0.0 { top } else { r.max.y - h };
            Rect::from_min_size(vec2(x, y), vec2(w, h))
        };
        [
            cell(0.0, 0.0),
            cell(1.0, 0.0),
            cell(0.0, 1.0),
            cell(1.0, 1.0),
        ]
    }

    /// Width the ability explanations wrap at.
    #[cfg(test)]
    pub fn box_text_width(&self) -> f32 {
        self.boxes[0].width() - 2.0 * BOX_PAD
    }

    /// Width the weapon's role wraps at.
    #[cfg(test)]
    pub fn role_text_width(&self) -> f32 {
        self.info.width() - 4.0
    }
}

pub struct LoadoutMenu {
    /// The slot being edited (0..4).
    slot: usize,
    /// The preset the preset row works on.
    preset: usize,
    /// Number columns (PvE, PvP).
    columns: Vec<Column>,
    /// Shown in the footer (in a match: when the change applies).
    note: Option<&'static str>,
    picker: Picker,
    /// Label of the control to focus next frame (inside the screen's id scope).
    focus: Option<String>,
    /// The focused control when the frame began.
    focus_start: Option<Id>,
    /// Preview yaw (radians; 0 = barrel away from the camera).
    spin: f32,
    /// Mouse x while dragging the preview.
    drag_x: Option<f32>,
    /// When the player last turned the preview (no idle spin for a while).
    turned_at: f64,
    /// Seconds the screen has been open (animation clock) and the UI time of the last frame.
    time: f64,
    last_frame: Option<f64>,
}

impl Default for LoadoutMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadoutMenu {
    /// For the menus: PvE and PvP numbers from the local tuning file.
    pub fn new() -> Self {
        Self::with_tuning(&Tuning::load_or_default(&Tuning::default_path()))
    }

    /// PvE and PvP numbers from `tuning` (its balance profiles).
    pub fn with_tuning(tuning: &Tuning) -> Self {
        let column = |label, mode, pvp| Column {
            label,
            tuning: tuning.for_mode(mode),
            pvp,
        };
        Self {
            slot: 0,
            preset: 0,
            columns: vec![
                column("PVE", GameMode::Campaign, false),
                column("PVP", GameMode::Deathmatch, true),
            ],
            note: None,
            picker: Picker::None,
            focus: Some(slot_label(0)),
            focus_start: None,
            spin: -1.1,
            drag_x: None,
            turned_at: f64::NEG_INFINITY,
            time: 0.0,
            last_frame: None,
        }
    }

    /// Shows `note` in the footer (in a match: when the change applies).
    pub fn with_note(mut self, note: &'static str) -> Self {
        self.note = Some(note);
        self
    }

    /// The slot being edited.
    pub fn slot(&self) -> usize {
        self.slot
    }

    /// The weapon picker is open.
    pub fn is_picking_weapon(&self) -> bool {
        self.picker == Picker::Weapon
    }

    /// The knack picker is open.
    pub fn is_picking_knack(&self) -> bool {
        self.picker == Picker::Knack
    }

    /// The ability picker is open for ability `k` (0 = A, 1 = B).
    pub fn picking_ability(&self) -> Option<usize> {
        match self.picker {
            Picker::Ability(k) => Some(k),
            _ => None,
        }
    }

    /// Draws the screen over the whole canvas, editing `s.loadout` (the bindings name the
    /// inputs), and the turning weapon into `preview`.
    pub fn ui(&mut self, ui: &mut Ui, s: &mut Settings, preview: Preview) -> LoadoutResponse {
        let now = ui.time();
        let dt = self.last_frame.map_or(0.0, |t| (now - t).clamp(0.0, 0.1));
        self.last_frame = Some(now);
        self.time += dt;
        let mut r = LoadoutResponse::default();
        let canvas = ui.canvas();
        let lay = Layout::new(canvas);
        let loadout = &mut s.loadout;
        self.slot = self.slot.min(LOADOUT_SLOTS - 1);
        if loadout.presets.is_empty() {
            loadout.presets.push(LoadoutPreset {
                name: "PRESET 1".into(),
                loadout: loadout.current,
            });
        }
        self.preset = self.preset.min(loadout.presets.len() - 1);
        if loadout.current.slots[self.slot].is_none() && matches!(self.picker, Picker::Ability(_)) {
            // No weapon left to pick an ability for.
            self.picker = Picker::None;
        }

        ui.push_id("##loadout");
        // Escape / B closes a picker (before drawing: its opener takes the focus this frame)
        // or leaves.
        let back = ui.back_pressed();
        if back && self.picker != Picker::None {
            self.close_picker();
            r.sound = Some(Sfx::UiBack);
        } else if back {
            r.back = true;
        }
        if let Some(label) = self.focus.take() {
            ui.request_focus(&label);
        }
        // Left / Right only move a focus that was there when the frame began.
        self.focus_start = ui.state().focused();
        let card_visible = !matches!(self.picker, Picker::Ability(_) | Picker::Knack);
        let hole = card_visible.then_some(lay.card);
        let theme = ui.theme();
        draw_frame(ui.painter(), theme, canvas, &lay, hole);

        let picking = self.picker != Picker::None;
        let knack_key = s
            .bindings
            .hint(Action::ThrowThorn)
            .filter(|k| k.len() <= 3)
            .unwrap_or_else(|| "Q".into());
        ui.enabled_scope(!picking, |ui| {
            self.slot_cards(ui, &lay, &loadout.current, &knack_key, &mut r);
        });
        let held = loadout.current.slots[self.slot];
        let hints = InputHints::new(&s.bindings, self.slot);
        // What turns on the card: the slot's weapon, or the picker's candidate.
        let mut shown = held.map(|b| (b.weapon, b.abilities[0]));
        match self.picker {
            Picker::None => {
                self.showcase(ui, &lay, held, &mut r);
                if let Some(build) = held {
                    if let Some(glow) = self.ability_boxes(ui, &lay, build, &hints, &mut r) {
                        shown = Some((build.weapon, glow));
                    }
                } else {
                    let dim = TextStyle::new(ui.theme().ash);
                    let at = vec2(lay.boxes[0].min.x + BOX_PAD, lay.boxes[0].min.y + BOX_PAD);
                    ui.painter()
                        .text(at, "NO WEAPON ON THIS KEY, NO ABILITIES.", &dim);
                }
            }
            Picker::Weapon => {
                let peek = self.weapon_grid(ui, &lay, &mut loadout.current, &mut r);
                shown = peek.map(|w| (w, w.default_build().abilities[0]));
                self.peek_info(ui, &lay, peek, &loadout.current);
            }
            Picker::Ability(k) => {
                if let Some(build) = held {
                    self.ability_grid(ui, &lay, k, build, &mut loadout.current, &mut r);
                }
            }
            Picker::Knack => self.knack_grid(ui, &lay, &mut loadout.current, &knack_key, &mut r),
        }
        ui.enabled_scope(!picking, |ui| {
            r.changed |= self.presets(ui, &lay, loadout, &mut r.sound);
        });
        self.footer(ui, &lay, loadout, &mut r);

        // The turntable.
        match shown.filter(|_| card_visible) {
            Some((weapon, glow)) => {
                self.turntable(ui, lay.card, dt as f32);
                draw_turning_weapon(
                    preview, canvas, lay.card, weapon, glow, self.spin, self.time,
                );
            }
            None => {
                preview.viewmodel.clear();
                if card_visible {
                    let style = TextStyle::new(ui.theme().ash).centered();
                    ui.painter().text_in(lay.card, "EMPTY SLOT", &style);
                }
            }
        }

        ui.pop_id();
        r
    }

    /// Left / Right on control `own` (focused since the frame began, so a jump never chains)
    /// move the focus to these controls (by label).
    fn nav(&self, ui: &mut Ui, own: &str, left: Option<&str>, right: Option<&str>) {
        if self.focus_start != Some(ui.id(own)) {
            return;
        }
        let target = if ui.input().key(UiKey::Left) {
            left
        } else if ui.input().key(UiKey::Right) {
            right
        } else {
            None
        };
        if let Some(target) = target {
            ui.request_focus(target);
        }
    }

    fn close_picker(&mut self) {
        self.focus = Some(match self.picker {
            Picker::Ability(k) => box_label(k),
            Picker::Knack => KNACK_CARD.into(),
            _ => CHANGE_WEAPON.into(),
        });
        self.picker = Picker::None;
    }

    // ---- left column ----------------------------------------------------------------------

    /// The four slot cards and the knack card. Clicking a slot card (or walking onto it with
    /// the keys) selects the slot; Enter on one opens its weapon picker; the knack card opens
    /// the knack picker; Right jumps to the showcase.
    fn slot_cards(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        loadout: &Loadout,
        knack_key: &str,
        r: &mut LoadoutResponse,
    ) {
        heading(ui, lay.left.min, lay.left.width(), "SLOTS", "");
        let walked =
            ui.input().key(UiKey::Up) || ui.input().key(UiKey::Down) || ui.input().key(UiKey::Tab);
        for i in 0..LOADOUT_SLOTS {
            let rect = lay.slot_card(i);
            let id = ui.id(&slot_label(i));
            let resp = ui.interact(id, rect, true);
            let build = loadout.slot(i as u8);
            let (name, line) = match build {
                Some(b) => (
                    b.weapon.name().to_uppercase(),
                    ability_pair(b, rect.width() - 28.0),
                ),
                None => ("EMPTY".to_string(), "NOTHING ON THIS KEY".to_string()),
            };
            draw_card(
                ui,
                rect,
                &resp,
                i == self.slot,
                &(i + 1).to_string(),
                &name,
                &line,
            );
            if !ui.is_enabled() {
                continue;
            }
            let pick = resp.focused && ui.input().key(UiKey::Enter);
            if resp.clicked || pick || (resp.focused && walked) {
                if self.slot != i {
                    r.sound = Some(Sfx::UiMove);
                }
                self.slot = i;
            }
            if pick {
                self.open_weapon_picker(build.copied());
                r.sound = Some(Sfx::UiSelect);
            }
            if resp.focused {
                self.nav(ui, &slot_label(i), None, Some(CHANGE_WEAPON));
            }
        }
        let rect = lay.slot_card(LOADOUT_SLOTS);
        let id = ui.id(KNACK_CARD);
        let resp = ui.interact(id, rect, true);
        let k = loadout.knack;
        let line = loadout_text::knack_line(k, &self.columns[0].tuning.combat);
        let name = k.name().to_uppercase();
        draw_card(ui, rect, &resp, false, knack_key, &name, &line);
        if !ui.is_enabled() {
            return;
        }
        if resp.activated {
            self.picker = Picker::Knack;
            self.focus = Some(knack_label(k));
            r.sound = Some(Sfx::UiSelect);
        }
        if resp.focused {
            self.nav(ui, KNACK_CARD, None, Some(CHANGE_WEAPON));
        }
    }

    /// Presets: the PRESET choice, LOAD / SAVE / NEW / DELETE, the NAME field, and the gun game
    /// note. Returns whether anything changed.
    fn presets(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        s: &mut LoadoutSettings,
        sound: &mut Option<Sfx>,
    ) -> bool {
        let mut changed = false;
        let top = lay.slot_card(LOADOUT_SLOTS).max.y + 9.0;
        let area = Rect::new(vec2(lay.left.min.x, top), lay.left.max);
        ui.area(area, |ui| {
            let y = heading(ui, area.min, area.width(), "PRESETS", "");
            ui.spacing(y - area.min.y);
            let names: Vec<String> = s.presets.iter().map(|p| p.name.clone()).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            if ui.choice("PRESET", &mut self.preset, &names) {
                *sound = Some(Sfx::UiMove);
            }
            let preset = self.preset;
            ui.row(2, |ui| {
                if ui.button("LOAD") {
                    s.current = s.presets[preset].loadout;
                    changed = true;
                }
                if ui.button("SAVE") {
                    s.presets[preset].loadout = s.current;
                    changed = true;
                }
            });
            ui.row(2, |ui| {
                ui.enabled_scope(s.presets.len() < MAX_PRESETS, |ui| {
                    if ui.button("NEW") {
                        s.presets.push(LoadoutPreset {
                            name: format!("PRESET {}", s.presets.len() + 1),
                            loadout: s.current,
                        });
                        self.preset = s.presets.len() - 1;
                        changed = true;
                    }
                });
                ui.enabled_scope(s.presets.len() > 1, |ui| {
                    if ui.button("DELETE") {
                        s.presets.remove(preset);
                        self.preset = preset.min(s.presets.len() - 1);
                        changed = true;
                    }
                });
            });
            let last = s.presets.len() - 1;
            let p = &mut s.presets[self.preset.min(last)];
            let edit = ui.text_field("NAME", &mut p.name, PRESET_NAME_LEN, TextFilter::Name);
            if edit.changed {
                p.name = p.name.to_uppercase();
                changed = true;
            }
            ui.spacing(4.0);
            let dim = TextStyle::new(ui.theme().ash);
            ui.text_wrapped(
                "GUN GAME HANDS OUT ITS OWN WEAPONS: IT KEEPS ONLY YOUR KNACK.",
                &dim,
            );
        });
        if changed {
            *sound = Some(Sfx::UiSelect);
        }
        changed
    }

    // ---- showcase ---------------------------------------------------------------------------

    /// The selected slot's weapon: name, class, role, numbers and CHANGE WEAPON.
    fn showcase(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        held: Option<WeaponBuild>,
        r: &mut LoadoutResponse,
    ) {
        let t = ui.theme();
        let info = lay.info;
        let mut y = match held {
            Some(b) => weapon_header(ui, info, b.weapon, &self.columns),
            None => {
                let big = TextStyle::new(t.bone).scale(2.0).shadow(t.crimson_dark);
                ui.painter().text(info.min, "EMPTY SLOT", &big);
                let dim = TextStyle::new(t.bone_dim);
                let text = "Nothing on this key: switching weapons skips it. Pick a weapon to \
                            fill it.";
                paragraph(
                    ui.painter(),
                    info.min + vec2(0.0, 22.0),
                    info.width(),
                    text,
                    3,
                    &dim,
                );
                info.min.y + 22.0 + 3.0 * LINE_H
            }
        };
        y = y.max(info.max.y - 16.0);
        let button = Rect::new(vec2(info.min.x, y), vec2(info.max.x, info.max.y));
        let mut open = false;
        ui.area(button, |ui| {
            if ui.button(CHANGE_WEAPON) {
                open = true;
            }
        });
        let id = ui.id(CHANGE_WEAPON);
        if ui.state().focused() == Some(id) {
            let boxes = held.map(|_| box_label(0));
            self.nav(
                ui,
                CHANGE_WEAPON,
                Some(&slot_label(self.slot)),
                boxes.as_deref(),
            );
        }
        if open {
            self.open_weapon_picker(held);
            r.sound = Some(Sfx::UiSelect);
        }
    }

    fn open_weapon_picker(&mut self, held: Option<WeaponBuild>) {
        self.picker = Picker::Weapon;
        self.focus = Some(match held {
            Some(b) => weapon_label(b.weapon),
            None => EMPTY_TILE.into(),
        });
    }

    /// The two ability boxes. Returns the ability under the cursor / focus (the preview glows
    /// in its colour).
    fn ability_boxes(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        build: WeaponBuild,
        hints: &InputHints,
        r: &mut LoadoutResponse,
    ) -> Option<Ability> {
        let lower = lay.lower;
        let right = format!(
            "ALT FIRE = {}   SWITCH A/B: {} AGAIN",
            hints.alt_fire, hints.slot_key
        );
        heading(ui, lower.min, lower.width(), "ABILITIES", &right);
        let mut lit = None;
        for k in 0..2 {
            let label = box_label(k);
            let id = ui.id(&label);
            let resp = ui.interact(id, lay.boxes[k], true);
            let a = build.abilities[k];
            let badge = if k == 0 { "A" } else { "B" };
            let look = BoxLook {
                badge: Some(badge),
                selected: false,
                tag: "CHANGE",
            };
            let explain = Explain::ability(a, build.weapon, &self.columns);
            explain_box(ui, lay.boxes[k], &resp, look, &explain);
            if resp.hovered || resp.focused {
                lit = Some(a);
            }
            if resp.focused {
                let (left, right) = if k == 0 {
                    (slot_label(self.slot), Some(box_label(1)))
                } else {
                    (box_label(0), None)
                };
                self.nav(ui, &label, Some(&left), right.as_deref());
            }
            if resp.activated {
                self.picker = Picker::Ability(k);
                self.focus = Some(ability_label(a));
                r.sound = Some(Sfx::UiSelect);
            }
        }
        lit
    }

    // ---- pickers ----------------------------------------------------------------------------

    /// The weapon picker under the showcase: a column per class, a tile per weapon, and EMPTY
    /// SLOT. Picking applies the editing rules and closes it. Returns the weapon to preview
    /// (the tile under the cursor / focus; `None`: the empty slot).
    fn weapon_grid(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        loadout: &mut Loadout,
        r: &mut LoadoutResponse,
    ) -> Option<WeaponKind> {
        let lower = lay.lower;
        let slot = self.slot;
        let current = loadout.slot(slot as u8).map(|b| b.weapon);
        let mut peek = current;
        let title = format!("WEAPON FOR SLOT {}", slot + 1);
        let top = heading(
            ui,
            lower.min,
            lower.width(),
            &title,
            "ENTER EQUIPS   ESC CANCELS",
        );
        let classes = WeaponClass::ALL;
        let n = classes.len() as f32;
        let col_w = ((lower.width() - (n - 1.0) * 4.0) / n).floor();
        let t = ui.theme();
        let mut picked: Option<Option<WeaponKind>> = None;
        // Tile ids per column, for Left / Right.
        let columns: Vec<Vec<WeaponKind>> = classes
            .iter()
            .map(|c| {
                WeaponKind::ALL
                    .into_iter()
                    .filter(|w| w.class() == *c)
                    .collect()
            })
            .collect();
        for (ci, (class, weapons)) in classes.iter().zip(&columns).enumerate() {
            let x = (lower.min.x + ci as f32 * (col_w + 4.0)).round();
            let dim = TextStyle::new(t.bone_dim);
            ui.painter().text(vec2(x + 2.0, top), class.name(), &dim);
            draw::hline(ui.painter(), x, x + col_w, top + 10.0, t.panel_edge);
            for (ri, &w) in weapons.iter().enumerate() {
                let rect = Rect::from_min_size(
                    vec2(x, top + 13.0 + ri as f32 * (TILE_H + 3.0)),
                    vec2(col_w, TILE_H),
                );
                let id = ui.id(&weapon_label(w));
                let resp = ui.interact(id, rect, true);
                let tag = match loadout.slot_of(w) {
                    Some(s) if s as usize == slot => Some("EQUIPPED".to_string()),
                    Some(s) => Some(format!("ON SLOT {}", s + 1)),
                    None => None,
                };
                draw_tile(
                    ui,
                    rect,
                    &resp,
                    current == Some(w),
                    &w.name().to_uppercase(),
                    tag.as_deref(),
                );
                if resp.hovered || resp.focused {
                    peek = Some(w);
                }
                if resp.activated {
                    picked = Some(Some(w));
                }
                if resp.focused {
                    let jump = |d: isize| {
                        let c = ci as isize + d;
                        let col = columns.get(usize::try_from(c).ok()?)?;
                        col.get(ri.min(col.len().saturating_sub(1))).copied()
                    };
                    let left = jump(-1).map(weapon_label);
                    let right = jump(1).map(weapon_label);
                    self.nav(ui, &weapon_label(w), left.as_deref(), right.as_deref());
                }
            }
        }
        // The empty slot (not for the last weapon).
        let bottom = Rect::new(vec2(lower.min.x, lower.max.y - 16.0), lower.max);
        let can_empty = current.is_none() || loadout.builds().count() > 1;
        let empty = Rect::from_min_size(bottom.min, vec2((col_w * 1.6).round(), 16.0));
        let id = ui.id(EMPTY_TILE);
        let resp = ui.enabled_scope(can_empty, |ui| {
            let resp = ui.interact(id, empty, true);
            draw_tile(ui, empty, &resp, current.is_none(), "EMPTY SLOT", None);
            resp
        });
        if resp.hovered || resp.focused {
            peek = None;
        }
        if resp.activated && can_empty {
            picked = Some(None);
        }
        if !can_empty {
            let dim = TextStyle::new(t.ash);
            ui.painter().text(
                vec2(empty.max.x + 6.0, empty.min.y + 4.0),
                "YOUR LAST WEAPON STAYS",
                &dim,
            );
        }
        if let Some(choice) = picked {
            if set_weapon(loadout, slot, choice) {
                r.changed = true;
            }
            self.close_picker();
            r.sound = Some(Sfx::UiSelect);
        }
        peek
    }

    /// The showcase's panel while the weapon picker is open: the candidate's name, class,
    /// role, numbers, the abilities it comes with and what picking it does.
    fn peek_info(&self, ui: &mut Ui, lay: &Layout, peek: Option<WeaponKind>, loadout: &Loadout) {
        let t = ui.theme();
        let info = lay.info;
        let dim = TextStyle::new(t.bone_dim);
        let Some(w) = peek else {
            let big = TextStyle::new(t.bone).scale(2.0).shadow(t.crimson_dark);
            ui.painter().text(info.min, "EMPTY SLOT", &big);
            let text = "Nothing on this key: switching weapons skips it. Your last weapon \
                        can't be emptied.";
            paragraph(
                ui.painter(),
                info.min + vec2(0.0, 22.0),
                info.width(),
                text,
                3,
                &dim,
            );
            return;
        };
        let y = weapon_header(ui, info, w, &self.columns);
        let held = loadout.slot(self.slot as u8);
        let note = if held.is_some_and(|b| b.weapon == w) {
            "EQUIPPED ON THIS SLOT".to_string()
        } else if let Some(other) = loadout.slot_of(w) {
            format!("ON SLOT {}: EQUIPPING SWAPS THE TWO", other + 1)
        } else {
            let [a, b] = w.default_build().abilities;
            format!("COMES WITH {} + {}", a.name(), b.name()).to_uppercase()
        };
        let line = truncate(&note, info.width(), 1.0);
        let at = vec2(info.min.x, y.max(info.max.y - 12.0));
        ui.painter().text(at, &line, &TextStyle::new(t.bone));
    }

    /// The ability picker: the class's pool as four boxes; picking one applies the editing
    /// rules (the other ability: the two swap) and closes it.
    fn ability_grid(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        k: usize,
        build: WeaponBuild,
        loadout: &mut Loadout,
        r: &mut LoadoutResponse,
    ) {
        let pool = build.weapon.abilities();
        let title = format!(
            "ABILITY {} FOR THE {}",
            if k == 0 { "A" } else { "B" },
            build.weapon.name().to_uppercase()
        );
        let right = format!("{} POOL   ESC CANCELS", build.weapon.class().name());
        heading(ui, lay.right.min, lay.right.width(), &title, &right);
        let cells = lay.grid();
        let mut picked = None;
        for (i, (&a, &rect)) in pool.iter().zip(&cells).enumerate() {
            let id = ui.id(&ability_label(a));
            let resp = ui.interact(id, rect, true);
            let badge = build
                .abilities
                .iter()
                .position(|&b| b == a)
                .map(|slot| if slot == 0 { "A" } else { "B" });
            let tag = match badge {
                Some(_) if build.abilities[k] == a => "EQUIPPED",
                Some(_) => "SWAPS",
                None => "PICK",
            };
            let look = BoxLook {
                badge,
                selected: build.abilities[k] == a,
                tag,
            };
            let explain = Explain::ability(a, build.weapon, &self.columns);
            explain_box(ui, rect, &resp, look, &explain);
            if resp.activated {
                picked = Some(a);
            }
            if resp.focused {
                // 2 × 2: Left / Right across a row.
                let other = pool.get(i ^ 1).copied().map(ability_label);
                let (left, right) = if i % 2 == 0 {
                    (None, other)
                } else {
                    (other, None)
                };
                self.nav(ui, &ability_label(a), left.as_deref(), right.as_deref());
            }
        }
        if let Some(a) = picked {
            r.changed |= set_ability(loadout, self.slot, k, a);
            self.close_picker();
            r.sound = Some(Sfx::UiSelect);
        }
    }

    /// The knack picker: a tile per knack (PvE-only ones say so) and the box explaining the
    /// one under the cursor / focus. Picking puts it on Q and closes it.
    fn knack_grid(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        loadout: &mut Loadout,
        knack_key: &str,
        r: &mut LoadoutResponse,
    ) {
        let title = format!("KNACK ({knack_key})");
        let right = "PVE-ONLY KNACKS BECOME SURGE IN PVP   ESC CANCELS";
        heading(ui, lay.right.min, lay.right.width(), &title, right);
        let current = loadout.knack;
        let mut peek = current;
        let mut picked = None;
        for (i, k) in Knack::ALL.into_iter().enumerate() {
            let rect = lay.knack_tile(i);
            let id = ui.id(&knack_label(k));
            let resp = ui.interact(id, rect, true);
            let line = loadout_text::knack_line(k, &self.columns[0].tuning.combat);
            draw_card(
                ui,
                rect,
                &resp,
                k == current,
                "",
                &k.name().to_uppercase(),
                &line,
            );
            if resp.hovered || resp.focused {
                peek = k;
            }
            if resp.activated {
                picked = Some(k);
            }
        }
        let look = BoxLook {
            badge: Some(knack_key),
            selected: peek == current,
            tag: if peek == current { "EQUIPPED" } else { "PICK" },
        };
        let detail = Response::default();
        explain_box(
            ui,
            lay.knack_detail(),
            &detail,
            look,
            &Explain::knack(peek, &self.columns),
        );
        if let Some(k) = picked {
            r.changed |= set_knack(loadout, k);
            self.close_picker();
            r.sound = Some(Sfx::UiSelect);
        }
    }

    // ---- footer and turntable ---------------------------------------------------------------

    /// The note or the controls hint, DEFAULT LOADOUT and BACK (CANCEL while picking).
    fn footer(
        &mut self,
        ui: &mut Ui,
        lay: &Layout,
        s: &mut LoadoutSettings,
        r: &mut LoadoutResponse,
    ) {
        let t = ui.theme();
        let foot = lay.foot;
        let buttons_w = 196.0f32.min(foot.width() * 0.45).round();
        let (text_rect, buttons) = foot.split_right(buttons_w);
        let picking = self.picker != Picker::None;
        let (text, color) = match self.note {
            Some(note) => (note, t.bone),
            None if picking => ("ENTER / CLICK PICKS   ESC / B CANCELS", t.ash),
            None => (
                "ENTER / A CHANGES   DRAG OR SHOULDERS TURN THE WEAPON",
                t.ash,
            ),
        };
        let text = truncate(text, text_rect.width() - 8.0, 1.0);
        ui.painter()
            .text_in(text_rect, &text, &TextStyle::new(color));
        ui.area(buttons, |ui| {
            ui.row_weights(&[3.0, 2.0], |ui| {
                ui.enabled_scope(!picking, |ui| {
                    if ui.button("DEFAULT LOADOUT") {
                        s.current = Loadout::default();
                        r.changed = true;
                        r.sound = Some(Sfx::UiSelect);
                    }
                });
                if ui.button(if picking {
                    "CANCEL##back"
                } else {
                    "BACK##back"
                }) {
                    if picking {
                        self.close_picker();
                        r.sound = Some(Sfx::UiBack);
                    } else {
                        r.back = true;
                    }
                }
            });
        });
    }

    /// Drag on the card, PageUp / PageDown (shoulders), or the idle spin.
    fn turntable(&mut self, ui: &mut Ui, card: Rect, dt: f32) {
        let id = ui.id("##turn");
        let resp = ui.interact(id, card, false);
        let mouse_x = ui.input().mouse_pos.x;
        if resp.held {
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
        let t = ui.theme();
        let hint = TextStyle::new(fade(t.bone_dim, 0.8)).centered();
        ui.painter().text(
            vec2(card.center().x, card.max.y - 12.0),
            "DRAG TO TURN",
            &hint,
        );
    }
}

/// Ids of the screen's controls (inside its id scope).
const CHANGE_WEAPON: &str = "CHANGE WEAPON##change";
const EMPTY_TILE: &str = "##empty";
const KNACK_CARD: &str = "##knack";

fn slot_label(i: usize) -> String {
    format!("##slot{i}")
}

fn box_label(k: usize) -> String {
    format!("##box{k}")
}

fn weapon_label(w: WeaponKind) -> String {
    format!("##weapon{}", w.id())
}

fn ability_label(a: Ability) -> String {
    format!("##ability{}", a.id())
}

fn knack_label(k: Knack) -> String {
    format!("##knack{}", k.id())
}

/// Short names of the inputs the boxes mention ("MOUSE 2", "1").
struct InputHints {
    alt_fire: String,
    slot_key: String,
}

impl InputHints {
    fn new(b: &crate::controls::Bindings, slot: usize) -> Self {
        let slot_action = [
            Action::Weapon1,
            Action::Weapon2,
            Action::Weapon3,
            Action::Weapon4,
        ][slot.min(LOADOUT_SLOTS - 1)];
        Self {
            alt_fire: b.hint(Action::AltFire).unwrap_or_else(|| "UNBOUND".into()),
            slot_key: b
                .hint(slot_action)
                .unwrap_or_else(|| (slot + 1).to_string()),
        }
    }
}

/// "BRAND / SPLINTER", shortened to the first words of long names when it doesn't fit.
fn ability_pair(b: &WeaponBuild, width: f32) -> String {
    let [a, c] = b.abilities.map(|a| a.name().to_uppercase());
    let full = format!("{a} / {c}");
    if line_width(&full, 1.0) <= width {
        return full;
    }
    let first = |s: &str| s.split(' ').next().unwrap_or_default().to_string();
    truncate(&format!("{} / {}", first(&a), first(&c)), width, 1.0)
}

/// Draws `text` (capitalised) wrapped at `width`, at most `max_lines` (the last one cut with
/// `..` when there is more). Returns the height used.
fn paragraph(
    list: &mut UiDrawList,
    pos: Vec2,
    width: f32,
    text: &str,
    max_lines: usize,
    style: &TextStyle,
) -> f32 {
    let upper = text.to_uppercase();
    let lines = wrap_text(&upper, width, 1.0);
    let n = lines.len().min(max_lines);
    for (i, line) in lines.iter().take(n).enumerate() {
        let y = pos.y + i as f32 * LINE_H;
        if i + 1 == n && lines.len() > n {
            let cut = truncate(&format!("{line} {}", lines[n]), width, 1.0);
            list.text(vec2(pos.x, y), &cut, style);
        } else {
            list.text(vec2(pos.x, y), line, style);
        }
    }
    n as f32 * LINE_H
}

/// A section heading at `pos` (crimson tab, text, a rule) with dim text on the right. Returns
/// the y under it.
fn heading(ui: &mut Ui, pos: Vec2, width: f32, text: &str, right: &str) -> f32 {
    let t = ui.theme();
    let list = ui.painter();
    let y = pos.y + 3.0;
    draw::fill(
        list,
        Rect::from_min_size(vec2(pos.x, y), vec2(3.0, 7.0)),
        t.crimson_bright,
    );
    let r = list.text(
        vec2(pos.x + 6.0, y),
        text,
        &TextStyle::new(t.bone).shadow(t.shadow),
    );
    let mut rule_end = pos.x + width;
    if !right.is_empty() {
        let room = pos.x + width - r.max.x - 14.0;
        let shown = truncate(right, room, 1.0);
        let style = TextStyle::new(t.ash).right();
        let rr = list.text(vec2(pos.x + width, y), &shown, &style);
        rule_end = rr.min.x - 6.0;
    }
    if r.max.x + 6.0 < rule_end {
        draw::hline(list, r.max.x + 5.0, rule_end, y + 3.0, t.panel_edge);
    }
    pos.y + HEADING_H
}

/// The page: dimmed surroundings, the panel body (with a window for the turning weapon), its
/// title bar and frame.
fn draw_frame(list: &mut UiDrawList, t: &Theme, canvas: Rect, lay: &Layout, hole: Option<Rect>) {
    let frame = lay.frame;
    fill_around(list, canvas, frame, fade(t.ink, 0.6));
    // The body is opaque, unlike a panel: nothing of the HUD or the world (the warm-up's lit
    // ready board) reads through it.
    for color in [t.ink, t.panel] {
        match hole {
            Some(h) => fill_around(list, frame, h, color),
            None => draw::fill(list, frame, color),
        }
    }
    draw::border(list, frame, 1.0, t.panel_edge);
    // Title bar, as a panel's.
    let bar = Rect::new(frame.min, vec2(frame.max.x, frame.min.y + TITLE_H));
    draw::fill(list, bar, t.crimson_dark);
    let title = "LOADOUT";
    let slab = Rect::new(
        bar.min,
        vec2(
            bar.min.x + line_width(title, 1.0) + 22.0 + bar.height(),
            bar.max.y,
        ),
    );
    draw::slanted(list, slab, true, t.crimson);
    let h = bar.height() - 6.0;
    for i in 0..3 {
        let x = slab.max.x - 2.0 + i as f32 * 7.0;
        draw::parallelogram(
            list,
            x,
            bar.min.y + 3.0,
            3.0,
            h,
            fade(t.crimson, 0.7 - i as f32 * 0.2),
        );
        let xr = bar.max.x - 30.0 + i as f32 * 7.0;
        draw::parallelogram(
            list,
            xr,
            bar.min.y + 3.0,
            3.0,
            h,
            fade(t.crimson, 0.3 + i as f32 * 0.2),
        );
    }
    draw::hline(list, frame.min.x, frame.max.x, bar.max.y, t.crimson);
    let style = TextStyle::new(t.bone).shadow(t.crimson_dark);
    list.text(vec2(bar.min.x + 8.0, bar.min.y + 4.0), title, &style);
    draw::corner_brackets(list, frame.expand(2.0), 7.0, 1.0, fade(t.bone_dim, 0.8));
    if let Some(h) = hole {
        draw::border(list, h, 1.0, t.panel_edge);
        draw::corner_brackets(list, h.expand(1.0), 5.0, 1.0, fade(t.bone_dim, 0.7));
    }
}

/// Fills `outer` except `hole` (four bands).
fn fill_around(list: &mut UiDrawList, outer: Rect, hole: Rect, color: Vec4) {
    let hole = hole.intersect(outer);
    if hole.is_empty() {
        draw::fill(list, outer, color);
        return;
    }
    for r in [
        Rect::new(outer.min, vec2(outer.max.x, hole.min.y)),
        Rect::new(vec2(outer.min.x, hole.max.y), outer.max),
        Rect::new(vec2(outer.min.x, hole.min.y), vec2(hole.min.x, hole.max.y)),
        Rect::new(vec2(hole.max.x, hole.min.y), vec2(outer.max.x, hole.max.y)),
    ] {
        if !r.is_empty() {
            draw::fill(list, r, color);
        }
    }
}

/// The focus look shared by the screen's cards: a red edge and bar, corner brackets.
fn draw_focus(list: &mut UiDrawList, t: &Theme, rect: Rect, resp: &Response, enabled: bool) {
    if resp.focused && enabled {
        draw::border(list, rect, 1.0, t.crimson_bright);
        draw::fill(
            list,
            Rect::new(rect.min, vec2(rect.min.x + 2.0, rect.max.y)),
            t.crimson_bright,
        );
        draw::corner_brackets(list, rect.expand(1.0), 4.0, 1.0, t.bone);
    } else {
        draw::border(list, rect, 1.0, t.panel_edge);
    }
}

/// A key badge: the key in a small box.
fn badge(list: &mut UiDrawList, t: &Theme, pos: Vec2, key: &str, lit: bool) {
    let w = line_width(key, 1.0).max(5.0) + 6.0;
    let rect = Rect::from_min_size(pos, vec2(w, 11.0));
    draw::fill(list, rect, if lit { t.crimson } else { t.ink });
    draw::border(
        list,
        rect,
        1.0,
        if lit { t.crimson_bright } else { t.bone_dim },
    );
    let style = TextStyle::new(t.bone).centered();
    list.text_in(rect, key, &style);
}

/// A slot (or knack) card: key badge, name, a second line.
fn draw_card(
    ui: &mut Ui,
    rect: Rect,
    resp: &Response,
    selected: bool,
    key: &str,
    name: &str,
    line: &str,
) {
    let t = ui.theme();
    let enabled = ui.is_enabled();
    let list = ui.painter();
    let bg = if selected {
        fade(t.crimson, 0.5)
    } else if resp.hovered {
        t.widget_hover
    } else {
        t.widget
    };
    draw::fill(list, rect, bg);
    draw_focus(list, t, rect, resp, enabled);
    if selected {
        draw::fill(
            list,
            Rect::new(vec2(rect.max.x - 3.0, rect.min.y), rect.max),
            t.crimson_bright,
        );
    }
    // (No key: no badge.)
    let x = if key.is_empty() {
        rect.min.x + 8.0
    } else {
        badge(list, t, rect.min + vec2(5.0, 8.0), key, selected);
        rect.min.x + 22.0
    };
    let width = rect.max.x - x - 6.0;
    let (main, sub) = if !enabled {
        (t.ash, t.ash)
    } else if selected || resp.focused {
        (t.bone, t.bone_dim)
    } else {
        (t.bone_dim, t.ash)
    };
    list.text(
        vec2(x, rect.min.y + 4.0),
        &truncate(name, width, 1.0),
        &TextStyle::new(main),
    );
    list.text(
        vec2(x, rect.min.y + 15.0),
        &truncate(line, width, 1.0),
        &TextStyle::new(sub),
    );
}

/// A weapon tile in the picker: its name, and a tag under it (where it is equipped).
fn draw_tile(
    ui: &mut Ui,
    rect: Rect,
    resp: &Response,
    current: bool,
    name: &str,
    tag: Option<&str>,
) {
    let t = ui.theme();
    let enabled = ui.is_enabled();
    let list = ui.painter();
    let bg = if current {
        fade(t.crimson, 0.5)
    } else if resp.hovered && enabled {
        t.widget_hover
    } else {
        t.widget
    };
    draw::fill(list, rect, bg);
    draw_focus(list, t, rect, resp, enabled);
    let color = if !enabled {
        t.ash
    } else if current || resp.focused || resp.hovered {
        t.bone
    } else {
        t.bone_dim
    };
    let width = rect.width() - 9.0;
    let y = if tag.is_some() {
        rect.min.y + 3.0
    } else {
        (rect.center().y - 4.0).round()
    };
    let shown = truncate(name, width, 1.0);
    list.text(vec2(rect.min.x + 6.0, y), &shown, &TextStyle::new(color));
    if let Some(tag) = tag {
        let sub = if enabled { t.bone_dim } else { t.ash };
        let shown = truncate(tag, width, 1.0);
        list.text(
            vec2(rect.min.x + 6.0, y + 10.0),
            &shown,
            &TextStyle::new(sub),
        );
    }
}

/// The weapon's name (big), class chip, role and numbers in the info panel. Returns the y
/// under them.
fn weapon_header(ui: &mut Ui, info: Rect, w: WeaponKind, columns: &[Column]) -> f32 {
    let t = ui.theme();
    let list = ui.painter();
    let big = TextStyle::new(t.bone).scale(2.0).shadow(t.crimson_dark);
    let name = list.text(info.min, &w.name().to_uppercase(), &big);
    // The class chip, right of the name.
    let class = w.class().name();
    let chip = Rect::from_min_size(
        vec2(name.max.x + 8.0, info.min.y + 3.0),
        vec2(line_width(class, 1.0) + 8.0, 11.0),
    );
    if chip.max.x <= info.max.x {
        draw::border(list, chip, 1.0, t.crimson);
        list.text_in(chip, class, &TextStyle::new(t.bone_dim).centered());
    }
    let dim = TextStyle::new(t.bone_dim);
    let mut y = info.min.y + 22.0;
    y += paragraph(
        list,
        vec2(info.min.x, y),
        info.width() - 4.0,
        loadout_text::weapon_role(w),
        ROLE_LINES,
        &dim,
    );
    y += 4.0;
    let stats: Vec<(&str, Vec<Stat>)> = columns
        .iter()
        .map(|c| (c.label, loadout_text::weapon_stats(w, &c.tuning.combat)))
        .collect();
    y + stat_table(list, t, info.min.x, info.max.x, y, &stats) + 4.0
}

/// A table of numbers: a header with the column labels, then a row per stat (label left,
/// one right-aligned value per column; a PvP value that differs is lit). Returns its height.
fn stat_table(
    list: &mut UiDrawList,
    t: &Theme,
    x0: f32,
    x1: f32,
    y: f32,
    columns: &[(&str, Vec<Stat>)],
) -> f32 {
    // Value columns of up to 11 characters; on narrow screens everything is cut to fit.
    let n = columns.len();
    let col_w = 70.0f32.min(((x1 - x0) / (n as f32 + 1.0)).floor());
    let label_w = x1 - x0 - n as f32 * col_w - 4.0;
    let right = |i: usize| x1 - (n - 1 - i) as f32 * col_w - 2.0;
    for (i, (label, _)) in columns.iter().enumerate() {
        list.text(vec2(right(i), y), label, &TextStyle::new(t.ash).right());
    }
    let rows = columns.first().map_or(0, |(_, s)| s.len());
    for row in 0..rows {
        let ry = y + 10.0 + row as f32 * LINE_H;
        let label = truncate(columns[0].1[row].0, label_w, 1.0);
        list.text(vec2(x0, ry), &label, &TextStyle::new(t.bone_dim));
        let first = &columns[0].1[row].1;
        for (i, (_, stats)) in columns.iter().enumerate() {
            let Some((_, value)) = stats.get(row) else {
                continue;
            };
            let color = if i == 0 || value != first {
                t.bone
            } else {
                t.bone_dim
            };
            let value = truncate(value, col_w - 4.0, 1.0);
            list.text(vec2(right(i), ry), &value, &TextStyle::new(color).right());
        }
    }
    10.0 + rows as f32 * LINE_H
}

/// How an ability box looks besides its ability.
struct BoxLook<'a> {
    /// "A" / "B": where it sits in the build.
    badge: Option<&'a str>,
    /// Lit (the picker's current choice).
    selected: bool,
    /// Right of the name: what activating it does.
    tag: &'a str,
}

/// What an explanation box says: an ability (the A / B boxes, the ability picker) or a knack.
struct Explain {
    name: String,
    /// Its colour, as a bar before the name (an ability's: the weapon glows in it).
    color: Option<Vec4>,
    /// How it is fired.
    input: &'static str,
    /// What it does and how to use it, and how many lines that may take.
    text: &'static str,
    lines: usize,
    /// A lit line under the text (a knack's PvP rule).
    note: Option<&'static str>,
    /// Its numbers per column.
    stats: Vec<(&'static str, Vec<Stat>)>,
}

impl Explain {
    fn ability(a: Ability, weapon: WeaponKind, columns: &[Column]) -> Explain {
        Explain {
            name: a.name().to_uppercase(),
            color: Some(ability_color(a)),
            input: loadout_text::ability_input(a, weapon),
            text: loadout_text::ability_text(a, weapon),
            lines: BOX_LINES,
            note: None,
            stats: columns
                .iter()
                .map(|c| {
                    let stats = loadout_text::ability_stats(a, weapon, &c.tuning, c.pvp);
                    (c.label, stats)
                })
                .collect(),
        }
    }

    fn knack(k: Knack, columns: &[Column]) -> Explain {
        Explain {
            name: k.name().to_uppercase(),
            color: None,
            input: loadout_text::knack_input(k),
            text: loadout_text::knack_text(k),
            lines: KNACK_LINES,
            note: (!k.pvp_allowed()).then_some("PVE ONLY: IN PVP YOU CARRY SURGE INSTEAD."),
            stats: columns
                .iter()
                .map(|c| (c.label, loadout_text::knack_stats(k, &c.tuning, c.pvp)))
                .collect(),
        }
    }
}

/// An explanation box: badge, colour, name, what activating does; how it fires; what it
/// does; its numbers per column at the bottom.
fn explain_box(ui: &mut Ui, rect: Rect, resp: &Response, look: BoxLook, e: &Explain) {
    let t = ui.theme();
    let enabled = ui.is_enabled();
    let list = ui.painter();
    let bg = if look.selected {
        fade(t.crimson, 0.35)
    } else if resp.hovered {
        t.widget_hover
    } else {
        t.widget
    };
    draw::fill(list, rect, bg);
    draw_focus(list, t, rect, resp, enabled);
    let inner = rect.shrink(BOX_PAD);
    let mut y = rect.min.y + 5.0;
    let mut x = inner.min.x;
    if let Some(b) = look.badge {
        badge(list, t, vec2(x, y), b, look.selected || resp.focused);
        x += 16.0;
    }
    if let Some(color) = e.color {
        let swatch = Rect::from_min_size(vec2(x, y + 2.0), vec2(3.0, 7.0));
        draw::fill(list, swatch, color);
        x += 7.0;
    }
    list.text(
        vec2(x, y + 2.0),
        &e.name,
        &TextStyle::new(t.bone).shadow(t.shadow),
    );
    let tag_color = if resp.focused || resp.hovered {
        t.bone
    } else {
        t.ash
    };
    list.text(
        vec2(inner.max.x, y + 2.0),
        look.tag,
        &TextStyle::new(tag_color).right(),
    );
    y += 15.0;
    // How it fires, after a red marker.
    draw::fill(
        list,
        Rect::from_min_size(vec2(inner.min.x, y + 3.0), vec2(4.0, 3.0)),
        t.crimson_bright,
    );
    let input = truncate(e.input, inner.width() - 7.0, 1.0);
    list.text(vec2(inner.min.x + 7.0, y), &input, &TextStyle::new(t.bone));
    y += LINE_H + 1.0;
    let body = TextStyle::new(t.bone_dim);
    y += paragraph(
        list,
        vec2(inner.min.x, y),
        inner.width(),
        e.text,
        e.lines,
        &body,
    );
    if let Some(note) = e.note {
        let note = truncate(note, inner.width(), 1.0);
        list.text(vec2(inner.min.x, y + 3.0), &note, &TextStyle::new(t.bone));
    }
    // The numbers sit at the bottom of the box.
    let rows = e.stats.first().map_or(0, |(_, s)| s.len());
    let table_y = inner.max.y - 10.0 - rows as f32 * LINE_H + 2.0;
    stat_table(list, t, inner.min.x, inner.max.x, table_y, &e.stats);
}

/// Draws `weapon` turning on the card (canvas rect `card`) into the viewmodel pass: the ready
/// board's slate stretched behind it hides the world, a lamp lights it, its glow parts take
/// `glow`'s colour.
fn draw_turning_weapon(
    p: Preview,
    canvas: Rect,
    card: Rect,
    weapon: WeaponKind,
    glow: Ability,
    spin: f32,
    time: f64,
) {
    p.viewmodel.clear();
    let Some(lib) = p.lib else {
        return;
    };
    let tan = (p.fov.clamp(30.0, 100.0).to_radians() * 0.5).tan();
    let aspect = canvas.width() / canvas.height().max(1.0);
    // The view-space point at depth `d` under canvas point `c`, and a canvas unit there.
    let at = |c: Vec2, d: f32| {
        let nx = (c.x - canvas.min.x) / canvas.width() * 2.0 - 1.0;
        let ny = 1.0 - (c.y - canvas.min.y) / canvas.height() * 2.0;
        Vec3::new(nx * d * tan * aspect, ny * d * tan, -d)
    };
    let unit = |d: f32| 2.0 * d * tan / canvas.height().max(1.0);
    let depth = 2.0;
    // The card: the slate (5.6 × 2.5 m around (0, 1.45)) stretched over it, behind.
    if let Some(slate) = lib.get(ModelId::ReadyBoard).parts.get(1) {
        let back = depth + 1.5;
        let size = (card.size() + Vec2::splat(2.0)) * unit(back);
        let transform = Mat4::from_translation(at(card.center(), back))
            * Mat4::from_scale(Vec3::new(size.x / 5.6, size.y / 2.5, 1.0))
            * Mat4::from_translation(-Vec3::new(0.0, 1.45, -0.045));
        p.viewmodel.push(DrawItem {
            mesh: slate.mesh,
            texture: slate.texture,
            transform,
            tint: Vec4::new(0.9, 0.72, 0.7, 1.0),
            unlit: true,
        });
    }
    // The weapon, fitted to the card whichever way it faces.
    let (lo, hi) = lib.bounds(weapon.model());
    let ext = (hi - lo).max(Vec3::splat(0.01));
    let mid = (lo + hi) * 0.5;
    let across = (ext.x * ext.x + ext.z * ext.z).sqrt();
    let tall = ext.y * TILT.cos() + across * TILT.sin();
    let fit =
        (card.width() * 0.92 * unit(depth) / across).min(card.height() * 0.7 * unit(depth) / tall);
    let center = at(card.center() - vec2(0.0, 4.0), depth);
    let root = Mat4::from_translation(center)
        * Mat4::from_quat(Quat::from_rotation_x(TILT) * Quat::from_rotation_y(spin))
        * Mat4::from_scale(Vec3::splat(fit))
        * Mat4::from_translation(-mid);
    let pose = weapon_pose(&HeldWeapon {
        weapon,
        ability: glow,
        time,
        ..Default::default()
    });
    lib.get(weapon.model())
        .draw(&pose, root, &Tint::team(DEFAULT_TEAM_COLOR), p.viewmodel);
    // A lamp in front of it, up and to the left, and a cool rim light behind (the world's
    // lights may be far away).
    let cam = p.camera;
    for (offset, color, radius) in [
        (Vec3::new(-0.6, 0.7, 1.0), Vec3::new(1.9, 1.72, 1.5), 6.0),
        (Vec3::new(0.9, 0.4, -0.5), Vec3::new(0.7, 0.72, 0.85), 3.0),
    ] {
        p.lights.push(PointLight {
            position: cam.position + cam.rotation * (center + offset),
            color,
            radius,
        });
    }
}

/// Puts `weapon` (or nothing) on `slot`. A weapon another slot holds swaps the two slots; the
/// last weapon can't be removed. Returns whether the loadout changed.
pub fn set_weapon(loadout: &mut Loadout, slot: usize, weapon: Option<WeaponKind>) -> bool {
    let Some(current) = loadout.slots.get(slot).copied() else {
        return false;
    };
    match weapon {
        None => {
            if current.is_none() || loadout.builds().count() <= 1 {
                return false;
            }
            loadout.slots[slot] = None;
        }
        Some(w) if current.is_some_and(|b| b.weapon == w) => return false,
        Some(w) => match loadout.slot_of(w) {
            Some(other) => loadout.slots.swap(slot, other as usize),
            None => loadout.slots[slot] = Some(w.default_build()),
        },
    }
    true
}

/// Sets ability `k` (0 = A, 1 = B) of `slot`'s build; picking the other ability swaps them.
/// Returns whether the loadout changed.
pub fn set_ability(loadout: &mut Loadout, slot: usize, k: usize, ability: Ability) -> bool {
    let Some(Some(build)) = loadout.slots.get_mut(slot) else {
        return false;
    };
    if k > 1 || !ability.fits(build.weapon) || build.abilities[k] == ability {
        return false;
    }
    if build.abilities[1 - k] == ability {
        build.abilities.swap(0, 1);
    } else {
        build.abilities[k] = ability;
    }
    true
}

/// Puts `knack` on Q (every knack is allowed; PvP worlds swap PvE-only ones for Surge
/// themselves). Returns whether the loadout changed.
pub fn set_knack(loadout: &mut Loadout, knack: Knack) -> bool {
    let changed = loadout.knack != knack;
    loadout.knack = knack;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_weapons_keeps_the_loadout_valid() {
        let mut l = Loadout::default();
        // A new weapon replaces the slot's with its default build.
        assert!(set_weapon(&mut l, 1, Some(WeaponKind::Longwick)));
        assert_eq!(l.slot(1), Some(&WeaponKind::Longwick.default_build()));
        // A weapon another slot holds swaps the two.
        let quill = *l.slot(0).unwrap();
        assert!(set_weapon(&mut l, 3, Some(WeaponKind::Quill)));
        assert_eq!(l.slot(3), Some(&quill));
        assert_eq!(l.slot(0).map(|b| b.weapon), Some(WeaponKind::Tumbler));
        assert!(l.is_valid());
        // Emptying works until one weapon is left.
        assert!(set_weapon(&mut l, 0, None));
        assert!(set_weapon(&mut l, 1, None));
        assert!(set_weapon(&mut l, 2, None));
        assert!(!set_weapon(&mut l, 3, None), "the last weapon stays");
        assert!(l.is_valid());
        assert!(
            !set_weapon(&mut l, 9, Some(WeaponKind::Kiln)),
            "no such slot"
        );
        // Same weapon again: nothing to do.
        assert!(!set_weapon(&mut l, 3, Some(WeaponKind::Quill)));
    }

    #[test]
    fn picking_abilities_swaps_and_refuses_other_classes() {
        let mut l = Loadout::single(WeaponKind::Longwick.default_build());
        let [a, b] = l.slot(0).unwrap().abilities;
        assert!(set_ability(&mut l, 0, 0, b));
        assert_eq!(l.slot(0).unwrap().abilities, [b, a], "swapped");
        assert!(set_ability(&mut l, 0, 1, Ability::EmberArrow));
        assert!(
            !set_ability(&mut l, 0, 1, Ability::Gust),
            "not a Draw ability"
        );
        assert!(!set_ability(&mut l, 1, 0, Ability::Brand), "empty slot");
        assert!(l.is_valid());
        // Melee pools are their own: no Gust on the shears, a Whirl swaps in.
        let mut l = Loadout::single(WeaponKind::Trimmer.default_build());
        assert!(!set_ability(&mut l, 0, 0, Ability::Gust));
        assert!(set_ability(&mut l, 0, 1, Ability::Whirl));
        assert_eq!(
            l.slot(0).unwrap().abilities,
            [Ability::Lunge, Ability::Whirl]
        );
    }

    #[test]
    fn picking_a_knack_keeps_the_weapons() {
        let mut l = Loadout::default();
        assert!(!set_knack(&mut l, Knack::Thorn), "already on Q");
        for k in Knack::ALL.into_iter().rev() {
            assert!(set_knack(&mut l, k), "{k:?} replaces the last one");
            assert_eq!(l.knack, k);
            assert_eq!(l.slots, Loadout::default().slots);
            assert!(l.is_valid(), "every knack is allowed, PvE-only ones too");
        }
    }

    #[test]
    fn the_layout_fits_the_canvas_without_overlaps() {
        for (w, h) in [(640.0, 360.0), (480.0, 360.0), (854.0, 360.0)] {
            let canvas = Rect::from_min_size(Vec2::ZERO, vec2(w, h));
            let lay = Layout::new(canvas);
            assert!(canvas.contains_rect(lay.frame));
            for r in [lay.left, lay.right, lay.card, lay.info, lay.lower, lay.foot] {
                assert!(lay.frame.contains_rect(r), "{w}: {r:?}");
            }
            let apart = |a: Rect, b: Rect| a.intersect(b).is_empty();
            assert!(apart(lay.left, lay.right) && apart(lay.card, lay.info));
            assert!(apart(lay.boxes[0], lay.boxes[1]) && apart(lay.card, lay.boxes[0]));
            assert!(apart(lay.right, lay.foot) && apart(lay.left, lay.foot));
            for i in 0..Knack::COUNT {
                assert!(lay.right.contains_rect(lay.knack_tile(i)), "{w}: knack {i}");
                assert!(apart(lay.knack_tile(i), lay.knack_detail()));
            }
            let g = lay.grid();
            for i in 0..4 {
                assert!(lay.right.contains_rect(g[i]));
                for j in i + 1..4 {
                    assert!(apart(g[i], g[j]), "{w}: cells {i} {j}");
                }
            }
        }
        // 16:9: the explanations have room for their budgets (loadout_text's tests).
        let lay = Layout::new(Rect::from_min_size(Vec2::ZERO, vec2(640.0, 360.0)));
        assert!(lay.box_text_width() >= 200.0, "{}", lay.box_text_width());
        assert!(lay.grid()[0].height() >= 130.0);
    }
}
