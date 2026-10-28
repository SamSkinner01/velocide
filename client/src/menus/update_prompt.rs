//! The update prompt on the main menu (docs/distribution.md "Update check"): UPDATE NOW /
//! LATER when a newer release is out, the download's progress, and a failure with the
//! release page to fall back on. A modal box like the menu's notices; what it shows comes from
//! `Common::update` ([`Updater::prompt`]).

use engine::audio::Sfx;
use engine::ui::draw::{self, fade};
use engine::ui::{Rect, TextStyle, Ui, text};
use glam::vec2;

use super::settings_menu;
use crate::update::{Prompt, Updater};

/// Panel width (canvas units).
const WIDTH: f32 = 340.0;

/// Which box is up, so each one starts with its first button focused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Available,
    Downloading,
    Failed,
    Installed,
}

/// What happened in the prompt this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PromptResponse {
    pub sound: Option<Sfx>,
    /// The new game is in place: quit (`main` starts it).
    pub restart: bool,
    /// The prompt closed: focus goes back to the menu.
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct UpdatePrompt {
    /// The box shown last frame.
    stage: Option<Stage>,
}

impl UpdatePrompt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws `prompt` over the menu (dimmed) and acts on its buttons.
    pub fn ui(&mut self, ui: &mut Ui, updater: &mut Updater, prompt: &Prompt) -> PromptResponse {
        let mut r = PromptResponse::default();
        let stage = match prompt {
            Prompt::Available { .. } => Stage::Available,
            Prompt::Downloading { .. } => Stage::Downloading,
            Prompt::Failed { .. } => Stage::Failed,
            Prompt::Installed { .. } => Stage::Installed,
        };
        let entered = self.stage != Some(stage);
        self.stage = Some(stage);

        settings_menu::dim(ui, 0.35);
        let canvas = ui.canvas();
        let pos = vec2(
            (canvas.center().x - WIDTH * 0.5).round(),
            canvas.center().y - 70.0,
        );
        let dim = TextStyle::new(ui.theme().bone_dim);
        match prompt {
            Prompt::Available { new, running } => {
                ui.panel_auto(pos, WIDTH, "UPDATE AVAILABLE", |ui| {
                    ui.value_row("NEW VERSION", new);
                    ui.value_row("YOU HAVE", running);
                    ui.spacing(2.0);
                    ui.label_wrapped(
                        "EVERYONE IN A GAME NEEDS THE SAME VERSION: THE HOST AND EVERY PLAYER. \
                         THE GAME RESTARTS WHEN THE DOWNLOAD IS DONE.",
                    );
                    ui.spacing(4.0);
                    if entered {
                        ui.request_focus("UPDATE NOW");
                    }
                    ui.row(2, |ui| {
                        if ui.button("UPDATE NOW") {
                            r.sound = Some(Sfx::UiSelect);
                            updater.update_now();
                        }
                        if ui.button("LATER") {
                            r.sound = Some(Sfx::UiBack);
                            updater.later();
                        }
                    });
                });
                if r.sound.is_none() && ui.back_pressed() {
                    r.sound = Some(Sfx::UiBack);
                    updater.later();
                }
            }
            Prompt::Downloading { new, done, total } => {
                ui.panel_auto(pos, WIDTH, "UPDATING", |ui| {
                    ui.value_row("DOWNLOADING", new);
                    let fraction = if *total > 0 {
                        *done as f32 / *total as f32
                    } else {
                        0.0
                    };
                    progress_bar(ui, fraction);
                    ui.text(
                        &format!(
                            "{} OF {}   {:.0}%",
                            megabytes(*done),
                            megabytes(*total),
                            fraction * 100.0
                        ),
                        &dim,
                    );
                    ui.spacing(4.0);
                    if entered {
                        ui.request_focus("CANCEL");
                    }
                    if ui.button("CANCEL") {
                        r.sound = Some(Sfx::UiBack);
                        updater.later();
                    }
                });
                if r.sound.is_none() && ui.back_pressed() {
                    r.sound = Some(Sfx::UiBack);
                    updater.later();
                }
            }
            Prompt::Failed { new, reason, page } => {
                ui.panel_auto(pos, WIDTH, "UPDATE FAILED", |ui| {
                    ui.label_wrapped(&reason.to_uppercase());
                    ui.spacing(2.0);
                    ui.text_wrapped(
                        &format!("GET {} FROM ITS RELEASE PAGE INSTEAD:", new.to_uppercase()),
                        &dim,
                    );
                    // The address as typed (case kept), broken after slashes.
                    let max = text::max_chars(ui.available_rect().width() - 8.0, 1.0);
                    let url = url_lines(page, max).join("\n");
                    ui.text_wrapped(&url, &TextStyle::new(ui.theme().bone));
                    ui.spacing(4.0);
                    if entered {
                        ui.request_focus("OPEN RELEASE PAGE");
                    }
                    ui.row(2, |ui| {
                        if ui.button("OPEN RELEASE PAGE") {
                            r.sound = Some(Sfx::UiSelect);
                            updater.open_release_page();
                        }
                        if ui.button("LATER") {
                            r.sound = Some(Sfx::UiBack);
                            updater.later();
                        }
                    });
                });
                if r.sound.is_none() && ui.back_pressed() {
                    r.sound = Some(Sfx::UiBack);
                    updater.later();
                }
            }
            Prompt::Installed { new } => {
                ui.panel_auto(pos, WIDTH, "UPDATED", |ui| {
                    ui.label_wrapped(&format!("RESTARTING INTO {}...", new.to_uppercase()));
                });
                r.restart = updater.restart();
            }
        }
        if updater.prompt().is_none() {
            self.stage = None;
            r.closed = true;
        }
        r
    }
}

