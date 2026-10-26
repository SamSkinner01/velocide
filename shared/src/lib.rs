//! The game simulation shared by client and server: movement, combat rules, enemies, style
//! scoring and the network protocol. Everything here must be deterministic given the same
//! inputs and starting state, because clients run the same code for prediction.

/// Character customisation (plain data; the client draws it).
pub mod cosmetics;
pub mod input;
pub mod level;
/// Loadouts and weapon builds (plain data; `sim::weapons` makes them fire).
pub mod loadout;
/// Lobby / match flow model (game mode, match settings, lobby state and messages).
pub mod lobby;
pub mod movement;
pub mod protocol;
pub mod sim;
pub mod tuning;

/// Name of the game (original title).
pub const GAME_NAME: &str = "VELOCIDE";

/// Default UDP port for hosted games.
pub const DEFAULT_PORT: u16 = 47420;

/// Root directory for runtime assets (levels, tuning, shaders). See [`engine::assets_dir`].
pub fn assets_dir() -> std::path::PathBuf {
    engine::assets_dir()
}
