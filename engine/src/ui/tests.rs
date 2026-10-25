//! Behaviour tests for the widget toolkit, driven by scripted `UiInput`s.

use glam::{Vec2, vec2};

use super::*;
use crate::gfx::UiDrawList;

struct Harness {
    list: UiDrawList,
    state: UiState,
    theme: Theme,
    time: f64,
}

impl Harness {
    fn new() -> Self {
        Self {
            list: UiDrawList::new(vec2(640.0, 360.0)),
            state: UiState::new(),
            theme: Theme::default(),
            time: 0.0,
        }
    }

    /// Run one frame with `input` (time is advanced automatically) inside a panel.
    fn frame<R>(&mut self, mut input: UiInput, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.time += 1.0 / 60.0;
        input.time = self.time;
        self.list.clear();
        let mut ui = Ui::new(&mut self.list, &mut self.state, &input, &self.theme);
        ui.panel(Rect::new(vec2(100.0, 40.0), vec2(500.0, 340.0)), "TEST", f)
    }

    fn idle<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.frame(UiInput::default(), f)
    }
}

fn keys(keys: &[UiKey]) -> UiInput {
    UiInput {
        keys: keys.to_vec(),
        ..Default::default()
    }
}

fn mouse(pos: Vec2, down: bool, pressed: bool, released: bool) -> UiInput {
    UiInput {
        mouse_pos: pos,
        mouse_down: down,
        mouse_pressed: pressed,
        mouse_released: released,
        ..Default::default()
    }
}

fn typed(text: &str, k: &[UiKey]) -> UiInput {
    UiInput {
        text: text.to_string(),
        keys: k.to_vec(),
        ..Default::default()
    }
}

/// Top-left of the next widget, read from the layout before drawing it.
fn next_pos(ui: &Ui) -> Vec2 {
    ui.available_rect().min
}

#[test]
fn button_click_needs_press_and_release_inside() {
    let mut h = Harness::new();
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        ui.button("FIRE");
        p
    });
    let inside = pos + vec2(30.0, 6.0);
    let outside = pos + vec2(30.0, 100.0);
    assert!(!h.frame(mouse(inside, true, true, false), |ui| ui.button("FIRE")));
    assert!(!h.frame(mouse(inside, true, false, false), |ui| ui.button("FIRE")));
    assert!(h.frame(mouse(inside, false, false, true), |ui| ui.button("FIRE")));
    // Press inside, release outside: no click.
    assert!(!h.frame(mouse(inside, true, true, false), |ui| ui.button("FIRE")));
    assert!(!h.frame(mouse(outside, false, false, true), |ui| ui.button("FIRE")));
    // Press outside, release inside: no click either.
    assert!(!h.frame(mouse(outside, true, true, false), |ui| ui.button("FIRE")));
    assert!(!h.frame(mouse(inside, false, false, true), |ui| ui.button("FIRE")));
    // Press and release within one frame still counts.
    assert!(h.frame(mouse(inside, false, true, true), |ui| ui.button("FIRE")));
}

fn three_buttons(ui: &mut Ui) -> [bool; 3] {
    [ui.button("A"), ui.button("B"), ui.button("C")]
}

#[test]
fn keyboard_focus_navigation() {
    let mut h = Harness::new();
    h.idle(three_buttons);
    assert_eq!(h.state.focused(), None);
    h.frame(keys(&[UiKey::Down]), three_buttons);
    let first = h.state.focused();
    assert!(first.is_some());
    h.frame(keys(&[UiKey::Down, UiKey::Tab]), three_buttons);
    assert_eq!(
        h.frame(keys(&[UiKey::Enter]), three_buttons),
        [false, false, true]
    );
    // Wraps around both ways.
    h.frame(keys(&[UiKey::Down]), three_buttons);
    assert_eq!(h.state.focused(), first);
    h.frame(keys(&[UiKey::Up]), three_buttons);
    assert_eq!(
        h.frame(keys(&[UiKey::Enter]), three_buttons),
        [false, false, true]
    );
    // Up with nothing focused goes to the last widget.
    h.state.set_focus(None);
    h.idle(three_buttons);
    h.frame(keys(&[UiKey::Up]), three_buttons);
    assert_eq!(
        h.frame(keys(&[UiKey::Enter]), three_buttons),
        [false, false, true]
    );
}

