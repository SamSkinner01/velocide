//! Keyboard, mouse and gamepad state.
//!
//! The app feeds winit / gilrs events into [`InputState`] as they arrive and calls
//! [`InputState::end_frame`] after every `Game::frame`, so "pressed" and "released" are true for
//! exactly one frame no matter how many events arrived in between (a key pressed and released
//! within one frame still reports `pressed` once).

use std::collections::HashSet;

use glam::Vec2;
use serde::{Deserialize, Serialize};
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode as Key;

/// Sticks below this magnitude read as zero (radial deadzone, rescaled outside it).
pub const STICK_DEADZONE: f32 = 0.18;
/// Triggers below this read as zero.
pub const TRIGGER_DEADZONE: f32 = 0.06;
/// Axis value at which a [`Binding::GamepadAxis`] counts as "down".
pub const AXIS_BUTTON_THRESHOLD: f32 = 0.5;
/// Scroll-wheel pixels (touchpads) per line.
const PIXELS_PER_LINE: f32 = 40.0;

/// Gamepad buttons (layout-agnostic names; South = A / Cross).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Mode,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl GamepadButton {
    pub fn from_gilrs(button: gilrs::Button) -> Option<Self> {
        use gilrs::Button as B;
        Some(match button {
            B::South => Self::South,
            B::East => Self::East,
            B::North => Self::North,
            B::West => Self::West,
            B::LeftTrigger => Self::LeftBumper,
            B::RightTrigger => Self::RightBumper,
            B::LeftTrigger2 => Self::LeftTrigger,
            B::RightTrigger2 => Self::RightTrigger,
            B::Select => Self::Select,
            B::Start => Self::Start,
            B::Mode => Self::Mode,
            B::LeftThumb => Self::LeftStick,
            B::RightThumb => Self::RightStick,
            B::DPadUp => Self::DPadUp,
            B::DPadDown => Self::DPadDown,
            B::DPadLeft => Self::DPadLeft,
            B::DPadRight => Self::DPadRight,
            B::C | B::Z | B::Unknown => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::South => "Pad A",
            Self::East => "Pad B",
            Self::North => "Pad Y",
            Self::West => "Pad X",
            Self::LeftBumper => "Pad LB",
            Self::RightBumper => "Pad RB",
            Self::LeftTrigger => "Pad LT",
            Self::RightTrigger => "Pad RT",
            Self::Select => "Pad Select",
            Self::Start => "Pad Start",
            Self::Mode => "Pad Home",
            Self::LeftStick => "Pad L3",
            Self::RightStick => "Pad R3",
            Self::DPadUp => "D-Pad Up",
            Self::DPadDown => "D-Pad Down",
            Self::DPadLeft => "D-Pad Left",
            Self::DPadRight => "D-Pad Right",
        }
    }
}

/// Analog gamepad axes. Sticks are in `[-1, 1]` with +Y = up; triggers in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamepadAxis {
    LeftStickX,
    LeftStickY,
    RightStickX,
    RightStickY,
    LeftTrigger,
    RightTrigger,
}

impl GamepadAxis {
    const ALL: [GamepadAxis; 6] = [
        Self::LeftStickX,
        Self::LeftStickY,
        Self::RightStickX,
        Self::RightStickY,
        Self::LeftTrigger,
        Self::RightTrigger,
    ];

    fn index(self) -> usize {
        self as usize
    }

