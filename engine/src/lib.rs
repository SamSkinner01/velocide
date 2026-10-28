//! A small, purpose-built engine for VELOCIDE.
//!
//! Always available (also on the headless dedicated server):
//! - [`asset_files`]: the files of `assets/`, from disk or compiled into the binary
//! - [`time`]: fixed-timestep accumulator
//! - [`physics`]: static collision world with swept-capsule / ray queries (parry3d)
//! - [`hotreload`]: file watcher used for `tuning.ron` and shader reloads
//! - [`color_vision`]: colour-vision deficiency simulation / correction and CIEDE2000
//!
//! Behind the `client` feature: windowing, input, rendering, audio, assets and UI.

pub mod asset_files;
pub mod color_vision;
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

/// The assets folder on disk (levels, tuning, shaders, models). Overridable with the
/// `VELOCIDE_ASSETS` environment variable; otherwise `./assets` if it exists, else the first
/// `assets/` directory found next to or above the executable, else `./assets` (which then
/// doesn't exist). Read files through [`asset_files`]: a file missing here, or the whole
/// folder, falls back to the copy compiled into the binary. Hot reload watches this folder;
/// the tuning editor and the asset generators write into it.
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
