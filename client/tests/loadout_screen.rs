//! The LOADOUT screen driven headless (no GPU, no models): keyboard / gamepad walks, the
//! weapon and ability pickers with their editing rules, presets, the mouse, Escape, and that
//! everything it draws stays on the canvas.

use client::menus::loadout::{Layout, LoadoutMenu, LoadoutResponse, Preview};
use client::settings::Settings;
use engine::gfx::{Camera, DrawItem, PointLight, UiDrawList};
use engine::ui::{Rect, Theme, Ui, UiInput, UiKey, UiState};
use glam::{Vec2, vec2};
use shared::loadout::{Ability, Loadout, WeaponClass, WeaponKind};
use shared::tuning::Tuning;

use UiKey::*;

struct Screen {
    menu: LoadoutMenu,
    settings: Settings,
    state: UiState,
    list: UiDrawList,
    theme: Theme,
    time: f64,
    viewmodel: Vec<DrawItem>,
    lights: Vec<PointLight>,
}

impl Screen {
    fn new() -> Screen {
        Screen {
            menu: LoadoutMenu::with_tuning(&Tuning::default()),
            settings: Settings::default(),
            state: UiState::new(),
            list: UiDrawList::new(UiDrawList::canvas_for_window(1280, 720)),
            theme: Theme::default(),
            time: 1.0,
            viewmodel: Vec::new(),
            lights: Vec::new(),
        }
    }

    fn run(&mut self, input: UiInput) -> LoadoutResponse {
        self.time += 1.0 / 60.0;
        self.list.clear();
        let input = UiInput {
            time: self.time,
            ..input
        };
        let mut ui = Ui::new(&mut self.list, &mut self.state, &input, &self.theme);
        let preview = Preview {
            lib: None,
            viewmodel: &mut self.viewmodel,
            lights: &mut self.lights,
            camera: Camera::default(),
            fov: 62.0,
        };
        self.menu.ui(&mut ui, &mut self.settings, preview)
    }

    /// One frame with these keys (the mouse far away).
    fn keys(&mut self, keys: &[UiKey]) -> LoadoutResponse {
        self.run(UiInput {
            mouse_pos: Vec2::splat(-100.0),
            keys: keys.to_vec(),
            ..Default::default()
        })
    }

    /// Each key in its own frame followed by a quiet one (nobody presses twice in 16 ms);
    /// the responses merged.
    fn press(&mut self, keys: &[UiKey]) -> LoadoutResponse {
        let mut out = LoadoutResponse::default();
        for &k in keys {
            for r in [self.keys(&[k]), self.keys(&[])] {
                out.changed |= r.changed;
                out.back |= r.back;
                out.sound = r.sound.or(out.sound);
            }
        }
        out
    }

    /// A click at `pos` (move there, press, release).
    fn click(&mut self, pos: Vec2) -> LoadoutResponse {
        let at = |down, pressed, released| UiInput {
            mouse_pos: pos,
            mouse_down: down,
            mouse_pressed: pressed,
            mouse_released: released,
            ..Default::default()
        };
        self.run(at(false, false, false));
        self.run(at(true, true, false));
        let r = self.run(at(false, false, true));
        self.run(at(false, false, false));
        r
    }

    fn layout(&self) -> Layout {
        Layout::new(Rect::from_min_size(Vec2::ZERO, self.list.canvas_size))
    }

    fn loadout(&self) -> Loadout {
        self.settings.loadout.current
    }
}

/// The weapon picker's tiles in focus order: class by class.
fn tile_order() -> Vec<WeaponKind> {
    WeaponClass::ALL
        .iter()
        .flat_map(|c| WeaponKind::ALL.into_iter().filter(move |w| w.class() == *c))
        .collect()
}

#[test]
fn walking_the_slot_cards_selects_them() {
    let mut s = Screen::new();
    s.keys(&[]);
    assert_eq!(s.menu.slot(), 0, "opens on slot 1");
    s.press(&[Down, Down]);
    assert_eq!(s.menu.slot(), 2);
    s.press(&[Up]);
    assert_eq!(s.menu.slot(), 1);
    // Enter on a card opens its weapon picker.
    s.press(&[Enter]);
    assert!(s.menu.is_picking_weapon());
    assert_eq!(s.loadout(), Loadout::default(), "looking changes nothing");
}

#[test]
fn the_ability_picker_swaps_and_replaces() {
    let mut s = Screen::new();
    s.keys(&[]);
    // Slot card → (Right) CHANGE WEAPON → (Down) ability box A → its picker on Brand.
    s.press(&[Right, Down, Enter]);
    assert_eq!(s.menu.picking_ability(), Some(0));
    // Right: Splinter, already ability B: picking it for A swaps the two.
    let r = s.press(&[Right, Enter]);
    assert!(r.changed);
    assert_eq!(s.menu.picking_ability(), None, "picking closes it");
    let quill = s.loadout().slots[0].unwrap();
    assert_eq!(quill.abilities, [Ability::Splinter, Ability::Brand]);
    // The focus is back on box A: open it again (on Splinter), Down to Skewer, pick.
    s.press(&[Enter, Down, Enter]);
    let quill = s.loadout().slots[0].unwrap();
    assert_eq!(quill.abilities, [Ability::Skewer, Ability::Brand]);
    assert!(s.loadout().is_valid());
}

