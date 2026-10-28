//! Loadouts and weapon builds (`docs/arsenal.md`).
//!
//! A player carries a [`Loadout`]: up to [`LOADOUT_SLOTS`] weapon builds on the slot keys
//! 1–4. A [`WeaponBuild`] is a base weapon ([`WeaponKind`]: fixed stats and primary fire) plus
//! two alt-fire [`Ability`]s chosen from the pool of the weapon's [`WeaponClass`]. Pressing a
//! slot's key again cycles between its two abilities (`PlayerInput::weapon_mode`).
//!
//! The Q slot holds one [`Knack`] (a utility the player picks: the Thorn by default), part of
//! the loadout too.
//!
//! This module is plain data: ids, names, the class / ability compatibility table, defaults
//! and validation. What each weapon and ability *does* lives in `sim::weapons` and friends.
//! The ids are part of the network protocol (replicated player state, lobby messages); never
//! renumber them, only append.

use serde::{Deserialize, Serialize};

/// Weapon slots on the number keys (`PlayerInput::weapon` 0..LOADOUT_SLOTS).
pub const LOADOUT_SLOTS: usize = 4;
/// Alt-fire abilities per build (`PlayerInput::weapon_mode` 0 = A, 1 = B).
pub const BUILD_ABILITIES: usize = 2;

/// What a weapon is for; decides which abilities fit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WeaponClass {
    /// Single accurate shots (hitscan).
    Precision,
    /// Pellet spreads, close range.
    Scatter,
    /// Sustained fire while held.
    Rapid,
    /// Launchers: splash damage, blast-jumps.
    Explosive,
    /// Bows: hold to draw, release to loose.
    Draw,
    /// Blades and hammers: swings in an arc in front of the eye.
    Melee,
}

impl WeaponClass {
    pub const ALL: [WeaponClass; 6] = [
        WeaponClass::Precision,
        WeaponClass::Scatter,
        WeaponClass::Rapid,
        WeaponClass::Explosive,
        WeaponClass::Draw,
        WeaponClass::Melee,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WeaponClass::Precision => "PRECISION",
            WeaponClass::Scatter => "SCATTER",
            WeaponClass::Rapid => "RAPID",
            WeaponClass::Explosive => "EXPLOSIVE",
            WeaponClass::Draw => "DRAW",
            WeaponClass::Melee => "MELEE",
        }
    }

    /// The abilities a weapon of this class can take, in menu order.
    pub fn abilities(self) -> &'static [Ability] {
        use Ability::*;
        match self {
            WeaponClass::Precision => &[Brand, Wickline, Tally, Waxwork],
            WeaponClass::Scatter => &[Gust, Bolas, Bulwark, Flak],
            WeaponClass::Rapid => &[Wisp, Discharge, Stoke, Lantern],
            WeaponClass::Explosive => &[Breach, Gravewell, Limpet, Kite],
            WeaponClass::Draw => &[Brand, BlinkArrow, PinningArrow, BeaconArrow],
            WeaponClass::Melee => &[Lunge, Whirl, Hurl, Riposte],
        }
    }
}

/// A base weapon. The discriminant is the wire id.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum WeaponKind {
    /// Precision spike-caster (hitscan; its shots ricochet off Thorns).
    #[default]
    Quill = 0,
    /// Twin-barrel blast cannon.
    Bellows = 1,
    /// Arc caster: a beam that chains between targets.
    Arclash = 2,
    /// Bouncing-bomb launcher.
    Tumbler = 3,
    /// Fast semi-automatic needle pistol (returning, redesigned).
    Lancet = 4,
    /// Drum-fed auto-scattergun: weaker pellets, quick follow-ups (returning, redesigned).
    Thresher = 5,
    /// Pneumatic rivet driver: fast hot-rivet projectiles (returning, redesigned).
    Riveter = 6,
    /// Box launcher: a salvo of three small cinder rockets per trigger (returning, redesigned).
    Cinderbox = 7,
    /// Recurve bow: hold to draw, release to loose a taper arrow.
    Longwick = 8,
    /// Ember sprayer: a short cone of burning embers that sets targets alight.
    Kiln = 9,
    // ---- refit (docs/refit.md): melee ----
    /// Hooked wick-shears: fast alternating slashes that also deflect projectiles.
    Trimmer = 10,
    /// Foundry ramming hammer: a slow, heavy swing through everything in its arc.
    Tamper = 11,
    // ---- rework (docs/rework.md): the sniper ----
    /// A lamplighter's pole turned rifle: hold to focus a hitscan beam, release to fire.
    Lamplighter = 12,
}