    pub fn from_gilrs(axis: gilrs::Axis) -> Option<Self> {
        Some(match axis {
            gilrs::Axis::LeftStickX => Self::LeftStickX,
            gilrs::Axis::LeftStickY => Self::LeftStickY,
            gilrs::Axis::RightStickX => Self::RightStickX,
            gilrs::Axis::RightStickY => Self::RightStickY,
            gilrs::Axis::LeftZ => Self::LeftTrigger,
            gilrs::Axis::RightZ => Self::RightTrigger,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::LeftStickX => "Left Stick X",
            Self::LeftStickY => "Left Stick Y",
            Self::RightStickX => "Right Stick X",
            Self::RightStickY => "Right Stick Y",
            Self::LeftTrigger => "Pad LT",
            Self::RightTrigger => "Pad RT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisDirection {
    Positive,
    Negative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WheelDirection {
    Up,
    Down,
}

/// Anything that can be bound to a game action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
    /// One notch of the scroll wheel (down for exactly the frame it scrolled).
    Wheel(WheelDirection),
    GamepadButton(GamepadButton),
    /// A stick / trigger direction pushed past [`AXIS_BUTTON_THRESHOLD`].
    GamepadAxis(GamepadAxis, AxisDirection),
}

impl Binding {
    /// Human-readable name for menus ("W", "Space", "Mouse 4", "Pad RT", "Left Stick Up").
    pub fn name(&self) -> String {
        match *self {
            Binding::Key(key) => key_name(key),
            Binding::Mouse(button) => match button {
                MouseButton::Left => "Mouse 1".into(),
                MouseButton::Right => "Mouse 2".into(),
                MouseButton::Middle => "Mouse 3".into(),
                MouseButton::Back => "Mouse 4".into(),
                MouseButton::Forward => "Mouse 5".into(),
                MouseButton::Other(n) => format!("Mouse {}", n as u32 + 1),
            },
            Binding::Wheel(WheelDirection::Up) => "Wheel Up".into(),
            Binding::Wheel(WheelDirection::Down) => "Wheel Down".into(),
            Binding::GamepadButton(b) => b.name().into(),
            Binding::GamepadAxis(axis, dir) => {
                let suffix = match (axis, dir) {
                    (GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger, _) => {
                        return axis.name().into();
                    }
                    (
                        GamepadAxis::LeftStickX | GamepadAxis::RightStickX,
                        AxisDirection::Positive,
                    ) => "Right",
                    (
                        GamepadAxis::LeftStickX | GamepadAxis::RightStickX,
                        AxisDirection::Negative,
                    ) => "Left",
                    (_, AxisDirection::Positive) => "Up",
                    (_, AxisDirection::Negative) => "Down",
                };
                let stick = if matches!(axis, GamepadAxis::LeftStickX | GamepadAxis::LeftStickY) {
                    "Left Stick"
                } else {
                    "Right Stick"
                };
                format!("{stick} {suffix}")
            }
        }
    }
}

/// Short display name of a key.
pub fn key_name(key: KeyCode) -> String {
    use KeyCode as K;
    let s = match key {
        K::Space => "Space",
        K::Enter => "Enter",
        K::Escape => "Esc",
        K::Tab => "Tab",
        K::Backspace => "Backspace",
        K::ShiftLeft => "L Shift",
        K::ShiftRight => "R Shift",
        K::ControlLeft => "L Ctrl",
        K::ControlRight => "R Ctrl",
        K::AltLeft => "L Alt",
        K::AltRight => "R Alt",
        K::SuperLeft | K::SuperRight => "Super",
        K::CapsLock => "Caps Lock",
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::Backquote => "`",
        K::Minus => "-",
        K::Equal => "=",
        K::BracketLeft => "[",
        K::BracketRight => "]",
        K::Backslash => "\\",
        K::Semicolon => ";",
        K::Quote => "'",
        K::Comma => ",",
        K::Period => ".",
        K::Slash => "/",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "Page Up",
        K::PageDown => "Page Down",
        other => {
            // KeyA -> "A", Digit1 -> "1", Numpad5 -> "Num 5", F7 -> "F7".
            let debug = format!("{other:?}");
            return if let Some(rest) = debug.strip_prefix("Key") {
                rest.to_owned()
            } else if let Some(rest) = debug.strip_prefix("Digit") {
                rest.to_owned()
            } else if let Some(rest) = debug.strip_prefix("Numpad") {
                format!("Num {rest}")
            } else {
                debug
            };
        }
    };
    s.to_owned()
}

/// Held / pressed / released sets for one kind of button.
#[derive(Clone, Debug)]
struct ButtonSet<T> {
    down: HashSet<T>,
    pressed: HashSet<T>,
    released: HashSet<T>,
}

impl<T> Default for ButtonSet<T> {
    fn default() -> Self {
        Self {
            down: HashSet::new(),
            pressed: HashSet::new(),
            released: HashSet::new(),
        }
    }
}

impl<T: Copy + Eq + std::hash::Hash> ButtonSet<T> {
    /// Returns true if this was a fresh press.
    fn press(&mut self, b: T) -> bool {
        let fresh = self.down.insert(b);
        if fresh {
            self.pressed.insert(b);
        }
        fresh
    }

    fn release(&mut self, b: T) {
        if self.down.remove(&b) {
            self.released.insert(b);
        }
    }

    fn release_all(&mut self) {
        for b in self.down.drain() {
            self.released.insert(b);
        }
    }

    fn end_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
    }
}

/// Gamepad input event (converted from gilrs by the app).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GamepadEvent {
    Button(GamepadButton, bool),
    Axis(GamepadAxis, f32),
    Connected,
    Disconnected,
}

/// Input state of the current frame. See the module docs for the per-frame semantics.
#[derive(Clone, Debug, Default)]
pub struct InputState {
    keys: ButtonSet<KeyCode>,
    keys_repeated: HashSet<KeyCode>,
    mouse: ButtonSet<MouseButton>,
    pad: ButtonSet<GamepadButton>,
    pad_axes: ButtonSet<(GamepadAxis, AxisDirection)>,
    wheel: ButtonSet<WheelDirection>,
    axes: [f32; 6],
    mouse_delta: Vec2,
    scroll_delta: Vec2,
    typed_text: String,
    cursor: Option<Vec2>,
    focused: bool,
    last_pressed: Option<Binding>,
    gamepads_connected: u32,
    modifiers: winit::keyboard::ModifiersState,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            focused: true,
            ..Default::default()
        }
    }

    // ---- feeding events (called by `app`) ---------------------------------------------------

    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            if self.keys.press(code) {
                                self.note_press(Binding::Key(code));
                            }
                            self.keys_repeated.insert(code);
                        }
                        ElementState::Released => self.keys.release(code),
                    }
                }
                if event.state == ElementState::Pressed
                    && let Some(text) = &event.text
                {
                    self.typed_text
                        .extend(text.chars().filter(|c| !c.is_control()));
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    if self.mouse.press(*button) {
                        self.note_press(Binding::Mouse(*button));
                    }
                }
                ElementState::Released => self.mouse.release(*button),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Vec2::new(*x, *y),
                    MouseScrollDelta::PixelDelta(p) => {
                        Vec2::new(p.x as f32, p.y as f32) / PIXELS_PER_LINE
                    }
                };
                self.scroll_delta += lines;
                if lines.y != 0.0 {
                    let dir = if lines.y > 0.0 {
                        WheelDirection::Up
                    } else {
                        WheelDirection::Down
                    };
                    if self.wheel.press(dir) {
                        self.note_press(Binding::Wheel(dir));
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::Focused(focused) => {
                self.focused = *focused;
                if !focused {
                    // Avoid stuck keys when alt-tabbing away mid-press.
                    self.keys.release_all();
                    self.mouse.release_all();
                }
            }
            _ => {}
        }
    }

    /// Raw mouse motion (`DeviceEvent::MouseMotion`), unaffected by cursor locking/acceleration.
    pub fn handle_mouse_motion(&mut self, delta: (f64, f64)) {
        if self.focused {
            self.mouse_delta += Vec2::new(delta.0 as f32, delta.1 as f32);
        }
    }

    pub fn handle_gamepad_event(&mut self, event: GamepadEvent) {
        match event {
            GamepadEvent::Button(button, true) => {
                if self.pad.press(button) {
                    self.note_press(Binding::GamepadButton(button));
                }
            }
            GamepadEvent::Button(button, false) => self.pad.release(button),
            GamepadEvent::Axis(axis, value) => {
                let value = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                self.axes[axis.index()] = value;
                for dir in [AxisDirection::Positive, AxisDirection::Negative] {
                    let signed = if dir == AxisDirection::Positive {
                        value
                    } else {
                        -value
                    };
                    if signed >= AXIS_BUTTON_THRESHOLD {
                        if self.pad_axes.press((axis, dir)) {
                            self.note_press(Binding::GamepadAxis(axis, dir));
                        }
                    } else if signed < AXIS_BUTTON_THRESHOLD * 0.8 {
                        // A little hysteresis so a stick resting on the threshold doesn't chatter.
                        self.pad_axes.release((axis, dir));
                    }
                }
            }
            GamepadEvent::Connected => self.gamepads_connected += 1,
            GamepadEvent::Disconnected => {
                self.gamepads_connected = self.gamepads_connected.saturating_sub(1);
                if self.gamepads_connected == 0 {
                    self.pad.release_all();
                    self.pad_axes.release_all();
                    self.axes = [0.0; 6];
                }
            }
        }
    }

    /// Resets the per-frame state. Called by the app after every `Game::frame`.
    pub fn end_frame(&mut self) {
        self.keys.end_frame();
        self.keys_repeated.clear();
        self.mouse.end_frame();
        self.pad.end_frame();
        self.pad_axes.end_frame();
        // Wheel "buttons" are only down for the frame they scrolled.
        self.wheel.release_all();
        self.wheel.end_frame();
        self.mouse_delta = Vec2::ZERO;
        self.scroll_delta = Vec2::ZERO;
        self.typed_text.clear();
        self.last_pressed = None;
    }

    fn note_press(&mut self, binding: Binding) {
        if self.last_pressed.is_none() {
            self.last_pressed = Some(binding);
        }
    }

    // ---- queries ----------------------------------------------------------------------------

    pub fn key_down(&self, key: KeyCode) -> bool {
        self.keys.down.contains(&key)
    }

    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.keys.pressed.contains(&key)
    }

    pub fn key_released(&self, key: KeyCode) -> bool {
        self.keys.released.contains(&key)
    }

    /// Pressed this frame, including OS key repeats (for text fields / menu navigation).
    pub fn key_pressed_or_repeated(&self, key: KeyCode) -> bool {
        self.keys_repeated.contains(&key)
    }

    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse.down.contains(&button)
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse.pressed.contains(&button)
    }

    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.mouse.released.contains(&button)
    }

    pub fn gamepad_down(&self, button: GamepadButton) -> bool {
        self.pad.down.contains(&button)
    }

    pub fn gamepad_pressed(&self, button: GamepadButton) -> bool {
        self.pad.pressed.contains(&button)
    }

    pub fn gamepad_released(&self, button: GamepadButton) -> bool {
        self.pad.released.contains(&button)
    }

    pub fn is_down(&self, binding: Binding) -> bool {
        match binding {
            Binding::Key(k) => self.keys.down.contains(&k),
            Binding::Mouse(b) => self.mouse.down.contains(&b),
            Binding::Wheel(d) => self.wheel.pressed.contains(&d),
            Binding::GamepadButton(b) => self.pad.down.contains(&b),
            Binding::GamepadAxis(a, d) => self.pad_axes.down.contains(&(a, d)),
        }
    }

    pub fn pressed(&self, binding: Binding) -> bool {
        match binding {
            Binding::Key(k) => self.keys.pressed.contains(&k),
            Binding::Mouse(b) => self.mouse.pressed.contains(&b),
            Binding::Wheel(d) => self.wheel.pressed.contains(&d),
            Binding::GamepadButton(b) => self.pad.pressed.contains(&b),
            Binding::GamepadAxis(a, d) => self.pad_axes.pressed.contains(&(a, d)),
        }
    }

    pub fn released(&self, binding: Binding) -> bool {
        match binding {
            Binding::Key(k) => self.keys.released.contains(&k),
            Binding::Mouse(b) => self.mouse.released.contains(&b),
            Binding::Wheel(_) => false,
            Binding::GamepadButton(b) => self.pad.released.contains(&b),
            Binding::GamepadAxis(a, d) => self.pad_axes.released.contains(&(a, d)),
        }
    }

    /// The first binding pressed this frame (for "press a key to rebind" menus).
    pub fn last_pressed_binding(&self) -> Option<Binding> {
        self.last_pressed
    }

    /// Raw mouse movement this frame in device units (counts), +Y = down.
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    /// Scroll this frame in lines, +Y = away from the user.
    pub fn scroll_delta(&self) -> Vec2 {
        self.scroll_delta
    }

    /// Text typed this frame (no control characters).
    pub fn typed_text(&self) -> &str {
        &self.typed_text
    }

    /// Cursor position in window pixels (origin top-left), `None` when outside the window.
    pub fn cursor_position(&self) -> Option<Vec2> {
        self.cursor
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn modifiers(&self) -> winit::keyboard::ModifiersState {
        self.modifiers
    }

    pub fn gamepad_connected(&self) -> bool {
        self.gamepads_connected > 0
    }

    /// Raw axis value without deadzone.
    pub fn axis_raw(&self, axis: GamepadAxis) -> f32 {
        self.axes[axis.index()]
    }

    /// Left stick with radial deadzone, +Y = up.
    pub fn left_stick(&self) -> Vec2 {
        apply_deadzone(Vec2::new(
            self.axis_raw(GamepadAxis::LeftStickX),
            self.axis_raw(GamepadAxis::LeftStickY),
        ))
    }

    /// Right stick with radial deadzone, +Y = up.
    pub fn right_stick(&self) -> Vec2 {
        apply_deadzone(Vec2::new(
            self.axis_raw(GamepadAxis::RightStickX),
            self.axis_raw(GamepadAxis::RightStickY),
        ))
    }

    /// Left trigger in `[0, 1]` with deadzone.
    pub fn left_trigger(&self) -> f32 {
        trigger_deadzone(self.axis_raw(GamepadAxis::LeftTrigger))
    }

    /// Right trigger in `[0, 1]` with deadzone.
    pub fn right_trigger(&self) -> f32 {
        trigger_deadzone(self.axis_raw(GamepadAxis::RightTrigger))
    }

    /// Any axis with the appropriate deadzone applied.
    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        match axis {
            GamepadAxis::LeftStickX => self.left_stick().x,
            GamepadAxis::LeftStickY => self.left_stick().y,
            GamepadAxis::RightStickX => self.right_stick().x,
            GamepadAxis::RightStickY => self.right_stick().y,
            GamepadAxis::LeftTrigger => self.left_trigger(),
            GamepadAxis::RightTrigger => self.right_trigger(),
        }
    }

    /// Every axis, for debugging.
    pub fn axes(&self) -> impl Iterator<Item = (GamepadAxis, f32)> + '_ {
        GamepadAxis::ALL.into_iter().map(|a| (a, self.axis(a)))
    }
}

