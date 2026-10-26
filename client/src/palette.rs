//! The semantic palette: every colour that *means* something in VELOCIDE (whose player, what an
//! enemy is about to do, how hurt you are...) by role, with one variant per colour-vision mode.
//!
//! ```ignore
//! use crate::palette::palette;
//! let p = palette();                       // follows the player's colour-vision setting
//! tint.glow = p.parry_window * 3.0;        // world colours are sRGB 0..1; scale for glow
//! let trim = p.team(player_index);
//! feedback.flash(p.flash_damage, 0.4);
//! ```
//!
//! `palette()` reads a process-wide mode that the app sets from the settings
//! ([`set_accessibility`]), so code deep in the renderer glue can use it without plumbing.
//! The UI theme's HUD colours (health, hard damage, stamina, style ranks, warnings) come from
//! [`Palette::theme`], so anything drawn with `common.theme` follows the mode too.
//!
//! The standard palette ([`ColorVisionMode::Off`]) is the original look. The others were chosen
//! by simulating each deficiency (Machado et al. 2009, `engine::color_vision`) and checking the
//! CIEDE2000 distance of every critical pair *as that viewer sees it* (the tests below), and of
//! world signals against the foundry's own colours (brick, rust, molten slag): those are warm,
//! so warm signals vanish into them for a dichromat. Enemy wind-ups turn violet-blue / azure,
//! parry windows white, dark reds that protanopes can't see get lifted. World colours are also
//! checked after the full-screen correction, which runs on the 3D scene but not on the UI.
//!
//! Non-colour cues ([`shape_cues`]): name-tag / scoreboard marker shapes per player, the parry
//! icon over enemies, `+N` / `-N` health popups. Hard damage is always hatched.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use engine::color_vision::ColorVisionMode;
use engine::ui::Theme;
use engine::ui::theme::rgb;
use glam::{Vec3, Vec4};

/// Colours by role. All sRGB-encoded `0..1` (gamma space, like the rest of the game); alpha 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub mode: ColorVisionMode,

    // --- players -------------------------------------------------------------------------------
    /// Per-player colours (lobby colour index / player id modulo 8): Shrike trim and wick, name
    /// tags, chat names, scoreboard, results. Pair them with [`marker_shape`] where you can.
    pub team: [Vec4; 8],
    /// Team-mode allies / opponents (markers, outlines).
    pub friendly: Vec4,
    pub hostile: Vec4,

    // --- enemies (world) -----------------------------------------------------------------------
    /// Attack wind-up glow that you can't parry (dodge it).
    pub telegraph: Vec4,
    /// The parryable window of an attack (strobe it; [`shape_cues`] adds an icon).
    pub parry_window: Vec4,
    /// Glowing weak points (heads, lanterns, heart sacs).
    pub weak_point: Vec4,
    /// A sniper's aim laser (the Heron).
    pub aim_laser: Vec4,
    /// Enemy projectiles in flight (ember bombs, needles).
    pub hostile_projectile: Vec4,
    /// A projectile you parried / deflected: now yours.
    pub deflected: Vec4,
    /// The Quill's brand sigil on a marked target.
    pub brand: Vec4,

    // --- the player ----------------------------------------------------------------------------
    /// Embers drawn into your wick (kindling), heal popups.
    pub heal: Vec4,
    /// Damage numbers / popups taken.
    pub damage: Vec4,

    // --- screen flashes (rgb, mixed over the corrected scene) ----------------------------------
    pub flash_damage: Vec3,
    pub flash_parry: Vec3,
    /// A hit blocked by spawn protection.
    pub flash_shield: Vec3,
    pub flash_death: Vec3,
    /// Kindling heal (the standard one is a faint red, like the damage flash).
    pub flash_heal: Vec3,

    // --- HUD (copied into the UI theme) --------------------------------------------------------
    pub health: Vec4,
    pub health_low: Vec4,
    /// Smothered capacity (hard damage); the bar also hatches it.
    pub hard_damage: Vec4,
    pub stamina: Vec4,
    pub stamina_empty: Vec4,
    /// Warnings and bad news: low health, DOWNED, LOCKED (the theme's `blood_bright`). Good news
    /// (READY, CLEARED, CHECKPOINT) uses `stamina`.
    pub warning: Vec4,
    /// Style ranks, DULL .. VELOCIDE.
    pub ranks: [Vec4; 8],
}