impl WeaponKind {
    pub const ALL: [WeaponKind; 13] = [
        WeaponKind::Quill,
        WeaponKind::Bellows,
        WeaponKind::Arclash,
        WeaponKind::Tumbler,
        WeaponKind::Lancet,
        WeaponKind::Thresher,
        WeaponKind::Riveter,
        WeaponKind::Cinderbox,
        WeaponKind::Longwick,
        WeaponKind::Kiln,
        WeaponKind::Trimmer,
        WeaponKind::Tamper,
        WeaponKind::Lamplighter,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub fn from_id(id: u8) -> Option<WeaponKind> {
        Self::ALL.get(id as usize).copied()
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    /// Display name ("Quill").
    pub fn name(self) -> &'static str {
        match self {
            WeaponKind::Quill => "Quill",
            WeaponKind::Bellows => "Bellows",
            WeaponKind::Arclash => "Arclash",
            WeaponKind::Tumbler => "Tumbler",
            WeaponKind::Lancet => "Lancet",
            WeaponKind::Thresher => "Thresher",
            WeaponKind::Riveter => "Riveter",
            WeaponKind::Cinderbox => "Cinderbox",
            WeaponKind::Longwick => "Longwick",
            WeaponKind::Kiln => "Kiln",
            WeaponKind::Trimmer => "Trimmer",
            WeaponKind::Tamper => "Tamper",
            WeaponKind::Lamplighter => "Lamplighter",
        }
    }

    /// What it is, in a few words (menus).
    pub fn blurb(self) -> &'static str {
        match self {
            WeaponKind::Quill => "precision spike-caster",
            WeaponKind::Bellows => "twin-barrel blast cannon",
            WeaponKind::Arclash => "chaining arc caster",
            WeaponKind::Tumbler => "bouncing-bomb launcher",
            WeaponKind::Lancet => "rapid needle pistol",
            WeaponKind::Thresher => "drum-fed auto-scattergun",
            WeaponKind::Riveter => "pneumatic rivet driver",
            WeaponKind::Cinderbox => "salvo box launcher",
            WeaponKind::Longwick => "recurve bow",
            WeaponKind::Kiln => "ember sprayer",
            WeaponKind::Trimmer => "hooked wick-shears",
            WeaponKind::Tamper => "foundry ramming hammer",
            WeaponKind::Lamplighter => "focusing lens rifle",
        }
    }

    pub fn class(self) -> WeaponClass {
        match self {
            WeaponKind::Quill | WeaponKind::Lancet | WeaponKind::Lamplighter => {
                WeaponClass::Precision
            }
            WeaponKind::Bellows | WeaponKind::Thresher => WeaponClass::Scatter,
            WeaponKind::Arclash | WeaponKind::Riveter | WeaponKind::Kiln => WeaponClass::Rapid,
            WeaponKind::Tumbler | WeaponKind::Cinderbox => WeaponClass::Explosive,
            WeaponKind::Longwick => WeaponClass::Draw,
            WeaponKind::Trimmer | WeaponKind::Tamper => WeaponClass::Melee,
        }
    }

    /// The abilities this weapon can take.
    pub fn abilities(self) -> &'static [Ability] {
        self.class().abilities()
    }

    /// The build a fresh loadout uses for this weapon.
    pub fn default_build(self) -> WeaponBuild {
        use Ability::*;
        let abilities = match self {
            WeaponKind::Quill => [Brand, Wickline],
            WeaponKind::Bellows => [Gust, Bulwark],
            WeaponKind::Arclash => [Wisp, Discharge],
            WeaponKind::Tumbler => [Breach, Gravewell],
            WeaponKind::Lancet => [Tally, Waxwork],
            WeaponKind::Thresher => [Bolas, Flak],
            WeaponKind::Riveter => [Stoke, Lantern],
            WeaponKind::Cinderbox => [Limpet, Kite],
            WeaponKind::Longwick => [BlinkArrow, PinningArrow],
            WeaponKind::Kiln => [Lantern, Discharge],
            WeaponKind::Trimmer => [Lunge, Hurl],
            WeaponKind::Tamper => [Whirl, Riposte],
            WeaponKind::Lamplighter => [Wickline, Waxwork],
        };
        WeaponBuild {
            weapon: self,
            abilities,
        }
    }
}

