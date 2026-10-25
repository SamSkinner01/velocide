//! VELOCIDE game client: rendering, input, prediction, interpolation, UI and audio.
//!
//! [`net`] is the networking core (connection, prediction/reconciliation, interpolation,
//! clock sync, net stats, sessions); the game client is built on top of it.

pub mod net;
pub mod level_mesh;
pub mod models;