const fn c(r: f32, g: f32, b: f32) -> Vec4 {
    Vec4::new(r, g, b, 1.0)
}

/// The original look.
pub const STANDARD: Palette = Palette {
    mode: ColorVisionMode::Off,
    team: [
        c(0.93, 0.15, 0.17),
        c(0.38, 0.78, 1.0),
        c(1.0, 0.72, 0.2),
        c(0.55, 1.0, 0.45),
        c(0.85, 0.4, 1.0),
        c(1.0, 0.5, 0.75),
        c(0.35, 1.0, 0.9),
        c(1.0, 1.0, 0.55),
    ],
    friendly: c(0.38, 0.78, 1.0),
    hostile: c(0.93, 0.15, 0.17),
    telegraph: c(1.0, 0.15, 0.08),
    parry_window: c(1.0, 0.9, 0.2),
    weak_point: c(1.0, 0.6, 0.25),
    aim_laser: c(1.0, 0.12, 0.1),
    hostile_projectile: c(1.0, 0.55, 0.15),
    deflected: c(1.0, 0.97, 0.9),
    brand: c(1.0, 0.3, 0.85),
    heal: c(1.0, 0.62, 0.22),
    damage: rgb(238, 38, 44),
    flash_damage: Vec3::new(0.9, 0.05, 0.04),
    flash_parry: Vec3::new(1.0, 0.97, 0.9),
    flash_shield: Vec3::new(0.7, 0.9, 1.0),
    flash_death: Vec3::new(0.6, 0.0, 0.0),
    flash_heal: Vec3::new(0.95, 0.15, 0.2),
    health: rgb(238, 228, 206),
    health_low: rgb(238, 38, 44),
    hard_damage: rgb(120, 16, 24),
    stamina: rgb(96, 200, 255),
    stamina_empty: Vec4::new(40.0 / 255.0, 60.0 / 255.0, 76.0 / 255.0, 200.0 / 255.0),
    warning: rgb(238, 38, 44),
    ranks: [
        rgb(128, 120, 112), // DULL: ash grey
        rgb(190, 160, 96),  // CRUDE: tarnished brass
        rgb(232, 132, 40),  // FERAL: orange
        rgb(255, 84, 32),   // VICIOUS: hot orange-red
        rgb(238, 26, 44),   // MERCILESS: red
        rgb(210, 32, 120),  // GRISLY: magenta
        rgb(255, 60, 60),   // 7th rank: bright red (throbs)
        rgb(255, 244, 214), // VELOCIDE: white-hot
    ],
};

/// Red-blind: reds, oranges and molten slag are one dark yellow-brown, so wind-ups, lasers and
/// enemy projectiles turn violet-blue and the parry window white-cyan (looks the foundry never
/// uses); heals are gold. Reds that must read (damage, warnings, low health) are lifted to
/// orange so they don't sink into the dark.
pub const PROTANOPIA: Palette = Palette {
    mode: ColorVisionMode::Protanopia,
    team: [
        rgb(234, 106, 106), // salmon
        rgb(106, 149, 255), // blue
        rgb(234, 212, 0),   // yellow
        rgb(170, 255, 234), // pale aqua
        rgb(234, 64, 191),  // magenta
        rgb(234, 149, 170), // rose
        rgb(106, 212, 255), // sky
        rgb(255, 255, 170), // pale yellow
    ],
    friendly: c(0.3, 0.65, 1.0),
    hostile: c(1.0, 0.55, 0.05),
    telegraph: c(0.55, 0.3, 1.0),
    parry_window: c(0.75, 0.95, 1.0),
    weak_point: c(1.0, 0.92, 0.55),
    aim_laser: c(0.4, 0.45, 1.0),
    hostile_projectile: c(0.75, 0.35, 1.0),
    deflected: c(1.0, 0.97, 0.9),
    brand: c(0.8, 1.0, 0.2),
    heal: c(1.0, 0.8, 0.2),
    damage: rgb(255, 110, 40),
    flash_damage: Vec3::new(1.0, 0.3, 0.0),
    flash_parry: Vec3::new(1.0, 0.97, 0.9),
    flash_shield: Vec3::new(0.7, 0.9, 1.0),
    flash_death: Vec3::new(0.5, 0.2, 0.0),
    flash_heal: Vec3::new(1.0, 0.8, 0.2),
    health: rgb(238, 228, 206),
    health_low: rgb(255, 110, 40),
    hard_damage: rgb(150, 90, 255),
    stamina: rgb(96, 200, 255),
    stamina_empty: STANDARD.stamina_empty,
    warning: rgb(255, 110, 40),
    ranks: [
        rgb(128, 120, 112), // grey
        rgb(190, 160, 96),  // brass
        rgb(245, 205, 40),  // gold
        rgb(255, 140, 30),  // orange
        rgb(80, 140, 255),  // blue
        rgb(160, 225, 255), // ice
        rgb(255, 110, 200), // pink
        rgb(255, 244, 214), // white-hot
    ],
};