/// An alt-fire ability. The discriminant is the wire id. The rework (`docs/rework.md`) retired
/// eleven abilities and gave their ids to new ones; the kept ones kept theirs. A name an older
/// build saved that no longer exists loads as the weapon's default ([`WeaponBuild`]'s
/// deserialisation).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum Ability {
    /// Marking shot: the target takes extra damage from everyone for a while.
    #[default]
    Brand = 0,
    /// A burning wick-thread: zips you to the level, yanks a light target to you.
    Wickline = 1,
    /// Air blast: knockback, turns hostile projectiles around.
    Gust = 2,
    /// Wax-weighted balls on a cord: bind the first hostile (and one near it).
    Bolas = 3,
    /// Ball lightning orbiting you, zapping what comes near; press again to fling it.
    Wisp = 4,
    /// Hold to charge a shock nova around you.
    Discharge = 5,
    /// Point-blank blast (blast-jump).
    Breach = 6,
    /// Canister that pulls enemies together, then implodes.
    Gravewell = 7,
    /// Hold to sweep the crosshair over targets, release to shoot each one.
    Tally = 8,
    /// A wax double of you that draws the Moulds, then bursts.
    Waxwork = 9,
    /// Hold up a slab of hardened wax that absorbs what hits it from the front.
    Bulwark = 10,
    /// A shell that airbursts into a ring of pellets at range.
    Flak = 11,
    /// Faster fire that sets targets alight, then the weapon overheats.
    Stoke = 12,
    /// A lantern on the floor that shoots the nearest hostile with your weapon.
    Lantern = 13,
    /// Charges that stick to anything; press again to detonate them.
    Limpet = 14,
    /// An ember kite you steer with your crosshair.
    Kite = 15,
    /// An arrow you can step to.
    BlinkArrow = 16,
    /// An arrow that pins its target in place.
    PinningArrow = 17,
    /// An arrow that lights a beacon: what is near it is revealed and takes more damage.
    BeaconArrow = 18,
    /// A dash along the aim; the first target the body reaches takes a heavy hit.
    Lunge = 19,
    /// A spin that hits everything around and knocks it outward.
    Whirl = 20,
    /// Throws a spinning blade / hammer-head that comes back.
    Hurl = 21,
    /// Hold to guard; release to counter-swing if anything was blocked.
    Riposte = 22,
}

impl Ability {
    pub const ALL: [Ability; 23] = [
        Ability::Brand,
        Ability::Wickline,
        Ability::Gust,
        Ability::Bolas,
        Ability::Wisp,
        Ability::Discharge,
        Ability::Breach,
        Ability::Gravewell,
        Ability::Tally,
        Ability::Waxwork,
        Ability::Bulwark,
        Ability::Flak,
        Ability::Stoke,
        Ability::Lantern,
        Ability::Limpet,
        Ability::Kite,
        Ability::BlinkArrow,
        Ability::PinningArrow,
        Ability::BeaconArrow,
        Ability::Lunge,
        Ability::Whirl,
        Ability::Hurl,
        Ability::Riposte,
    ];
    pub const COUNT: usize = Self::ALL.len();

