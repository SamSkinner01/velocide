//! In-game text chat: a log that fades when closed, and a text field opened with the Chat
//! binding (T / Enter). Enter sends (`NetClient::send_chat`), Escape cancels. While it is
//! open, gameplay input is suppressed and the cursor is released.

use engine::gfx::UiDrawList;
use engine::ui::hud::{self, ChatLine};
use engine::ui::{Rect, TextFilter, Theme, Ui};
use glam::{Vec4, vec2};
use shared::protocol::MAX_CHAT_CHARS;

/// Lines kept in the log.
const MAX_LINES: usize = 64;
/// Seconds a line stays visible while chat is closed.
const FADE_AFTER: f64 = 8.0;

pub enum ChatResult {
    None,
    Send(String),
    Cancel,
}

#[derive(Default)]
pub struct Chat {
    pub lines: Vec<ChatLine>,
    open: bool,
    /// Opened this frame: skip the field until the next frame so the key that opened chat
    /// isn't typed into it.
    fresh: bool,
    pub draft: String,
}

impl Chat {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.fresh = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.draft.clear();
    }

    /// A player message.
    pub fn push(&mut self, name: &str, name_color: Vec4, text: &str, time: f64) {
        self.lines.push(ChatLine {
            name: name.to_string(),
            name_color,
            text: text.to_string(),
            time,
        });
        if self.lines.len() > MAX_LINES {
            self.lines.remove(0);
        }
    }

    /// A system line (joins, leaves, server messages), drawn entirely in `color`.
    pub fn system(&mut self, text: &str, color: Vec4, time: f64) {
        self.push("", color, text, time);
    }

    /// Log area for a canvas.
    pub fn log_rect(canvas: Rect) -> Rect {
        Rect::new(
            vec2(10.0, canvas.max.y - 170.0),
            vec2(250.0, canvas.max.y - 76.0),
        )
    }

    pub fn draw_log(&self, list: &mut UiDrawList, theme: &Theme, canvas: Rect, time: f64) {
        hud::chat_box(
            list,
            theme,
            Self::log_rect(canvas),
            &self.lines,
            time,
            self.open,
            FADE_AFTER,
        );
    }

    /// The input field (call while open, inside the frame's `Ui`).
    pub fn input_ui(&mut self, ui: &mut Ui) -> ChatResult {
        if !self.open {
            return ChatResult::None;
        }
        if std::mem::take(&mut self.fresh) {
            return ChatResult::None;
        }
        let canvas = ui.canvas();
        let log = Self::log_rect(canvas);
        let rect = Rect::new(
            vec2(log.min.x, log.max.y + 2.0),
            vec2(log.max.x, log.max.y + 18.0),
        );
        let mut result = ChatResult::None;
        ui.area(rect, |ui| {
            ui.request_focus("##chat");
            let r = ui.text_field("##chat", &mut self.draft, MAX_CHAT_CHARS, TextFilter::Any);
            if r.submitted {
                let text = self.draft.trim().to_string();
                result = if text.is_empty() {
                    ChatResult::Cancel
                } else {
                    ChatResult::Send(text)
                };
            }
        });
        if ui.back_pressed() {
            result = ChatResult::Cancel;
        }
        if !matches!(result, ChatResult::None) {
            self.close();
        }
        result
    }
}