/// Radial deadzone, rescaled so output starts at 0 just outside the deadzone.
pub fn apply_deadzone(stick: Vec2) -> Vec2 {
    let len = stick.length();
    if len <= STICK_DEADZONE {
        return Vec2::ZERO;
    }
    let scaled = ((len - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).min(1.0);
    stick / len * scaled
}

fn trigger_deadzone(v: f32) -> f32 {
    if v <= TRIGGER_DEADZONE {
        0.0
    } else {
        ((v - TRIGGER_DEADZONE) / (1.0 - TRIGGER_DEADZONE)).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_lasts_exactly_one_frame() {
        let mut input = InputState::new();
        input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::South, true));
        // Duplicate press events (e.g. OS repeats) don't re-trigger.
        input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::South, true));
        assert!(input.gamepad_pressed(GamepadButton::South));
        assert!(input.is_down(Binding::GamepadButton(GamepadButton::South)));
        assert_eq!(
            input.last_pressed_binding(),
            Some(Binding::GamepadButton(GamepadButton::South))
        );
        input.end_frame();
        assert!(!input.gamepad_pressed(GamepadButton::South));
        assert!(input.gamepad_down(GamepadButton::South));
        assert_eq!(input.last_pressed_binding(), None);
        // Press + release inside one frame still reports pressed once.
        input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::South, false));
        input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::East, true));
        input.handle_gamepad_event(GamepadEvent::Button(GamepadButton::East, false));
        assert!(input.gamepad_released(GamepadButton::South));
        assert!(input.gamepad_pressed(GamepadButton::East));
        assert!(input.gamepad_released(GamepadButton::East));
        assert!(!input.gamepad_down(GamepadButton::East));
        input.end_frame();
        assert!(!input.gamepad_pressed(GamepadButton::East));
    }

    #[test]
    fn axis_as_button_with_hysteresis() {
        let mut input = InputState::new();
        let up = Binding::GamepadAxis(GamepadAxis::LeftStickY, AxisDirection::Positive);
        input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickY, 0.7));
        assert!(input.pressed(up));
        input.end_frame();
        input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickY, 0.45));
        assert!(input.is_down(up), "inside the hysteresis band");
        input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickY, 0.1));
        assert!(input.released(up));
        let down = Binding::GamepadAxis(GamepadAxis::LeftStickY, AxisDirection::Negative);
        input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickY, -0.9));
        assert!(input.pressed(down));
    }

    #[test]
    fn deadzones() {
        assert_eq!(apply_deadzone(Vec2::new(0.1, 0.1)), Vec2::ZERO);
        let full = apply_deadzone(Vec2::new(1.0, 0.0));
        assert!((full.x - 1.0).abs() < 1e-6);
        let mut input = InputState::new();
        input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::RightTrigger, 0.03));
        assert_eq!(input.right_trigger(), 0.0);
        input.handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::RightTrigger, 1.0));
        assert_eq!(input.right_trigger(), 1.0);
    }

    #[test]
    fn mouse_delta_accumulates_and_resets() {
        let mut input = InputState::new();
        input.handle_mouse_motion((3.0, -1.0));
        input.handle_mouse_motion((2.0, 4.0));
        assert_eq!(input.mouse_delta(), Vec2::new(5.0, 3.0));
        input.end_frame();
        assert_eq!(input.mouse_delta(), Vec2::ZERO);
    }

    #[test]
    fn binding_names_and_serde() {
        assert_eq!(Binding::Key(KeyCode::KeyW).name(), "W");
        assert_eq!(Binding::Key(KeyCode::Digit3).name(), "3");
        assert_eq!(Binding::Key(KeyCode::ShiftLeft).name(), "L Shift");
        assert_eq!(Binding::Key(KeyCode::F5).name(), "F5");
        assert_eq!(Binding::Mouse(MouseButton::Back).name(), "Mouse 4");
        assert_eq!(Binding::Wheel(WheelDirection::Down).name(), "Wheel Down");
        assert_eq!(
            Binding::GamepadAxis(GamepadAxis::RightStickX, AxisDirection::Negative).name(),
            "Right Stick Left"
        );
        let bindings = vec![
            Binding::Key(KeyCode::Space),
            Binding::Mouse(MouseButton::Left),
            Binding::GamepadButton(GamepadButton::RightBumper),
            Binding::GamepadAxis(GamepadAxis::LeftTrigger, AxisDirection::Positive),
        ];
        let text = ron::to_string(&bindings).unwrap();
        let back: Vec<Binding> = ron::from_str(&text).unwrap();
        assert_eq!(back, bindings);
    }
}
