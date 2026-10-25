//! A small, purpose-built engine for VELOCIDE.
//!
//! Always available (also on the headless dedicated server):
//! - [`time`]: fixed-timestep accumulator
//! - [`physics`]: static collision world with swept-capsule / ray queries (parry3d)
//! - [`hotreload`]: file watcher used for `tuning.ron` and shader reloads
//!
//! Behind the `client` feature: windowing, input, rendering, audio, assets and UI.

pub mod hotreload;
pub mod physics;
pub mod time;

#[cfg(feature = "client")]
pub mod app;
#[cfg(feature = "client")]
pub mod assets;
#[cfg(feature = "client")]
pub mod audio;
#[cfg(feature = "client")]
pub mod gfx;
#[cfg(feature = "client")]
pub mod input;
#[cfg(feature = "client")]
pub mod render;
#[cfg(feature = "client")]
pub mod ui;

#[cfg(feature = "debug-ui")]
pub mod debug_ui;

pub use glam;

/// Root directory for runtime assets (levels, tuning, shaders, models). Overridable with the
/// `VELOCIDE_ASSETS` environment variable; otherwise `./assets` if it exists, else the first
/// `assets/` directory found next to or above the executable.
pub fn assets_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("VELOCIDE_ASSETS") {
        return dir.into();
    }
    let cwd = std::path::PathBuf::from("assets");
    if cwd.is_dir() {
        return cwd;
    }
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1) {
            let candidate = dir.join("assets");
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    cwd
}
