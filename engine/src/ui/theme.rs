//! Colours and metrics for menus and HUD.

use glam::Vec4;

/// sRGB colour from 8-bit components.
pub const fn rgb(r: u8, g: u8, b: u8) -> Vec4 {
    rgba(r, g, b, 255)
}

/// sRGB colour with alpha from 8-bit components.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Vec4 {
    Vec4::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

/// Palette and layout metrics. The default is VELOCIDE's crimson / bone / black look.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    // --- palette ---
    /// Primary accent: deep kiln crimson.
    pub crimson: Vec4,
    /// Bright crimson for focus, highlights and warnings.
    pub crimson_bright: Vec4,
    /// Very dark crimson for pressed states, inactive fills, shadows with colour.
    pub crimson_dark: Vec4,
    /// Main text colour: bone white.
    pub bone: Vec4,
    /// Secondary text (labels, values not in focus).
    pub bone_dim: Vec4,
    /// Tertiary text, disabled widgets, frame lines.
    pub ash: Vec4,
    /// Near-black.
    pub ink: Vec4,
    /// Translucent dark panel background.
    pub panel: Vec4,
    /// Panel frame lines.
    pub panel_edge: Vec4,
    /// Widget slab (buttons, fields, tracks).
    pub widget: Vec4,
    /// Widget slab under the mouse.
    pub widget_hover: Vec4,
    /// Row highlight behind the focused widget.
    pub focus_bg: Vec4,
    /// Drop shadows under text.
    pub shadow: Vec4,

    // --- metrics (canvas units) ---
    pub text_scale: f32,
    /// Height of one widget row.
    pub row_height: f32,
    /// Vertical gap between widgets and horizontal gap between row columns.
    pub spacing: f32,
    /// Inner padding of panels (also leaves room for the `>` focus marker).
    pub padding: f32,
    /// Height of a panel's title bar.
    pub title_height: f32,
    /// Fraction of a row given to the label in label + control widgets.
    pub label_fraction: f32,
    /// Seconds per blink of the focus marker / text cursor.
    pub blink_period: f32,

    // --- HUD ---
    pub health: Vec4,
    pub health_low: Vec4,
    pub hard_damage: Vec4,
    pub stamina: Vec4,
    pub stamina_empty: Vec4,
    /// Colours of the eight style ranks, lowest to highest (DULL .. VELOCIDE).
    pub rank_colors: [Vec4; 8],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            crimson: rgb(168, 18, 28),
            crimson_bright: rgb(238, 38, 44),
            crimson_dark: rgb(70, 8, 12),
            bone: rgb(238, 228, 206),
            bone_dim: rgb(170, 156, 136),
            ash: rgb(102, 90, 84),
            ink: rgb(10, 6, 7),
            panel: rgba(12, 7, 8, 222),
            panel_edge: rgb(64, 22, 24),
            widget: rgba(30, 16, 17, 235),
            widget_hover: rgba(52, 22, 24, 240),
            focus_bg: rgba(150, 14, 22, 90),
            shadow: rgba(0, 0, 0, 200),

            text_scale: 1.0,
            row_height: 14.0,
            spacing: 3.0,
            padding: 12.0,
            title_height: 15.0,
            label_fraction: 0.46,
            blink_period: 0.8,

            health: rgb(238, 228, 206),
            health_low: rgb(238, 38, 44),
            hard_damage: rgb(120, 16, 24),
            stamina: rgb(96, 200, 255),
            stamina_empty: rgba(40, 60, 76, 200),
            rank_colors: [
                rgb(128, 120, 112), // DULL: ash grey
                rgb(190, 160, 96),  // CRUDE: tarnished brass
                rgb(232, 132, 40),  // FERAL: orange
                rgb(255, 84, 32),   // VICIOUS: hot orange-red
                rgb(238, 26, 44),   // MERCILESS: hot red
                rgb(210, 32, 120),  // GRISLY: magenta
                rgb(255, 60, 60),   // SEARING: red-hot (pulses in the HUD)
                rgb(255, 244, 214), // VELOCIDE: white-hot
            ],
        }
    }
}

impl Theme {
    /// Colour of style rank `index` (clamped to the highest rank).
    pub fn rank_color(&self, index: usize) -> Vec4 {
        self.rank_colors[index.min(self.rank_colors.len() - 1)]
    }

    /// Whether a blinking element is in its visible phase at `time` seconds.
    pub fn blink(&self, time: f64) -> bool {
        (time / self.blink_period as f64).fract() < 0.6
    }
}
