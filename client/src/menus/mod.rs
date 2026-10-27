//! Menus: the main menu (with single-player level select, host, join and settings pages),
//! the lobby screen for hosted / joined games, the shared settings screen and the drawing
//! helpers that give them their look.
//!
//! The pause menu lives with the in-game code (`ingame::pause`) but reuses
//! [`settings_menu::SettingsMenu`].

pub mod backdrop;
/// The CUSTOMISE screen: the Shrike's cosmetics with a turning preview.
pub mod customize;
/// The LOADOUT screen (the arsenal: weapons per slot, abilities, presets).
pub mod loadout;
/// What the LOADOUT screen says about each weapon and ability, and their numbers.
pub mod loadout_text;
pub mod lobby;
pub mod main_menu;
/// End-of-match results screens (campaign tally / co-op table / deathmatch standings).
pub mod results;
pub mod settings_menu;
pub mod style;
