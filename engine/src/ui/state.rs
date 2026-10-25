//! Widget ids and the UI state that persists between frames.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use glam::Vec2;

/// Stable widget identity: a hash of the widget's label (or `##id` suffix) and the id stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Id(pub u64);

impl Id {
    /// Root id (empty id stack).
    pub const ROOT: Id = Id(0xcbf2_9ce4_8422_2325);

    /// Child id of `self` derived from any hashable value. Deterministic across runs and
    /// platforms (FNV-1a), so ids can be logged or stored.
    pub fn with(self, source: impl Hash) -> Id {
        let mut h = Fnv(self.0);
        source.hash(&mut h);
        Id(h.finish())
    }
}

/// FNV-1a; `std`'s default hasher is not guaranteed stable between Rust versions.
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ScrollState {
    pub offset: f32,
    pub content_height: f32,
    pub view_height: f32,
    pub at_bottom: bool,
}

impl ScrollState {
    pub fn max_offset(&self) -> f32 {
        (self.content_height - self.view_height).max(0.0)
    }
}

/// UI state that must survive between frames: focus, mouse capture, text cursor, key capture
/// and scroll offsets. Create one per menu stack (or one for the whole game) and pass it to
/// [`super::Ui::new`] every frame.
#[derive(Clone, Debug, Default)]
pub struct UiState {
    /// Widget with keyboard / gamepad focus.
    pub(crate) focused: Option<Id>,
    /// Widget that owns the mouse (pressed on it and not released yet).
    pub(crate) active: Option<Id>,
    /// Keybind widget waiting for the game to deliver a key.
    pub(crate) capture: Option<Id>,
    /// Capture started this frame (not reported as `capturing` until the next frame so the
    /// key/click that started it is not bound).
    pub(crate) capture_fresh: bool,
    /// Capture cancelled by Escape at the start of this frame.
    pub(crate) capture_cancelled: Option<Id>,
    /// Text field the cursor below belongs to.
    pub(crate) cursor_owner: Option<Id>,
    /// Text cursor position in characters.
    pub(crate) cursor: usize,
    /// First visible character of the focused text field.
    pub(crate) text_scroll: usize,
    /// Time of the last edit (keeps the cursor solid while typing).
    pub(crate) last_edit: f64,
    pub(crate) scroll: HashMap<Id, ScrollState>,
    /// Scroll areas that contained the focused widget last frame (they take PageUp/PageDown).
    pub(crate) scroll_focus: HashSet<Id>,
    /// Focusable widgets of the previous frame in layout order (for keyboard navigation).
    pub(crate) focus_order: Vec<Id>,
    pub(crate) last_mouse: Option<Vec2>,
    /// Focus moved via keys this frame: scroll areas bring the focused widget into view.
    pub(crate) keyboard_nav: bool,
    /// Focus was set programmatically this frame; don't drop it if the widget wasn't seen.
    pub(crate) focus_requested: bool,
    /// Last time each widget's value changed (for short "flash" animations).
    pub(crate) changed_at: HashMap<Id, f64>,
    pub(crate) text_focused: bool,
    pub(crate) mouse_over_ui: bool,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The focused widget, if any.
    pub fn focused(&self) -> Option<Id> {
        self.focused
    }

    /// Focus a widget (e.g. the chat field when chat opens). Use [`super::Ui::id`] to get ids.
    pub fn set_focus(&mut self, id: Option<Id>) {
        self.focused = id;
        self.focus_requested = true;
    }

    /// Forget focus, mouse capture and key capture (e.g. when switching menu screens).
    pub fn reset_interaction(&mut self) {
        self.focused = None;
        self.active = None;
        self.capture = None;
        self.cursor_owner = None;
    }

    /// Whether a keybind widget is waiting for a key. While true the game should not treat
    /// key presses as game actions; the UI itself ignores everything except Escape.
    pub fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// The keybind widget that is ready to receive a key: the game should deliver the next
    /// pressed key / button to it and then call [`UiState::finish_capture`]. `None` during
    /// (and after) the frame the capture started, so the activating click or Enter press is
    /// not bound; check it after building the frame's UI (or use
    /// [`super::KeybindResponse::capturing`] inside it).
    pub fn capture_target(&self) -> Option<Id> {
        self.capture.filter(|_| !self.capture_fresh)
    }

    /// End key capture after the game assigned the new binding.
    pub fn finish_capture(&mut self) {
        self.capture = None;
        self.capture_fresh = false;
    }

    /// Abort key capture without changing the binding.
    pub fn cancel_capture(&mut self) {
        self.finish_capture();
    }

    /// A text field has focus: the game should not interpret typed letters as game actions.
    pub fn wants_text_input(&self) -> bool {
        self.text_focused
    }

    /// The mouse was over a panel or widget last frame (clicks should not reach the game).
    pub fn mouse_over_ui(&self) -> bool {
        self.mouse_over_ui
    }

    /// Current scroll offset of a scroll area (0 = top).
    pub fn scroll_offset(&self, id: Id) -> f32 {
        self.scroll.get(&id).map_or(0.0, |s| s.offset)
    }

    /// Set a scroll area's offset (clamped when the area is next drawn). `f32::INFINITY`
    /// scrolls to the bottom.
    pub fn set_scroll_offset(&mut self, id: Id, offset: f32) {
        self.scroll.entry(id).or_default().offset = offset;
    }

    pub(crate) fn move_focus(&mut self, delta: isize) {
        let n = self.focus_order.len();
        if n == 0 {
            return;
        }
        let current = self
            .focused
            .and_then(|f| self.focus_order.iter().position(|&x| x == f));
        let next = match current {
            Some(i) => (i as isize + delta).rem_euclid(n as isize) as usize,
            None if delta > 0 => 0,
            None => n - 1,
        };
        self.focused = Some(self.focus_order[next]);
        self.keyboard_nav = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_distinct() {
        let a = Id::ROOT.with("Volume");
        assert_eq!(a, Id::ROOT.with("Volume"));
        assert_ne!(a, Id::ROOT.with("Volume2"));
        assert_ne!(a.with(1usize), a.with(2usize));
    }

    #[test]
    fn focus_wraps() {
        let mut s = UiState::new();
        s.focus_order = vec![Id(1), Id(2), Id(3)];
        s.move_focus(1);
        assert_eq!(s.focused, Some(Id(1)));
        s.move_focus(-1);
        assert_eq!(s.focused, Some(Id(3)));
        s.move_focus(1);
        assert_eq!(s.focused, Some(Id(1)));
    }
}
