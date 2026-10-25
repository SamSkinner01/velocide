//! Menus: the main menu (with single-player level select, host, join and settings pages),
//! the shared settings screen and the drawing helpers that give them their look.
//!
//! The pause menu lives with the in-game code (`ingame::pause`) but reuses
//! [`settings_menu::SettingsMenu`].

pub mod backdrop;
pub mod main_menu;
pub mod settings_menu;
pub mod style;