#[test]
fn hover_focuses_only_when_mouse_moves() {
    let mut h = Harness::new();
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        three_buttons(ui);
        p
    });
    let over_a = pos + vec2(20.0, 5.0);
    h.frame(mouse(over_a, false, false, false), three_buttons);
    h.frame(
        mouse(over_a + vec2(1.0, 0.0), false, false, false),
        three_buttons,
    );
    let a = h.state.focused();
    assert!(a.is_some());
    // Keyboard moves focus away; a resting mouse doesn't steal it back.
    h.frame(
        UiInput {
            keys: vec![UiKey::Down],
            ..mouse(over_a + vec2(1.0, 0.0), false, false, false)
        },
        three_buttons,
    );
    h.frame(
        mouse(over_a + vec2(1.0, 0.0), false, false, false),
        three_buttons,
    );
    assert_ne!(h.state.focused(), a);
}

#[test]
fn default_focus_and_disabled_widgets() {
    let mut h = Harness::new();
    let draw = |ui: &mut Ui| {
        ui.default_focus();
        ui.set_enabled(false);
        let a = ui.button("LOCKED");
        ui.set_enabled(true);
        let b = ui.button("OPEN");
        [a, b]
    };
    h.idle(draw);
    assert_eq!(h.frame(keys(&[UiKey::Enter]), draw), [false, true]);
    // Disabled widgets are skipped by navigation.
    h.frame(keys(&[UiKey::Down]), draw);
    assert_eq!(h.frame(keys(&[UiKey::Enter]), draw), [false, true]);
}

#[test]
fn push_id_separates_repeated_labels() {
    let mut h = Harness::new();
    let draw = |ui: &mut Ui| {
        let mut kicked = None;
        for i in 0..3 {
            ui.with_id(i, |ui| {
                if ui.button("KICK") {
                    kicked = Some(i);
                }
            });
        }
        kicked
    };
    h.idle(draw);
    h.frame(keys(&[UiKey::Down, UiKey::Down]), draw);
    assert_eq!(h.frame(keys(&[UiKey::Enter]), draw), Some(1));
    // "##" suffixes also make distinct ids while showing the same text.
    assert_eq!(display_text("PLAY##coop"), "PLAY");
    let ids = h.idle(|ui| (ui.id("PLAY##coop"), ui.id("PLAY##pvp")));
    assert_ne!(ids.0, ids.1);
}

fn field(ui: &mut Ui, value: &mut String, filter: TextFilter) -> TextFieldResponse {
    ui.text_field("ADDRESS", value, 12, filter)
}

