//! VELOCIDE game client: rendering, input, prediction, interpolation, UI and audio.
//!
//! - [`net`]: networking core (connection, prediction/reconciliation, interpolation, clock
//!   sync, net stats, sessions).
//! - [`app`]: the `engine::app::Game` implementation and the screen state machine.
//! - [`menus`]: main menu, settings screen; [`ingame`]: everything while playing.
//! - [`palette`]: colour-blind-safe colours by role (follows the colour-vision setting).
//! - [`settings`], [`controls`], [`ui_input`], [`cli`], [`bot`], [`frame`]: shared plumbing.
//! - [`level_mesh`]: level geometry for the renderer.
//!
//! Architecture and extension points: `docs/client.md`.

pub mod app;
pub mod bot;
pub mod cli;
pub mod controls;
pub mod frame;
pub mod ingame;
pub mod level_mesh;
pub mod menus;
pub mod models;
pub mod net;
pub mod palette;
pub mod settings;
pub mod ui_input;
