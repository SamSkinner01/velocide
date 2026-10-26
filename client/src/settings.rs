//! Player settings, persisted as RON.
//!
//! Location: `$VELOCIDE_CONFIG` (a file path) if set, else
//! `$XDG_CONFIG_HOME/velocide/settings.ron`, else `~/.config/velocide/settings.ron`
//! (`%APPDATA%\velocide\settings.ron` on Windows), else `./settings.ron`. A missing or invalid
//! file gives the defaults (with a warning) and never stops the game; unknown fields are
//! ignored and missing ones use their defaults, so old files keep working. Accessibility
//! options (colour vision, shape cues) are their own `accessibility` section.

use std::path::{Path, PathBuf};

use engine::color_vision::{ColorVision, ColorVisionMode};
use engine::gfx::RetroSettings;
use serde::{Deserialize, Serialize};

use crate::controls::Bindings;

/// Crosshair styles offered in the menu.
pub const CROSSHAIRS: [&str; 4] = ["CROSS + DOT", "CROSS", "DOT", "NONE"];

/// When the extra non-colour cues (player marker shapes, the parry icon, health popups) show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CueMode {
    /// With any colour-vision mode.
    #[default]
    Auto,
    On,
    Off,
}

impl CueMode {
    pub const ALL: [CueMode; 3] = [CueMode::Auto, CueMode::On, CueMode::Off];
    /// Menu labels, in [`CueMode::ALL`] order.
    pub const LABELS: [&'static str; 3] = ["AUTO", "ON", "OFF"];

    pub fn index(self) -> usize {
        match self {
            CueMode::Auto => 0,
            CueMode::On => 1,
            CueMode::Off => 2,
        }
    }
}

/// The ACCESSIBILITY page (its own section in the settings file).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Accessibility {
    /// Colour-vision mode (palette swap + full-screen correction) and correction strength.
    pub color_vision: ColorVision,
    pub shape_cues: CueMode,
}

impl Accessibility {
    /// Whether the non-colour cues are drawn right now.
    pub fn shape_cues_on(&self) -> bool {
        match self.shape_cues {
            CueMode::Auto => self.color_vision.mode != ColorVisionMode::Off,
            CueMode::On => true,
            CueMode::Off => false,
        }
    }
}

