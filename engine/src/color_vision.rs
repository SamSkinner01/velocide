//! Colour-vision deficiency (CVD): simulation, correction ("daltonisation") and colour
//! difference maths. Plain data and math, no wgpu: the renderer turns a [`ColorVision`] into one
//! 3x3 matrix for the post pass, and the game uses the same functions to check its palettes.
//!
//! - **Simulation**: Machado, Oliveira & Fernandes 2009, "A Physiologically-based Model for
//!   Simulation of Color Vision Deficiency" (severity 1.0 = dichromacy), applied to *linear*
//!   RGB. Each matrix keeps white white (rows sum to 1).
//! - **Correction**: error redistribution after Fidaner, Lin & Ozguven 2005: the information a
//!   viewer loses, `c - sim(c)`, is spread by a per-deficiency matrix into channels that viewer
//!   still sees (blue-yellow for protan/deutan, red-green for tritan) and added back:
//!   `corrected = c + strength * D * (c - S c)`, one matrix `I + strength * D (I - S)`.
//!   Greys have no error, so neutral colours never change. The spread matrices were tuned on
//!   random colour pairs (see [`spread_matrix`]).
//! - **Colour difference**: CIEDE2000 in CIELAB (D65), see [`delta_e`].
//!
//! Everything in VELOCIDE is shaded in gamma space, so colours here are sRGB-encoded `0..1`
//! unless a function says linear.

use glam::{Mat3, Vec3};
use serde::{Deserialize, Serialize};

/// The deficiency the player wants compensated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorVisionMode {
    /// No correction, standard palette.
    #[default]
    Off,
    /// Missing / weak L cones: reds look dark, red-green confusion.
    Protanopia,
    /// Missing / weak M cones: red-green confusion (the most common).
    Deuteranopia,
    /// Missing / weak S cones: blue-green and yellow-violet confusion.
    Tritanopia,
}

impl ColorVisionMode {
    pub const ALL: [ColorVisionMode; 4] = [
        ColorVisionMode::Off,
        ColorVisionMode::Protanopia,
        ColorVisionMode::Deuteranopia,
        ColorVisionMode::Tritanopia,
    ];

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            ColorVisionMode::Off => "OFF",
            ColorVisionMode::Protanopia => "PROTANOPIA",
            ColorVisionMode::Deuteranopia => "DEUTERANOPIA",
            ColorVisionMode::Tritanopia => "TRITANOPIA",
        }
    }

    /// One-line explanation for the settings screen.
    pub fn description(self) -> &'static str {
        match self {
            ColorVisionMode::Off => "Standard colours.",
            ColorVisionMode::Protanopia => {
                "Red-weak / red-blind: reds look dark, red and green mix up."
            }
            ColorVisionMode::Deuteranopia => {
                "Green-weak / green-blind: red, orange and green mix up."
            }
            ColorVisionMode::Tritanopia => {
                "Blue-weak / blue-blind: blue and green, yellow and pink mix up."
            }
        }
    }

    /// Position in [`ColorVisionMode::ALL`].
    pub fn index(self) -> usize {
        match self {
            ColorVisionMode::Off => 0,
            ColorVisionMode::Protanopia => 1,
            ColorVisionMode::Deuteranopia => 2,
            ColorVisionMode::Tritanopia => 3,
        }
    }

    /// Inverse of [`ColorVisionMode::index`] (out of range gives `Off`).
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }
}

/// The player's colour-vision setting: which deficiency to compensate and how strongly the
/// full-screen correction is applied (palette swaps don't depend on the strength).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorVision {
    pub mode: ColorVisionMode,
    /// Full-screen correction strength, 0..1 (0 = palette swap only).
    pub strength: f32,
}

impl Default for ColorVision {
    fn default() -> Self {
        Self {
            mode: ColorVisionMode::Off,
            strength: 1.0,
        }
    }
}

impl ColorVision {
    pub fn new(mode: ColorVisionMode, strength: f32) -> Self {
        Self { mode, strength }.sanitized()
    }

    /// Strength clamped to 0..1 (NaN = full).
    pub fn sanitized(self) -> Self {
        let strength = if self.strength.is_finite() {
            self.strength.clamp(0.0, 1.0)
        } else {
            1.0
        };
        Self { strength, ..self }
    }

    /// Whether the full-screen correction changes anything.
    pub fn filter_active(&self) -> bool {
        self.mode != ColorVisionMode::Off && self.strength > 0.0
    }

    /// The post-pass matrix, applied to linear RGB.
    pub fn correction_matrix(&self) -> Mat3 {
        let s = self.sanitized();
        correction_matrix(s.mode, s.strength)
    }
}