/// Green-blind (the most common): like protanopia; damage and warnings stay redder and heals
/// lighter (deuteranopes see reds brighter than protanopes, so the gap comes from lightness).
pub const DEUTERANOPIA: Palette = Palette {
    mode: ColorVisionMode::Deuteranopia,
    team: [
        rgb(255, 0, 149),   // raspberry
        rgb(42, 128, 255),  // blue
        rgb(234, 170, 85),  // amber
        rgb(212, 255, 191), // pale green
        rgb(149, 170, 255), // periwinkle
        rgb(255, 149, 212), // pink
        rgb(64, 255, 212),  // aqua
        rgb(255, 255, 85),  // yellow
    ],
    heal: c(1.0, 0.85, 0.25),
    damage: rgb(240, 60, 30),
    flash_damage: Vec3::new(0.95, 0.2, 0.0),
    health_low: rgb(240, 60, 30),
    warning: rgb(240, 60, 30),
    ..PROTANOPIA
};

/// Blue-blind: blue / green and yellow / pink merge, reds are fine. The foundry stays red, so
/// wind-ups, lasers and enemy projectiles turn azure (seen as teal) and the parry window pale
/// yellow-white; hard damage moves off red.
pub const TRITANOPIA: Palette = Palette {
    mode: ColorVisionMode::Tritanopia,
    team: [
        rgb(255, 42, 0),    // red
        rgb(149, 191, 255), // light blue
        rgb(255, 212, 0),   // yellow
        rgb(128, 255, 149), // green
        rgb(191, 85, 234),  // purple
        rgb(234, 149, 234), // lilac
        rgb(234, 255, 128), // lime
        rgb(255, 0, 170),   // magenta
    ],
    friendly: c(0.2, 0.85, 0.6),
    telegraph: c(0.2, 0.45, 1.0),
    parry_window: c(1.0, 0.95, 0.6),
    weak_point: c(1.0, 0.55, 0.6),
    aim_laser: c(0.2, 0.45, 1.0),
    hostile_projectile: c(0.25, 0.75, 1.0),
    heal: c(1.0, 0.8, 0.45),
    flash_heal: Vec3::new(1.0, 0.8, 0.45),
    hard_damage: rgb(60, 140, 200),
    health_low: rgb(255, 40, 40),
    damage: rgb(255, 40, 40),
    warning: rgb(255, 40, 40),
    ranks: [
        rgb(128, 120, 112), // grey
        rgb(190, 160, 96),  // brass
        rgb(232, 132, 40),  // orange
        rgb(255, 60, 40),   // red
        rgb(150, 230, 90),  // green
        rgb(210, 32, 120),  // magenta
        rgb(255, 140, 40),  // ember
        rgb(255, 244, 214), // white-hot
    ],
    ..STANDARD
};

/// Indexed by [`ColorVisionMode::index`].
pub static PALETTES: [Palette; 4] = [STANDARD, PROTANOPIA, DEUTERANOPIA, TRITANOPIA];

static MODE: AtomicU8 = AtomicU8::new(0);
static SHAPE_CUES: AtomicBool = AtomicBool::new(false);