    pub fn from_id(id: u8) -> Option<Ability> {
        Self::ALL.get(id as usize).copied()
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    /// Display name ("Pinning Arrow").
    pub fn name(self) -> &'static str {
        match self {
            Ability::Brand => "Brand",
            Ability::Wickline => "Wickline",
            Ability::Gust => "Gust",
            Ability::Bolas => "Bolas",
            Ability::Wisp => "Wisp",
            Ability::Discharge => "Discharge",
            Ability::Breach => "Breach",
            Ability::Gravewell => "Gravewell",
            Ability::Tally => "Tally",
            Ability::Waxwork => "Waxwork",
            Ability::Bulwark => "Bulwark",
            Ability::Flak => "Flak",
            Ability::Stoke => "Stoke",
            Ability::Lantern => "Lantern",
            Ability::Limpet => "Limpet",
            Ability::Kite => "Kite",
            Ability::BlinkArrow => "Blink Arrow",
            Ability::PinningArrow => "Pinning Arrow",
            Ability::BeaconArrow => "Beacon Arrow",
            Ability::Lunge => "Lunge",
            Ability::Whirl => "Whirl",
            Ability::Hurl => "Hurl",
            Ability::Riposte => "Riposte",
        }
    }

    /// One-line description (menus).
    pub fn blurb(self) -> &'static str {
        match self {
            Ability::Brand => "marks the target: it takes extra damage from everyone",
            Ability::Wickline => "zip to where it anchors; yanks light targets to you",
            Ability::Gust => "air blast: shoves, turns projectiles around",
            Ability::Bolas => "binds the first hostile, and a second one near it",
            Ability::Wisp => "orbiting ball lightning; press again to fling it",
            Ability::Discharge => "hold to charge a shock nova around you",
            Ability::Breach => "point-blank blast for blast-jumps",
            Ability::Gravewell => "pulls enemies together, then implodes",
            Ability::Tally => "hold to mark targets, release to shoot every one",
            Ability::Waxwork => "a wax double that draws the Moulds, then bursts",
            Ability::Bulwark => "hold up a wax slab that absorbs hits from the front",
            Ability::Flak => "a shell that airbursts into a ring of pellets",
            Ability::Stoke => "faster fire that burns, then the weapon overheats",
            Ability::Lantern => "a lantern that shoots the nearest hostile for you",
            Ability::Limpet => "sticky charges; press again to detonate",
            Ability::Kite => "an ember kite you steer with your crosshair",
            Ability::BlinkArrow => "an arrow you can step to",
            Ability::PinningArrow => "pins the target in place",
            Ability::BeaconArrow => "reveals what is near it; they take more damage",
            Ability::Lunge => "dash forward: the first target takes a heavy hit",
            Ability::Whirl => "spin: hits everything around you, knocks it away",
            Ability::Hurl => "throws a blade or hammer-head that comes back",
            Ability::Riposte => "hold to guard; release to counter what you blocked",
        }
    }

    /// Whether a weapon can take this ability.
    pub fn fits(self, weapon: WeaponKind) -> bool {
        weapon.abilities().contains(&self)
    }
}

/// The utility on the Q slot (`Buttons::THROW`, the KNACK control), one per loadout. The
/// discriminant is the wire id (4 bits; append only). What each does: `sim::knacks`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[repr(u8)]
pub enum Knack {
    /// The relay spike: every weapon's shots and blows relay off it (PvE only).
    #[default]
    Thorn = 0,
    /// A flask you drink: haste and healing over a few seconds.
    KindleFlask = 1,
    /// A wax trap: the first hostile to come close is pinned.
    Snare = 2,
    /// A cloud of soot the Moulds can't see through.
    SootPot = 3,
    /// A few seconds of half damage and no knockback.
    TallowSkin = 4,
    /// A burst of speed along your look.
    Surge = 5,
}

impl Knack {
    pub const ALL: [Knack; 6] = [
        Knack::Thorn,
        Knack::KindleFlask,
        Knack::Snare,
        Knack::SootPot,
        Knack::TallowSkin,
        Knack::Surge,
    ];
    pub const COUNT: usize = Self::ALL.len();
    /// What a knack that isn't [`Knack::pvp_allowed`] becomes in PvP.
    pub const PVP_FALLBACK: Knack = Knack::Surge;

    pub fn from_id(id: u8) -> Option<Knack> {
        Self::ALL.get(id as usize).copied()
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    /// Display name ("Kindle Flask").
    pub fn name(self) -> &'static str {
        match self {
            Knack::Thorn => "Thorn",
            Knack::KindleFlask => "Kindle Flask",
            Knack::Snare => "Snare",
            Knack::SootPot => "Soot Pot",
            Knack::TallowSkin => "Tallow Skin",
            Knack::Surge => "Surge",
        }
    }