/// `url` in lines of at most `max` characters, broken after a `/` where possible.
fn url_lines(url: &str, max: usize) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = url;
    while let Some((end, _)) = rest.char_indices().nth(max.max(1)) {
        let cut = rest[..end].rfind('/').map_or(end, |i| i + 1);
        lines.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    lines.push(rest);
    lines
}

/// "12.3 MB".
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// A full-width bar (like the revive bar in game) on a row of its own.
fn progress_bar(ui: &mut Ui, fraction: f32) {
    let t = ui.theme().clone();
    let row = ui.allocate(t.row_height);
    let bar = Rect::new(
        vec2(row.min.x + 6.0, row.min.y + 3.0),
        vec2(row.max.x - 4.0, row.max.y - 3.0),
    );
    let list = ui.painter();
    draw::fill(list, bar.expand(1.0), fade(t.ink, 0.85));
    draw::fill(list, bar, fade(t.crimson, 0.25));
    let w = (bar.width() * fraction.clamp(0.0, 1.0)).round();
    draw::fill(
        list,
        Rect::new(bar.min, vec2(bar.min.x + w, bar.max.y)),
        t.crimson_bright,
    );
    if fraction > 0.0 {
        draw::fill(
            list,
            Rect::new(
                vec2(bar.min.x + w - 1.0, bar.min.y - 1.0),
                vec2(bar.min.x + w + 1.0, bar.max.y + 1.0),
            ),
            t.bone,
        );
    }
    // Scanlines over the fill, like the menu column.
    let mut y = bar.min.y + 1.0;
    while y < bar.max.y {
        draw::hline(list, bar.min.x, bar.min.x + w, y, fade(t.ink, 0.25));
        y += 3.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_break_after_slashes() {
        let url = "https://github.com/SamSkinner01/velocide/releases/tag/build-43";
        assert_eq!(
            url_lines(url, 51),
            [
                "https://github.com/SamSkinner01/velocide/releases/",
                "tag/build-43"
            ]
        );
        assert_eq!(url_lines(url, 80), [url]);
        // No slash to break at: cut anyway.
        assert_eq!(url_lines("abcdefgh", 3), ["abc", "def", "gh"]);
    }
}