/// A 3x3 matrix from its rows (glam is column-major).
const fn rows(r: [[f32; 3]; 3]) -> Mat3 {
    Mat3::from_cols(
        Vec3::new(r[0][0], r[1][0], r[2][0]),
        Vec3::new(r[0][1], r[1][1], r[2][1]),
        Vec3::new(r[0][2], r[1][2], r[2][2]),
    )
}

/// Machado et al. 2009, severity 1.0, linear RGB -> what a dichromat perceives (linear RGB).
pub fn simulation_matrix(mode: ColorVisionMode) -> Mat3 {
    match mode {
        ColorVisionMode::Off => Mat3::IDENTITY,
        ColorVisionMode::Protanopia => rows([
            [0.152286, 1.052583, -0.204868],
            [0.114503, 0.786281, 0.099216],
            [-0.003882, -0.048116, 1.051998],
        ]),
        ColorVisionMode::Deuteranopia => rows([
            [0.367322, 0.860646, -0.227968],
            [0.280085, 0.672501, 0.047413],
            [-0.011820, 0.042940, 0.968881],
        ]),
        ColorVisionMode::Tritanopia => rows([
            [1.255528, -0.076749, -0.178779],
            [-0.078411, 0.930809, 0.147602],
            [0.004733, 0.691367, 0.303900],
        ]),
    }
}

/// How the lost information `c - sim(c)` (linear RGB) is added back.
///
/// Fidaner et al.'s original spread (`G += 0.7 eR + eG`, `B += 0.7 eR + eB`) recovers about two
/// thirds of the confused red-green pairs for deuteranopes and darkens reds for protanopes. These
/// matrices came from a search over 40 000 random sRGB pairs that look different with normal
/// vision (ΔE76 > 25) but alike to the dichromat (< 12), scoring pairs made distinguishable
/// again (> 15) minus pairs it breaks and minus darkening as seen by the dichromat:
///
/// - protan: reds gain green (they turn orange and brighter), greens gain blue (teal);
///   ~92 % of the confused pairs separate, without the darkening.
/// - deutan: greens gain blue (teal / cyan), reds stay red; ~85 %.
/// - tritan: blue-yellow error moves into red-green (yellows warm up, blues turn azure); ~74 %.
pub fn spread_matrix(mode: ColorVisionMode) -> Mat3 {
    match mode {
        ColorVisionMode::Off => Mat3::ZERO,
        ColorVisionMode::Protanopia => rows([[0.0, 0.0, 0.0], [0.4, 1.0, 0.0], [-0.4, 0.0, 0.0]]),
        ColorVisionMode::Deuteranopia => rows([[0.4, 0.4, 0.0], [0.0, 0.0, 0.0], [-0.4, 0.7, 0.0]]),
        ColorVisionMode::Tritanopia => rows([[0.0, 0.0, -1.0], [0.0, -0.7, 0.4], [0.0, 0.0, 0.0]]),
    }
}

/// `I + strength * D (I - S)`: the full correction as one matrix on linear RGB.
pub fn correction_matrix(mode: ColorVisionMode, strength: f32) -> Mat3 {
    if mode == ColorVisionMode::Off {
        return Mat3::IDENTITY;
    }
    let error = Mat3::IDENTITY - simulation_matrix(mode);
    Mat3::IDENTITY + spread_matrix(mode) * error * strength
}

/// sRGB transfer function, one channel.
pub fn srgb_channel_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse sRGB transfer function, one channel (clamped to 0..1).
pub fn linear_channel_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

pub fn srgb_to_linear(c: Vec3) -> Vec3 {
    Vec3::new(
        srgb_channel_to_linear(c.x),
        srgb_channel_to_linear(c.y),
        srgb_channel_to_linear(c.z),
    )
}

pub fn linear_to_srgb(c: Vec3) -> Vec3 {
    Vec3::new(
        linear_channel_to_srgb(c.x),
        linear_channel_to_srgb(c.y),
        linear_channel_to_srgb(c.z),
    )
}

/// What a dichromat of `mode` sees of the sRGB colour `c` (sRGB, clamped to the gamut).
pub fn simulate(c: Vec3, mode: ColorVisionMode) -> Vec3 {
    linear_to_srgb(simulation_matrix(mode) * srgb_to_linear(c))
}

/// The post pass's correction of an sRGB colour, done on the CPU (the shader does the same).
pub fn correct(c: Vec3, cv: ColorVision) -> Vec3 {
    if !cv.filter_active() {
        return c;
    }
    linear_to_srgb(cv.correction_matrix() * srgb_to_linear(c))
}