    /// One-line description (menus).
    pub fn blurb(self) -> &'static str {
        match self {
            Knack::Thorn => "relay spike: your shots and blows relay off it",
            Knack::KindleFlask => "drink it: haste and healing for a few seconds",
            Knack::Snare => "wax trap: pins the first hostile that steps close",
            Knack::SootPot => "soot cloud: Moulds can't see into or through it",
            Knack::TallowSkin => "half damage and no knockback for a few seconds",
            Knack::Surge => "a burst of speed along your look",
        }
    }

    /// Usable in PvP matches. A knack that isn't is replaced by [`Knack::PVP_FALLBACK`] when
    /// the loadout applies in a PvP world (the player's request keeps their pick).
    pub fn pvp_allowed(self) -> bool {
        !matches!(self, Knack::Thorn)
    }

    /// This knack as it applies in a PvP (`pvp`) or PvE world.
    pub fn for_world(self, pvp: bool) -> Knack {
        if pvp && !self.pvp_allowed() {
            Self::PVP_FALLBACK
        } else {
            self
        }
    }
}

/// A weapon with its two chosen abilities (alt A, alt B).
///
/// Read from a human-readable format (RON: settings, tuning files), an ability name this
/// build doesn't know (one the rework retired, `"Splinter"`) becomes the weapon's default for
/// that slot instead of failing the whole file; `sanitized` then repairs the rest. The wire
/// (bincode) keeps the plain derived encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct WeaponBuild {
    pub weapon: WeaponKind,
    pub abilities: [Ability; BUILD_ABILITIES],
}

impl<'de> Deserialize<'de> for WeaponBuild {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename = "WeaponBuild")]
        struct Exact {
            weapon: WeaponKind,
            abilities: [Ability; BUILD_ABILITIES],
        }
        #[derive(Deserialize)]
        #[serde(rename = "WeaponBuild")]
        struct Lenient {
            weapon: WeaponKind,
            abilities: [LenientAbility; BUILD_ABILITIES],
        }
        if !d.is_human_readable() {
            let Exact { weapon, abilities } = Exact::deserialize(d)?;
            return Ok(WeaponBuild { weapon, abilities });
        }
        let Lenient { weapon, abilities } = Lenient::deserialize(d)?;
        let default = weapon.default_build().abilities;
        Ok(WeaponBuild {
            weapon,
            abilities: [0, 1].map(|i| abilities[i].0.unwrap_or(default[i])),
        })
    }
}

/// An [`Ability`] read by name, `None` for a name this build doesn't know.
struct LenientAbility(Option<Ability>);

impl<'de> Deserialize<'de> for LenientAbility {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = LenientAbility;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an ability name")
            }

            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<Self::Value, E> {
                // The variant names (`format!("{a:?}")`: `PinningArrow`).
                let known = Ability::ALL.into_iter().find(|a| format!("{a:?}") == name);
                Ok(LenientAbility(known))
            }

            fn visit_enum<A: serde::de::EnumAccess<'de>>(
                self,
                data: A,
            ) -> Result<Self::Value, A::Error> {
                use serde::de::VariantAccess;
                let (Ident(name), variant) = data.variant()?;
                variant.unit_variant()?;
                self.visit_str(&name)
            }
        }
        /// A variant name, read as an identifier (any name).
        struct Ident(String);
        impl<'de> Deserialize<'de> for Ident {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Name;
                impl serde::de::Visitor<'_> for Name {
                    type Value = Ident;

                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("an ability name")
                    }

                    fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<Ident, E> {
                        Ok(Ident(name.to_string()))
                    }
                }
                d.deserialize_identifier(Name)
            }
        }
        // The variant list is only a hint (the visitor accepts any name).
        d.deserialize_enum("Ability", &[], Visit)
    }
}

impl Default for WeaponBuild {
    fn default() -> Self {
        WeaponKind::Quill.default_build()
    }
}

impl WeaponBuild {
    /// Ability of alt mode `mode` (0 = A, 1 = B; anything else → A).
    pub fn ability(&self, mode: u8) -> Ability {
        self.abilities
            .get(mode as usize)
            .copied()
            .unwrap_or(self.abilities[0])
    }