/// The palette for the player's current colour-vision setting.
pub fn palette() -> &'static Palette {
    Palette::for_mode(color_vision_mode())
}

/// The active colour-vision mode (what [`palette`] follows).
pub fn color_vision_mode() -> ColorVisionMode {
    ColorVisionMode::from_index(MODE.load(Ordering::Relaxed) as usize)
}

/// Whether extra non-colour cues (marker shapes, parry icon, health popups) are drawn.
pub fn shape_cues() -> bool {
    SHAPE_CUES.load(Ordering::Relaxed)
}

/// Sets the process-wide mode and cue switch (the app does this whenever settings change).
pub fn set_accessibility(mode: ColorVisionMode, shape_cues: bool) {
    MODE.store(mode.index() as u8, Ordering::Relaxed);
    SHAPE_CUES.store(shape_cues, Ordering::Relaxed);
}

impl Palette {
    pub fn for_mode(mode: ColorVisionMode) -> &'static Palette {
        &PALETTES[mode.index()]
    }

    /// Colour of player / lobby slot `index` (wraps every 8).
    pub fn team(&self, index: usize) -> Vec4 {
        self.team[index % self.team.len()]
    }

    /// Colour of style rank `index` (clamped to the highest).
    pub fn rank(&self, index: usize) -> Vec4 {
        self.ranks[index.min(self.ranks.len() - 1)]
    }

    /// The default UI theme with this palette's HUD colours.
    pub fn theme(&self) -> Theme {
        let mut t = Theme::default();
        self.apply_to_theme(&mut t);
        t
    }

    /// Copies the HUD colours into `theme` (the rest of the look stays).
    pub fn apply_to_theme(&self, theme: &mut Theme) {
        theme.health = self.health;
        theme.health_low = self.health_low;
        theme.hard_damage = self.hard_damage;
        theme.stamina = self.stamina;
        theme.stamina_empty = self.stamina_empty;
        theme.blood_bright = self.warning;
        theme.rank_colors = self.ranks;
    }
}

/// Shapes that tell players apart without colour: name tags, scoreboard and lobby rows use the
/// same shape for the same slot (slot `i % 8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerShape {
    Square,
    Diamond,
    TriangleUp,
    Ring,
    Cross,
    TriangleDown,
    Bars,
    Chevron,
}

impl MarkerShape {
    pub const ALL: [MarkerShape; 8] = [
        MarkerShape::Square,
        MarkerShape::Diamond,
        MarkerShape::TriangleUp,
        MarkerShape::Ring,
        MarkerShape::Cross,
        MarkerShape::TriangleDown,
        MarkerShape::Bars,
        MarkerShape::Chevron,
    ];

    /// 5x5 pixel mask, top row first (bit 4 = left column).
    pub fn mask(self) -> [u8; 5] {
        match self {
            MarkerShape::Square => [0b11111, 0b11111, 0b11111, 0b11111, 0b11111],
            MarkerShape::Diamond => [0b00100, 0b01110, 0b11111, 0b01110, 0b00100],
            MarkerShape::TriangleUp => [0b00100, 0b00100, 0b01110, 0b01110, 0b11111],
            MarkerShape::Ring => [0b01110, 0b11011, 0b10001, 0b11011, 0b01110],
            MarkerShape::Cross => [0b00100, 0b00100, 0b11111, 0b00100, 0b00100],
            MarkerShape::TriangleDown => [0b11111, 0b01110, 0b01110, 0b00100, 0b00100],
            MarkerShape::Bars => [0b11011, 0b11011, 0b11011, 0b11011, 0b11011],
            MarkerShape::Chevron => [0b10001, 0b11011, 0b01110, 0b00100, 0b00000],
        }
    }
}

/// Marker shape of player / lobby slot `index`.
pub fn marker_shape(index: usize) -> MarkerShape {
    MarkerShape::ALL[index % MarkerShape::ALL.len()]
}

/// World art (models, particles, beams, lights) and the palette.
impl Palette {
    /// Whether world art takes its signal colours from this palette. The standard palette keeps
    /// the art's own colours (every enemy and effect has its own shade of red, orange or
    /// yellow, often an HDR multiplier); the colour-blind palettes replace them by role.
    pub fn recolours_world(&self) -> bool {
        self.mode != ColorVisionMode::Off
    }

