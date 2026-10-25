//! The immediate-mode frame builder [`Ui`] and its widgets.

use glam::{Vec2, Vec4};

use super::draw::{self, Rect, border, clip_quads, fade, fill, mix, slanted};
use super::font::{CAP_HEIGHT, GLYPH_ADVANCE, GLYPH_H};
use super::input::{UiInput, UiKey};
use super::state::{Id, ScrollState, UiState};
use super::text::{self, Align, TextStyle, draw_glyph, draw_text, line_width, max_chars};
use super::theme::Theme;
use crate::gfx::UiDrawList;

/// Which characters a text field accepts.
#[derive(Clone, Copy, Debug, Default)]
pub enum TextFilter {
    /// Any printable ASCII character (the font has no other glyphs).
    #[default]
    Any,
    /// `0-9` only (ports, counts).
    Digits,
    /// Digits, `.` and `-`.
    Decimal,
    /// Host names, IPv4 / IPv6 addresses and ports: letters, digits, `. : - _ [ ]`.
    Address,
    /// Player names: letters, digits, space, `_ - .`.
    Name,
    /// Custom predicate.
    Custom(fn(char) -> bool),
}

impl TextFilter {
    pub fn accepts(&self, c: char) -> bool {
        if !(' '..='~').contains(&c) {
            return false;
        }
        match self {
            TextFilter::Any => true,
            TextFilter::Digits => c.is_ascii_digit(),
            TextFilter::Decimal => c.is_ascii_digit() || c == '.' || c == '-',
            TextFilter::Address => c.is_ascii_alphanumeric() || ".:-_[]".contains(c),
            TextFilter::Name => c.is_ascii_alphanumeric() || " _-.".contains(c),
            TextFilter::Custom(f) => f(c),
        }
    }
}

/// Interaction result of a single widget (see [`Ui::interact`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Response {
    pub rect: Rect,
    /// Mouse is over the widget (and inside the current clip area).
    pub hovered: bool,
    /// Mouse button went down on the widget this frame.
    pub pressed: bool,
    /// Mouse button is held after being pressed on the widget (dragging).
    pub held: bool,
    /// Pressed and released inside the widget.
    pub clicked: bool,
    /// Widget has keyboard / gamepad focus.
    pub focused: bool,
    /// Clicked, or Enter pressed while focused.
    pub activated: bool,
}

/// Result of [`Ui::text_field`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextFieldResponse {
    /// The text was edited this frame.
    pub changed: bool,
    /// Enter was pressed while the field had focus.
    pub submitted: bool,
    pub focused: bool,
}

/// Result of [`Ui::keybind`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeybindResponse {
    /// The widget was activated this frame and now waits for a key. The key press / click
    /// that activated it is *not* meant to be bound, so don't resolve on this frame.
    pub started: bool,
    /// Waiting for a key: deliver the first pressed key / mouse button / gamepad button,
    /// then call [`Ui::finish_capture`] (or [`UiState::finish_capture`]).
    pub capturing: bool,
    /// Escape cancelled the capture this frame; keep the old binding.
    pub cancelled: bool,
    /// Backspace / Delete pressed while focused (not capturing): the player wants to unbind.
    pub cleared: bool,
}

#[derive(Clone, Debug)]
struct Row {
    widths: Vec<f32>,
    index: usize,
}

#[derive(Clone, Debug)]
struct Layout {
    rect: Rect,
    cursor: Vec2,
    /// Lowest y used by any widget so far.
    max_y: f32,
    row: Option<Row>,
}

impl Layout {
    fn vertical(rect: Rect) -> Self {
        Self {
            rect,
            cursor: rect.min,
            max_y: rect.min.y,
            row: None,
        }
    }
}

/// One frame of immediate-mode UI. Create it after clearing the draw list, call widgets, then
/// drop it (or call [`Ui::end`]) to store focus order and other state for the next frame.
///
/// ```ignore
/// let mut ui = Ui::new(&mut draw_list, &mut ui_state, &ui_input, &theme);
/// ui.panel(Rect::from_center_size(center, vec2(300.0, 200.0)), "PAUSED", |ui| {
///     if ui.button("RESUME") { paused = false; }
///     ui.slider("VOLUME", &mut volume, 0.0, 1.0, 0.05, "{%}%");
/// });
/// ```
pub struct Ui<'a> {
    list: &'a mut UiDrawList,
    state: &'a mut UiState,
    input: &'a UiInput,
    theme: &'a Theme,
    layouts: Vec<Layout>,
    id_stack: Vec<Id>,
    clip: Option<Rect>,
    /// Open scroll areas: (id, visible rect).
    scrolls: Vec<(Id, Rect)>,
    /// Scroll adjustment requested by keyboard navigation: (area id, delta).
    scroll_request: Option<(Id, f32)>,
    /// Innermost scroll area that contains the focused widget this frame.
    focus_scroll: Option<Id>,
    focus_order: Vec<Id>,
    /// Left / Right already moved the focus between row buttons this frame.
    row_nav_done: bool,
    enabled: bool,
    escape_consumed: bool,
    mouse_moved: bool,
    default_focus: bool,
    text_focused: bool,
    capture_seen: bool,
    /// Widget the mouse moved onto this frame; it gets focus when the frame ends so two
    /// widgets are never drawn focused in the same frame.
    hover_focus: Option<Id>,
}

impl<'a> Ui<'a> {
    /// Begin a UI frame. Processes focus navigation (Up / Down / Tab) and key-capture
    /// cancellation (Escape) up front, so widgets see the new focus this frame.
    pub fn new(
        list: &'a mut UiDrawList,
        state: &'a mut UiState,
        input: &'a UiInput,
        theme: &'a Theme,
    ) -> Self {
        let mouse_moved = state
            .last_mouse
            .is_some_and(|m| m.distance_squared(input.mouse_pos) > 1e-4);
        state.keyboard_nav = false;
        state.capture_cancelled = None;
        // A capture started last frame is now ready for the game to resolve.
        state.capture_fresh = false;
        state.mouse_over_ui = false;
        let mut escape_consumed = false;
        for &key in &input.keys {
            if let Some(capturing) = state.capture {
                // While capturing, every key belongs to the game's binding logic except Escape.
                if key == UiKey::Escape {
                    state.capture = None;
                    state.capture_fresh = false;
                    state.capture_cancelled = Some(capturing);
                    escape_consumed = true;
                }
                continue;
            }
            match key {
                UiKey::Up => state.move_focus(-1),
                UiKey::Down | UiKey::Tab => state.move_focus(1),
                _ => {}
            }
        }
        let canvas = Rect::canvas(list);
        Self {
            list,
            state,
            input,
            theme,
            layouts: vec![Layout::vertical(canvas)],
            id_stack: vec![Id::ROOT],
            clip: None,
            scrolls: Vec::new(),
            scroll_request: None,
            focus_scroll: None,
            focus_order: Vec::new(),
            row_nav_done: false,
            enabled: true,
            escape_consumed,
            mouse_moved,
            default_focus: false,
            text_focused: false,
            capture_seen: false,
            hover_focus: None,
        }
    }

    /// End the frame (same as dropping the `Ui`).
    pub fn end(self) {}