#[test]
fn text_field_editing() {
    let mut h = Harness::new();
    let mut value = String::from("abc");
    h.idle(|ui| field(ui, &mut value, TextFilter::Any));
    h.frame(keys(&[UiKey::Down]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert!(h.state.wants_text_input());
    // Cursor starts at the end.
    let r = h.frame(typed("de", &[]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert!(r.changed && !r.submitted);
    assert_eq!(value, "abcde");
    h.frame(
        typed("", &[UiKey::Left, UiKey::Left, UiKey::Backspace]),
        |ui| field(ui, &mut value, TextFilter::Any),
    );
    assert_eq!(value, "abde");
    h.frame(typed("X", &[]), |ui| field(ui, &mut value, TextFilter::Any));
    assert_eq!(value, "abXde");
    h.frame(typed("", &[UiKey::Home, UiKey::Delete]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert_eq!(value, "bXde");
    h.frame(typed("", &[UiKey::End, UiKey::Backspace]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert_eq!(value, "bXd");
    // Max length is enforced (12 chars).
    h.frame(typed("0123456789abcdef", &[]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert_eq!(value.chars().count(), 12);
    let r = h.frame(keys(&[UiKey::Enter]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert!(r.submitted && !r.changed);
    // Control characters are ignored.
    let before = value.clone();
    h.frame(typed("\u{8}\t", &[]), |ui| {
        field(ui, &mut value, TextFilter::Any)
    });
    assert_eq!(value, before);
}

#[test]
fn text_field_filters() {
    assert!(TextFilter::Digits.accepts('7') && !TextFilter::Digits.accepts('a'));
    for c in "192.168.0.1:27015 my-host.example [::1]"
        .chars()
        .filter(|c| *c != ' ')
    {
        assert!(TextFilter::Address.accepts(c), "{c}");
    }
    assert!(!TextFilter::Address.accepts(' ') && !TextFilter::Address.accepts('/'));
    assert!(TextFilter::Name.accepts(' ') && !TextFilter::Name.accepts('@'));
    assert!(!TextFilter::Any.accepts('\u{e9}') && TextFilter::Any.accepts('~'));
    assert!(TextFilter::Custom(|c| c == 'x').accepts('x'));

    let mut h = Harness::new();
    let mut port = String::new();
    h.idle(|ui| field(ui, &mut port, TextFilter::Digits));
    h.frame(keys(&[UiKey::Down]), |ui| {
        field(ui, &mut port, TextFilter::Digits)
    });
    h.frame(typed("27a0x15", &[]), |ui| {
        field(ui, &mut port, TextFilter::Digits)
    });
    assert_eq!(port, "27015");
}

#[test]
fn text_field_click_places_cursor_and_unfocused_field_ignores_typing() {
    let mut h = Harness::new();
    let mut value = String::from("hello");
    h.frame(typed("zz", &[]), |ui| {
        ui.text_field("##chat", &mut value, 64, TextFilter::Any)
    });
    assert_eq!(value, "hello");
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        ui.text_field("##chat", &mut value, 64, TextFilter::Any);
        p
    });
    // Hidden label: the field starts at the row's left edge; text starts 4 units in.
    let after_he = pos + vec2(4.0 + 2.0 * 6.0, 6.0);
    h.frame(mouse(after_he, true, true, false), |ui| {
        ui.text_field("##chat", &mut value, 64, TextFilter::Any)
    });
    h.frame(typed("-", &[]), |ui| {
        ui.text_field("##chat", &mut value, 64, TextFilter::Any)
    });
    assert_eq!(value, "he-llo");
}

fn slider(ui: &mut Ui, v: &mut f32) -> bool {
    ui.slider("VOLUME", v, 0.0, 1.0, 0.1, "{%}%")
}

#[test]
fn slider_keys_and_drag() {
    let mut h = Harness::new();
    let mut v = 0.5;
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        slider(ui, &mut v);
        p
    });
    h.frame(keys(&[UiKey::Down]), |ui| slider(ui, &mut v));
    assert!(h.frame(keys(&[UiKey::Right, UiKey::Right]), |ui| slider(ui, &mut v)));
    assert!((v - 0.7).abs() < 1e-5);
    h.frame(keys(&[UiKey::Left; 20]), |ui| slider(ui, &mut v));
    assert_eq!(v, 0.0, "clamped at min");
    assert!(
        !h.frame(keys(&[UiKey::Left]), |ui| slider(ui, &mut v)),
        "no change at the limit"
    );

    // Drag: track spans from the control's left edge (+2) to the value column.
    let width = 400.0 - 24.0; // panel content width
    let control_x = pos.x + (width * h.theme.label_fraction).round() + 2.0;
    let start = vec2(control_x + 3.0, pos.y + 7.0);
    h.frame(mouse(start, true, true, false), |ui| slider(ui, &mut v));
    assert!(
        v < 0.05,
        "pressing at the left end selects the minimum, got {v}"
    );
    // Dragging far right (outside the widget) clamps to the maximum while held.
    h.frame(
        mouse(start + vec2(1000.0, 40.0), true, false, false),
        |ui| slider(ui, &mut v),
    );
    assert_eq!(v, 1.0);
    h.frame(
        mouse(start + vec2(1000.0, 40.0), false, false, true),
        |ui| slider(ui, &mut v),
    );
    // After release, moving the mouse doesn't change the value.
    h.frame(mouse(start, false, false, false), |ui| slider(ui, &mut v));
    assert_eq!(v, 1.0);
}

#[test]
fn value_formatting() {
    assert_eq!(format_value("{%}%", 0.5, 0.05), "50%");
    assert_eq!(format_value("{}", 0.25, 0.05), "0.25");
    assert_eq!(format_value("{} deg", 90.0, 1.0), "90 deg");
    assert_eq!(format_value("x{:.3}", 1.0, 0.1), "x1.000");
    assert_eq!(format_value("{bad}", 1.0, 1.0), "{bad}");
    assert_eq!(widgets::snap(0.27, 0.0, 1.0, 0.1), 0.3);
    assert_eq!(widgets::snap(5.0, 0.0, 1.0, 0.1), 1.0);
}

#[test]
fn choice_cycles_with_keys_and_clicks() {
    let opts = ["LOW", "MEDIUM", "HIGH"];
    let mut h = Harness::new();
    let mut sel = 0usize;
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        ui.choice("QUALITY", &mut sel, &opts);
        p
    });
    h.frame(keys(&[UiKey::Down]), |ui| {
        ui.choice("QUALITY", &mut sel, &opts)
    });
    assert!(h.frame(keys(&[UiKey::Right]), |ui| {
        ui.choice("QUALITY", &mut sel, &opts)
    }));
    assert_eq!(sel, 1);
    h.frame(keys(&[UiKey::Right, UiKey::Right]), |ui| {
        ui.choice("QUALITY", &mut sel, &opts)
    });
    assert_eq!(sel, 0, "wraps forward");
    h.frame(keys(&[UiKey::Left]), |ui| {
        ui.choice("QUALITY", &mut sel, &opts)
    });
    assert_eq!(sel, 2, "wraps backward");
    h.frame(keys(&[UiKey::Enter]), |ui| {
        ui.choice("QUALITY", &mut sel, &opts)
    });
    assert_eq!(sel, 0, "Enter advances");
    // Clicking the right end advances, the left arrow goes back.
    let right_end = vec2(500.0 - 12.0 - 6.0, pos.y + 7.0);
    let left_arrow = vec2(
        pos.x + ((400.0 - 24.0) * h.theme.label_fraction).round() + 6.0,
        pos.y + 7.0,
    );
    for (p, expect) in [(right_end, 1), (left_arrow, 0)] {
        h.frame(mouse(p, true, true, false), |ui| {
            ui.choice("QUALITY", &mut sel, &opts)
        });
        h.frame(mouse(p, false, false, true), |ui| {
            ui.choice("QUALITY", &mut sel, &opts)
        });
        assert_eq!(sel, expect);
    }
    // Out-of-range selections are clamped, empty option lists are harmless.
    sel = 99;
    h.idle(|ui| ui.choice("QUALITY", &mut sel, &opts));
    assert_eq!(sel, 2);
    assert!(!h.idle(|ui| ui.choice("EMPTY", &mut sel, &[])));
}

#[test]
fn toggle_flips() {
    let mut h = Harness::new();
    let mut on = false;
    h.idle(|ui| ui.toggle("DITHER", &mut on));
    h.frame(keys(&[UiKey::Down]), |ui| ui.toggle("DITHER", &mut on));
    assert!(h.frame(keys(&[UiKey::Enter]), |ui| ui.toggle("DITHER", &mut on)));
    assert!(on);
    assert!(!h.frame(keys(&[UiKey::Left, UiKey::Right]), |ui| {
        ui.toggle("DITHER", &mut on)
    }));
    assert!(on, "two flips cancel out");
    h.frame(keys(&[UiKey::Right]), |ui| ui.toggle("DITHER", &mut on));
    assert!(!on);
}

#[test]
fn keybind_capture_flow() {
    let mut h = Harness::new();
    let mut binding = String::from("SPACE");
    let draw = |ui: &mut Ui, b: &str| (ui.keybind("JUMP", b), ui.button("BACK"));
    h.idle(|ui| draw(ui, &binding));
    h.frame(keys(&[UiKey::Down]), |ui| draw(ui, &binding));
    let (r, _) = h.frame(keys(&[UiKey::Enter]), |ui| draw(ui, &binding));
    assert!(r.started && !r.capturing);
    assert!(h.state.is_capturing());
    assert_eq!(
        h.state.capture_target(),
        None,
        "not ready on the starting frame"
    );
    // While capturing, navigation keys and Enter are left to the game.
    let (r, back) = h.frame(keys(&[UiKey::Down, UiKey::Enter]), |ui| {
        let r = draw(ui, &binding);
        if r.0.capturing {
            binding = "ENTER".into();
            ui.finish_capture();
        }
        r
    });
    assert!(r.capturing && !back);
    assert_eq!(binding, "ENTER");
    assert!(!h.state.is_capturing());

    // Escape cancels a capture and is not reported as "back".
    h.frame(keys(&[UiKey::Enter]), |ui| draw(ui, &binding));
    assert!(h.state.is_capturing());
    let (r, back_pressed) = h.frame(keys(&[UiKey::Escape]), |ui| {
        let r = draw(ui, &binding);
        (r.0, ui.back_pressed())
    });
    assert!(r.cancelled && !r.capturing && !back_pressed);
    assert!(!h.state.is_capturing());
    assert!(h.frame(keys(&[UiKey::Escape]), |ui| ui.back_pressed()));

    // Clicking starts capture too; Backspace on a focused bind asks to clear it.
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        draw(ui, &binding);
        p
    });
    let on_bind = pos + vec2(300.0, 6.0);
    h.frame(mouse(on_bind, true, true, false), |ui| draw(ui, &binding));
    let (r, _) = h.frame(mouse(on_bind, false, false, true), |ui| draw(ui, &binding));
    assert!(r.started);
    h.state.finish_capture();
    let (r, _) = h.frame(keys(&[UiKey::Backspace]), |ui| draw(ui, &binding));
    assert!(r.cleared);
}

#[test]
fn tabs_switch_with_keys_and_clicks() {
    let tabs = ["VIDEO", "AUDIO", "CONTROLS", "GAMEPLAY"];
    let mut h = Harness::new();
    let mut tab = 0usize;
    let pos = h.idle(|ui| {
        let p = next_pos(ui);
        ui.tabs("settings", &mut tab, &tabs);
        p
    });
    h.frame(keys(&[UiKey::Down]), |ui| {
        ui.tabs("settings", &mut tab, &tabs)
    });
    h.frame(keys(&[UiKey::Right, UiKey::Right]), |ui| {
        ui.tabs("settings", &mut tab, &tabs)
    });
    assert_eq!(tab, 2);
    h.frame(keys(&[UiKey::Right; 5]), |ui| {
        ui.tabs("settings", &mut tab, &tabs)
    });
    assert_eq!(tab, 3, "clamped, no wrap");
    let second = pos + vec2(376.0 * 0.375, 6.0);
    h.frame(mouse(second, true, true, false), |ui| {
        ui.tabs("settings", &mut tab, &tabs)
    });
    assert!(h.frame(mouse(second, false, false, true), |ui| {
        ui.tabs("settings", &mut tab, &tabs)
    }));
    assert_eq!(tab, 1);
}

fn long_list(ui: &mut Ui) -> Vec2 {
    let p = next_pos(ui);
    ui.scroll_area("log", 60.0, |ui| {
        for i in 0..20 {
            ui.label(&format!("LINE {i}"));
        }
    });
    p
}

#[test]
fn scroll_area_clamps_and_clips() {
    let mut h = Harness::new();
    let pos = h.idle(long_list);
    let over = pos + vec2(50.0, 30.0);
    let wheel = |amount: f32| UiInput {
        mouse_pos: over,
        scroll: amount,
        ..Default::default()
    };
    let log_id = |h: &mut Harness| h.idle(|ui| ui.id("log"));
    let lid = log_id(&mut h);
    assert_eq!(h.state.scroll_offset(lid), 0.0);
    h.frame(wheel(-100.0), long_list);
    let max = h.state.scroll_offset(lid);
    // 20 labels of 11 units + 3 spacing, minus the 54-unit view.
    assert!(
        (max - (20.0 * 14.0 - 3.0 - 54.0)).abs() < 0.01,
        "max offset {max}"
    );
    h.frame(wheel(-1.0), long_list);
    assert_eq!(h.state.scroll_offset(lid), max, "clamped at the bottom");
    h.frame(wheel(100.0), long_list);
    assert_eq!(h.state.scroll_offset(lid), 0.0, "clamped at the top");
    // Page keys work while hovered.
    h.frame(
        UiInput {
            keys: vec![UiKey::PageDown],
            ..wheel(0.0)
        },
        long_list,
    );
    assert!(h.state.scroll_offset(lid) > 0.0);
    // Everything drawn inside is clipped to the view.
    h.idle(long_list);
    let view_top = pos.y;
    let view_bottom = pos.y + 60.0;
    for q in &h.list.quads {
        if q.texture == crate::gfx::UiTexture::Font && q.min.x < 480.0 && q.min.y > 60.0 {
            assert!(q.min.y >= view_top && q.max.y <= view_bottom, "{q:?}");
        }
    }
}

#[test]
fn sticky_scroll_follows_new_content() {
    let mut h = Harness::new();
    let draw = |ui: &mut Ui, n: usize| {
        ui.scroll_area_sticky("chat", 40.0, |ui| {
            for i in 0..n {
                ui.label(&format!("MSG {i}"));
            }
        });
    };
    let lid = h.idle(|ui| ui.id("chat"));
    h.idle(|ui| draw(ui, 2));
    assert_eq!(h.state.scroll_offset(lid), 0.0, "fits: no scrolling");
    h.idle(|ui| draw(ui, 10));
    let bottom = h.state.scroll_offset(lid);
    assert!(bottom > 0.0);
    h.idle(|ui| draw(ui, 12));
    assert!(h.state.scroll_offset(lid) > bottom, "keeps following");
    // Scrolling up stops following.
    let over = h.idle(|ui| next_pos(ui)) + vec2(20.0, 20.0);
    h.frame(
        UiInput {
            mouse_pos: over,
            scroll: 1.0,
            ..Default::default()
        },
        |ui| draw(ui, 12),
    );
    let held = h.state.scroll_offset(lid);
    h.idle(|ui| draw(ui, 14));
    assert_eq!(h.state.scroll_offset(lid), held);
}

#[test]
fn keyboard_navigation_scrolls_focus_into_view() {
    let mut h = Harness::new();
    let draw = |ui: &mut Ui| {
        ui.scroll_area("servers", 50.0, |ui| {
            for i in 0..10 {
                ui.selectable(&format!("SERVER {i}"), false);
            }
        });
    };
    let lid = h.idle(|ui| ui.id("servers"));
    h.idle(draw);
    for _ in 0..8 {
        h.frame(keys(&[UiKey::Down]), draw);
    }
    h.idle(draw);
    assert!(h.state.scroll_offset(lid) > 50.0);
}

#[test]
fn rows_split_width() {
    let mut h = Harness::new();
    let rects = h.idle(|ui| ui.row(2, |ui| [ui.allocate(10.0), ui.allocate(20.0)]));
    let after = h.idle(|ui| {
        ui.row(2, |ui| {
            ui.allocate(10.0);
            ui.allocate(20.0);
        });
        next_pos(ui)
    });
    assert_eq!(rects[0].width(), rects[1].width());
    assert_eq!(rects[0].min.y, rects[1].min.y);
    assert!(rects[1].min.x > rects[0].max.x);
    assert_eq!(
        after.y,
        rects[0].min.y + 20.0 + h.theme.spacing,
        "row height = tallest cell"
    );
}

#[test]
fn every_widget_draws_inside_canvas() {
    let mut h = Harness::new();
    let mut b = true;
    let mut f = 0.3;
    let mut c = 1;
    let mut s = String::from("name");
    let mut tab = 0;
    h.idle(|ui| {
        ui.tabs("t", &mut tab, &["ONE", "TWO"]);
        ui.heading("HEAD");
        ui.title("TITLE");
        ui.label("label");
        ui.label_wrapped("a long paragraph that will need to be wrapped across several lines");
        ui.value_row("PING", "42");
        ui.separator();
        ui.button("BUTTON");
        ui.menu_item("MENU");
        ui.toggle("TOGGLE", &mut b);
        ui.slider("SLIDER", &mut f, 0.0, 1.0, 0.1, "{}");
        ui.choice("CHOICE", &mut c, &["A", "B"]);
        ui.text_field("FIELD", &mut s, 16, TextFilter::Name);
        ui.keybind("BIND", "MOUSE1");
        ui.selectable("ROW", true);
    });
    assert!(!h.list.quads.is_empty());
    for q in &h.list.quads {
        assert!(
            q.min.x >= 0.0 && q.min.y >= 0.0 && q.max.x <= 640.0 && q.max.y <= 360.0,
            "{q:?}"
        );
    }
}