    /// A model's emissive override for `role` (`models::Tint::glow_color` / `core_color`):
    /// `None` in the standard palette (the model's own colours).
    pub fn model_glow(&self, role: Vec4) -> Option<Vec4> {
        self.recolours_world().then_some(role)
    }

    /// An effect colour (particle, glow sprite, beam colour): `standard` in the standard
    /// palette; otherwise `role` scaled by `level` with `standard`'s alpha. Additive effects
    /// may use `level` > 1 like the art does (the core then whitens a little, the halo keeps
    /// the hue).
    pub fn effect(&self, standard: Vec4, role: Vec4, level: f32) -> Vec4 {
        if self.recolours_world() {
            (role.truncate() * level).extend(standard.w)
        } else {
            standard
        }
    }

    /// A light colour (a plain multiplier): `standard`, or `role` scaled by `level`.
    pub fn light(&self, standard: Vec3, role: Vec4, level: f32) -> Vec3 {
        if self.recolours_world() {
            role.truncate() * level
        } else {
            standard
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::color_vision::{self, ColorVision, delta_e, srgb_to_lab};

    /// How a colour reaches the viewer.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Path {
        /// UI and screen flashes: drawn as-is.
        Direct,
        /// The 3D scene: through the full-screen correction (strength 0, 0.5 and 1).
        World,
    }

    /// Every rendition of `c` the viewer of `mode` may see on `path`, as they see it.
    fn seen(c: Vec4, mode: ColorVisionMode, path: Path) -> Vec<Vec3> {
        let c = c.truncate();
        let strengths: &[f32] = match (mode, path) {
            (ColorVisionMode::Off, _) | (_, Path::Direct) => &[0.0],
            (_, Path::World) => &[0.0, 0.5, 1.0],
        };
        strengths
            .iter()
            .map(|&s| {
                let shown = color_vision::correct(c, ColorVision::new(mode, s));
                color_vision::simulate(shown, mode)
            })
            .collect()
    }

    /// Smallest difference between `a` and `b` over every rendition.
    fn min_delta(a: Vec4, b: Vec4, mode: ColorVisionMode, path: Path) -> f32 {
        seen(a, mode, path)
            .into_iter()
            .zip(seen(b, mode, path))
            .map(|(x, y)| delta_e(x, y))
            .fold(f32::MAX, f32::min)
    }

    fn v3(c: Vec3) -> Vec4 {
        c.extend(1.0)
    }

    /// The empty part of the health bar (`BarStyle::health`) and a dark foundry wall.
    const BAR_EMPTY: Vec4 = Vec4::new(42.0 / 255.0, 35.0 / 255.0, 34.0 / 255.0, 1.0);
    const DARK_WALL: Vec4 = Vec4::new(40.0 / 255.0, 32.0 / 255.0, 30.0 / 255.0, 1.0);

    fn critical_pairs(p: &Palette) -> Vec<(&'static str, Vec4, Vec4, Path, f32)> {
        use Path::*;
        vec![
            // HUD.
            (
                "health / hard damage",
                p.health,
                p.hard_damage,
                Direct,
                20.0,
            ),
            (
                "hard damage / empty bar",
                p.hard_damage,
                BAR_EMPTY,
                Direct,
                15.0,
            ),
            (
                "hard damage / low health",
                p.hard_damage,
                p.health_low,
                Direct,
                15.0,
            ),
            ("health / low health", p.health, p.health_low, Direct, 20.0),
            (
                "stamina (good news) / warning",
                p.stamina,
                p.warning,
                Direct,
                25.0,
            ),
            ("heal / damage", p.heal, p.damage, Direct, 20.0),
            // Flashes.
            (
                "damage flash / parry flash",
                v3(p.flash_damage),
                v3(p.flash_parry),
                Direct,
                20.0,
            ),
            (
                "damage flash / shield flash",
                v3(p.flash_damage),
                v3(p.flash_shield),
                Direct,
                20.0,
            ),
            (
                "death flash / parry flash",
                v3(p.flash_death),
                v3(p.flash_parry),
                Direct,
                20.0,
            ),
            // World.
            (
                "telegraph / parry window",
                p.telegraph,
                p.parry_window,
                World,
                25.0,
            ),
            (
                "telegraph / deflected",
                p.telegraph,
                p.deflected,
                World,
                20.0,
            ),
            (
                "hostile / deflected projectile",
                p.hostile_projectile,
                p.deflected,
                World,
                20.0,
            ),
            ("friendly / hostile", p.friendly, p.hostile, World, 25.0),
            (
                "weak point / telegraph",
                p.weak_point,
                p.telegraph,
                World,
                15.0,
            ),
            (
                "weak point / parry window",
                p.weak_point,
                p.parry_window,
                World,
                15.0,
            ),
            ("heal / telegraph", p.heal, p.telegraph, World, 15.0),
            ("brand / parry window", p.brand, p.parry_window, World, 12.0),
            ("aim laser / dark wall", p.aim_laser, DARK_WALL, World, 25.0),
            ("telegraph / dark wall", p.telegraph, DARK_WALL, World, 25.0),
            ("brand / telegraph", p.brand, p.telegraph, World, 12.0),
            (
                "parry window / dark wall",
                p.parry_window,
                DARK_WALL,
                World,
                30.0,
            ),
        ]
    }

    #[test]
    fn critical_pairs_stay_distinguishable_in_their_mode() {
        let mut failures = Vec::new();
        for p in &PALETTES {
            let mut pairs = critical_pairs(p);
            if p.mode != ColorVisionMode::Off {
                // The standard heal flash is a faint red like the damage flash (original look);
                // the colour-blind palettes must tell them apart.
                let (heal, hurt) = (v3(p.flash_heal), v3(p.flash_damage));
                pairs.push(("heal flash / damage flash", heal, hurt, Path::Direct, 20.0));
            }
            for (name, a, b, path, min) in pairs {
                let d = min_delta(a, b, p.mode, path);
                if d < min {
                    failures.push(format!("{:?} {name}: ΔE {d:.1} < {min}", p.mode));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Colours the foundry levels are built from (brick, rust, scorched floor, concrete, slag
    /// glow, shadow), sRGB.
    const FOUNDRY: [Vec4; 6] = [
        Vec4::new(0.55, 0.2, 0.15, 1.0),
        Vec4::new(0.45, 0.25, 0.15, 1.0),
        Vec4::new(0.5, 0.4, 0.35, 1.0),
        Vec4::new(0.5, 0.5, 0.48, 1.0),
        Vec4::new(1.0, 0.4, 0.05, 1.0),
        Vec4::new(0.2, 0.12, 0.1, 1.0),
    ];

    #[test]
    fn enemy_signals_stand_out_from_the_foundry() {
        // For a dichromat, red brick, rust and molten slag are all one yellow-brown, so the
        // colour-blind palettes keep enemy signals away from them. (The standard palette's red
        // telegraph and orange projectiles are close to the slag glow even for normal vision,
        // ΔE ~10-12; that's the original art direction, so it isn't checked here.)
        let mut failures = Vec::new();
        for p in PALETTES.iter().filter(|p| p.mode != ColorVisionMode::Off) {
            for (name, c) in [
                ("telegraph", p.telegraph),
                ("parry window", p.parry_window),
                ("aim laser", p.aim_laser),
                ("hostile projectile", p.hostile_projectile),
                ("deflected", p.deflected),
            ] {
                let d = FOUNDRY
                    .iter()
                    .map(|&env| min_delta(c, env, p.mode, Path::World))
                    .fold(f32::MAX, f32::min);
                if d < 20.0 {
                    failures.push(format!("{:?} {name} vs foundry: ΔE {d:.1}", p.mode));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn team_colours_stay_apart_and_bright() {
        let mut failures = Vec::new();
        for p in &PALETTES {
            for i in 0..8 {
                for j in i + 1..8 {
                    for path in [Path::Direct, Path::World] {
                        let d = min_delta(p.team[i], p.team[j], p.mode, path);
                        // ~13 is the best 8 bright colours manage for a dichromat; marker
                        // shapes carry identity the rest of the way.
                        if d < 12.0 {
                            failures.push(format!("{:?} team {i}/{j} {path:?}: {d:.1}", p.mode));
                        }
                    }
                }
                // Readable on dark panels and in dark rooms.
                for s in seen(p.team[i], p.mode, Path::Direct) {
                    let l = srgb_to_lab(s).x;
                    if l < 45.0 {
                        failures.push(format!("{:?} team {i} too dark: L* {l:.0}", p.mode));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn neighbouring_style_ranks_differ() {
        for p in &PALETTES {
            for i in 0..7 {
                let d = min_delta(p.ranks[i], p.ranks[i + 1], p.mode, Path::Direct);
                assert!(d >= 8.0, "{:?} rank {i}/{}: {d:.1}", p.mode, i + 1);
            }
        }
    }

    #[test]
    fn standard_palette_is_the_original_look() {
        assert_eq!(STANDARD.theme(), Theme::default());
        assert_eq!(Palette::for_mode(ColorVisionMode::Off), &STANDARD);
        for mode in ColorVisionMode::ALL {
            assert_eq!(Palette::for_mode(mode).mode, mode);
        }
    }

    #[test]
    fn colour_blind_palettes_fix_the_standard_palettes_worst_pairs() {
        // The reason this module exists: for a protanope the standard hard-damage red sinks
        // into the empty bar, and the standard telegraph / parry pair is a brightness
        // difference only. The swapped palette must do clearly better.
        let std_hard = min_delta(
            STANDARD.hard_damage,
            BAR_EMPTY,
            ColorVisionMode::Protanopia,
            Path::Direct,
        );
        let new_hard = min_delta(
            PROTANOPIA.hard_damage,
            BAR_EMPTY,
            ColorVisionMode::Protanopia,
            Path::Direct,
        );
        assert!(new_hard > std_hard + 15.0, "{std_hard} -> {new_hard}");
        for mode in [ColorVisionMode::Deuteranopia, ColorVisionMode::Tritanopia] {
            let p = Palette::for_mode(mode);
            let std = min_delta(STANDARD.telegraph, STANDARD.parry_window, mode, Path::World);
            let new = min_delta(p.telegraph, p.parry_window, mode, Path::World);
            assert!(new > std + 10.0, "{mode:?}: {std} -> {new}");
        }
    }

    #[test]
    fn marker_shapes_are_distinct() {
        for (i, a) in MarkerShape::ALL.iter().enumerate() {
            for b in &MarkerShape::ALL[i + 1..] {
                assert_ne!(a.mask(), b.mask(), "{a:?} / {b:?}");
            }
            assert!(a.mask().iter().all(|row| *row < 32));
        }
        assert_eq!(marker_shape(9), MarkerShape::Diamond);
    }

    #[test]
    fn global_mode_round_trips() {
        // The only test touching the process-wide switch; it puts the default back.
        set_accessibility(ColorVisionMode::Tritanopia, true);
        assert_eq!(color_vision_mode(), ColorVisionMode::Tritanopia);
        assert!(shape_cues());
        assert_eq!(palette(), &TRITANOPIA);
        set_accessibility(ColorVisionMode::Off, false);
        assert_eq!(palette(), &STANDARD);
    }

    #[test]
    fn world_art_keeps_its_own_colours_in_the_standard_palette() {
        let art = Vec4::new(2.2, 0.35, 0.6, 0.95);
        let light = Vec3::new(1.6, 0.25, 0.15);
        assert!(!STANDARD.recolours_world());
        assert_eq!(STANDARD.effect(art, STANDARD.brand, 1.6), art);
        assert_eq!(STANDARD.light(light, STANDARD.aim_laser, 1.6), light);
        assert_eq!(STANDARD.model_glow(STANDARD.telegraph), None);
        for p in PALETTES.iter().filter(|p| p.mode != ColorVisionMode::Off) {
            assert!(p.recolours_world());
            let c = p.effect(art, p.brand, 1.6);
            assert_eq!(c, (p.brand.truncate() * 1.6).extend(0.95), "{:?}", p.mode);
            assert_eq!(p.light(light, p.aim_laser, 2.0), p.aim_laser.truncate() * 2.0);
            assert_eq!(p.model_glow(p.telegraph), Some(p.telegraph));
        }
    }
}