    // ------------------------------------------------------------------ accessors

    pub fn theme(&self) -> &'a Theme {
        self.theme
    }

    pub fn input(&self) -> &'a UiInput {
        self.input
    }

    pub fn time(&self) -> f64 {
        self.input.time
    }

    /// The draw list, for custom drawing between widgets.
    pub fn painter(&mut self) -> &mut UiDrawList {
        self.list
    }

    /// Persistent state (focus, capture, scroll offsets).
    pub fn state(&mut self) -> &mut UiState {
        self.state
    }

    /// The whole canvas.
    pub fn canvas(&self) -> Rect {
        Rect::canvas(self.list)
    }

    /// Escape was pressed and not consumed by a widget (e.g. cancelling a key capture):
    /// use it for "back" / "close menu".
    pub fn back_pressed(&self) -> bool {
        self.input.key(UiKey::Escape) && !self.escape_consumed
    }

    /// End key capture after assigning the binding (see [`KeybindResponse::capturing`]).
    pub fn finish_capture(&mut self) {
        self.state.finish_capture();
    }

    /// Disable (grey out, make non-interactive and unfocusable) the following widgets.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Run `f` with widgets enabled or disabled, restoring the previous setting afterwards.
    pub fn enabled_scope<R>(&mut self, enabled: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        let prev = self.enabled;
        self.enabled = prev && enabled;
        let r = f(self);
        self.enabled = prev;
        r
    }

    /// If nothing has focus, focus the next focusable widget. Call at the top of a menu so
    /// keyboard / gamepad players start on its first entry.
    pub fn default_focus(&mut self) {
        self.default_focus = self.state.focused.is_none();
    }

    /// Focus the widget with this label (in the current id scope).
    pub fn request_focus(&mut self, label: &str) {
        let id = self.id(label);
        self.state.set_focus(Some(id));
    }

    // ------------------------------------------------------------------ ids

    /// Id of a widget label in the current scope. The text after `##` is not displayed, so
    /// `"PLAY##coop"` shows "PLAY" but has its own id.
    pub fn id(&self, label: &str) -> Id {
        self.id_stack
            .last()
            .copied()
            .unwrap_or(Id::ROOT)
            .with(label)
    }

    /// Push an id scope (for repeated labels, e.g. one row per player).
    pub fn push_id(&mut self, source: impl std::hash::Hash) {
        let id = self
            .id_stack
            .last()
            .copied()
            .unwrap_or(Id::ROOT)
            .with(source);
        self.id_stack.push(id);
    }

    pub fn pop_id(&mut self) {
        if self.id_stack.len() > 1 {
            self.id_stack.pop();
        }
    }

    /// Run `f` inside an id scope.
    pub fn with_id<R>(
        &mut self,
        source: impl std::hash::Hash,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.push_id(source);
        let r = f(self);
        self.pop_id();
        r
    }

    fn label_id<'l>(&self, label: &'l str) -> (&'l str, Id) {
        (display_text(label), self.id(label))
    }

    // ------------------------------------------------------------------ layout

    /// Reserve the next `height` units of the current layout (full width in a column, the
    /// next cell in a row). Use for custom widgets.
    pub fn allocate(&mut self, height: f32) -> Rect {
        let spacing = self.theme.spacing;
        let Some(l) = self.layouts.last_mut() else {
            return Rect::ZERO;
        };
        let r = match &mut l.row {
            None => {
                let r = Rect::from_min_size(l.cursor, Vec2::new(l.rect.width(), height));
                l.cursor.y += height + spacing;
                r
            }
            Some(row) => {
                let w = row
                    .widths
                    .get(row.index)
                    .copied()
                    .unwrap_or_else(|| (l.rect.max.x - l.cursor.x).max(0.0));
                row.index += 1;
                let r = Rect::from_min_size(l.cursor, Vec2::new(w, height));
                l.cursor.x += w + spacing;
                r
            }
        };
        l.max_y = l.max_y.max(r.max.y);
        r
    }

    /// Remaining area of the current layout (from the cursor to the layout's bottom-right).
    pub fn available_rect(&self) -> Rect {
        self.layouts.last().map_or(Rect::ZERO, |l| {
            Rect::new(l.cursor, l.rect.max.max(l.cursor))
        })
    }

    /// Vertical gap.
    pub fn spacing(&mut self, height: f32) {
        if let Some(l) = self.layouts.last_mut() {
            if l.row.is_some() {
                l.cursor.x += height;
            } else {
                l.cursor.y += height;
                l.max_y = l.max_y.max(l.cursor.y);
            }
        }
    }

    /// Lay out widgets in a horizontal row of `columns` equal cells.
    pub fn row<R>(&mut self, columns: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        let weights = vec![1.0; columns.max(1)];
        self.row_weights(&weights, f)
    }

    /// Lay out widgets in a horizontal row with cell widths proportional to `weights`.
    pub fn row_weights<R>(&mut self, weights: &[f32], f: impl FnOnce(&mut Self) -> R) -> R {
        let spacing = self.theme.spacing;
        let (start, width) = match self.layouts.last() {
            Some(l) => (l.cursor, l.rect.max.x - l.cursor.x),
            None => (Vec2::ZERO, 0.0),
        };
        let total: f32 = weights.iter().sum::<f32>().max(1e-6);
        let free = (width - spacing * (weights.len().saturating_sub(1)) as f32).max(0.0);
        let widths = weights.iter().map(|w| (free * w / total).floor()).collect();
        self.layouts.push(Layout {
            rect: Rect::new(start, Vec2::new(start.x + width, start.y)),
            cursor: start,
            max_y: start.y,
            row: Some(Row { widths, index: 0 }),
        });
        let r = f(self);
        let row = self.layouts.pop();
        if let (Some(row), Some(parent)) = (row, self.layouts.last_mut()) {
            parent.cursor.y = row.max_y + spacing;
            parent.max_y = parent.max_y.max(row.max_y);
        }
        r
    }

    /// Lay out widgets vertically inside `rect` without drawing a frame.
    pub fn area<R>(&mut self, rect: Rect, f: impl FnOnce(&mut Self) -> R) -> R {
        self.layouts.push(Layout::vertical(rect));
        let r = f(self);
        self.layouts.pop();
        r
    }

    // ------------------------------------------------------------------ panels

    /// A framed panel with an optional title bar; widgets inside are laid out vertically.
    /// Widget ids inside are scoped by the title.
    pub fn panel<R>(&mut self, rect: Rect, title: &str, f: impl FnOnce(&mut Self) -> R) -> R {
        self.panel_impl(rect, title, false, f)
    }

    /// Like [`Ui::panel`] but the height grows to fit the content (top-left at `pos`).
    pub fn panel_auto<R>(
        &mut self,
        pos: Vec2,
        width: f32,
        title: &str,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.panel_impl(
            Rect::from_min_size(pos, Vec2::new(width, 0.0)),
            title,
            true,
            f,
        )
    }

    fn panel_impl<R>(
        &mut self,
        rect: Rect,
        title: &str,
        auto_height: bool,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let t = self.theme;
        let rect = rect.round();
        let start = self.list.quads.len();
        let title_h = if title.is_empty() {
            2.0
        } else {
            t.title_height
        };
        let content = Rect::new(
            rect.min + Vec2::new(t.padding, title_h + t.padding * 0.75),
            Vec2::new(
                rect.max.x - t.padding,
                rect.max.y.max(rect.min.y) - t.padding,
            ),
        );
        self.push_id(title);
        self.layouts.push(Layout::vertical(content));
        let r = f(self);
        let layout = self.layouts.pop();
        self.pop_id();
        let bottom = match (auto_height, layout) {
            (true, Some(l)) => (l.max_y + t.padding).round(),
            _ => rect.max.y,
        };
        let frame = Rect::new(rect.min, Vec2::new(rect.max.x, bottom));
        let mut bg = UiDrawList::new(self.list.canvas_size);
        draw_panel_frame(&mut bg, frame, title, t);
        self.list.quads.splice(start..start, bg.quads);
        if frame.contains(self.input.mouse_pos) {
            self.state.mouse_over_ui = true;
        }
        r
    }

    // ------------------------------------------------------------------ interaction

    /// Core interaction logic for a widget occupying `rect`. Custom widgets call this and
    /// draw themselves based on the response.
    pub fn interact(&mut self, id: Id, rect: Rect, focusable: bool) -> Response {
        let focusable = focusable && self.enabled;
        if focusable {
            self.focus_order.push(id);
            if self.default_focus && self.state.focused.is_none() {
                self.state.focused = Some(id);
                self.default_focus = false;
            }
        }
        let live = self.enabled && self.state.capture.is_none();
        let mouse = self.input.mouse_pos;
        let hovered = live && rect.contains(mouse) && self.clip.is_none_or(|c| c.contains(mouse));
        if hovered {
            self.state.mouse_over_ui = true;
            if focusable && self.mouse_moved {
                self.hover_focus = Some(id);
            }
        }
        let pressed = hovered && self.input.mouse_pressed;
        if pressed {
            self.state.active = Some(id);
            if focusable {
                self.state.focused = Some(id);
            }
        }
        let active = live && self.state.active == Some(id);
        let clicked = active && self.input.mouse_released && hovered;
        let held = active && self.input.mouse_down;
        let focused = focusable && self.state.focused == Some(id);
        if focused && let Some(&(scroll_id, view)) = self.scrolls.last() {
            self.focus_scroll = Some(scroll_id);
            if self.state.keyboard_nav && !view.contains_rect(rect) {
                let delta = if rect.min.y < view.min.y || rect.height() > view.height() {
                    rect.min.y - view.min.y
                } else {
                    rect.max.y - view.max.y
                };
                self.scroll_request = Some((scroll_id, delta));
            }
        }
        let activated = clicked || (focused && live && self.input.key(UiKey::Enter));
        Response {
            rect,
            hovered,
            pressed,
            held,
            clicked,
            focused,
            activated,
        }
    }

    fn key_count(&self, key: UiKey) -> usize {
        if self.state.capture.is_some() {
            0
        } else {
            self.input.key_count(key)
        }
    }

    fn mark_changed(&mut self, id: Id) {
        self.state.changed_at.insert(id, self.input.time);
    }

    /// Seconds since the widget's value last changed (large if never).
    fn since_change(&self, id: Id) -> f64 {
        self.state
            .changed_at
            .get(&id)
            .map_or(f64::MAX, |t| self.input.time - t)
    }

    // ------------------------------------------------------------------ drawing helpers

    fn text_color(&self, focused: bool) -> Vec4 {
        if !self.enabled {
            self.theme.ash
        } else if focused {
            self.theme.bone
        } else {
            self.theme.bone_dim
        }
    }

    /// Row highlight and left bar for focused widgets (the `>` marker is drawn with the label).
    fn draw_focus(&mut self, rect: Rect, resp: &Response) {
        let t = self.theme;
        if resp.focused {
            fill(self.list, rect, t.focus_bg);
            fill(
                self.list,
                Rect::new(rect.min, Vec2::new(rect.min.x + 2.0, rect.max.y)),
                t.blood_bright,
            );
        } else if resp.hovered {
            fill(self.list, rect, fade(t.widget_hover, 0.6));
        }
    }

    /// The animated focus marker: a `>` that nudges right and blinks between red and bone.
    fn draw_marker(&mut self, x: f32, rect: Rect) {
        let t = self.theme;
        let time = self.input.time;
        let nudge = if (time * 5.0).fract() < 0.5 { 0.0 } else { 1.0 };
        let color = if t.blink(time) {
            t.blood_bright
        } else {
            t.bone
        };
        let y = (rect.center().y - CAP_HEIGHT * 0.5).round();
        draw_glyph(self.list, Vec2::new(x + nudge, y), '>', 1.0, color);
    }

    /// Row label; when focused it slides right to make room for the `>` marker.
    fn draw_label(&mut self, rect: Rect, label: &str, focused: bool) {
        let color = self.text_color(focused);
        self.draw_label_colored(rect, label, focused, color);
    }

    fn draw_label_colored(&mut self, rect: Rect, label: &str, focused: bool, color: Vec4) {
        let style = TextStyle::new(color).scale(self.theme.text_scale);
        let indent = if focused && self.enabled {
            self.draw_marker(rect.min.x + 5.0, rect);
            15.0
        } else {
            6.0
        };
        let shown = text::truncate(label, rect.width() - indent - 2.0, style.scale);
        let y = text::text_top_in(rect, &shown, style.scale, 0.0);
        draw_text(self.list, Vec2::new(rect.min.x + indent, y), &shown, &style);
    }

    fn text_in(&mut self, rect: Rect, s: &str, style: &TextStyle) {
        let x = match style.align {
            Align::Left => rect.min.x,
            Align::Center => rect.center().x,
            Align::Right => rect.max.x,
        };
        let y = text::text_top_in(rect, s, style.scale, text::text_size(s, style).y);
        draw_text(self.list, Vec2::new(x, y), s, style);
    }

    /// Allocate a row, draw focus + label, return (row, control rect, response).
    fn control_row(&mut self, label: &str, id: Id) -> (Rect, Rect, Response) {
        let t = self.theme;
        let rect = self.allocate(t.row_height);
        let resp = self.interact(id, rect, true);
        self.draw_focus(rect, &resp);
        let (label_rect, control) = rect.split_left((rect.width() * t.label_fraction).round());
        self.draw_label(label_rect, label, resp.focused);
        (rect, control.shrink2(Vec2::new(2.0, 1.0)), resp)
    }

    // ------------------------------------------------------------------ static widgets

    /// One line of secondary text.
    pub fn label(&mut self, text: &str) {
        let style = TextStyle::new(self.text_color(false)).scale(self.theme.text_scale);
        self.text(text, &style);
    }

    /// One line of text in a custom colour.
    pub fn label_colored(&mut self, text: &str, color: Vec4) {
        let style = TextStyle::new(color).scale(self.theme.text_scale);
        self.text(text, &style);
    }

    /// Text with a full custom style, aligned inside the allocated row / cell.
    pub fn text(&mut self, text: &str, style: &TextStyle) {
        let size = text::text_size(text, style);
        let rect = self.allocate(size.y + 2.0 * style.scale);
        let inner = Rect::new(
            rect.min + Vec2::new(6.0, 0.0),
            rect.max - Vec2::new(2.0, 0.0),
        );
        self.text_in(inner, text, style);
    }

    /// Word-wrapped paragraph.
    pub fn label_wrapped(&mut self, text: &str) {
        let style = TextStyle::new(self.text_color(false)).scale(self.theme.text_scale);
        let width = self.available_rect().width() - 8.0;
        let lines = text::wrap_text(text, width, style.scale);
        let h = lines.len() as f32 * style.line_height();
        let rect = self.allocate(h);
        text::draw_lines(
            self.list,
            rect.min + Vec2::new(6.0, 1.0),
            lines.into_iter(),
            &style,
        );
    }

    /// A wrapped chat message (`name:` in `name_color`), e.g. inside a sticky scroll area.
    /// An empty `name` draws a system line in `name_color`.
    pub fn chat_line(&mut self, name: &str, name_color: Vec4, message: &str) {
        let t = self.theme;
        let width = self.available_rect().width() - 4.0;
        let lines = super::hud::chat_wrap(name, message, width).len();
        let rect = self.allocate(lines as f32 * super::hud::CHAT_LINE_HEIGHT - 1.0);
        let body = if name.is_empty() { name_color } else { t.bone };
        super::hud::draw_chat_line(
            self.list,
            t,
            rect.min + Vec2::new(2.0, 0.0),
            width,
            name,
            name_color,
            message,
            body,
            1.0,
        );
    }

    /// Label on the left, value right-aligned (info rows: ping, server name...).
    pub fn value_row(&mut self, label: &str, value: &str) {
        let t = self.theme;
        let rect = self.allocate(t.row_height);
        self.draw_label(rect, label, false);
        let style = TextStyle::new(if self.enabled { t.bone } else { t.ash }).right();
        self.text_in(rect.shrink2(Vec2::new(4.0, 0.0)), value, &style);
    }

    /// Section heading: a blood-red tab, the text, and a rule to the right.
    pub fn heading(&mut self, text: &str) {
        let t = self.theme;
        let rect = self.allocate(t.row_height);
        let y = (rect.center().y - CAP_HEIGHT * 0.5).round();
        fill(
            self.list,
            Rect::from_min_size(Vec2::new(rect.min.x, y), Vec2::new(3.0, CAP_HEIGHT)),
            t.blood_bright,
        );
        let style = TextStyle::new(t.bone).shadow(t.shadow);
        let r = draw_text(self.list, Vec2::new(rect.min.x + 6.0, y), text, &style);
        let line_y = y + (CAP_HEIGHT * 0.5).floor();
        if r.max.x + 6.0 < rect.max.x {
            draw::hline(self.list, r.max.x + 5.0, rect.max.x, line_y, t.panel_edge);
        }
    }

    /// Big title text (scale 2) with a blood shadow.
    pub fn title(&mut self, text: &str) {
        let t = self.theme;
        let style = TextStyle::new(t.bone).scale(2.0).shadow(t.blood);
        self.text(text, &style);
    }

    /// Thin horizontal rule.
    pub fn separator(&mut self) {
        let t = self.theme;
        let rect = self.allocate(3.0);
        let y = rect.min.y + 1.0;
        draw::hline(self.list, rect.min.x, rect.max.x, y, t.panel_edge);
        draw::hline(self.list, rect.min.x, rect.min.x + 8.0, y, t.blood);
    }

    // ------------------------------------------------------------------ buttons

    /// A push button; returns true when clicked or activated with Enter.
    pub fn button(&mut self, label: &str) -> bool {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let in_row = self.layouts.last().is_some_and(|l| l.row.is_some());
        let rect = self.allocate(t.row_height + 2.0);
        let resp = self.interact(id, rect, true);
        // Buttons side by side (PLAY | BACK, RESTART | MAIN MENU): Left / Right (d-pad) move
        // between them like Up / Down; the new focus shows from the next frame.
        if resp.focused && in_row && self.enabled && !self.row_nav_done {
            let steps =
                self.key_count(UiKey::Right) as isize - self.key_count(UiKey::Left) as isize;
            if steps != 0 {
                self.state.move_focus(steps.signum());
                self.state.focus_requested = true;
                self.row_nav_done = true;
            }
        }
        let down = resp.held && resp.hovered;
        let bg = if !self.enabled {
            fade(t.widget, 0.5)
        } else if down {
            t.blood_dark
        } else if resp.focused {
            t.blood
        } else if resp.hovered {
            t.widget_hover
        } else {
            t.widget
        };
        fill(self.list, rect, bg);
        // Bevel: light top edge, dark bottom edge.
        draw::hline(
            self.list,
            rect.min.x,
            rect.max.x,
            rect.min.y,
            fade(t.bone, 0.10),
        );
        draw::hline(
            self.list,
            rect.min.x,
            rect.max.x,
            rect.max.y - 1.0,
            fade(t.ink, 0.8),
        );
        if resp.focused {
            draw::corner_brackets(self.list, rect, 4.0, 1.0, t.bone);
            self.draw_marker(rect.min.x + 5.0, rect);
        }
        let color = if self.enabled {
            if resp.focused { t.bone } else { t.bone_dim }
        } else {
            t.ash
        };
        let mut style = TextStyle::new(color).centered();
        if resp.focused {
            style = style.shadow(t.blood_dark);
        }
        let text_rect = rect.translate(Vec2::new(0.0, if down { 1.0 } else { 0.0 }));
        self.text_in(text_rect, text, &style);
        resp.activated && self.enabled
    }

    /// Large main-menu entry (scale-2 text, slanted highlight when focused).
    pub fn menu_item(&mut self, label: &str) -> bool {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let scale = 2.0;
        let rect = self.allocate(GLYPH_H * scale + 6.0);
        let resp = self.interact(id, rect, true);
        let time = self.input.time;
        let text_x = rect.min.x + 14.0;
        let text_w = line_width(text, scale);
        if resp.focused {
            let slab = Rect::new(
                rect.min,
                Vec2::new(text_x + text_w + 10.0 + rect.height(), rect.max.y),
            );
            slanted(
                self.list,
                slab,
                true,
                if resp.held { t.blood_dark } else { t.blood },
            );
            fill(
                self.list,
                Rect::new(rect.min, Vec2::new(rect.min.x + 3.0, rect.max.y)),
                t.blood_bright,
            );
            // Speed streaks trailing the slab.
            let x0 = slab.max.x + 4.0;
            for (i, len) in [18.0, 10.0, 26.0].into_iter().enumerate() {
                let phase = ((time * 1.7 + i as f64 * 0.37).fract() as f32) * 8.0;
                let y = rect.min.y + 5.0 + i as f32 * 6.0;
                draw::hline(
                    self.list,
                    x0 + phase,
                    x0 + phase + len,
                    y,
                    fade(t.blood_bright, 0.7),
                );
            }
        } else if resp.hovered {
            fill(self.list, rect, fade(t.widget_hover, 0.6));
        } else {
            fill(
                self.list,
                Rect::new(rect.min, Vec2::new(rect.min.x + 2.0, rect.max.y)),
                fade(t.ash, 0.6),
            );
        }
        let color = if !self.enabled {
            t.ash
        } else if resp.focused {
            t.bone
        } else {
            t.bone_dim
        };
        let style = TextStyle::new(color).scale(scale).shadow(if resp.focused {
            t.blood_dark
        } else {
            t.shadow
        });
        let y = text::text_top_in(rect, text, scale, 0.0) + if resp.held { 1.0 } else { 0.0 };
        draw_text(self.list, Vec2::new(text_x, y), text, &style);
        resp.activated && self.enabled
    }

    /// A list row that can be selected (server browser, player list). Returns true when
    /// clicked / activated.
    pub fn selectable(&mut self, label: &str, selected: bool) -> bool {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let rect = self.allocate(t.row_height);
        let resp = self.interact(id, rect, true);
        if selected {
            fill(self.list, rect, fade(t.blood, 0.55));
        }
        self.draw_focus(rect, &resp);
        let color = if selected && self.enabled {
            t.bone
        } else {
            self.text_color(resp.focused)
        };
        self.draw_label_colored(rect, text, resp.focused, color);
        resp.activated
    }

    // ------------------------------------------------------------------ value widgets

    /// Checkbox-style toggle. Click, Enter or Left/Right flips it. Returns true if changed.
    pub fn toggle(&mut self, label: &str, value: &mut bool) -> bool {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let (_, control, resp) = self.control_row(text, id);
        let flips = usize::from(resp.activated)
            + if resp.focused {
                self.key_count(UiKey::Left) + self.key_count(UiKey::Right)
            } else {
                0
            };
        let changed = flips % 2 == 1;
        if changed {
            *value = !*value;
            self.mark_changed(id);
        }
        let flash = (1.0 - self.since_change(id) as f32 / 0.2).clamp(0.0, 1.0);
        let size = 9.0;
        let box_rect = Rect::from_min_size(
            Vec2::new(control.min.x, (control.center().y - size * 0.5).round()),
            Vec2::splat(size),
        );
        fill(self.list, box_rect, t.ink);
        let edge = if !self.enabled {
            t.ash
        } else if resp.focused {
            t.bone
        } else {
            t.bone_dim
        };
        border(self.list, box_rect, 1.0, mix(edge, t.bone, flash));
        if *value {
            let inner = if self.enabled {
                mix(t.blood_bright, t.bone, flash)
            } else {
                t.ash
            };
            fill(self.list, box_rect.shrink(2.0), inner);
        }
        let (word, color) = if *value {
            ("ON", if self.enabled { t.bone } else { t.ash })
        } else {
            ("OFF", t.ash)
        };
        let style = TextStyle::new(color);
        let text_rect = Rect::new(Vec2::new(box_rect.max.x + 5.0, control.min.y), control.max);
        self.text_in(text_rect, word, &style);
        changed
    }

    /// Horizontal slider. Drag with the mouse or step with Left/Right. `format` is a template
    /// where `{}` is the value (decimals from `step`), `{:.N}` the value with N decimals and
    /// `{%}` the value times 100 (see [`format_value`]). Returns true if the value changed.
    #[allow(clippy::too_many_arguments)]
    pub fn slider(
        &mut self,
        label: &str,
        value: &mut f32,
        min: f32,
        max: f32,
        step: f32,
        format: &str,
    ) -> bool {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let (_, control, resp) = self.control_row(text, id);
        let (lo, hi) = (min.min(max), min.max(max));
        let before = *value;
        // Room for the widest value text ("600 DEG/S"), so it never runs into the track.
        let widest = line_width(&format_value(format, lo, step), 1.0)
            .max(line_width(&format_value(format, hi, step), 1.0));
        let value_w = (widest + 6.0).max(44.0);
        let (track_area, value_rect) = control.split_right(value_w);
        let track = Rect::new(
            Vec2::new(
                track_area.min.x + 2.0,
                (track_area.center().y - 1.0).round(),
            ),
            Vec2::new(
                track_area.max.x - 6.0,
                (track_area.center().y + 2.0).round(),
            ),
        );
        if resp.held && track.width() > 0.0 {
            let f = ((self.input.mouse_pos.x - track.min.x) / track.width()).clamp(0.0, 1.0);
            *value = snap(lo + f * (hi - lo), lo, hi, step);
        }
        if resp.focused {
            let key_step = if step > 0.0 { step } else { (hi - lo) / 20.0 };
            let steps = self.key_count(UiKey::Right) as f32 - self.key_count(UiKey::Left) as f32;
            if steps != 0.0 {
                *value = snap(*value + steps * key_step, lo, hi, step);
            }
        }
        if !value.is_finite() {
            *value = lo;
        }
        let changed = *value != before;
        if changed {
            self.mark_changed(id);
        }
        // Track with quarter ticks, filled part and a handle.
        let frac = if hi > lo {
            ((*value - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        fill(self.list, track.expand(1.0), t.ink);
        fill(self.list, track, t.widget_hover);
        for q in 0..=4 {
            let x = (track.min.x + track.width() * q as f32 / 4.0)
                .round()
                .min(track.max.x - 1.0);
            fill(
                self.list,
                Rect::new(
                    Vec2::new(x, track.max.y + 2.0),
                    Vec2::new(x + 1.0, track.max.y + 4.0),
                ),
                t.ash,
            );
        }
        let fill_color = if !self.enabled {
            t.ash
        } else if resp.focused {
            t.blood_bright
        } else {
            t.blood
        };
        let hx = (track.min.x + track.width() * frac).round();
        fill(
            self.list,
            Rect::new(track.min, Vec2::new(hx, track.max.y)),
            fill_color,
        );
        let handle = Rect::new(
            Vec2::new(hx - 1.0, track.min.y - 3.0),
            Vec2::new(hx + 2.0, track.max.y + 3.0),
        );
        fill(self.list, handle.expand(1.0), t.ink);
        let handle_color = if self.enabled {
            if resp.focused || resp.held {
                t.bone
            } else {
                t.bone_dim
            }
        } else {
            t.ash
        };
        fill(self.list, handle, handle_color);
        let style = TextStyle::new(self.text_color(resp.focused)).right();
        let s = format_value(format, *value, step);
        self.text_in(value_rect, &s, &style);
        changed
    }

    /// Pick one of `options`: Left/Right (or clicking the arrows) cycles, Enter / click
    /// advances. Returns true if the selection changed.
    pub fn choice(&mut self, label: &str, selected: &mut usize, options: &[&str]) -> bool {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let (_, control, resp) = self.control_row(text, id);
        let n = options.len();
        let before = *selected;
        if n > 0 {
            *selected = (*selected).min(n - 1);
            let mut delta: isize = 0;
            if resp.clicked {
                let left_half =
                    self.input.mouse_pos.x < control.center().x - control.width() * 0.25;
                delta += if left_half { -1 } else { 1 };
            } else if resp.activated {
                delta += 1;
            }
            if resp.focused {
                delta +=
                    self.key_count(UiKey::Right) as isize - self.key_count(UiKey::Left) as isize;
            }
            *selected = (*selected as isize + delta).rem_euclid(n as isize) as usize;
        }
        let changed = *selected != before;
        if changed {
            self.mark_changed(id);
        }
        let flash = (1.0 - self.since_change(id) as f32 / 0.15).clamp(0.0, 1.0);
        fill(
            self.list,
            control,
            if resp.focused {
                fade(t.ink, 0.9)
            } else {
                fade(t.ink, 0.5)
            },
        );
        let arrow_color = if !self.enabled {
            t.ash
        } else if resp.focused {
            mix(t.blood_bright, t.bone, flash)
        } else {
            t.blood
        };
        let y = (control.center().y - CAP_HEIGHT * 0.5).round();
        let push = flash.round();
        draw_glyph(
            self.list,
            Vec2::new(control.min.x + 2.0 - push, y),
            '<',
            1.0,
            arrow_color,
        );
        draw_glyph(
            self.list,
            Vec2::new(control.max.x - 7.0 + push, y),
            '>',
            1.0,
            arrow_color,
        );
        let shown = options.get(*selected).copied().unwrap_or("-");
        let shown = text::truncate(shown, control.width() - 18.0, 1.0);
        let style = TextStyle::new(self.text_color(resp.focused)).centered();
        self.text_in(control, &shown, &style);
        changed
    }

    /// Single-line text input. Focus it (click / navigate) and type; Backspace, Delete,
    /// Left, Right, Home and End edit; Enter submits. A label after `##` is hidden, and an
    /// empty visible label makes the field span the whole row.
    pub fn text_field(
        &mut self,
        label: &str,
        value: &mut String,
        max_len: usize,
        filter: TextFilter,
    ) -> TextFieldResponse {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let rect = self.allocate(t.row_height);
        let resp = self.interact(id, rect, true);
        self.draw_focus(rect, &resp);
        let field = if text.is_empty() {
            rect
        } else {
            let (label_rect, control) = rect.split_left((rect.width() * t.label_fraction).round());
            self.draw_label(label_rect, text, resp.focused);
            control.shrink2(Vec2::new(2.0, 1.0))
        };
        let text_x = field.min.x + 4.0;
        let visible = max_chars(field.width() - 8.0, 1.0);
        let mut out = TextFieldResponse {
            focused: resp.focused,
            ..Default::default()
        };
        let mut chars: Vec<char> = value.chars().collect();
        let mut scroll = 0;
        let mut cursor = chars.len();
        if resp.focused {
            self.text_focused = true;
            if self.state.cursor_owner != Some(id) {
                self.state.cursor_owner = Some(id);
                self.state.cursor = chars.len();
                self.state.text_scroll = 0;
            }
            cursor = self.state.cursor.min(chars.len());
            scroll = self.state.text_scroll.min(chars.len());
            if resp.pressed {
                let col = ((self.input.mouse_pos.x - text_x) / GLYPH_ADVANCE)
                    .round()
                    .max(0.0);
                cursor = (scroll + col as usize).min(chars.len());
            }
            let live = self.state.capture.is_none();
            if live {
                for c in self.input.text.chars() {
                    if filter.accepts(c) && chars.len() < max_len {
                        chars.insert(cursor, c);
                        cursor += 1;
                        out.changed = true;
                    }
                }
                for &key in &self.input.keys {
                    match key {
                        UiKey::Backspace if cursor > 0 => {
                            chars.remove(cursor - 1);
                            cursor -= 1;
                            out.changed = true;
                        }
                        UiKey::Delete if cursor < chars.len() => {
                            chars.remove(cursor);
                            out.changed = true;
                        }
                        UiKey::Left => cursor = cursor.saturating_sub(1),
                        UiKey::Right => cursor = (cursor + 1).min(chars.len()),
                        UiKey::Home => cursor = 0,
                        UiKey::End => cursor = chars.len(),
                        UiKey::Enter => out.submitted = true,
                        _ => {}
                    }
                }
            }
            if out.changed {
                *value = chars.iter().collect();
                self.state.last_edit = self.input.time;
                self.mark_changed(id);
            }
            // Keep the cursor inside the visible window (one cell of room at the end).
            let room = visible.saturating_sub(1).max(1);
            if cursor < scroll {
                scroll = cursor;
            } else if cursor > scroll + room {
                scroll = cursor - room;
            }
            self.state.cursor = cursor;
            self.state.text_scroll = scroll;
        }
        // Field body.
        fill(self.list, field, fade(t.ink, 0.92));
        let edge = if resp.focused { t.blood_bright } else { t.ash };
        border(
            self.list,
            field,
            1.0,
            fade(edge, if self.enabled { 1.0 } else { 0.5 }),
        );
        if resp.focused {
            draw::hline(
                self.list,
                field.min.x,
                field.max.x,
                field.max.y - 2.0,
                t.blood_bright,
            );
        }
        let shown: String = chars.iter().skip(scroll).take(visible).collect();
        let color = if self.enabled { t.bone } else { t.ash };
        let y = text::text_top_in(field, &shown, 1.0, 0.0);
        draw_text(
            self.list,
            Vec2::new(text_x, y),
            &shown,
            &TextStyle::new(color),
        );
        if scroll > 0 {
            fill(
                self.list,
                Rect::new(
                    field.min + Vec2::splat(1.0),
                    Vec2::new(field.min.x + 2.0, field.max.y - 1.0),
                ),
                t.blood_bright,
            );
        }
        if resp.focused {
            let typing = self.input.time - self.state.last_edit < 0.5;
            if typing || t.blink(self.input.time) {
                let cx = text_x + (cursor - scroll) as f32 * GLYPH_ADVANCE - 1.0;
                fill(
                    self.list,
                    Rect::new(
                        Vec2::new(cx, y - 1.0),
                        Vec2::new(cx + 1.0, y + CAP_HEIGHT + 1.0),
                    ),
                    t.blood_bright,
                );
            }
        }
        out
    }

    /// Key-binding row showing `binding_name`. Click / Enter starts capture ("PRESS A KEY");
    /// the game then delivers the key and calls [`Ui::finish_capture`]. Escape cancels.
    pub fn keybind(&mut self, label: &str, binding_name: &str) -> KeybindResponse {
        let (text, id) = self.label_id(label);
        let t = self.theme;
        let (_, control, resp) = self.control_row(text, id);
        let mut out = KeybindResponse {
            cancelled: self.state.capture_cancelled == Some(id),
            ..Default::default()
        };
        if self.state.capture == Some(id) {
            self.capture_seen = true;
            out.capturing = !self.state.capture_fresh;
        } else if self.state.capture.is_none() && resp.activated && self.enabled {
            self.state.capture = Some(id);
            self.state.capture_fresh = true;
            self.capture_seen = true;
            out.started = true;
        } else if resp.focused
            && (self.input.key(UiKey::Backspace) || self.input.key(UiKey::Delete))
        {
            out.cleared = true;
        }
        let capturing = self.state.capture == Some(id);
        let time = self.input.time;
        if capturing {
            let on = t.blink(time);
            fill(self.list, control, if on { t.blood } else { t.blood_dark });
            border(self.list, control, 1.0, t.blood_bright);
            let style = TextStyle::new(t.bone).centered();
            self.text_in(control, "PRESS A KEY", &style);
        } else {
            fill(
                self.list,
                control,
                if resp.focused {
                    t.widget_hover
                } else {
                    t.widget
                },
            );
            border(
                self.list,
                control,
                1.0,
                if resp.focused {
                    t.bone_dim
                } else {
                    t.panel_edge
                },
            );
            let (shown, color) = if binding_name.is_empty() {
                ("---", t.ash)
            } else {
                (binding_name, self.text_color(resp.focused))
            };
            let shown = text::truncate(shown, control.width() - 6.0, 1.0);
            self.text_in(control, &shown, &TextStyle::new(color).centered());
        }
        out
    }

    /// Tab strip (settings categories). Click a tab, or Left/Right while focused.
    pub fn tabs(&mut self, id_source: &str, selected: &mut usize, labels: &[&str]) -> bool {
        let id = self.id(id_source);
        let t = self.theme;
        let rect = self.allocate(t.row_height + 3.0);
        let resp = self.interact(id, rect, true);
        let n = labels.len();
        if n == 0 {
            return false;
        }
        let before = *selected;
        *selected = (*selected).min(n - 1);
        let gap = 2.0;
        let w = ((rect.width() - gap * (n - 1) as f32) / n as f32).floor();
        let tab_rect = |i: usize| {
            Rect::from_min_size(
                Vec2::new(rect.min.x + i as f32 * (w + gap), rect.min.y),
                Vec2::new(w, rect.height() - 2.0),
            )
        };
        if resp.clicked
            && let Some(i) = (0..n).find(|&i| tab_rect(i).contains(self.input.mouse_pos))
        {
            *selected = i;
        }
        if resp.focused {
            let delta =
                self.key_count(UiKey::Right) as isize - self.key_count(UiKey::Left) as isize;
            *selected = (*selected as isize + delta).clamp(0, n as isize - 1) as usize;
        }
        for (i, label) in labels.iter().enumerate() {
            let r = tab_rect(i);
            let is_sel = i == *selected;
            let hovered = resp.hovered && r.contains(self.input.mouse_pos);
            if is_sel {
                slanted(
                    self.list,
                    r,
                    true,
                    if resp.focused {
                        t.blood_bright
                    } else {
                        t.blood
                    },
                );
            } else {
                fill(
                    self.list,
                    r,
                    if hovered { t.widget_hover } else { t.widget },
                );
            }
            let color = if !self.enabled {
                t.ash
            } else if is_sel {
                t.bone
            } else {
                t.bone_dim
            };
            let shown = text::truncate(label, w - 10.0, 1.0);
            let mut style = TextStyle::new(color).centered();
            if is_sel {
                style = style.shadow(t.blood_dark);
            }
            self.text_in(r.translate(Vec2::new(-2.0, 0.0)), &shown, &style);
        }
        fill(
            self.list,
            Rect::new(Vec2::new(rect.min.x, rect.max.y - 2.0), rect.max),
            if resp.focused {
                t.blood_bright
            } else {
                t.blood
            },
        );
        if resp.focused {
            let y = (rect.min.y + (rect.height() - 2.0 - CAP_HEIGHT) * 0.5).round();
            let color = if t.blink(self.input.time) {
                t.blood_bright
            } else {
                t.bone
            };
            draw_glyph(self.list, Vec2::new(rect.min.x - 9.0, y), '>', 1.0, color);
        }
        let changed = *selected != before;
        if changed {
            self.mark_changed(id);
        }
        changed
    }

    // ------------------------------------------------------------------ scroll areas

    /// A fixed-height scrollable region (mouse wheel, PageUp/PageDown when hovered or when it
    /// contains the focus, draggable scrollbar). Content is clipped to the region.
    pub fn scroll_area<R>(
        &mut self,
        id_source: &str,
        height: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.scroll_impl(id_source, height, false, f)
    }

    /// Like [`Ui::scroll_area`] but follows the bottom while scrolled to the end (chat logs,
    /// kill logs): new content stays in view until the player scrolls up.
    pub fn scroll_area_sticky<R>(
        &mut self,
        id_source: &str,
        height: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.scroll_impl(id_source, height, true, f)
    }

    fn scroll_impl<R>(
        &mut self,
        id_source: &str,
        height: f32,
        sticky: bool,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let id = self.id(id_source);
        let t = self.theme;
        let rect = self.allocate(height);
        let mut st = self.state.scroll.get(&id).copied().unwrap_or(ScrollState {
            at_bottom: sticky,
            ..Default::default()
        });
        let bar_w = 4.0;
        let view = Rect::new(
            rect.min + Vec2::new(2.0, 3.0),
            rect.max - Vec2::new(bar_w + 4.0, 3.0),
        );
        st.view_height = view.height();
        fill(self.list, rect, fade(t.ink, 0.55));
        border(self.list, rect, 1.0, t.panel_edge);

        // Input before layout so the content is drawn at the new offset this frame.
        let mouse = self.input.mouse_pos;
        let live = self.enabled && self.state.capture.is_none();
        let hovered = live && rect.contains(mouse) && self.clip.is_none_or(|c| c.contains(mouse));
        let had_focus = st_has_focus(self.state, id);
        let mut follow = sticky && st.at_bottom;
        let mut offset = if follow { st.max_offset() } else { st.offset };
        let line = t.row_height + t.spacing;
        let mut user_delta = 0.0;
        if hovered {
            user_delta -= self.input.scroll * line * 2.0;
        }
        if live && (hovered || had_focus) {
            user_delta += (self.input.key_count(UiKey::PageDown) as f32
                - self.input.key_count(UiKey::PageUp) as f32)
                * (view.height() - line).max(line);
        }
        if user_delta != 0.0 {
            offset += user_delta;
            follow = false;
        }
        offset = offset.clamp(0.0, st.max_offset());

        let start = self.list.quads.len();
        let prev_clip = self.clip;
        let clip = prev_clip.map_or(view, |c| c.intersect(view));
        self.clip = Some(clip);
        self.scrolls.push((id, view));
        let prev_focus_scroll = self.focus_scroll.take();
        // Sticky areas (chat) keep short content anchored to the bottom of the view.
        let bottom_pad = |content_h: f32| {
            if sticky {
                (view.height() - content_h).max(0.0)
            } else {
                0.0
            }
        };
        let prev_pad = bottom_pad(st.content_height);
        let content_top = view.min.y - offset + prev_pad;
        self.layouts.push(Layout::vertical(Rect::new(
            Vec2::new(view.min.x, content_top),
            Vec2::new(view.max.x, content_top),
        )));
        let r = f(self);
        let layout = self.layouts.pop();
        self.scrolls.pop();
        self.clip = prev_clip;
        let contains_focus = self.focus_scroll == Some(id);
        if !contains_focus {
            self.focus_scroll = prev_focus_scroll;
        }

        // Re-clamp with this frame's content and apply keyboard-navigation requests.
        st.content_height = layout.map_or(0.0, |l| l.max_y - content_top);
        let mut new_offset = offset;
        if let Some((rid, delta)) = self.scroll_request
            && rid == id
        {
            new_offset += delta;
            follow = false;
            self.scroll_request = None;
        }
        if follow {
            new_offset = st.max_offset();
        }
        new_offset = new_offset.clamp(0.0, st.max_offset());

        // Shift what was drawn if the offset / anchoring changed after layout, then clip.
        let shift = (offset - new_offset + bottom_pad(st.content_height) - prev_pad).round();
        if shift != 0.0 {
            for q in &mut self.list.quads[start..] {
                q.min.y += shift;
                q.max.y += shift;
            }
        }
        clip_quads(self.list, start, clip);

        // Scrollbar (outside the clip; a drag applies to the next frame).
        let track = Rect::new(
            Vec2::new(rect.max.x - bar_w - 2.0, view.min.y),
            Vec2::new(rect.max.x - 2.0, view.max.y),
        );
        if st.max_offset() > 0.0 {
            let thumb_h =
                (track.height() * view.height() / st.content_height).clamp(6.0, track.height());
            let bar = self.interact(id.with("##scrollbar"), track, false);
            if bar.held {
                let f = ((mouse.y - track.min.y - thumb_h * 0.5)
                    / (track.height() - thumb_h).max(1.0))
                .clamp(0.0, 1.0);
                new_offset = f * st.max_offset();
                follow = false;
            }
            let frac = new_offset / st.max_offset();
            let thumb = Rect::from_min_size(
                Vec2::new(
                    track.min.x,
                    (track.min.y + frac * (track.height() - thumb_h)).round(),
                ),
                Vec2::new(bar_w, thumb_h.round()),
            );
            fill(self.list, track, fade(t.widget, 0.9));
            let thumb_color = if bar.held {
                t.blood_bright
            } else if bar.hovered {
                t.bone
            } else {
                t.bone_dim
            };
            fill(self.list, thumb, thumb_color);
        }
        if sticky && st.max_offset() > 0.0 && !follow && new_offset < st.max_offset() - 0.5 {
            // Hint that newer lines exist below.
            let marker = Rect::new(
                Vec2::new(view.min.x, view.max.y - 1.0),
                Vec2::new(view.max.x, view.max.y + 1.0),
            );
            fill(self.list, marker, fade(t.blood_bright, 0.8));
        }
        st.offset = new_offset;
        st.at_bottom = new_offset >= st.max_offset() - 0.5;
        let entry = self.state.scroll.entry(id).or_default();
        *entry = st;
        if contains_focus {
            self.state.scroll_focus.insert(id);
        } else {
            self.state.scroll_focus.remove(&id);
        }
        r
    }
}

fn st_has_focus(state: &UiState, id: Id) -> bool {
    state.scroll_focus.contains(&id)
}

impl Drop for Ui<'_> {
    fn drop(&mut self) {
        let st = &mut *self.state;
        if !self.input.mouse_down {
            st.active = None;
        }
        if let Some(id) = self.hover_focus
            && st.focused != Some(id)
        {
            st.focused = Some(id);
            st.focus_requested = true;
        }
        if let Some(f) = st.focused
            && !st.focus_requested
            && !self.focus_order.contains(&f)
        {
            st.focused = None;
        }
        if st.capture.is_some() && !self.capture_seen {
            st.capture = None;
        }
        st.focus_requested = false;
        if st.cursor_owner != st.focused {
            st.cursor_owner = None;
        }
        st.text_focused = self.text_focused;
        st.focus_order = std::mem::take(&mut self.focus_order);
        st.last_mouse = Some(self.input.mouse_pos);
        let now = self.input.time;
        st.changed_at.retain(|_, t| now - *t < 2.0);
    }
}

/// Text shown for a label: everything before `##`.
pub fn display_text(label: &str) -> &str {
    label.split_once("##").map_or(label, |(shown, _)| shown)
}

/// Snap `v` to `step` increments from `min` and clamp to `[min, max]`.
pub fn snap(v: f32, min: f32, max: f32, step: f32) -> f32 {
    let v = v.clamp(min, max);
    if step > 0.0 {
        (min + ((v - min) / step).round() * step).clamp(min, max)
    } else {
        v
    }
}

/// Decimals needed to show multiples of `step` exactly (up to 4).
fn decimals_for(step: f32) -> usize {
    if step <= 0.0 {
        return 2;
    }
    (0..4)
        .find(|&d| {
            let s = step * 10f32.powi(d as i32);
            (s - s.round()).abs() < 1e-3
        })
        .unwrap_or(4)
}

/// Format a slider value with a tiny template language: `{}` = value with decimals derived
/// from `step`, `{:.N}` = N decimals, `{%}` = value x 100 with no decimals. Other text is kept.
/// Example: `format_value("{%}%", 0.5, 0.05)` == `"50%"`.
pub fn format_value(template: &str, value: f32, step: f32) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            out.push_str(after);
            return out;
        };
        let spec = &after[1..close];
        match spec {
            "" => out.push_str(&format!("{:.*}", decimals_for(step), value)),
            "%" => out.push_str(&format!("{:.0}", value * 100.0)),
            s => match s.strip_prefix(":.").and_then(|n| n.parse::<usize>().ok()) {
                Some(d) => out.push_str(&format!("{value:.d$}")),
                None => out.push_str(&after[..=close]),
            },
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Panel background, frame and title bar.
fn draw_panel_frame(list: &mut UiDrawList, rect: Rect, title: &str, t: &Theme) {
    fill(list, rect, t.panel);
    border(list, rect, 1.0, t.panel_edge);
    if title.is_empty() {
        fill(
            list,
            Rect::new(rect.min, Vec2::new(rect.max.x, rect.min.y + 2.0)),
            t.blood,
        );
    } else {
        let bar = Rect::new(rect.min, Vec2::new(rect.max.x, rect.min.y + t.title_height));
        fill(list, bar, t.blood_dark);
        let tw = line_width(title, 1.0);
        let slab = Rect::new(
            bar.min,
            Vec2::new(bar.min.x + tw + 22.0 + bar.height(), bar.max.y),
        );
        slanted(list, slab, true, t.blood);
        // Three fading slashes after the slab, three at the far right.
        let h = bar.height() - 6.0;
        for i in 0..3 {
            let x = slab.max.x - 2.0 + i as f32 * 7.0;
            draw::parallelogram(
                list,
                x,
                bar.min.y + 3.0,
                3.0,
                h,
                fade(t.blood, 0.7 - i as f32 * 0.2),
            );
            let xr = bar.max.x - 30.0 + i as f32 * 7.0;
            draw::parallelogram(
                list,
                xr,
                bar.min.y + 3.0,
                3.0,
                h,
                fade(t.blood, 0.3 + i as f32 * 0.2),
            );
        }
        draw::hline(list, rect.min.x, rect.max.x, bar.max.y, t.blood);
        let y = (bar.min.y + (bar.height() - CAP_HEIGHT) * 0.5).round();
        let style = TextStyle::new(t.bone).shadow(t.blood_dark);
        draw_text(list, Vec2::new(bar.min.x + 8.0, y), title, &style);
    }
    draw::corner_brackets(list, rect.expand(2.0), 7.0, 1.0, fade(t.bone_dim, 0.8));
}
