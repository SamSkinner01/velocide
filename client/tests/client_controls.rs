//! Controls: weapon selection rule, input building, look, bindings and the rebinding flow of
//! the settings screen. Input is fed through gamepad events and raw mouse motion (the only
//! engine input events constructible outside winit).

use client::controls::{
    Action, Bindings, Controls, MODES_PER_WEAPON, WEAPONS, WeaponChange, WeaponSelect,
    apply_mouse_look,
};
use client::menus::settings_menu::SettingsMenu;
use client::settings::Settings;
use engine::gfx::UiDrawList;
use engine::input::{Binding, GamepadAxis, GamepadButton, GamepadEvent, InputState, Key};
use engine::ui::{Rect, Theme, Ui, UiInput, UiKey, UiState};
use glam::{Vec2, vec2};
use shared::input::Buttons;
use shared::protocol::MAX_PITCH;

#[test]
fn slot_selects_then_cycles_the_alt_mode() {
    let mut w = WeaponSelect::default();
    assert_eq!((w.current, w.mode()), (0, 0));
    assert_eq!(w.press_slot(2), WeaponChange::Weapon);
    assert_eq!((w.current, w.mode()), (2, 0));
    assert_eq!(w.press_slot(2), WeaponChange::Mode);
    assert_eq!(w.mode(), 1);
    assert_eq!(w.press_slot(2), WeaponChange::Mode);
    assert_eq!(w.mode(), 0, "modes wrap after {MODES_PER_WEAPON}");
    w.press_slot(2);
    // Each weapon remembers its own mode.
    w.press_slot(0);
    assert_eq!(w.mode(), 0);
    w.press_slot(2);
    assert_eq!(w.mode(), 1);
    assert_eq!(w.info().name, WEAPONS[2].name);
}

#[test]
fn wheel_cycles_weapons_and_wraps() {
    let mut w = WeaponSelect::default();
    w.prev();
    assert_eq!(w.current as usize, WEAPONS.len() - 1);
    w.next();
    assert_eq!(w.current, 0);
    w.next();
    assert_eq!(w.current, 1);
}

#[test]
fn mouse_look_turns_and_clamps_pitch() {
    let s = Settings::default();
    let (mut yaw, mut pitch) = (0.0f32, 0.0f32);
    apply_mouse_look(&mut yaw, &mut pitch, Vec2::new(100.0, 0.0), &s);
    assert!(
        yaw < 0.0,
        "moving the mouse right turns right (negative yaw)"
    );
    let expected = (100.0 * s.mouse_sensitivity * 0.022f32).to_radians();
    assert!((yaw + expected).abs() < 1e-5);
    apply_mouse_look(&mut yaw, &mut pitch, Vec2::new(0.0, -100.0), &s);
    assert!(pitch > 0.0, "moving the mouse up looks up");
    apply_mouse_look(&mut yaw, &mut pitch, Vec2::new(0.0, -1.0e6), &s);
    assert_eq!(pitch, MAX_PITCH);
    let inverted = Settings {
        invert_y: true,
        ..Default::default()
    };
    let (mut y2, mut p2) = (0.0, 0.0);
    apply_mouse_look(&mut y2, &mut p2, Vec2::new(0.0, -100.0), &inverted);
    assert!(p2 < 0.0);
}

#[test]
fn a_tap_between_ticks_still_reaches_the_simulation() {
    let s = Settings::default();
    let mut c = Controls::new();
    let mut input = InputState::new();
    // Jump pressed and released within one frame, no tick in that frame.
    input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::South, true));
    input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::South, false));
    c.latch(&input, &s.bindings);
    input.end_frame();
    let tick = c.build(&input, &s.bindings, true);
    assert!(tick.buttons.contains(Buttons::JUMP));
    // ...and only once.
    let next = c.build(&input, &s.bindings, true);
    assert!(!next.buttons.contains(Buttons::JUMP));
}

#[test]
fn build_uses_latest_view_sticks_and_selection() {
    let s = Settings::default();
    let mut c = Controls::new();
    let mut input = InputState::new();
    input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickY, 1.0));
    input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::LeftBumper, true));
    input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::DPadRight, true));
    assert_eq!(c.latch(&input, &s.bindings), WeaponChange::Weapon);
    c.set_view(1.25, -0.5);
    let i = c.build(&input, &s.bindings, true);
    assert_eq!(i.forward, 127);
    assert_eq!(i.right, 0);
    assert!(i.buttons.contains(Buttons::DASH));
    assert_eq!((i.yaw, i.pitch), (1.25, -0.5));
    assert_eq!(i.weapon, 1);
    // Chat / menu open: neutral input, view and weapon kept.
    let neutral = c.build(&input, &s.bindings, false);
    assert_eq!((neutral.forward, neutral.buttons), (0, Buttons::NONE));
    assert_eq!((neutral.yaw, neutral.weapon), (1.25, 1));
}

