//! Character customisation (`docs/expansion.md`, "Cosmetics"): how a player's Shrike looks.
//!
//! Plain data: each field is an index into a fixed catalogue whose names are listed here; the
//! client's Shrike model (`client/src/models/shrike.rs`) decides what each option looks like.
//! Cosmetics never change gameplay or hit shapes. The server clamps whatever a client sends
//! ([`Cosmetics::sanitized`]) and shares it with everyone (`PlayerInfo`).

use serde::{Deserialize, Serialize};

/// Mask shapes.
pub const MASKS: [&str; 8] = [
    "SHRIKE", "HOOKBILL", "LANTERN", "VISOR", "HORNED", "WEEPER", "GRILLE", "CROWN",
];
/// Head crests / plumes.
pub const CRESTS: [&str; 6] = ["NONE", "QUILLS", "FIN", "ANTLERS", "TASSEL", "HALO"];
/// Cloak / mantle cuts.
pub const CLOAKS: [&str; 5] = ["NONE", "SHORT", "LONG", "TATTERED", "SCARF"];
/// Chest-lantern emblems.
pub const EMBLEMS: [&str; 8] = [
    "NONE", "BELL", "KEY", "EYE", "SPIRAL", "FANG", "STAR", "WING",
];
/// Body / trim colours (sRGB 0..1). The player's *team / identity* colour used for markers,
/// the scoreboard and colour-vision palettes is separate (lobby colour index); these are
/// purely decorative.
pub const COLORS: [(&str, [f32; 3]); 16] = [
    ("ASH", [0.42, 0.40, 0.38]),
    ("SOOT", [0.16, 0.15, 0.16]),
    ("BONE", [0.86, 0.82, 0.72]),
    ("TALLOW", [0.93, 0.86, 0.62]),
    ("RUST", [0.62, 0.30, 0.16]),
    ("OXBLOOD", [0.45, 0.10, 0.12]),
    ("EMBER", [0.95, 0.45, 0.12]),
    ("BRASS", [0.78, 0.62, 0.28]),
    ("MOSS", [0.36, 0.45, 0.22]),
    ("VERDIGRIS", [0.30, 0.62, 0.55]),
    ("SLATE", [0.30, 0.36, 0.44]),
    ("COBALT", [0.18, 0.30, 0.70]),
    ("WOAD", [0.30, 0.22, 0.52]),
    ("HEATHER", [0.62, 0.42, 0.62]),
    ("CHALK", [0.92, 0.92, 0.90]),
    ("PITCH", [0.06, 0.06, 0.08]),
];
/// Wick flame colours (the chest lantern and the death / kindling effects).
pub const FLAMES: [(&str, [f32; 3]); 8] = [
    ("AMBER", [1.0, 0.62, 0.18]),
    ("WHITE", [1.0, 0.96, 0.88]),
    ("CRIMSON", [1.0, 0.24, 0.16]),
    ("AZURE", [0.35, 0.65, 1.0]),
    ("VERDANT", [0.45, 1.0, 0.40]),
    ("VIOLET", [0.75, 0.40, 1.0]),
    ("GOLD", [1.0, 0.84, 0.30]),
    ("CYAN", [0.35, 1.0, 0.95]),
];

/// How a Shrike looks. Every field indexes the catalogue of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Cosmetics {
    pub mask: u8,
    pub crest: u8,
    pub cloak: u8,
    pub emblem: u8,
    /// Body / cloak colour ([`COLORS`]).
    pub primary: u8,
    /// Trim / mask colour ([`COLORS`]).
    pub secondary: u8,
    /// Wick flame ([`FLAMES`]).
    pub flame: u8,
}

impl Default for Cosmetics {
    fn default() -> Self {
        Self {
            mask: 0,
            crest: 1,
            cloak: 1,
            emblem: 0,
            primary: 0,
            secondary: 2,
            flame: 0,
        }
    }
}

impl Cosmetics {
    /// Every index wrapped into its catalogue (never fails).
    pub fn sanitized(self) -> Cosmetics {
        let wrap = |v: u8, n: usize| (v as usize % n) as u8;
        Cosmetics {
            mask: wrap(self.mask, MASKS.len()),
            crest: wrap(self.crest, CRESTS.len()),
            cloak: wrap(self.cloak, CLOAKS.len()),
            emblem: wrap(self.emblem, EMBLEMS.len()),
            primary: wrap(self.primary, COLORS.len()),
            secondary: wrap(self.secondary, COLORS.len()),
            flame: wrap(self.flame, FLAMES.len()),
        }
    }

    pub fn primary_rgb(&self) -> [f32; 3] {
        COLORS[self.sanitized().primary as usize].1
    }

    pub fn secondary_rgb(&self) -> [f32; 3] {
        COLORS[self.sanitized().secondary as usize].1
    }

    pub fn flame_rgb(&self) -> [f32; 3] {
        FLAMES[self.sanitized().flame as usize].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_wraps_every_index() {
        let c = Cosmetics {
            mask: 200,
            crest: 6,
            cloak: 5,
            emblem: 9,
            primary: 16,
            secondary: 255,
            flame: 8,
        }
        .sanitized();
        assert!((c.mask as usize) < MASKS.len());
        assert_eq!(c.crest, 0);
        assert_eq!(c.cloak, 0);
        assert_eq!(c.emblem, 1);
        assert_eq!(c.primary, 0);
        assert!((c.secondary as usize) < COLORS.len());
        assert_eq!(c.flame, 0);
        assert_eq!(Cosmetics::default().sanitized(), Cosmetics::default());
    }
}
