//! Converts the engine's [`InputState`] into the UI toolkit's [`UiInput`] every frame.
//!
//! Keyboard: arrows, Enter / keypad Enter, Escape, Backspace, Delete, Tab, Home, End, PageUp,
//! PageDown, including OS key repeat. Gamepad (docs/ui.md): d-pad and left stick → arrows
//! (with our own hold-to-repeat), A → Enter, B → Escape, shoulders → PageUp / PageDown.

use engine::input::{GamepadButton, InputState, Key, MouseButton};
use engine::ui::{UiInput, UiKey};
use glam::Vec2;

/// Seconds before a held d-pad / stick direction starts repeating, and the repeat interval.
const REPEAT_DELAY: f32 = 0.38;
const REPEAT_INTERVAL: f32 = 0.09;
/// Stick deflection that counts as a direction press.
const STICK_PRESS: f32 = 0.6;

const KEY_MAP: [(Key, UiKey); 14] = [
    (Key::ArrowUp, UiKey::Up),
    (Key::ArrowDown, UiKey::Down),
    (Key::ArrowLeft, UiKey::Left),
    (Key::ArrowRight, UiKey::Right),
    (Key::Enter, UiKey::Enter),
    (Key::NumpadEnter, UiKey::Enter),
    (Key::Escape, UiKey::Escape),
    (Key::Backspace, UiKey::Backspace),
    (Key::Delete, UiKey::Delete),
    (Key::Tab, UiKey::Tab),
    (Key::Home, UiKey::Home),
    (Key::End, UiKey::End),
    (Key::PageUp, UiKey::PageUp),
    (Key::PageDown, UiKey::PageDown),
];

/// Hold-to-repeat state of one gamepad direction.
#[derive(Clone, Copy, Debug, Default)]
struct Repeat {
    held: f32,
    next: f32,
}

impl Repeat {
    /// Returns how many presses to emit this frame.
    fn update(&mut self, down: bool, dt: f32) -> usize {
        if !down {
            *self = Repeat::default();
            return 0;
        }
        if self.held == 0.0 {
            self.held = dt.max(1e-4);
            self.next = REPEAT_DELAY;
            return 1;
        }
        self.held += dt;
        let mut n = 0;
        while self.held >= self.next {
            self.next += REPEAT_INTERVAL;
            n += 1;
        }
        n
    }
}

/// Builds [`UiInput`]s; keeps gamepad repeat timers.
#[derive(Clone, Debug, Default)]
pub struct UiInputMapper {
    dirs: [Repeat; 4],
}

impl UiInputMapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// `window_size` in pixels; `time` drives UI animations.
    pub fn build(
        &mut self,
        input: &InputState,
        window_size: (u32, u32),
        dt: f32,
        time: f64,
    ) -> UiInput {
        let size = Vec2::new(window_size.0 as f32, window_size.1 as f32);
        let mouse_pos = input.cursor_position().map_or(Vec2::splat(-1000.0), |p| {
            UiInput::canvas_from_window(p, size)
        });
        let mut keys = Vec::new();
        for (key, ui) in KEY_MAP {
            if input.key_pressed_or_repeated(key) {
                keys.push(ui);
            }
        }
        for (button, ui) in [
            (GamepadButton::South, UiKey::Enter),
            (GamepadButton::East, UiKey::Escape),
            (GamepadButton::LeftBumper, UiKey::PageUp),
            (GamepadButton::RightBumper, UiKey::PageDown),
        ] {
            if input.gamepad_pressed(button) {
                keys.push(ui);
            }
        }
        let stick = input.left_stick();
        let dirs = [
            (
                input.gamepad_down(GamepadButton::DPadUp) || stick.y > STICK_PRESS,
                UiKey::Up,
            ),
            (
                input.gamepad_down(GamepadButton::DPadDown) || stick.y < -STICK_PRESS,
                UiKey::Down,
            ),
            (
                input.gamepad_down(GamepadButton::DPadLeft) || stick.x < -STICK_PRESS,
                UiKey::Left,
            ),
            (
                input.gamepad_down(GamepadButton::DPadRight) || stick.x > STICK_PRESS,
                UiKey::Right,
            ),
        ];
        for (repeat, (down, key)) in self.dirs.iter_mut().zip(dirs) {
            for _ in 0..repeat.update(down, dt) {
                keys.push(key);
            }
        }
        UiInput {
            mouse_pos,
            mouse_down: input.mouse_down(MouseButton::Left),
            mouse_pressed: input.mouse_pressed(MouseButton::Left),
            mouse_released: input.mouse_released(MouseButton::Left),
            scroll: input.scroll_delta().y,
            text: input.typed_text().to_string(),
            keys,
            time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_direction_repeats_after_a_delay() {
        let mut r = Repeat::default();
        assert_eq!(r.update(true, 0.016), 1);
        let mut presses = 0;
        let mut t = 0.016; // the first frame already counted as held
        while t + 0.016 < REPEAT_DELAY {
            presses += r.update(true, 0.016);
            t += 0.016;
        }
        assert_eq!(presses, 0, "no repeat before the delay");
        let mut later = 0;
        for _ in 0..30 {
            later += r.update(true, 0.016);
        }
        assert!(later >= 4, "repeats while held ({later})");
        assert_eq!(r.update(false, 0.016), 0);
        assert_eq!(r.update(true, 0.016), 1, "fresh press after release");
    }
}