#[test]
fn per_frame_mouse_look_moves_the_view_between_ticks() {
    let s = Settings::default();
    let mut c = Controls::new();
    let mut input = InputState::new();
    input.handle_mouse_motion((40.0, 0.0));
    c.look(&input, &s, 1.0 / 240.0);
    let after_frame = c.yaw;
    assert!(after_frame < 0.0);
    input.end_frame();
    input.handle_mouse_motion((40.0, 0.0));
    c.look(&input, &s, 1.0 / 240.0);
    assert!(c.yaw < after_frame, "the view keeps turning every frame");
    let i = c.build(&input, &s.bindings, true);
    assert_eq!(i.yaw, c.yaw, "the tick sends the latest angle");
}

#[test]
fn binding_replaces_the_same_device_and_steals_from_others() {
    let mut b = Bindings::default();
    b.bind(Action::Jump, Binding::Key(Key::KeyQ));
    assert_eq!(b.get(Action::Jump)[0], Binding::Key(Key::KeyQ));
    assert!(!b.get(Action::Jump).contains(&Binding::Key(Key::Space)));
    assert!(
        b.get(Action::Jump)
            .contains(&Binding::GamepadButton(GamepadButton::South))
    );
    assert!(!b.get(Action::ThrowPrism).contains(&Binding::Key(Key::KeyQ)));
    b.bind(Action::Jump, Binding::GamepadButton(GamepadButton::North));
    assert!(
        !b.get(Action::Jump)
            .contains(&Binding::GamepadButton(GamepadButton::South))
    );
    assert!(b.get(Action::Jump).contains(&Binding::Key(Key::KeyQ)));
    assert_eq!(b.display(Action::Jump), "Q / PAD Y");
}

#[test]
fn defaults_cover_every_action_with_sensible_keys() {
    let b = Bindings::default();
    for a in Action::ALL {
        assert!(!b.get(a).is_empty(), "{a:?} has a default binding");
    }
    assert_eq!(b.get(Action::MoveForward), [Binding::Key(Key::KeyW)]);
    assert!(b.get(Action::Pause).contains(&Binding::Key(Key::Escape)));
    assert!(b.get(Action::DebugOverlay).contains(&Binding::Key(Key::F3)));
}

/// Runs one frame of the settings screen with scripted UI keys.
fn settings_frame(
    menu: &mut SettingsMenu,
    state: &mut UiState,
    s: &mut Settings,
    input: &InputState,
    keys: &[UiKey],
) {
    let theme = Theme::default();
    let mut list = UiDrawList::new(vec2(640.0, 360.0));
    let ui_input = UiInput {
        mouse_pos: Vec2::splat(-100.0),
        keys: keys.to_vec(),
        ..Default::default()
    };
    let mut ui = Ui::new(&mut list, state, &ui_input, &theme);
    let rect = Rect::from_center_size(vec2(320.0, 180.0), vec2(460.0, 330.0));
    menu.ui(&mut ui, s, input, rect);
}

#[test]
fn rebinding_through_the_settings_screen() {
    let mut menu = SettingsMenu::new();
    menu.tab = 2; // CONTROLS
    let mut state = UiState::new();
    let mut s = Settings::default();
    let mut input = InputState::new();
    settings_frame(&mut menu, &mut state, &mut s, &input, &[]);
    // Focus order: tabs, sensitivity, invert, pad look speed, pad invert, then one key bind
    // row per action in `Action::ALL` order; JUMP is the 5th.
    let downs = 4 + 5;
    settings_frame(
        &mut menu,
        &mut state,
        &mut s,
        &input,
        &vec![UiKey::Down; downs],
    );
    settings_frame(&mut menu, &mut state, &mut s, &input, &[UiKey::Enter]);
    assert!(state.is_capturing());
    // The next button pressed becomes the binding.
    input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::North, true));
    settings_frame(&mut menu, &mut state, &mut s, &input, &[]);
    assert!(!state.is_capturing());
    assert!(
        s.bindings
            .get(Action::Jump)
            .contains(&Binding::GamepadButton(GamepadButton::North))
    );
    assert!(
        s.bindings
            .get(Action::Jump)
            .contains(&Binding::Key(Key::Space))
    );
    assert!(
        !s.bindings
            .get(Action::ThrowPrism)
            .contains(&Binding::GamepadButton(GamepadButton::North))
    );
}
