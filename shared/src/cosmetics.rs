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
pub const COLORS: [(&str, [f32; 3]); 18] = [
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
    // The Shrike's own tanned coat and its darker straps.
    ("HIDE", [0.58, 0.37, 0.21]),
    ("UMBER", [0.34, 0.22, 0.15]),
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
    /// The classic Shrike: beaked porcelain mask, swept quills, a short mantle over the tanned
    /// coat, an amber wick.
    fn default() -> Self {
        Self {
            mask: 0,
            crest: 1,
            cloak: 1,
            emblem: 0,
            primary: 16,
            secondary: 2,
            flame: 0,
        }
    }
}

/// Which catalogue a [`Cosmetics`] field indexes (the customise screen's rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Mask,
    Crest,
    Cloak,
    Emblem,
    Primary,
    Secondary,
    Flame,
}

impl Slot {
    pub const ALL: [Slot; 7] = [
        Slot::Mask,
        Slot::Crest,
        Slot::Cloak,
        Slot::Emblem,
        Slot::Primary,
        Slot::Secondary,
        Slot::Flame,
    ];

    /// Menu label.
    pub fn title(self) -> &'static str {
        match self {
            Slot::Mask => "MASK",
            Slot::Crest => "CREST",
            Slot::Cloak => "CLOAK",
            Slot::Emblem => "EMBLEM",
            Slot::Primary => "COAT COLOUR",
            Slot::Secondary => "MASK COLOUR",
            Slot::Flame => "WICK FLAME",
        }
    }

    /// The catalogue's option names.
    pub fn names(self) -> Vec<&'static str> {
        match self {
            Slot::Mask => MASKS.to_vec(),
            Slot::Crest => CRESTS.to_vec(),
            Slot::Cloak => CLOAKS.to_vec(),
            Slot::Emblem => EMBLEMS.to_vec(),
            Slot::Primary | Slot::Secondary => COLORS.iter().map(|(n, _)| *n).collect(),
            Slot::Flame => FLAMES.iter().map(|(n, _)| *n).collect(),
        }
    }

    /// Number of options.
    pub fn len(self) -> usize {
        match self {
            Slot::Mask => MASKS.len(),
            Slot::Crest => CRESTS.len(),
            Slot::Cloak => CLOAKS.len(),
            Slot::Emblem => EMBLEMS.len(),
            Slot::Primary | Slot::Secondary => COLORS.len(),
            Slot::Flame => FLAMES.len(),
        }
    }

    /// Never empty (every catalogue has options).
    pub fn is_empty(self) -> bool {
        false
    }

    /// The swatch colour of option `index` (colour slots only).
    pub fn color(self, index: u8) -> Option<[f32; 3]> {
        match self {
            Slot::Primary | Slot::Secondary => COLORS.get(index as usize).map(|(_, c)| *c),
            Slot::Flame => FLAMES.get(index as usize).map(|(_, c)| *c),
            _ => None,
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

    /// The value of one slot.
    pub fn get(&self, slot: Slot) -> u8 {
        match slot {
            Slot::Mask => self.mask,
            Slot::Crest => self.crest,
            Slot::Cloak => self.cloak,
            Slot::Emblem => self.emblem,
            Slot::Primary => self.primary,
            Slot::Secondary => self.secondary,
            Slot::Flame => self.flame,
        }
    }

    /// Sets one slot (wrapped into its catalogue).
    pub fn set(&mut self, slot: Slot, value: u8) {
        let v = (value as usize % slot.len()) as u8;
        match slot {
            Slot::Mask => self.mask = v,
            Slot::Crest => self.crest = v,
            Slot::Cloak => self.cloak = v,
            Slot::Emblem => self.emblem = v,
            Slot::Primary => self.primary = v,
            Slot::Secondary => self.secondary = v,
            Slot::Flame => self.flame = v,
        }
    }

    /// A varied, deterministic look from a seed (server bots, the customise screen's
    /// RANDOMISE). The coat and trim colours always differ.
    pub fn from_seed(seed: u32) -> Cosmetics {
        // A small integer hash (splitmix-style), one draw per slot.
        let mut x = seed.wrapping_mul(0x9E37_79B9).wrapping_add(0x7F4A_7C15);
        let mut next = |n: usize| {
            x ^= x >> 16;
            x = x.wrapping_mul(0x85EB_CA6B);
            x ^= x >> 13;
            x = x.wrapping_mul(0xC2B2_AE35);
            x ^= x >> 16;
            (x as usize % n) as u8
        };
        let mut c = Cosmetics {
            mask: next(MASKS.len()),
            crest: next(CRESTS.len()),
            cloak: next(CLOAKS.len()),
            emblem: next(EMBLEMS.len()),
            primary: next(COLORS.len()),
            secondary: next(COLORS.len()),
            flame: next(FLAMES.len()),
        };
        if c.secondary == c.primary {
            c.secondary = ((c.primary as usize + COLORS.len() / 2) % COLORS.len()) as u8;
        }
        c
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
            primary: 34,
            secondary: 255,
            flame: 8,
        }
        .sanitized();
        assert!((c.mask as usize) < MASKS.len());
        assert_eq!(c.crest, 0);
        assert_eq!(c.cloak, 0);
        assert_eq!(c.emblem, 1);
        assert_eq!(c.primary, 16);
        assert!((c.secondary as usize) < COLORS.len());
        assert_eq!(c.flame, 0);
        assert_eq!(Cosmetics::default().sanitized(), Cosmetics::default());
    }

    #[test]
    fn slots_and_seeds() {
        let mut c = Cosmetics::default();
        for slot in Slot::ALL {
            assert_eq!(slot.names().len(), slot.len());
            c.set(slot, (slot.len() + 1) as u8);
            assert_eq!(c.get(slot), 1, "{slot:?} wraps");
        }
        assert_eq!(c, c.sanitized());
        let looks: Vec<Cosmetics> = (0..32).map(Cosmetics::from_seed).collect();
        for l in &looks {
            assert_eq!(*l, l.sanitized());
            assert_ne!(l.primary, l.secondary);
        }
        assert_eq!(Cosmetics::from_seed(7), Cosmetics::from_seed(7));
        let masks: std::collections::BTreeSet<u8> = looks.iter().map(|l| l.mask).collect();
        assert!(masks.len() >= 5, "seeds vary the look: {masks:?}");
    }
}