#[test]
fn the_weapon_picker_equips_swaps_and_keeps_the_last_weapon() {
    let mut s = Screen::new();
    s.keys(&[]);
    // CHANGE WEAPON, then Right across the classes to the Explosive column: the Tumbler,
    // which slot 4 holds: equipping it swaps the two slots.
    s.press(&[Right, Enter]);
    assert!(s.menu.is_picking_weapon());
    let explosive = WeaponClass::ALL
        .iter()
        .position(|c| *c == WeaponClass::Explosive)
        .unwrap();
    for _ in 0..explosive {
        s.press(&[Right]);
    }
    let r = s.press(&[Enter]);
    assert!(r.changed && !s.menu.is_picking_weapon());
    let l = s.loadout();
    assert_eq!(l.slots[0].map(|b| b.weapon), Some(WeaponKind::Tumbler));
    assert_eq!(l.slots[3].map(|b| b.weapon), Some(WeaponKind::Quill));

    // A weapon nobody carries comes with its default build: the tile before the Tumbler.
    s.press(&[Enter, Up]);
    let order = tile_order();
    let tumbler = order
        .iter()
        .position(|w| *w == WeaponKind::Tumbler)
        .unwrap();
    let kiln = order.iter().position(|w| *w == WeaponKind::Kiln).unwrap();
    assert_eq!(kiln + 1, tumbler, "the Kiln closes the Rapid column");
    s.press(&[Enter]);
    assert_eq!(s.loadout().slots[0], Some(WeaponKind::Kiln.default_build()));

    // Down past the last tile is EMPTY SLOT; with four weapons slot 1 can be emptied ...
    s.press(&[Enter]);
    let from = order.iter().position(|w| *w == WeaponKind::Kiln).unwrap();
    for _ in from..order.len() {
        s.press(&[Down]);
    }
    s.press(&[Enter]);
    assert_eq!(s.loadout().slots[0], None);
    assert!(s.loadout().is_valid());

    // ... but the last weapon stays (EMPTY SLOT can't be focused or clicked).
    s.settings.loadout.current = Loadout::single(WeaponKind::Lancet.default_build());
    s.press(&[Enter]);
    assert!(s.menu.is_picking_weapon());
    let empty = s.layout();
    let r = s.click(vec2(empty.lower.min.x + 10.0, empty.lower.max.y - 8.0));
    assert!(!r.changed && s.menu.is_picking_weapon());
    assert_eq!(
        s.loadout(),
        Loadout::single(WeaponKind::Lancet.default_build())
    );
}

#[test]
fn escape_closes_a_picker_then_leaves() {
    let mut s = Screen::new();
    s.keys(&[]);
    s.press(&[Right, Enter]);
    assert!(s.menu.is_picking_weapon());
    let r = s.press(&[Escape]);
    assert!(
        !s.menu.is_picking_weapon() && !r.back,
        "Escape cancels the picker"
    );
    // The focus is back on CHANGE WEAPON: Down reaches box A at once.
    s.press(&[Down, Enter]);
    assert_eq!(s.menu.picking_ability(), Some(0));
    s.press(&[Escape]);
    assert_eq!(s.menu.picking_ability(), None);
    assert!(s.press(&[Escape]).back, "then it leaves");
    assert_eq!(s.loadout(), Loadout::default());
}

#[test]
fn presets_load_through_the_keys() {
    let mut s = Screen::new();
    s.keys(&[]);
    // Slots 1–4, CHANGE WEAPON, boxes A and B, then the PRESET choice: next preset, LOAD.
    s.press(&[Down; 7]);
    s.press(&[Right, Down, Enter]);
    let hunter = &s.settings.loadout.presets[1];
    assert_eq!(hunter.name, "HUNTER");
    assert_eq!(s.loadout(), hunter.loadout);
}

#[test]
fn the_mouse_selects_slots_and_opens_the_boxes() {
    let mut s = Screen::new();
    s.keys(&[]);
    let lay = s.layout();
    s.click(lay.slot_card(2).center());
    assert_eq!(s.menu.slot(), 2);
    // Box B of the Arclash: Discharge's picker, then pick Volley (the third of the pool).
    s.click(lay.boxes[1].center());
    assert_eq!(s.menu.picking_ability(), Some(1));
    let cells = lay.grid();
    let r = s.click(cells[2].center());
    assert!(r.changed);
    let arclash = s.loadout().slots[2].unwrap();
    assert_eq!(arclash.abilities, [Ability::Wisp, Ability::Volley]);
    // The knack card is not a control (yet): clicking it changes nothing.
    let before = s.loadout();
    s.click(lay.slot_card(4).center());
    assert_eq!(s.loadout(), before);
}

#[test]
fn everything_stays_on_the_canvas_at_every_aspect() {
    for (w, h) in [(1280, 720), (1280, 800), (960, 720), (2560, 1080)] {
        let mut s = Screen::new();
        s.list = UiDrawList::new(UiDrawList::canvas_for_window(w, h));
        let canvas = Rect::from_min_size(Vec2::ZERO, s.list.canvas_size);
        for keys in [&[][..], &[Right, Enter][..], &[Escape, Down, Enter][..]] {
            s.press(keys);
            for q in &s.list.quads {
                let r = Rect::new(q.min, q.max);
                assert!(
                    canvas.expand(0.5).contains_rect(r),
                    "{w}x{h}: quad {r:?} off the canvas"
                );
            }
        }
        // Without models nothing goes into the viewmodel pass.
        assert!(s.viewmodel.is_empty() && s.lights.is_empty());
    }
}
