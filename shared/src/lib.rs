//! The game simulation shared by client and server: movement, combat rules, enemies, style
//! scoring and the network protocol. Everything here must be deterministic given the same
//! inputs and starting state, because clients run the same code for prediction.

pub mod input;
pub mod level;
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
