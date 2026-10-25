//! Immediate-mode retro UI: bitmap font, text helpers, menu widgets and HUD helpers, all
//! drawing into a [`crate::gfx::UiDrawList`] (canvas 360 units tall). Independent of winit:
//! the game converts its input into a [`UiInput`] each frame. See `docs/ui.md`.
//!
//! - [`font`]: the built-in pixel font (atlas + UVs, the renderer's contract)
//! - [`text`]: measure / wrap / draw text, [`UiDrawExt`] convenience trait
//! - [`Ui`]: frame builder with panels, rows, buttons, toggles, sliders, choices, text fields,
//!   key binds, tabs, scroll areas and keyboard / gamepad focus navigation
//! - [`hud`]: health bar, stamina pips, style meter, text feeds, banners, crosshair, chat
//! - [`raster`]: CPU rasteriser for tests and screenshots

pub mod draw;
pub mod font;
pub mod hud;
pub mod input;
pub mod raster;
pub mod state;
pub mod text;
pub mod theme;
pub mod widgets;

#[cfg(test)]
mod tests;

pub use draw::Rect;
pub use input::{UiInput, UiKey};
pub use state::{Id, UiState};
pub use text::{Align, TextStyle, UiDrawExt};
pub use theme::Theme;
pub use widgets::{
    KeybindResponse, Response, TextFieldResponse, TextFilter, Ui, display_text, format_value,
};