/// sRGB (D65) -> CIELAB (D65 white).
pub fn srgb_to_lab(c: Vec3) -> Vec3 {
    let l = srgb_to_linear(c.clamp(Vec3::ZERO, Vec3::ONE));
    let xyz = rows([
        [0.412_456_4, 0.357_576_1, 0.180_437_5],
        [0.212_672_9, 0.715_152_2, 0.072_175_0],
        [0.019_333_9, 0.119_192, 0.950_304_1],
    ]) * l;
    let white = Vec3::new(0.950_47, 1.0, 1.088_83);
    let f = |t: f32| {
        const D: f32 = 6.0 / 29.0;
        if t > D * D * D {
            t.cbrt()
        } else {
            t / (3.0 * D * D) + 4.0 / 29.0
        }
    };
    let (fx, fy, fz) = (f(xyz.x / white.x), f(xyz.y / white.y), f(xyz.z / white.z));
    Vec3::new(116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// CIEDE2000 colour difference of two CIELAB colours (kL = kC = kH = 1). About 1 is a
/// just-noticeable difference side by side; 10+ reads as clearly different colours.
pub fn delta_e_2000(lab1: Vec3, lab2: Vec3) -> f32 {
    // f64 inside: the hue terms are sensitive near the axes.
    let (l1, a1, b1) = (lab1.x as f64, lab1.y as f64, lab1.z as f64);
    let (l2, a2, b2) = (lab2.x as f64, lab2.y as f64, lab2.z as f64);
    let pow7 = |x: f64| x.powi(7);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let c_bar = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (pow7(c_bar) / (pow7(c_bar) + pow7(25.0))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = a1p.hypot(b1);
    let c2p = a2p.hypot(b2);
    let hue = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            b.atan2(a).to_degrees().rem_euclid(360.0)
        }
    };
    let h1p = hue(b1, a1p);
    let h2p = hue(b2, a2p);
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let chroma_zero = c1p * c2p == 0.0;
    let dhp = if chroma_zero {
        0.0
    } else {
        let d = h2p - h1p;
        if d > 180.0 {
            d - 360.0
        } else if d < -180.0 {
            d + 360.0
        } else {
            d
        }
    };
    let d_hp = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() / 2.0).sin();
    let l_bar = (l1 + l2) / 2.0;
    let cp_bar = (c1p + c2p) / 2.0;
    let hp_bar = if chroma_zero {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let cos = |deg: f64| deg.to_radians().cos();
    let t =
        1.0 - 0.17 * cos(hp_bar - 30.0) + 0.24 * cos(2.0 * hp_bar) + 0.32 * cos(3.0 * hp_bar + 6.0)
            - 0.20 * cos(4.0 * hp_bar - 63.0);
    let d_theta = 30.0 * (-((hp_bar - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (pow7(cp_bar) / (pow7(cp_bar) + pow7(25.0))).sqrt();
    let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cp_bar;
    let sh = 1.0 + 0.015 * cp_bar * t;
    let rt = -(2.0 * d_theta).to_radians().sin() * rc;
    let (tl, tc, th) = (dlp / sl, dcp / sc, d_hp / sh);
    (tl * tl + tc * tc + th * th + rt * tc * th).max(0.0).sqrt() as f32
}

/// CIEDE2000 difference of two sRGB colours.
pub fn delta_e(a: Vec3, b: Vec3) -> f32 {
    delta_e_2000(srgb_to_lab(a), srgb_to_lab(b))
}

/// Difference of two sRGB colours as a dichromat of `mode` sees them.
pub fn delta_e_seen(a: Vec3, b: Vec3, mode: ColorVisionMode) -> f32 {
    delta_e(simulate(a, mode), simulate(b, mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFICIENT: [ColorVisionMode; 3] = [
        ColorVisionMode::Protanopia,
        ColorVisionMode::Deuteranopia,
        ColorVisionMode::Tritanopia,
    ];

    #[test]
    fn simulation_keeps_white_and_black() {
        for mode in ColorVisionMode::ALL {
            let white = simulation_matrix(mode) * Vec3::ONE;
            assert!(
                (white - Vec3::ONE).abs().max_element() < 1e-5,
                "{mode:?}: {white}"
            );
            assert_eq!(simulation_matrix(mode) * Vec3::ZERO, Vec3::ZERO);
        }
    }

    #[test]
    fn correction_leaves_greys_and_off_alone() {
        for mode in DEFICIENT {
            let m = correction_matrix(mode, 1.0);
            for g in [0.0, 0.18, 0.5, 1.0] {
                let out = m * Vec3::splat(g);
                assert!(
                    (out - Vec3::splat(g)).abs().max_element() < 1e-5,
                    "{mode:?} {g}"
                );
            }
            assert_eq!(correction_matrix(mode, 0.0), Mat3::IDENTITY);
        }
        assert_eq!(correction_matrix(ColorVisionMode::Off, 1.0), Mat3::IDENTITY);
        let c = Vec3::new(0.9, 0.2, 0.1);
        assert_eq!(correct(c, ColorVision::default()), c);
        let half = ColorVision::new(ColorVisionMode::Deuteranopia, 0.0);
        assert_eq!(correct(c, half), c);
    }

    #[test]
    fn strength_is_sanitised() {
        assert_eq!(
            ColorVision::new(ColorVisionMode::Protanopia, 7.0).strength,
            1.0
        );
        assert_eq!(
            ColorVision::new(ColorVisionMode::Protanopia, -1.0).strength,
            0.0
        );
        assert_eq!(
            ColorVision::new(ColorVisionMode::Protanopia, f32::NAN).strength,
            1.0
        );
        assert!(!ColorVision::new(ColorVisionMode::Protanopia, 0.0).filter_active());
        assert!(!ColorVision::new(ColorVisionMode::Off, 1.0).filter_active());
        for (i, m) in ColorVisionMode::ALL.iter().enumerate() {
            assert_eq!(m.index(), i);
            assert_eq!(ColorVisionMode::from_index(i), *m);
        }
        assert_eq!(ColorVisionMode::from_index(99), ColorVisionMode::Off);
    }

    #[test]
    fn transfer_functions_round_trip() {
        for i in 0..=255 {
            let c = i as f32 / 255.0;
            let back = linear_channel_to_srgb(srgb_channel_to_linear(c));
            assert!((back - c).abs() < 1e-4, "{c} -> {back}");
        }
    }

    #[test]
    fn lab_reference_values() {
        let white = srgb_to_lab(Vec3::ONE);
        assert!(
            (white - Vec3::new(100.0, 0.0, 0.0)).abs().max_element() < 0.05,
            "{white}"
        );
        // sRGB red is L* 53.24, a* 80.09, b* 67.20.
        let red = srgb_to_lab(Vec3::X);
        assert!(
            (red - Vec3::new(53.24, 80.09, 67.20)).abs().max_element() < 0.1,
            "{red}"
        );
    }

    #[test]
    fn ciede2000_matches_sharma_test_data() {
        // Sharma, Wu & Dalal 2005, "The CIEDE2000 color-difference formula: implementation
        // notes, supplementary test data and mathematical observations", table 1.
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.49, -0.001], [50.0, -2.49, 0.0009], 7.1792),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            (
                [60.2574, -34.0099, 36.2677],
                [60.4626, -34.1751, 39.4387],
                1.2644,
            ),
            (
                [22.7233, 20.0904, -46.6940],
                [23.0331, 14.9730, -42.5619],
                2.0373,
            ),
        ];
        for (a, b, expected) in pairs {
            let d = delta_e_2000(Vec3::from(a), Vec3::from(b));
            assert!(
                (d - expected).abs() < 1e-3,
                "{a:?} {b:?}: {d} != {expected}"
            );
            let back = delta_e_2000(Vec3::from(b), Vec3::from(a));
            assert!((back - d).abs() < 1e-4, "symmetric");
        }
    }

    /// Small deterministic generator (tests only).
    fn rand01(state: &mut u64) -> f32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 40) as f32) / (1u64 << 24) as f32
    }

    #[test]
    fn correction_separates_confused_pairs() {
        // Pairs that look different with normal vision but alike to the dichromat: after the
        // full correction the dichromat should see most of them as different again, and pairs
        // they could already tell apart must stay apart.
        for (mode, want) in [
            (ColorVisionMode::Protanopia, 0.80),
            (ColorVisionMode::Deuteranopia, 0.75),
            (ColorVisionMode::Tritanopia, 0.60),
        ] {
            let cv = ColorVision::new(mode, 1.0);
            let mut state = 0x5eed_u64 + mode.index() as u64;
            let (mut confused, mut fixed, mut clear, mut broken) = (0, 0, 0, 0);
            for _ in 0..40_000 {
                let mut c =
                    || Vec3::new(rand01(&mut state), rand01(&mut state), rand01(&mut state));
                let (a, b) = (c(), c());
                if delta_e(a, b) < 15.0 {
                    continue;
                }
                let before = delta_e_seen(a, b, mode);
                let after = delta_e_seen(correct(a, cv), correct(b, cv), mode);
                if before < 6.0 {
                    confused += 1;
                    fixed += (after >= 8.0) as u32;
                } else if before > 12.0 {
                    clear += 1;
                    broken += (after < 6.0) as u32;
                }
            }
            let fixed_share = fixed as f32 / confused.max(1) as f32;
            let broken_share = broken as f32 / clear.max(1) as f32;
            assert!(
                confused > 150,
                "{mode:?}: only {confused} confused pairs sampled"
            );
            assert!(
                fixed_share >= want,
                "{mode:?}: {:.0}% of {confused} confused pairs separated",
                fixed_share * 100.0
            );
            assert!(
                broken_share < 0.03,
                "{mode:?}: {:.1}% of distinct pairs merged",
                broken_share * 100.0
            );
        }
    }
}