    /// Both abilities fit the weapon and differ.
    pub fn is_valid(&self) -> bool {
        self.abilities.iter().all(|a| a.fits(self.weapon)) && self.abilities[0] != self.abilities[1]
    }

    /// This build with anything invalid replaced by the weapon's defaults.
    pub fn sanitized(self) -> WeaponBuild {
        if self.is_valid() {
            return self;
        }
        let default = self.weapon.default_build();
        let first = if self.abilities[0].fits(self.weapon) {
            self.abilities[0]
        } else {
            default.abilities[0]
        };
        let second = [
            self.abilities[1],
            default.abilities[1],
            default.abilities[0],
        ]
        .into_iter()
        .chain(self.weapon.abilities().iter().copied())
        .find(|a| a.fits(self.weapon) && *a != first)
        .unwrap_or(default.abilities[1]);
        WeaponBuild {
            weapon: self.weapon,
            abilities: [first, second],
        }
    }
}

/// The weapons a player carries, one build per slot key, and the knack on Q. Empty slots are
/// skipped by weapon switching. A valid loadout has at least one weapon and no weapon twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Loadout {
    pub slots: [Option<WeaponBuild>; LOADOUT_SLOTS],
    /// The Q slot (presets saved before knacks existed load with the Thorn).
    #[serde(default)]
    pub knack: Knack,
}

impl Default for Loadout {
    /// Quill, Bellows, Arclash, Tumbler with their default builds, and the Thorn.
    fn default() -> Self {
        Self {
            slots: [
                Some(WeaponKind::Quill.default_build()),
                Some(WeaponKind::Bellows.default_build()),
                Some(WeaponKind::Arclash.default_build()),
                Some(WeaponKind::Tumbler.default_build()),
            ],
            knack: Knack::default(),
        }
    }
}

impl Loadout {
    /// One weapon in slot 1, nothing else, the default knack (gun game: see
    /// [`Loadout::with_knack`]).
    pub fn single(build: WeaponBuild) -> Self {
        Self {
            slots: [Some(build), None, None, None],
            knack: Knack::default(),
        }
    }

    /// No weapon at all: fists only. Only a game mode can hand this out
    /// (`SimWorld::force_loadout`); requests sanitise it into the default weapons.
    pub fn unarmed() -> Self {
        Self {
            slots: [None; LOADOUT_SLOTS],
            knack: Knack::default(),
        }
    }

    /// The same weapons with `knack` on Q.
    pub fn with_knack(self, knack: Knack) -> Self {
        Self { knack, ..self }
    }

    /// Carries no weapon ([`Loadout::unarmed`]).
    pub fn is_unarmed(&self) -> bool {
        self.first_slot().is_none()
    }

    pub fn slot(&self, slot: u8) -> Option<&WeaponBuild> {
        self.slots.get(slot as usize).and_then(Option::as_ref)
    }

    /// The first slot holding a weapon.
    pub fn first_slot(&self) -> Option<u8> {
        self.slots.iter().position(Option::is_some).map(|i| i as u8)
    }

    /// The slot holding `weapon`, if any.
    pub fn slot_of(&self, weapon: WeaponKind) -> Option<u8> {
        self.slots
            .iter()
            .position(|s| s.is_some_and(|b| b.weapon == weapon))
            .map(|i| i as u8)
    }

    pub fn builds(&self) -> impl Iterator<Item = &WeaponBuild> {
        self.slots.iter().flatten()
    }

    /// At least one weapon, every build valid, no weapon twice.
    pub fn is_valid(&self) -> bool {
        let mut seen = [false; WeaponKind::COUNT];
        let mut any = false;
        for b in self.builds() {
            if !b.is_valid() || std::mem::replace(&mut seen[b.weapon as usize], true) {
                return false;
            }
            any = true;
        }
        any
    }

