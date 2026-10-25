//! Per-frame UI input, independent of winit. The game converts its engine input into this.

use glam::Vec2;

/// Navigation / editing keys the UI understands. Gamepads map onto these too
/// (d-pad = arrows, A = Enter, B = Escape, shoulder buttons = PageUp/PageDown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Everything the UI needs from one frame of input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiInput {
    /// Mouse position in canvas units (see [`UiInput::canvas_from_window`]).
    pub mouse_pos: Vec2,
    /// Left mouse button held.
    pub mouse_down: bool,
    /// Left mouse button went down this frame.
    pub mouse_pressed: bool,
    /// Left mouse button went up this frame.
    pub mouse_released: bool,
    /// Wheel movement this frame in notches; positive = away from the user (scroll up).
    pub scroll: f32,
    /// Characters typed this frame (already filtered of control characters by the OS layer
    /// or not; the UI ignores control characters).
    pub text: String,
    /// Key presses this frame, in order, including OS key-repeat presses.
    pub keys: Vec<UiKey>,
    /// Seconds since start (drives blinking cursors and animations).
    pub time: f64,
}

impl UiInput {
    /// Whether `key` was pressed at least once this frame.
    pub fn key(&self, key: UiKey) -> bool {
        self.keys.contains(&key)
    }

    /// How many times `key` was pressed this frame (key repeat can produce several).
    pub fn key_count(&self, key: UiKey) -> usize {
        self.keys.iter().filter(|&&k| k == key).count()
    }

    /// Convert a window-space position in physical pixels to canvas units.
    /// `window_size` is the window size in pixels; the canvas is
    /// `UiDrawList::canvas_for_window(window_size)`, so one canvas unit is
    /// `window_height / 360` pixels in both axes.
    pub fn canvas_from_window(pos_px: Vec2, window_size: Vec2) -> Vec2 {
        pos_px * (crate::gfx::UI_CANVAS_HEIGHT / window_size.y.max(1.0))
    }

    /// Clear the per-frame events (presses, releases, text, keys, wheel) but keep held state
    /// and the mouse position. Handy if the game accumulates input across event callbacks.
    pub fn clear_events(&mut self) {
        self.mouse_pressed = false;
        self.mouse_released = false;
        self.scroll = 0.0;
        self.text.clear();
        self.keys.clear();
    }
}