/// Internal render heights offered as "resolution scale" (0 = native).
pub const RESOLUTIONS: [(u32, &str); 5] = [
    (0, "NATIVE"),
    (720, "720P"),
    (480, "480P"),
    (360, "360P"),
    (240, "240P"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Every retro effect toggle, the internal resolution and vsync (also shake / flashes /
    /// hitstop, which the game applies itself). Its `color_vision` is not saved here: the
    /// renderer gets [`Settings::render_settings`], which fills it from `accessibility`.
    pub retro: RetroSettings,
    /// Horizontal field of view in degrees at 16:9 (wider screens see more, "Hor+").
    pub fov: f32,
    /// Vertical FOV of the held weapon pass.
    pub viewmodel_fov: f32,
    /// Multiplier: degrees per mouse count = sensitivity x 0.022.
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    /// Degrees per second at full right-stick deflection.
    pub gamepad_look_sensitivity: f32,
    pub gamepad_invert_y: bool,
    /// Linear 0..1 slider values (squared before reaching the mixer, which feels even).
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub bindings: Bindings,
    pub player_name: String,
    /// Last address typed in the join menu.
    pub last_join_address: String,
    pub host_port: u16,
    pub host_max_players: u32,
    /// Game mode last picked in the host menu.
    pub host_mode: shared::lobby::GameMode,
    /// Last level picked in single-player / host menus.
    pub last_level: String,
    pub show_fps: bool,
    pub show_ping: bool,
    pub show_speed: bool,
    pub show_timer: bool,
    /// Index into [`CROSSHAIRS`].
    pub crosshair: usize,
    /// View bob amount (0 = off, 1 = full).
    pub view_bob: f32,
    pub show_viewmodel: bool,
    pub fullscreen: bool,
    pub accessibility: Accessibility,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            retro: RetroSettings::default(),
            fov: 105.0,
            viewmodel_fov: 62.0,
            mouse_sensitivity: 2.5,
            invert_y: false,
            gamepad_look_sensitivity: 240.0,
            gamepad_invert_y: false,
            master_volume: 0.8,
            sfx_volume: 1.0,
            music_volume: 0.7,
            bindings: Bindings::default(),
            player_name: "SHRIKE".into(),
            last_join_address: format!("127.0.0.1:{}", shared::DEFAULT_PORT),
            host_port: shared::DEFAULT_PORT,
            host_max_players: 4,
            host_mode: shared::lobby::GameMode::Campaign,
            // New players start on the campaign (the sandboxes are in the list too).
            last_level: "crucible_of_veins".into(),
            show_fps: true,
            show_ping: true,
            show_speed: true,
            show_timer: true,
            crosshair: 0,
            view_bob: 1.0,
            show_viewmodel: true,
            fullscreen: false,
            accessibility: Accessibility::default(),
        }
    }
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Settings {
    /// Where settings are stored (see the module docs).
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os("VELOCIDE_CONFIG").filter(|p| !p.is_empty()) {
            return PathBuf::from(p);
        }
        let env_dir = |var: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let base = env_dir("XDG_CONFIG_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".config")))
            .or_else(|| env_dir("APPDATA"));
        match base {
            Some(dir) => dir.join("velocide").join("settings.ron"),
            None => PathBuf::from("settings.ron"),
        }
    }

    /// Loads settings; a missing or broken file gives the defaults (never fails).
    pub fn load(path: &Path) -> Settings {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no settings file at {}, using defaults", path.display());
                return Settings::default();
            }
            Err(e) => {
                log::warn!("can't read {}: {e}; using default settings", path.display());
                return Settings::default();
            }
        };
        match Self::from_ron(&text) {
            Ok(s) => s,
            Err(e) => {
                log::warn!(
                    "invalid settings file {}: {e}; using defaults",
                    path.display()
                );
                Settings::default()
            }
        }
    }

    /// Parses (and sanitises) settings from RON text.
    pub fn from_ron(text: &str) -> Result<Settings, ron::error::SpannedError> {
        let mut s: Settings = ron::from_str(text)?;
        s.sanitize();
        Ok(s)
    }

    pub fn to_ron(&self) -> String {
        let pretty = ron::ser::PrettyConfig::new().depth_limit(3);
        ron::ser::to_string_pretty(self, pretty).unwrap_or_default()
    }

    /// Writes the file (via a temporary file, so a crash can't leave half a file). Falls back
    /// to `./settings.ron` if the config directory can't be created.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("ron.tmp");
        std::fs::write(&tmp, self.to_ron())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Clamps every value into a sane range (hand-edited files, NaNs).
    pub fn sanitize(&mut self) {
        let d = Settings::default();
        self.fov = finite_or(self.fov, d.fov).clamp(60.0, 140.0);
        self.viewmodel_fov = finite_or(self.viewmodel_fov, d.viewmodel_fov).clamp(40.0, 90.0);
        self.mouse_sensitivity =
            finite_or(self.mouse_sensitivity, d.mouse_sensitivity).clamp(0.05, 20.0);
        self.gamepad_look_sensitivity =
            finite_or(self.gamepad_look_sensitivity, d.gamepad_look_sensitivity).clamp(30.0, 900.0);
        for v in [
            &mut self.master_volume,
            &mut self.sfx_volume,
            &mut self.music_volume,
            &mut self.view_bob,
        ] {
            *v = finite_or(*v, 1.0).clamp(0.0, 1.0);
        }
        self.crosshair = self.crosshair.min(CROSSHAIRS.len() - 1);
        self.host_max_players = self.host_max_players.clamp(1, 8);
        self.player_name = shared::protocol::sanitize_name(&self.player_name);
        if self.player_name.trim().is_empty() {
            self.player_name = d.player_name;
        }
        let r = &mut self.retro;
        r.internal_height = r.internal_height.min(2160);
        r.vertex_snap_strength = finite_or(r.vertex_snap_strength, 1.0).clamp(0.25, 8.0);
        r.color_bits = r.color_bits.clamp(2, 8);
        self.accessibility.color_vision = self.accessibility.color_vision.sanitized();
        self.bindings.fill_missing();
    }

    /// What the renderer gets: the retro options plus the colour-vision correction (which is
    /// stored with the accessibility settings). Always pass this, not `retro`, to
    /// `Renderer::apply_settings`.
    pub fn render_settings(&self) -> RetroSettings {
        RetroSettings {
            color_vision: self.accessibility.color_vision,
            ..self.retro.clone()
        }
    }

    /// The mixer volumes (squared slider values: linear sliders feel bunched at the top).
    pub fn mixer_volumes(&self) -> (f32, f32, f32) {
        (
            self.master_volume * self.master_volume,
            self.sfx_volume * self.sfx_volume,
            self.music_volume * self.music_volume,
        )
    }

    /// Vertical FOV (degrees) for the main camera, from the horizontal FOV at 16:9 plus extra
    /// horizontal degrees (speed FOV kick).
    pub fn vertical_fov(&self, extra_horizontal_degrees: f32) -> f32 {
        let h = (self.fov + extra_horizontal_degrees)
            .clamp(30.0, 170.0)
            .to_radians();
        (2.0 * ((h * 0.5).tan() * 9.0 / 16.0).atan()).to_degrees()
    }
}