    /// This loadout made valid: invalid builds fixed, repeated weapons dropped, and the default
    /// weapons if nothing is left (the knack is kept: every knack is valid).
    pub fn sanitized(self) -> Loadout {
        let mut seen = [false; WeaponKind::COUNT];
        let mut out = Loadout::unarmed().with_knack(self.knack);
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(b) = slot
                && !std::mem::replace(&mut seen[b.weapon as usize], true)
            {
                out.slots[i] = Some(b.sanitized());
            }
        }
        if out.first_slot().is_none() {
            return Loadout::default().with_knack(self.knack);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_never_move() {
        for (i, w) in WeaponKind::ALL.iter().enumerate() {
            assert_eq!(w.id() as usize, i);
            assert_eq!(WeaponKind::from_id(w.id()), Some(*w));
        }
        for (i, a) in Ability::ALL.iter().enumerate() {
            assert_eq!(a.id() as usize, i);
            assert_eq!(Ability::from_id(a.id()), Some(*a));
        }
        for (i, k) in Knack::ALL.iter().enumerate() {
            assert_eq!(k.id() as usize, i);
            assert_eq!(Knack::from_id(k.id()), Some(*k));
        }
        assert_eq!(WeaponKind::from_id(WeaponKind::COUNT as u8), None);
        assert_eq!(Ability::from_id(Ability::COUNT as u8), None);
        assert_eq!(Knack::from_id(Knack::COUNT as u8), None);
        // Wire ids of the second-generation set and the refit (docs/refit.md) are fixed.
        assert_eq!(WeaponKind::Tumbler.id(), 3);
        assert_eq!(Ability::Gravewell.id(), 7);
        assert_eq!(
            (WeaponKind::Trimmer.id(), WeaponKind::Tamper.id()),
            (10, 11)
        );
        assert_eq!((Ability::Lunge.id(), Ability::Hurl.id()), (19, 21));
        assert_eq!((Knack::Thorn.id(), Knack::Surge.id()), (0, 5));
        // The rework (docs/rework.md): new abilities on the retired ids, the sniper is 12.
        let ids = [
            (Ability::Wickline, 1),
            (Ability::Bolas, 3),
            (Ability::Tally, 8),
            (Ability::Waxwork, 9),
            (Ability::Bulwark, 10),
            (Ability::Flak, 11),
            (Ability::Stoke, 12),
            (Ability::Lantern, 13),
            (Ability::Limpet, 14),
            (Ability::Kite, 15),
            (Ability::BlinkArrow, 16),
            (Ability::PinningArrow, 17),
            (Ability::BeaconArrow, 18),
            (Ability::Riposte, 22),
        ];
        for (a, id) in ids {
            assert_eq!(a.id(), id, "{a:?}");
        }
        assert_eq!(WeaponKind::Lamplighter.id(), 12);
        // Weapon ids stay below the unarmed marker, knack ids fit 4 bits.
        const { assert!(WeaponKind::COUNT < 15 && Knack::COUNT <= 16) };
    }

    #[test]
    fn every_class_offers_four_abilities_and_every_ability_has_a_home() {
        for class in WeaponClass::ALL {
            assert_eq!(class.abilities().len(), 4, "{class:?}");
        }
        for a in Ability::ALL {
            assert!(
                WeaponKind::ALL.iter().any(|w| a.fits(*w)),
                "{a:?} fits no weapon"
            );
        }
    }

    #[test]
    fn default_builds_and_loadout_are_valid() {
        for w in WeaponKind::ALL {
            assert!(w.default_build().is_valid(), "{w:?}");
        }
        let l = Loadout::default();
        assert!(l.is_valid());
        assert_eq!(l.slot(0).map(|b| b.weapon), Some(WeaponKind::Quill));
        assert_eq!(l.slot_of(WeaponKind::Tumbler), Some(3));
    }

    #[test]
    fn knacks_default_to_the_thorn_and_fall_back_to_surge_in_pvp() {
        assert_eq!(Loadout::default().knack, Knack::Thorn);
        assert!(!Knack::Thorn.pvp_allowed());
        assert_eq!(Knack::Thorn.for_world(true), Knack::Surge);
        assert_eq!(Knack::Thorn.for_world(false), Knack::Thorn);
        for k in Knack::ALL.into_iter().filter(|k| k.pvp_allowed()) {
            assert_eq!(k.for_world(true), k);
        }
        assert!(Knack::PVP_FALLBACK.pvp_allowed());
        // Presets saved before knacks existed still load (with the Thorn).
        let old =
            "(slots: (Some((weapon: Kiln, abilities: (Lantern, Discharge))), None, None, None))";
        let l: Loadout = ron::from_str(old).unwrap();
        assert_eq!(l.knack, Knack::Thorn);
        assert_eq!(l.slot(0).map(|b| b.weapon), Some(WeaponKind::Kiln));
    }

    #[test]
    fn retired_ability_names_load_as_the_weapons_defaults() {
        // A loadout saved before the rework: Splinter, Flare, Mortar, Upswing are gone.
        let old = "(slots: (Some((weapon: Quill, abilities: (Brand, Splinter))), \
                   Some((weapon: Kiln, abilities: (Flare, Discharge))), \
                   Some((weapon: Cinderbox, abilities: (Mortar, Limpet))), \
                   Some((weapon: Tamper, abilities: (Upswing, Whirl)))), knack: Snare)";
        let l: Loadout = ron::from_str(old).unwrap();
        assert_eq!(l.knack, Knack::Snare);
        let builds: Vec<WeaponBuild> = l.builds().copied().collect();
        // Each unknown name becomes that slot's default (Quill B: Wickline; Kiln A: Lantern;
        // Cinderbox A: Limpet, which repeats B: `sanitized` fixes it).
        assert_eq!(builds[0].abilities, [Ability::Brand, Ability::Wickline]);
        assert_eq!(builds[1].abilities, [Ability::Lantern, Ability::Discharge]);
        assert_eq!(builds[2].abilities, [Ability::Limpet, Ability::Limpet]);
        assert_eq!(builds[3].abilities, [Ability::Whirl, Ability::Whirl]);
        let fixed = l.sanitized();
        assert!(fixed.is_valid(), "{fixed:?}");
        assert_eq!(
            fixed.slot(2).map(|b| b.abilities),
            Some([Ability::Limpet, Ability::Kite])
        );
        // Unknown weapons still fail (a typo there is the settings' text format's business).
        assert!(
            ron::from_str::<WeaponBuild>("(weapon: Railgun, abilities: (Brand, Brand))").is_err()
        );
        // The wire keeps the exact encoding.
        let b = WeaponKind::Lamplighter.default_build();
        let bytes = crate::protocol::encode(&b);
        let back: WeaponBuild = crate::protocol::decode_server_message(&bytes).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn sanitizing_fixes_bad_builds_and_loadouts() {
        // An ability from another class and a repeated ability.
        let bad = WeaponBuild {
            weapon: WeaponKind::Longwick,
            abilities: [Ability::Gust, Ability::Gust],
        };
        assert!(!bad.is_valid());
        let fixed = bad.sanitized();
        assert!(fixed.is_valid(), "{fixed:?}");
        let dup = WeaponBuild {
            weapon: WeaponKind::Quill,
            abilities: [Ability::Tally, Ability::Tally],
        };
        let fixed = dup.sanitized();
        assert!(fixed.is_valid());
        assert_eq!(fixed.abilities[0], Ability::Tally);

        // Repeated weapon: the second copy is dropped. Empty: back to the default.
        let l = Loadout {
            slots: [
                Some(WeaponKind::Kiln.default_build()),
                Some(WeaponKind::Kiln.default_build()),
                None,
                Some(bad),
            ],
            knack: Knack::SootPot,
        };
        assert!(!l.is_valid());
        let s = l.sanitized();
        assert!(s.is_valid());
        assert_eq!(s.knack, Knack::SootPot);
        assert_eq!(s.slots[1], None);
        assert_eq!(s.slot(3).map(|b| b.weapon), Some(WeaponKind::Longwick));
        let empty = Loadout {
            slots: [None; 4],
            knack: Knack::Snare,
        };
        assert!(!empty.is_valid());
        assert_eq!(
            empty.sanitized(),
            Loadout::default().with_knack(Knack::Snare)
        );
        assert_eq!(empty.with_knack(Knack::Thorn), Loadout::unarmed());
        assert!(Loadout::unarmed().is_unarmed() && !Loadout::default().is_unarmed());
        let single = Loadout::single(WeaponKind::Cinderbox.default_build());
        assert!(single.is_valid());
        assert_eq!(single.first_slot(), Some(0));
        assert_eq!(single.slot(1), None);
    }
}
