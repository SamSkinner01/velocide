//! Weapons: the per-player weapon state machine and what each primary and ability does.
//!
//! Two halves:
//!
//! * [`step_weapons`] — a **pure, deterministic** function of `(WeaponState, PlayerInput,
//!   alive, tuning)` that decides *what* fires this input (cooldowns, charges, the Longwick's
//!   draw and the Lamplighter's focus, salvos, held and live abilities, the Stoke, punch, knack
//!   charges) and returns [`FireIntent`]s. All timers are integer ticks, so the owning client
//!   can run the same function on its predicted inputs (muzzle flash, sound and viewmodel recoil
//!   without waiting a round trip) and compare its prediction bit-exactly with the
//!   authoritative `OwnState::combat.weapons`.
//! * [`SimWorld::resolve_intent`] (server) — turns an intent into hitscan traces (`hitscan.rs`),
//!   area attacks (`area.rs`), projectiles (`projectiles.rs`), melee swings (`melee.rs`), the
//!   rework's abilities (`abilities/`, one file each), a punch/parry or a knack (`knacks.rs`;
//!   the Thorn: `thorn.rs`).
//!
//! The player carries a [`Loadout`] (`WeaponState::loadout`, set by the server: `loadouts.rs`):
//! up to four [`WeaponBuild`]s on the slot keys. `PlayerInput::weapon` is the slot (an empty
//! slot or an out-of-range value keeps the current weapon); `PlayerInput::weapon_mode` 0 / 1
//! picks the build's ability A / B (others → A). `FIRE` (held) = primary: automatic, a salvo
//! (Cinderbox), hold-to-draw / release (Longwick) or hold-to-focus / release (Lamplighter);
//! `ALT_FIRE` = the selected ability ([`AbilityStyle`]: held and automatic, hold-to-charge /
//! release, tap / hold, held for a while (a channel), press-only; a live ability answers a
//! second press; on the bow it arms the next release); `PUNCH` (press) = fist/parry; `THROW`
//! (press, the KNACK control) = the loadout's [`Knack`] while it has a charge.
//!
//! Some abilities' cooldowns depend on what happened on the server (a Wickline that missed, a
//! Tally with nothing tallied, a shattered Bulwark, a Waxwork destroyed early, a caught Hurl):
//! the server changes the owner's `WeaponState` then, and their client re-bases on it like on
//! any reconciliation.

use std::borrow::Cow;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::abilities::AbilityCast;
pub use super::abilities::AbilityPhase;
use super::combat::{Attack, DamageFlags, Owner, secs_to_ticks};
use super::hitscan::ShotSpec;
use super::projectiles::ProjectileKind;
use super::{HitboxHistory, PlayerId, SimEvent, SimWorld};
use crate::input::{Buttons, PlayerInput};
pub use crate::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponClass, WeaponKind};
use crate::loadout::{BUILD_ABILITIES, LOADOUT_SLOTS};
use crate::tuning::{BalanceScale, CombatTuning, Tuning};

/// Number of alt-fire modes (abilities) per weapon.
pub const ALT_MODES: u8 = BUILD_ABILITIES as u8;

/// How a weapon's primary fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimaryStyle {
    /// Automatic while held, one shot per interval.
    Auto,
    /// One trigger pull fires a salvo over several ticks (Cinderbox).
    Salvo,
    /// Hold to draw, release to loose (Longwick).
    Draw,
    /// Hold to focus, release to fire one hitscan beam (Lamplighter): the bow's draw, as a ray
    /// (`WeaponState::draw` holds the focus).
    Focus,
}

/// How an ability fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbilityStyle {
    /// Automatic while held, once per cooldown.
    Instant,
    /// Hold to charge (once off cooldown), fires on release (Discharge).
    Charge,
    /// Tap or hold (Limpet): released before the hold time it fires with `charge` 0 (throw),
    /// held to it with `charge` 1 (detonate).
    Hold,
    /// Held for a while (Tally, Bulwark, Riposte): the press (off cooldown) is its use, holding
    /// keeps it up for at most its `charge_time` (`WeaponState::charge` counts), letting go
    /// (or the limit, or switching away) is its release (`AbilityPhase::Release`); the
    /// cooldown starts then.
    Channel,
    /// Fires on the press only, once per cooldown (Kite: holding it steers; the Wisp: a
    /// second press while it is live flings it).
    Press,
}

impl WeaponKind {
    /// The attack of this weapon's primary fire.
    pub fn primary(self) -> Attack {
        Attack::primary(self)
    }

    pub fn primary_style(self) -> PrimaryStyle {
        match self {
            WeaponKind::Cinderbox => PrimaryStyle::Salvo,
            WeaponKind::Longwick => PrimaryStyle::Draw,
            WeaponKind::Lamplighter => PrimaryStyle::Focus,
            _ => PrimaryStyle::Auto,
        }
    }

    /// Seconds to a full draw (Longwick) or focus (Lamplighter); `None` for the others.
    pub fn draw_time(self, t: &CombatTuning) -> Option<f32> {
        match self {
            WeaponKind::Longwick => Some(t.longwick.draw_time),
            WeaponKind::Lamplighter => Some(t.lamplighter.focus_time),
            _ => None,
        }
    }

    /// Seconds between primary shots (salvos; the Longwick: after a release).
    pub fn primary_interval(self, t: &CombatTuning) -> f32 {
        match self {
            WeaponKind::Quill => t.quill.interval,
            WeaponKind::Bellows => t.bellows.interval,
            WeaponKind::Arclash => t.arclash.zap_interval,
            WeaponKind::Tumbler => t.tumbler.interval,
            WeaponKind::Lancet => t.lancet.interval,
            WeaponKind::Thresher => t.thresher.interval,
            WeaponKind::Riveter => t.riveter.interval,
            WeaponKind::Cinderbox => t.cinderbox.interval,
            WeaponKind::Longwick => t.longwick.interval,
            WeaponKind::Kiln => t.kiln.interval,
            WeaponKind::Trimmer => t.trimmer.interval,
            WeaponKind::Tamper => t.tamper.interval,
            WeaponKind::Lamplighter => t.lamplighter.interval,
        }
    }

    /// Damage of the abilities it carries relative to their home weapon's (1.0 = as listed).
    pub fn ability_power(self, t: &CombatTuning) -> f32 {
        match self {
            WeaponKind::Quill => t.quill.ability_power,
            WeaponKind::Bellows => t.bellows.ability_power,
            WeaponKind::Arclash => t.arclash.ability_power,
            WeaponKind::Tumbler => t.tumbler.ability_power,
            WeaponKind::Lancet => t.lancet.ability_power,
            WeaponKind::Thresher => t.thresher.ability_power,
            WeaponKind::Riveter => t.riveter.ability_power,
            WeaponKind::Cinderbox => t.cinderbox.ability_power,
            WeaponKind::Longwick => t.longwick.ability_power,
            WeaponKind::Kiln => t.kiln.ability_power,
            WeaponKind::Trimmer => t.trimmer.ability_power,
            WeaponKind::Tamper => t.tamper.ability_power,
            WeaponKind::Lamplighter => t.lamplighter.ability_power,
        }
    }

    /// Damage of one primary shot (a pellet, a zap, a rivet, a rocket, an un-drawn arrow, a
    /// swing, an un-focused beam).
    pub fn primary_damage(self, t: &CombatTuning) -> f32 {
        match self {
            WeaponKind::Quill => t.quill.damage,
            WeaponKind::Bellows => t.bellows.pellet_damage,
            WeaponKind::Arclash => t.arclash.zap_damage,
            WeaponKind::Tumbler => t.tumbler.bomb_damage,
            WeaponKind::Lancet => t.lancet.damage,
            WeaponKind::Thresher => t.thresher.pellet_damage,
            WeaponKind::Riveter => t.riveter.damage,
            WeaponKind::Cinderbox => t.cinderbox.damage,
            WeaponKind::Longwick => t.longwick.min_damage,
            WeaponKind::Kiln => t.kiln.damage,
            WeaponKind::Trimmer => t.trimmer.damage,
            WeaponKind::Tamper => t.tamper.damage,
            WeaponKind::Lamplighter => t.lamplighter.min_damage,
        }
    }
}

impl Ability {
    /// The attack this ability deals damage as.
    pub fn attack(self) -> Attack {
        Attack::ability(self)
    }

    /// The weapon whose numbers the ability's tuning is written for: the first weapon whose
    /// default build has it.
    pub fn home(self) -> WeaponKind {
        WeaponKind::ALL
            .into_iter()
            .find(|w| w.default_build().abilities.contains(&self))
            .or_else(|| WeaponKind::ALL.into_iter().find(|w| self.fits(*w)))
            .unwrap_or_default()
    }

    pub fn style(self) -> AbilityStyle {
        match self {
            Ability::Discharge => AbilityStyle::Charge,
            Ability::Limpet => AbilityStyle::Hold,
            Ability::Tally | Ability::Bulwark | Ability::Riposte => AbilityStyle::Channel,
            Ability::Kite | Ability::Wisp => AbilityStyle::Press,
            _ => AbilityStyle::Instant,
        }
    }

    /// Seconds before it can be used again (a live ability: from its end).
    pub fn cooldown(self, t: &CombatTuning) -> f32 {
        match self {
            Ability::Brand => t.quill.brand_cooldown,
            Ability::Wickline => t.wickline.cooldown,
            Ability::Gust => t.bellows.gust_cooldown,
            Ability::Bolas => t.bolas.cooldown,
            Ability::Wisp => t.arclash.wisp_cooldown,
            Ability::Discharge => t.arclash.discharge_cooldown,
            Ability::Breach => t.tumbler.breach_cooldown,
            Ability::Gravewell => t.tumbler.gravewell_cooldown,
            Ability::Tally => t.tally.cooldown,
            Ability::Waxwork => t.waxwork.cooldown,
            Ability::Bulwark => t.bulwark.cooldown,
            Ability::Flak => t.flak.cooldown,
            Ability::Stoke => t.stoke.cooldown,
            Ability::Lantern => t.lantern.cooldown,
            Ability::Limpet => t.limpet.cooldown,
            Ability::Kite => t.kite.cooldown,
            Ability::BlinkArrow => t.blink_arrow.cooldown,
            Ability::PinningArrow => t.pinning_arrow.cooldown,
            Ability::BeaconArrow => t.beacon_arrow.cooldown,
            Ability::Lunge => t.lunge.cooldown,
            Ability::Whirl => t.whirl.cooldown,
            Ability::Hurl => t.hurl.cooldown,
            Ability::Riposte => t.riposte.cooldown,
        }
    }

    /// Seconds to a full charge (Discharge), the Limpet's hold, the longest a held ability
    /// stays up (Tally, Bulwark, Riposte); 1 s for the others.
    pub fn charge_time(self, t: &CombatTuning) -> f32 {
        match self {
            Ability::Discharge => t.arclash.discharge_charge_time,
            Ability::Limpet => t.limpet.detonate_hold,
            Ability::Tally => t.tally.hold_time,
            Ability::Bulwark => t.bulwark.hold_time,
            Ability::Riposte => t.riposte.hold_time,
            _ => 1.0,
        }
    }

    /// Seconds a live ability lasts after its use (0: not live). While it is live
    /// (`WeaponState::live`) it can't be used again — a second press recasts it if it
    /// [`recasts`] —, the build's other ability stays usable on the shared cooldown, and its
    /// cooldown starts when it ends (the server may end it early). The Blink Arrow's is the
    /// arrow's whole flight plus its window (the server shortens it once the arrow sticks).
    ///
    /// [`recasts`]: Ability::recasts
    pub fn live_time(self, t: &CombatTuning) -> f32 {
        match self {
            Ability::Wisp => t.arclash.wisp_orbit_time,
            Ability::Waxwork => t.waxwork.duration,
            Ability::BlinkArrow => t.longwick.arrow_lifetime + t.blink_arrow.window,
            _ => 0.0,
        }
    }

    /// A second press while it is live is its [`AbilityPhase::Recast`] (the Wisp flung, the
    /// Blink Arrow's step).
    pub fn recasts(self) -> bool {
        matches!(self, Ability::Wisp | Ability::BlinkArrow)
    }

    /// The cooldown a recast starts (the flung Wisp's flight counts: its cooldown runs from
    /// when it is gone).
    pub fn recast_cooldown(self, t: &CombatTuning) -> f32 {
        match self {
            Ability::Wisp => t.arclash.wisp_lifetime + t.arclash.wisp_cooldown,
            _ => self.cooldown(t),
        }
    }

    /// Made of the carrying weapon's own shots (Stoke's faster fire, the Lantern's copies):
    /// their damage comes from its primary, so `ability_power` doesn't scale them again.
    pub fn uses_primary(self) -> bool {
        matches!(self, Ability::Stoke | Ability::Lantern)
    }

    /// One of the rework's abilities (`abilities/`; the Wisp's twist included): resolved by
    /// [`SimWorld::cast_ability`].
    pub fn is_rework(self) -> bool {
        matches!(
            self,
            Ability::Wickline
                | Ability::Tally
                | Ability::Waxwork
                | Ability::Bolas
                | Ability::Bulwark
                | Ability::Wisp
                | Ability::Stoke
                | Ability::Lantern
                | Ability::Kite
                | Ability::BlinkArrow
                | Ability::BeaconArrow
                | Ability::Riposte
        )
    }
}

/// A salvo in progress: rockets still to fire over the next ticks (the Cinderbox). Switching
/// weapon, dying or a new loadout cancels it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Burst {
    /// Shots left (0 = no burst).
    pub left: u8,
    /// Ticks to the next one.
    pub next: u16,
}

/// Weapons, fist and knack state of one player. Everything is in ticks/integers so client
/// prediction can reproduce it exactly. `Default` = fresh with the default loadout (all
/// ready, every knack charge).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeaponState {
    /// The weapons carried, one build per slot key, and the knack on Q. Only the server
    /// changes it (`SimWorld::request_loadout` / `force_loadout`, applied at (re)spawn; the
    /// knack as it applies in this world: `SimWorld::fit_loadout`). Packed into one integer on
    /// the wire (it rides along every `OwnState`).
    #[serde(with = "packed_loadout")]
    pub loadout: Loadout,
    /// Selected slot (index into `loadout`; a slot holding a weapon).
    pub slot: u8,
    /// Current alt mode (0 = ability A, 1 = ability B).
    pub mode: u8,
    /// Buttons of the previous input (edge detection, independent of movement's).
    pub prev_buttons: Buttons,
    /// Ticks until each slot's primary can fire again.
    pub primary_cooldown: [u16; LOADOUT_SLOTS],
    /// Ticks until each slot's abilities can fire again (a build's two abilities share it).
    pub alt_cooldown: [u16; LOADOUT_SLOTS],
    /// Ticks the alt button has been held: the Discharge's charge, the Limpet's hold, a held
    /// ability (Tally, Bulwark, Riposte) while it is up (0 = none).
    pub charge: u16,
    /// Longwick: ticks the primary has been drawn; Lamplighter: focused (0 = not drawing).
    pub draw: u16,
    /// Longwick: the ability armed for the next release (0 = none, 1 = A, 2 = B).
    pub armed: u8,
    /// Lancet bloom: heat ticks (each shot adds some, one drains per tick).
    pub bloom: u16,
    pub burst: Burst,
    pub punch_cooldown: u16,
    /// Ticks left in the parry window opened by the last punch.
    pub parry: u16,
    /// Knack charges spent and not yet recharged (charges = the knack's `charges` −
    /// `knack_used`, [`WeaponState::knack_charges`]).
    pub knack_used: u8,
    /// Ticks accumulated towards the next knack charge.
    pub knack_recharge: u16,
    // ---- rework (docs/rework.md) ----
    /// Per slot: ticks a live ability ([`Ability::live_time`]) has left (0 = none); while it
    /// lasts it can only be recast, and its cooldown starts when it ends.
    pub live: [u16; LOADOUT_SLOTS],
    /// Per slot: the mode (0 / 1) whose ability is live.
    pub live_mode: [u8; LOADOUT_SLOTS],
    /// Per slot: Stoke ticks left (the primary at `stoke.rate` × its rate) ...
    pub stoke: [u16; LOADOUT_SLOTS],
    /// ... then overheated for this many (no primary).
    pub overheat: [u16; LOADOUT_SLOTS],
}

/// A [`Loadout`] as one `u64`: 16 bits per slot (slot 0 lowest): weapon id + 1 (bits 0–4,
/// 0 = empty), ability A (5–9), ability B (10–14), and one bit of the knack id (15; slot 0
/// holds its lowest bit, empty slots carry theirs too). Lossless for every valid loadout;
/// unknown ids decode as an empty slot / the default ability / the default knack.
mod packed_loadout {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponKind};

    pub fn pack(loadout: &Loadout) -> u64 {
        let mut word = 0u64;
        for (i, slot) in loadout.slots.iter().enumerate() {
            let mut bits = (loadout.knack.id() as u64 >> i & 1) << 15;
            if let Some(b) = slot {
                bits |= (b.weapon.id() as u64 + 1) & 0x1F
                    | (b.abilities[0].id() as u64 & 0x1F) << 5
                    | (b.abilities[1].id() as u64 & 0x1F) << 10;
            }
            word |= bits << (16 * i);
        }
        word
    }

    pub fn unpack(word: u64) -> Loadout {
        let knack = (0..4).fold(0u8, |id, i| id | ((word >> (16 * i + 15) & 1) as u8) << i);
        let mut loadout = Loadout::unarmed().with_knack(Knack::from_id(knack).unwrap_or_default());
        for (i, slot) in loadout.slots.iter_mut().enumerate() {
            let bits = word >> (16 * i);
            let weapon = ((bits & 0x1F) as u8)
                .checked_sub(1)
                .and_then(WeaponKind::from_id);
            *slot = weapon.map(|weapon| WeaponBuild {
                weapon,
                abilities: [5, 10].map(|shift| {
                    Ability::from_id((bits >> shift & 0x1F) as u8).unwrap_or_default()
                }),
            });
        }
        loadout
    }

    pub fn serialize<S: Serializer>(loadout: &Loadout, s: S) -> Result<S::Ok, S::Error> {
        pack(loadout).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Loadout, D::Error> {
        Ok(unpack(u64::deserialize(d)?))
    }
}

/// Something a player's input fires this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FireIntent {
    /// The held weapon's primary fire. `charge` (0..=1): the Longwick's draw, the Lancet's
    /// bloom; 0 for the others.
    Primary {
        weapon: WeaponKind,
        charge: f32,
    },
    /// An ability of the held weapon. `charge` (0..=1): the Discharge charge, the Longwick's
    /// draw for an armed arrow, how long a held ability was held (its release); 0 for the
    /// others. `phase`: its use, a held ability's release or a live one's recast.
    Ability {
        weapon: WeaponKind,
        ability: Ability,
        charge: f32,
        phase: AbilityPhase,
    },
    Punch,
    /// The loadout's knack (a charge was spent).
    Knack(Knack),
}

impl FireIntent {
    /// The attack it fires (`None` for knacks: what they do later may deal damage, the use
    /// itself doesn't).
    pub fn attack(self) -> Option<Attack> {
        match self {
            FireIntent::Primary { weapon, .. } => Some(weapon.primary()),
            FireIntent::Ability { ability, .. } => Some(ability.attack()),
            FireIntent::Punch => Some(Attack::Fist),
            FireIntent::Knack(_) => None,
        }
    }

    /// The weapon that fires it (`None` for the fist and knacks).
    pub fn weapon(self) -> Option<WeaponKind> {
        match self {
            FireIntent::Primary { weapon, .. } | FireIntent::Ability { weapon, .. } => Some(weapon),
            FireIntent::Punch | FireIntent::Knack(_) => None,
        }
    }
}

impl WeaponState {
    /// Fresh state carrying `loadout`, its first weapon selected.
    pub fn new(loadout: Loadout) -> Self {
        Self {
            loadout,
            slot: loadout.first_slot().unwrap_or(0),
            ..Default::default()
        }
    }

    /// Fresh state for a (re)spawn with `loadout`, keeping the selected slot (if it holds a
    /// weapon in `loadout`) and mode.
    pub fn respawned(&self, loadout: Loadout) -> Self {
        Self {
            slot: self.slot_in(&loadout),
            mode: self.mode,
            ..Self::new(loadout)
        }
    }

    /// The weapons replaced by `loadout` now (warm-up, dead, forced): every weapon's state
    /// starts afresh; fist, parry window, selection, edge detection and the knack's charges
    /// (while it is the same knack; a new one starts full) carry on.
    pub fn with_loadout(&self, loadout: Loadout) -> Self {
        let (knack_used, knack_recharge) = if loadout.knack == self.loadout.knack {
            (self.knack_used, self.knack_recharge)
        } else {
            (0, 0)
        };
        Self {
            loadout,
            slot: self.slot_in(&loadout),
            mode: self.mode,
            prev_buttons: self.prev_buttons,
            punch_cooldown: self.punch_cooldown,
            parry: self.parry,
            knack_used,
            knack_recharge,
            ..Default::default()
        }
    }

    /// The current slot if `loadout` has a weapon there, else its first weapon's slot.
    fn slot_in(&self, loadout: &Loadout) -> u8 {
        if loadout.slot(self.slot).is_some() {
            self.slot
        } else {
            loadout.first_slot().unwrap_or(0)
        }
    }

    /// The build in hand; `None` when unarmed (fists only: `Loadout::unarmed`).
    pub fn held(&self) -> Option<WeaponBuild> {
        self.loadout
            .slot(self.slot)
            .or_else(|| self.loadout.builds().next())
            .copied()
    }

    /// Carries no weapon: the primary and abilities fire nothing, only the fist (and the
    /// knack).
    pub fn is_unarmed(&self) -> bool {
        self.loadout.is_unarmed()
    }

    /// The selected build (unarmed: the default build — check [`WeaponState::held`] or
    /// [`WeaponState::is_unarmed`] where it matters).
    pub fn build(&self) -> WeaponBuild {
        self.held().unwrap_or_default()
    }

    /// The weapon in hand (unarmed: the default, see [`WeaponState::build`]).
    pub fn current(&self) -> WeaponKind {
        self.build().weapon
    }

    /// The selected ability.
    pub fn ability(&self) -> Ability {
        self.build().ability(self.mode)
    }

    /// The knack on Q.
    pub fn knack(&self) -> Knack {
        self.loadout.knack
    }

    /// Knack charges available.
    pub fn knack_charges(&self, t: &CombatTuning) -> u32 {
        self.knack()
            .charges(t)
            .saturating_sub(self.knack_used as u32)
    }

    /// Progress (0..1) towards the next knack charge (0 while every charge is there).
    pub fn knack_recharge_fraction(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        let full = secs_to_ticks(self.knack().recharge_time(t), tick_rate, 1);
        (self.knack_recharge as f32 / full as f32).min(1.0)
    }

    /// Ticks to a full charge of the selected ability.
    pub fn charge_ticks(&self, t: &CombatTuning, tick_rate: u32) -> u16 {
        secs_to_ticks(self.ability().charge_time(t), tick_rate, 1)
    }

    /// Charge fraction (0..=1) of the selected ability, for display.
    pub fn charge_fraction(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        (self.charge as f32 / self.charge_ticks(t, tick_rate) as f32).min(1.0)
    }

    /// Ticks to a full Longwick draw.
    pub fn draw_ticks(t: &CombatTuning, tick_rate: u32) -> u16 {
        Self::draw_ticks_of(WeaponKind::Longwick, t, tick_rate)
    }

    /// Ticks to a full draw (Longwick) / focus (Lamplighter) of `weapon` (1 for the others).
    pub fn draw_ticks_of(weapon: WeaponKind, t: &CombatTuning, tick_rate: u32) -> u16 {
        secs_to_ticks(weapon.draw_time(t).unwrap_or(0.0), tick_rate, 1)
    }

    /// Draw (Longwick) / focus (Lamplighter) fraction of the weapon in hand (0..=1).
    pub fn draw_fraction(&self, t: &CombatTuning, tick_rate: u32) -> f32 {
        let full = Self::draw_ticks_of(self.current(), t, tick_rate);
        (self.draw as f32 / full as f32).min(1.0)
    }

    /// The selected slot's weapon is stoked (`stoke` ticks left).
    pub fn is_stoked(&self) -> bool {
        self.stoke.get(self.slot as usize).is_some_and(|s| *s > 0)
    }

    /// The selected slot's weapon is overheated after a Stoke (no primary).
    pub fn is_overheated(&self) -> bool {
        self.overheat
            .get(self.slot as usize)
            .is_some_and(|s| *s > 0)
    }

    /// The live ability of `slot` (`None` while nothing is live there).
    pub fn live_ability(&self, slot: u8) -> Option<Ability> {
        let i = slot as usize;
        if *self.live.get(i)? == 0 {
            return None;
        }
        Some(self.loadout.slot(slot)?.ability(self.live_mode[i]))
    }

    /// Server side: ends the live ability of `slot` now (a Waxwork destroyed, a wisp gone):
    /// its cooldown starts. The owner's client re-bases on the change.
    pub fn end_live(&mut self, slot: u8, t: &CombatTuning, tick_rate: u32) {
        if let Some(ability) = self.live_ability(slot) {
            let i = slot as usize;
            self.live[i] = 0;
            let cd = secs_to_ticks(ability.cooldown(t), tick_rate, 1);
            self.alt_cooldown[i] = self.alt_cooldown[i].max(cd);
        }
    }

    /// Server side: shortens the live ability of `slot` to at most `secs` more (the Blink
    /// Arrow stuck: its window).
    pub fn shorten_live(&mut self, slot: u8, secs: f32, tick_rate: u32) {
        if let Some(live) = self.live.get_mut(slot as usize)
            && *live > 0
        {
            *live = (*live).min(secs_to_ticks(secs, tick_rate, 1));
        }
    }

    /// Server side: the selected slot's ability cooldown becomes `secs` (a refund or a longer
    /// one: a Wickline that missed, a Tally with nothing tallied, a caught Hurl).
    pub fn set_alt_cooldown(&mut self, slot: u8, secs: f32, tick_rate: u32) {
        if let Some(cd) = self.alt_cooldown.get_mut(slot as usize) {
            *cd = secs_to_ticks(secs, tick_rate, 0);
        }
    }

    /// The ability armed for the bow's next release.
    pub fn armed_ability(&self) -> Option<Ability> {
        let mode = self.armed.checked_sub(1)?;
        Some(self.build().ability(mode))
    }

    /// Replicated `state` of the player entity: held weapon, ability and charge level, or the
    /// unarmed marker ([`crate::protocol::player_armed`]).
    pub fn replicated_state(&self, t: &CombatTuning, tick_rate: u32) -> u16 {
        if self.is_unarmed() {
            return crate::protocol::pack_player_unarmed();
        }
        crate::protocol::pack_player_weapon(
            self.current(),
            self.ability(),
            self.charge_level(t, tick_rate),
        )
    }

    /// Replicated charge level 0..=255: the draw / focus while drawing, else the ability
    /// charge (a held ability: how long it has been up).
    pub fn charge_level(&self, t: &CombatTuning, tick_rate: u32) -> u8 {
        let f = if self.draw > 0 {
            self.draw_fraction(t, tick_rate)
        } else {
            self.charge_fraction(t, tick_rate)
        };
        (f * 255.0).round() as u8
    }

    /// Drops everything in progress (charge, draw, arming, salvo).
    fn cancel(&mut self) {
        self.charge = 0;
        self.draw = 0;
        self.armed = 0;
        self.burst = Burst::default();
    }

    pub fn on_death(&mut self) {
        self.cancel();
        self.parry = 0;
    }
}

/// Lancet heat per shot and the cap (ticks).
fn bloom_ticks(t: &CombatTuning, tick_rate: u32) -> (u16, u16) {
    (
        secs_to_ticks(t.lancet.bloom_per_shot, tick_rate, 0),
        secs_to_ticks(t.lancet.bloom_max, tick_rate, 1),
    )
}

/// Advances one player's weapons by one input and returns what fires. Deterministic: the
/// server and the owning client's prediction run exactly this. `alive == false` only ticks
/// the timers.
pub fn step_weapons(
    s: &mut WeaponState,
    input: &PlayerInput,
    alive: bool,
    t: &CombatTuning,
    tick_rate: u32,
) -> Vec<FireIntent> {
    let mut out = Vec::new();
    let held = input.buttons;
    let prev = std::mem::replace(&mut s.prev_buttons, held);

    for cd in s
        .primary_cooldown
        .iter_mut()
        .chain(s.alt_cooldown.iter_mut())
    {
        *cd = cd.saturating_sub(1);
    }
    s.bloom = s.bloom.saturating_sub(1);
    s.punch_cooldown = s.punch_cooldown.saturating_sub(1);
    s.parry = s.parry.saturating_sub(1);
    step_slot_timers(s, t, tick_rate);
    let knack = s.knack();
    if s.knack_used > 0 {
        s.knack_recharge = s.knack_recharge.saturating_add(1);
        if s.knack_recharge >= secs_to_ticks(knack.recharge_time(t), tick_rate, 1) {
            s.knack_used -= 1;
            s.knack_recharge = 0;
        }
    } else {
        s.knack_recharge = 0;
    }
    if !alive {
        s.cancel();
        return out;
    }

    // Selection (instant; switching weapon or mode cancels a charge — a held ability is let
    // go first —, switching weapon a draw or salvo too). Empty slots are ignored.
    let mode = if input.weapon_mode < ALT_MODES {
        input.weapon_mode
    } else {
        0
    };
    if s.loadout.slot(s.slot).is_none() {
        s.slot = s.loadout.first_slot().unwrap_or(0);
    }
    if s.loadout.slot(input.weapon).is_some() && (input.weapon != s.slot || mode != s.mode) {
        end_channel(s, t, tick_rate, &mut out);
        s.charge = 0;
        if input.weapon != s.slot {
            s.cancel();
        }
        s.slot = input.weapon;
    }
    if mode != s.mode {
        end_channel(s, t, tick_rate, &mut out);
        s.charge = 0;
        s.armed = 0;
        s.mode = mode;
    }
    if let Some(build) = s.loadout.slot(s.slot).copied() {
        step_build(s, build, input, prev, t, tick_rate, &mut out);
    }

    // Fist / parry.
    if held.pressed(prev, Buttons::PUNCH) && s.punch_cooldown == 0 {
        out.push(FireIntent::Punch);
        s.punch_cooldown = secs_to_ticks(t.punch_cooldown, tick_rate, 1);
        s.parry = secs_to_ticks(t.parry_window, tick_rate, 1);
    }

    // Knack.
    if held.pressed(prev, Buttons::THROW) && s.knack_charges(t) > 0 {
        out.push(FireIntent::Knack(knack));
        if s.knack_used == 0 {
            s.knack_recharge = 0;
        }
        s.knack_used = s.knack_used.saturating_add(1);
    }
    out
}

/// The per-slot timers of the rework: live abilities run out (their cooldown starts), a
/// Stoke runs into its overheat, the overheat cools.
fn step_slot_timers(s: &mut WeaponState, t: &CombatTuning, tick_rate: u32) {
    for i in 0..LOADOUT_SLOTS {
        if s.live[i] > 0 {
            s.live[i] -= 1;
            if s.live[i] == 0
                && let Some(build) = s.loadout.slot(i as u8)
            {
                let ability = build.ability(s.live_mode[i]);
                let cd = secs_to_ticks(ability.cooldown(t), tick_rate, 1);
                s.alt_cooldown[i] = s.alt_cooldown[i].max(cd);
            }
        }
        if s.stoke[i] > 0 {
            s.stoke[i] -= 1;
            if s.stoke[i] == 0 {
                s.overheat[i] = secs_to_ticks(t.stoke.overheat, tick_rate, 0);
            }
        } else {
            s.overheat[i] = s.overheat[i].saturating_sub(1);
        }
    }
}

/// Lets go of a held ability (Tally, Bulwark, Riposte) of the selected build before the
/// selection changes: its release fires and its cooldown starts.
fn end_channel(s: &mut WeaponState, t: &CombatTuning, tick_rate: u32, out: &mut Vec<FireIntent>) {
    let Some(build) = s.held() else {
        return;
    };
    let ability = build.ability(s.mode);
    if ability.style() == AbilityStyle::Channel && s.charge > 0 {
        release_channel(s, build.weapon, ability, t, tick_rate, out);
    }
}

/// A held ability's release: the intent, the cooldown, the charge cleared.
fn release_channel(
    s: &mut WeaponState,
    weapon: WeaponKind,
    ability: Ability,
    t: &CombatTuning,
    tick_rate: u32,
    out: &mut Vec<FireIntent>,
) {
    let max = secs_to_ticks(ability.charge_time(t), tick_rate, 1);
    out.push(FireIntent::Ability {
        weapon,
        ability,
        charge: (s.charge as f32 / max as f32).min(1.0),
        phase: AbilityPhase::Release,
    });
    s.charge = 0;
    s.alt_cooldown[s.slot as usize] = secs_to_ticks(ability.cooldown(t), tick_rate, 1);
}

/// Ticks between the rockets of a salvo.
fn burst_gap(t: &CombatTuning, tick_rate: u32) -> u16 {
    secs_to_ticks(t.cinderbox.salvo_gap, tick_rate, 1)
}

/// Ticks between primary shots of `w` (stoked: `stoke.rate` × faster).
fn primary_interval_ticks(w: WeaponKind, stoked: bool, t: &CombatTuning, tick_rate: u32) -> u16 {
    let mut secs = w.primary_interval(t);
    if stoked {
        secs /= t.stoke.rate.max(0.1);
    }
    secs_to_ticks(secs, tick_rate, 1)
}

/// An ability's use: the intent, then its cooldown (or, live, its live time: the cooldown
/// starts when it ends).
fn use_ability(
    s: &mut WeaponState,
    weapon: WeaponKind,
    ability: Ability,
    charge: f32,
    t: &CombatTuning,
    tick_rate: u32,
    out: &mut Vec<FireIntent>,
) {
    out.push(FireIntent::Ability {
        weapon,
        ability,
        charge,
        phase: AbilityPhase::Use,
    });
    let si = s.slot as usize;
    let live = ability.live_time(t);
    if live > 0.0 {
        s.live[si] = secs_to_ticks(live, tick_rate, 1);
        s.live_mode[si] = s.mode;
    } else {
        s.alt_cooldown[si] = secs_to_ticks(ability.cooldown(t), tick_rate, 1);
    }
    if ability == Ability::Stoke {
        // The weapon in this slot is stoked (`step_slot_timers` runs it into its overheat).
        s.stoke[si] = secs_to_ticks(t.stoke.duration, tick_rate, 1);
        s.overheat[si] = 0;
    }
}

/// The held build's primary and ability for one input (part of [`step_weapons`]).
fn step_build(
    s: &mut WeaponState,
    build: WeaponBuild,
    input: &PlayerInput,
    prev: Buttons,
    t: &CombatTuning,
    tick_rate: u32,
    out: &mut Vec<FireIntent>,
) {
    let held = input.buttons;
    let w = build.weapon;
    let si = s.slot as usize;
    let interval = primary_interval_ticks(w, s.stoke[si] > 0, t, tick_rate);

    // A salvo in progress fires its next rocket when due.
    if s.burst.left > 0 {
        s.burst.next = s.burst.next.saturating_sub(1);
        if s.burst.next == 0 {
            s.burst.left -= 1;
            s.burst.next = burst_gap(t, tick_rate);
            out.push(FireIntent::Primary {
                weapon: w,
                charge: 0.0,
            });
        }
    }

    // The Whirl is the melee weapon's swing spun round: it waits for the swing to be ready and
    // uses it up, so the primary doesn't also swing that tick (a Tamper's swing and a Whirl on
    // one tick were 132 to a PvP body).
    let ability = build.ability(s.mode);
    let takes_shot = takes_primary_shot(ability);
    let ability_takes_shot = takes_shot
        && w.class() != WeaponClass::Draw
        && held.contains(Buttons::ALT_FIRE)
        && s.live[si] == 0
        && s.alt_cooldown[si] == 0
        && s.primary_cooldown[si] == 0;

    // Primary (none while overheated after a Stoke).
    let fire = held.contains(Buttons::FIRE) && s.overheat[si] == 0;
    match w.primary_style() {
        PrimaryStyle::Auto => {
            if fire && !ability_takes_shot && s.primary_cooldown[si] == 0 {
                let mut charge = 0.0;
                if w == WeaponKind::Lancet {
                    let (per_shot, max) = bloom_ticks(t, tick_rate);
                    charge = (s.bloom as f32 / max as f32).min(1.0);
                    s.bloom = (s.bloom + per_shot).min(max);
                }
                out.push(FireIntent::Primary { weapon: w, charge });
                s.primary_cooldown[si] = interval;
            }
        }
        PrimaryStyle::Salvo => {
            if fire && s.primary_cooldown[si] == 0 && s.burst.left == 0 {
                out.push(FireIntent::Primary {
                    weapon: w,
                    charge: 0.0,
                });
                s.primary_cooldown[si] = interval;
                let rockets = t.cinderbox.rockets.clamp(1, u8::MAX as u32) as u8;
                s.burst = Burst {
                    left: rockets - 1,
                    next: burst_gap(t, tick_rate),
                };
            }
        }
        PrimaryStyle::Draw | PrimaryStyle::Focus => {
            let max = WeaponState::draw_ticks_of(w, t, tick_rate);
            if fire {
                if s.draw > 0 || s.primary_cooldown[si] == 0 {
                    s.draw = (s.draw + 1).min(max);
                }
            } else {
                if held.released(prev, Buttons::FIRE) && s.draw > 0 {
                    let charge = s.draw_fraction(t, tick_rate);
                    match s.armed_ability() {
                        // The bow's armed arrow rides the release.
                        Some(ability) => {
                            s.armed = 0;
                            use_ability(s, w, ability, charge, t, tick_rate, out);
                        }
                        None => out.push(FireIntent::Primary { weapon: w, charge }),
                    }
                    s.primary_cooldown[si] = interval;
                }
                s.draw = 0;
            }
        }
    }

    // Ability.
    let alt_held = held.contains(Buttons::ALT_FIRE);
    let alt_pressed = held.pressed(prev, Buttons::ALT_FIRE);
    // The live ability itself (selected): a second press recasts it if it can. (The build's
    // other ability stays usable meanwhile, on the shared cooldown.)
    if s.live[si] > 0 && s.live_mode[si] == s.mode {
        if alt_pressed && ability.recasts() {
            out.push(FireIntent::Ability {
                weapon: w,
                ability,
                charge: 0.0,
                phase: AbilityPhase::Recast,
            });
            s.live[si] = 0;
            let cd = secs_to_ticks(ability.recast_cooldown(t), tick_rate, 1);
            s.alt_cooldown[si] = s.alt_cooldown[si].max(cd);
        }
        return;
    }
    let ready = s.alt_cooldown[si] == 0;
    if w.class() == WeaponClass::Draw {
        // Bow abilities arm the next release (pressing again disarms).
        if alt_pressed && ready {
            s.armed = if s.armed == s.mode + 1 { 0 } else { s.mode + 1 };
        }
        return;
    }
    match ability.style() {
        AbilityStyle::Charge => {
            let max = s.charge_ticks(t, tick_rate);
            if alt_held {
                if ready {
                    s.charge = (s.charge + 1).min(max);
                }
            } else {
                if held.released(prev, Buttons::ALT_FIRE) && s.charge > 0 {
                    let charge = (s.charge as f32 / max as f32).min(1.0);
                    use_ability(s, w, ability, charge, t, tick_rate, out);
                }
                s.charge = 0;
            }
        }
        AbilityStyle::Hold => {
            let hold = s.charge_ticks(t, tick_rate);
            if alt_held {
                if s.charge < hold {
                    s.charge += 1;
                    if s.charge == hold {
                        out.push(FireIntent::Ability {
                            weapon: w,
                            ability,
                            charge: 1.0,
                            phase: AbilityPhase::Use,
                        });
                    }
                }
            } else {
                if held.released(prev, Buttons::ALT_FIRE)
                    && s.charge > 0
                    && s.charge < hold
                    && ready
                {
                    use_ability(s, w, ability, 0.0, t, tick_rate, out);
                }
                s.charge = 0;
            }
        }
        AbilityStyle::Channel => {
            let max = s.charge_ticks(t, tick_rate);
            if s.charge > 0 {
                // Up: held on until let go or its limit.
                if alt_held && s.charge < max {
                    s.charge += 1;
                }
                if !alt_held || s.charge >= max {
                    release_channel(s, w, ability, t, tick_rate, out);
                }
            } else if alt_pressed && ready {
                out.push(FireIntent::Ability {
                    weapon: w,
                    ability,
                    charge: 0.0,
                    phase: AbilityPhase::Use,
                });
                s.charge = 1;
            }
        }
        AbilityStyle::Press => {
            if alt_pressed && ready {
                use_ability(s, w, ability, 0.0, t, tick_rate, out);
            }
        }
        AbilityStyle::Instant => {
            if alt_held && ready && (!takes_shot || s.primary_cooldown[si] == 0) {
                use_ability(s, w, ability, 0.0, t, tick_rate, out);
                if takes_shot {
                    // It was the primary's swing: the primary waits its interval.
                    s.primary_cooldown[si] = interval;
                }
            }
        }
    }
}

/// Abilities that are the primary's shot fired another way (the Whirl: the melee weapon's
/// swing spun round). They need the primary ready and use it up.
fn takes_primary_shot(ability: Ability) -> bool {
    matches!(ability, Ability::Whirl)
}

/// Deterministic pseudo-random numbers for spread (splitmix64 of a seed).
pub(crate) struct SpreadRng(u64);

impl SpreadRng {
    pub fn new(tick: u32, player: PlayerId, salt: u32) -> Self {
        Self(
            ((tick as u64) << 32) ^ ((player.0 as u64) << 24) ^ salt as u64 ^ 0x9E37_79B9_7F4A_7C15,
        )
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Orthonormal right/up vectors for an aim direction.
pub fn aim_basis(dir: Vec3) -> (Vec3, Vec3) {
    let right = dir.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(dir).normalize_or(Vec3::Y);
    (right, up)
}

/// A random direction within a cone of full aperture `cone_degrees` around `dir` (uniform
/// over the disc).
pub(crate) fn spread_dir(dir: Vec3, cone_degrees: f32, rng: &mut SpreadRng) -> Vec3 {
    let half = (cone_degrees * 0.5).to_radians();
    let r = rng.next_f32().sqrt() * half.tan();
    let a = rng.next_f32() * std::f32::consts::TAU;
    let (right, up) = aim_basis(dir);
    (dir + right * (r * a.cos()) + up * (r * a.sin())).normalize_or(dir)
}

/// `aim` rotated `degrees` upwards about its right axis (lobs); never below the aim.
pub(crate) fn lob_dir(aim: Vec3, degrees: f32) -> Vec3 {
    let (right, _) = aim_basis(aim);
    let lob = Quat::from_axis_angle(right, degrees.to_radians()) * aim;
    if lob.y < aim.y { aim } else { lob }
}

/// The tuning an ability fired from `weapon` uses: its damage scaled by the carrier's
/// `ability_power` relative to the ability's home weapon, never above the listed numbers
/// (unchanged on the home weapon, on a weapon at least as powerful and for abilities made of
/// the weapon's own shots).
pub fn ability_tuning(t: &Tuning, weapon: WeaponKind, ability: Ability) -> Cow<'_, Tuning> {
    let c = &t.combat;
    let home = ability.home().ability_power(c);
    let power = (weapon.ability_power(c) / home.max(1e-3)).min(1.0);
    if (power - 1.0).abs() < 1e-6 || ability.uses_primary() {
        return Cow::Borrowed(t);
    }
    let mut scaled = t.clone();
    scaled
        .combat
        .scale_ability(ability, &BalanceScale::damage(power));
    Cow::Owned(scaled)
}

impl SimWorld {
    /// Executes one fire intent of player `id` (server). `input` is the input that fired it
    /// (view angles, lag-compensation view time).
    pub fn resolve_intent(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        intent: FireIntent,
    ) {
        match intent {
            FireIntent::Punch => self.punch(t, history, id, input),
            FireIntent::Knack(knack) => self.use_knack(t, id, input, knack),
            FireIntent::Primary { weapon, charge } => {
                self.fire_primary(t, history, id, input, weapon, charge)
            }
            FireIntent::Ability {
                weapon,
                ability,
                charge,
                phase,
            } => {
                let t = ability_tuning(t, weapon, ability);
                if ability.is_rework() {
                    // One file each (`abilities/`).
                    self.cast_ability(&AbilityCast {
                        t: &t,
                        history,
                        id,
                        input,
                        weapon,
                        ability,
                        charge,
                        phase,
                    });
                } else {
                    self.fire_ability(&t, history, id, input, weapon, ability, charge);
                }
            }
        }
    }

    /// A weapon's primary fire.
    fn fire_primary(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        weapon: WeaponKind,
        charge: f32,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let owner = Owner::Player(id);
        let c = &t.combat;
        let attack = weapon.primary();
        let mut rng = SpreadRng::new(self.tick, id, input.seq);
        match weapon {
            WeaponKind::Quill => {
                let shot =
                    ShotSpec::new(eye, aim, c.hitscan_range, c.quill.damage, attack).ricochet(true);
                self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
            }
            WeaponKind::Lancet => {
                let l = &c.lancet;
                let dir = spread_dir(aim, l.bloom_degrees * charge.clamp(0.0, 1.0), &mut rng);
                let shot = ShotSpec::new(eye, dir, c.hitscan_range, l.damage, attack);
                self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
            }
            // A fixed pellet pattern, point blank, the stagger (`scatter.rs`).
            WeaponKind::Bellows | WeaponKind::Thresher => {
                self.fire_blast(t, history, id, input, weapon)
            }
            WeaponKind::Arclash => self.arc_zaps(t, history, id, input, &[aim], 1.0, attack),
            WeaponKind::Tumbler => {
                self.spawn_projectile(
                    t,
                    ProjectileKind::Bomb,
                    owner,
                    eye,
                    aim * c.tumbler.bomb_speed,
                );
                self.push_fired(id, attack, eye);
            }
            WeaponKind::Riveter => {
                let r = &c.riveter;
                let dir = spread_dir(aim, r.spread_degrees, &mut rng);
                let start = self.safe_launch_point(eye, eye + aim * 0.4);
                self.spawn_player_projectile(
                    t,
                    ProjectileKind::Rivet,
                    id,
                    start,
                    dir * r.rivet_speed,
                    r.damage,
                    attack,
                );
                self.push_fired(id, attack, eye);
            }
            WeaponKind::Cinderbox => {
                let cb = &c.cinderbox;
                let dir = spread_dir(aim, cb.spread_degrees, &mut rng);
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                self.spawn_projectile(
                    t,
                    ProjectileKind::CinderRocket,
                    owner,
                    start,
                    dir * cb.rocket_speed,
                );
                self.push_fired(id, attack, eye);
            }
            WeaponKind::Longwick => {
                self.loose_arrows(t, id, eye, aim, charge, ProjectileKind::Arrow, attack, 1.0);
            }
            WeaponKind::Kiln => self.kiln_spray(t, history, id, input),
            WeaponKind::Trimmer | WeaponKind::Tamper => {
                self.melee_swing(t, history, id, input, weapon)
            }
            WeaponKind::Lamplighter => self.lamplighter_beam(t, history, id, input, charge),
        }
    }

    /// The Lamplighter's beam at focus `focus` (0..=1): exact hitscan, damage by the focus; a
    /// full focus pierces the first target into `lamplighter.pierce` − 1 more behind it.
    fn lamplighter_beam(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        focus: f32,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let l = &t.combat.lamplighter;
        let k = focus.clamp(0.0, 1.0);
        let damage = l.min_damage + (l.max_damage - l.min_damage) * k;
        let attack = Attack::Lamplighter;
        let shot = ShotSpec::new(
            eye,
            input.look_dir(),
            t.combat.hitscan_range,
            damage,
            attack,
        );
        if k >= 1.0 && l.pierce > 1 {
            self.fire_piercing(t, history, id, input, &shot, l.pierce, l.pierce_falloff);
        } else {
            self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
        }
    }

    /// One of the kept abilities (`t` is already scaled for the carrying weapon, see
    /// [`ability_tuning`]); the rework's go to `abilities/` ([`SimWorld::cast_ability`]).
    #[allow(clippy::too_many_arguments)]
    fn fire_ability(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        weapon: WeaponKind,
        ability: Ability,
        charge: f32,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let owner = Owner::Player(id);
        let c = &t.combat;
        let attack = ability.attack();
        // Only the Quill's shots ricochet off thorns (its identity), whichever ability.
        let ricochet = weapon == WeaponKind::Quill;
        // Bow abilities change the arrow of the release (`charge` = the draw).
        if weapon.class() == WeaponClass::Draw {
            let (kind, scale) = match ability {
                Ability::PinningArrow => (ProjectileKind::PinArrow, c.pinning_arrow.damage_scale),
                Ability::Brand => (ProjectileKind::BrandArrow, 1.0),
                _ => (ProjectileKind::Arrow, 1.0),
            };
            self.loose_arrows(t, id, eye, aim, charge, kind, attack, scale);
            return;
        }
        match ability {
            Ability::Brand => {
                let shot = ShotSpec::new(eye, aim, c.hitscan_range, c.quill.brand_damage, attack)
                    .ricochet(ricochet);
                let (_, results) = self.fire_hitscan_group(t, history, id, input, attack, &[shot]);
                // Whatever the (last leg of the) shot hit is branded, if it survived.
                for impact in results.iter().filter_map(|r| r.impact) {
                    if let Some((target, _)) = impact.target {
                        self.brand_target(t, id, target, impact.point);
                    }
                }
            }
            Ability::Gust => self.gust(t, history, id, input),
            Ability::Flak => {
                let f = &c.flak;
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                self.spawn_player_projectile(
                    t,
                    ProjectileKind::FlakShell,
                    id,
                    start,
                    aim * f.shell_speed,
                    f.pellet_damage,
                    attack,
                );
                self.push_fired(id, attack, eye);
            }
            Ability::Discharge => self.discharge(t, history, id, input, charge),
            Ability::Breach => self.breach(t, history, id, input),
            Ability::Gravewell => {
                let tb = &c.tumbler;
                let dir = lob_dir(aim, tb.gravewell_angle_degrees);
                self.spawn_projectile(
                    t,
                    ProjectileKind::Gravewell,
                    owner,
                    eye,
                    dir * tb.gravewell_speed,
                );
                self.push_fired(id, attack, eye);
            }
            Ability::Limpet => {
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                self.limpet(t, id, charge, start, aim * c.limpet.speed);
            }
            Ability::Lunge | Ability::Whirl | Ability::Hurl => {
                self.melee_ability(t, history, id, input, weapon, ability)
            }
            // Bow-only and the rework's abilities never reach here (above, `abilities/`).
            _ => {}
        }
    }

    /// Arclash zaps along `dirs` (one hitscan group), each chaining from what it hits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn arc_zaps(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        dirs: &[Vec3],
        damage_scale: f32,
        attack: Attack,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let a = &t.combat.arclash;
        let damage = a.zap_damage * damage_scale;
        let shots: Vec<ShotSpec> = dirs
            .iter()
            .map(|d| ShotSpec::new(eye, *d, a.zap_range, damage, attack))
            .collect();
        let (view, results) = self.fire_hitscan_group(t, history, id, input, attack, &shots);
        for impact in results.iter().filter_map(|r| r.impact) {
            if let Some((target, _)) = impact.target {
                self.arc_chain(
                    t,
                    Owner::Player(id),
                    &view.hitboxes,
                    target,
                    impact.point,
                    damage,
                );
            }
        }
    }

    /// The bow's armed rework arrows (Blink, Beacon): an arrow of `kind` with the release's
    /// draw (`c.charge`), its hit `damage_scale` × a plain arrow's.
    pub(crate) fn loose_ability_arrow(
        &mut self,
        c: &AbilityCast,
        kind: ProjectileKind,
        damage_scale: f32,
    ) -> Option<crate::protocol::NetId> {
        let p = self.players.get(&c.id)?;
        let eye = p.movement.eye_position(&c.t.movement);
        let aim = c.input.look_dir();
        let attack = c.ability.attack();
        self.loose_arrows(c.t, c.id, eye, aim, c.charge, kind, attack, damage_scale)
    }

    /// Looses an arrow of `kind` along `dir` at draw `draw` (0..=1): speed and damage by the
    /// draw (Longwick), damage × `damage_scale`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn loose_arrows(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        eye: Vec3,
        dir: Vec3,
        draw: f32,
        kind: ProjectileKind,
        attack: Attack,
        damage_scale: f32,
    ) -> Option<crate::protocol::NetId> {
        let l = &t.combat.longwick;
        let k = draw.clamp(0.0, 1.0);
        let speed = l.min_speed + (l.max_speed - l.min_speed) * k;
        let damage = (l.min_damage + (l.max_damage - l.min_damage) * k) * damage_scale;
        let start = self.safe_launch_point(eye, eye + dir * 0.4);
        let arrow = self.spawn_player_projectile(t, kind, id, start, dir * speed, damage, attack);
        self.push_fired(id, attack, eye);
        arrow
    }

    /// A player projectile dealing `damage` on a direct hit as `attack`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_player_projectile(
        &mut self,
        t: &Tuning,
        kind: ProjectileKind,
        id: PlayerId,
        position: Vec3,
        velocity: Vec3,
        damage: f32,
        attack: Attack,
    ) -> Option<crate::protocol::NetId> {
        let spawned = self.spawn_projectile(t, kind, Owner::Player(id), position, velocity);
        if let Some(p) = spawned.and_then(|pid| self.projectiles.get_mut(&pid)) {
            p.damage = damage;
            p.attack = attack;
        }
        spawned
    }

    /// Projectile weapons: "shooter fired" event for remote muzzle flashes / sounds.
    pub(crate) fn push_fired(&mut self, id: PlayerId, attack: Attack, origin: Vec3) {
        self.events.push(SimEvent::Shot(super::ShotEvent {
            shooter: id,
            attack,
            origin,
            ends: Vec::new(),
        }));
    }

    /// `want` unless the level blocks the way from `eye` (then `eye`): don't start inside a wall
    /// right in front of the shooter.
    pub(crate) fn safe_launch_point(&self, eye: Vec3, want: Vec3) -> Vec3 {
        if self
            .collision
            .world
            .line_blocked(eye, want, engine::physics::MASK_SHOTS)
        {
            eye
        } else {
            want
        }
    }

    /// Flags every hitscan hit carries.
    pub(crate) fn hitscan_flags(_attack: Attack) -> DamageFlags {
        DamageFlags::HITSCAN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(buttons: Buttons, weapon: u8, mode: u8) -> PlayerInput {
        PlayerInput {
            buttons,
            weapon,
            weapon_mode: mode,
            ..Default::default()
        }
    }

    fn primary(weapon: WeaponKind) -> FireIntent {
        FireIntent::Primary {
            weapon,
            charge: 0.0,
        }
    }

    #[test]
    fn primary_respects_interval_and_switching_is_instant() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let mut shots = 0;
        for _ in 0..60 {
            shots += step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60).len();
        }
        // 0.28 s interval (17 ticks): shots at ticks 0, 17, 34, 51.
        assert_eq!(shots, 4);
        // Switch to the Bellows: it fires immediately (own cooldown).
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 1, 0), true, &t, 60);
        assert_eq!(out, vec![primary(WeaponKind::Bellows)]);
        // The Arclash zaps every 0.1 s while held.
        let zaps: usize = (0..60)
            .map(|_| step_weapons(&mut s, &input(Buttons::FIRE, 2, 0), true, &t, 60).len())
            .sum();
        assert_eq!(zaps, 10);
    }

    #[test]
    fn discharge_charges_and_fires_on_release() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        for _ in 0..30 {
            assert!(step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60).is_empty());
        }
        assert_eq!(s.charge, 30);
        let out = step_weapons(&mut s, &input(Buttons::NONE, 2, 1), true, &t, 60);
        match out.as_slice() {
            [FireIntent::Ability { charge, .. }] => {
                assert!((charge - 0.5).abs() < 1e-6, "{charge}")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(s.charge, 0);
        // On cooldown (2 s): holding doesn't charge.
        for _ in 0..10 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        assert_eq!(s.charge, 0);
        // The charge caps at 1.0 s.
        let mut s = WeaponState::default();
        for _ in 0..200 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        assert_eq!(s.charge, 60);
        assert_eq!(s.charge_fraction(&t, 60), 1.0);
        // Switching mode or weapon cancels it without firing.
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 0), true, &t, 60);
        assert_eq!(s.charge, 0);
        assert!(
            out.iter()
                .all(|i| !matches!(i, FireIntent::Ability { charge, .. } if *charge > 0.0))
        );
        for _ in 0..10 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 1), true, &t, 60);
        assert_eq!(s.charge, 0);
    }

    fn ability(weapon: WeaponKind, ability: Ability, phase: AbilityPhase) -> FireIntent {
        FireIntent::Ability {
            weapon,
            ability,
            charge: 0.0,
            phase,
        }
    }

    #[test]
    fn the_whirl_takes_the_swing() {
        let t = CombatTuning::default();
        let tamper = || WeaponState::new(Loadout::single(WeaponKind::Tamper.default_build()));
        let mut s = tamper();
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        assert_eq!(
            out,
            vec![ability(
                WeaponKind::Tamper,
                Ability::Whirl,
                AbilityPhase::Use
            )]
        );
        assert_eq!(
            s.primary_cooldown[0],
            secs_to_ticks(t.tamper.interval, 60, 1)
        );
        assert!(step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60).is_empty());
        // Both buttons on one tick: the Whirl takes the swing, the primary doesn't also swing.
        let both = Buttons::FIRE | Buttons::ALT_FIRE;
        let out = step_weapons(&mut tamper(), &input(both, 0, 0), true, &t, 60);
        assert!(
            matches!(
                out[..],
                [FireIntent::Ability {
                    ability: Ability::Whirl,
                    ..
                }]
            ),
            "{out:?}"
        );
        // It waits for the swing: a swing, then the Whirl only once the swing's interval is
        // over (even with its own cooldown ready long before).
        let mut s = tamper();
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60);
        assert!(matches!(out[..], [FireIntent::Primary { .. }]));
        let interval = secs_to_ticks(t.tamper.interval, 60, 1);
        for _ in 1..interval {
            let out = step_weapons(&mut s, &input(both, 0, 0), true, &t, 60);
            assert!(out.is_empty(), "{out:?}");
        }
        let out = step_weapons(&mut s, &input(both, 0, 0), true, &t, 60);
        assert!(
            matches!(
                out[..],
                [FireIntent::Ability {
                    ability: Ability::Whirl,
                    ..
                }]
            ),
            "{out:?}"
        );
        // The Lunge doesn't.
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Trimmer.default_build()));
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        assert_eq!(s.primary_cooldown[0], 0);
    }

    #[test]
    fn held_abilities_use_on_the_press_and_release_on_letting_go() {
        let t = CombatTuning::default();
        // The Bellows' Bulwark (mode 1): the press raises it, holding keeps it up.
        let mut s = WeaponState::default();
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 1, 1), true, &t, 60);
        assert_eq!(
            out,
            vec![ability(
                WeaponKind::Bellows,
                Ability::Bulwark,
                AbilityPhase::Use
            )]
        );
        for _ in 0..30 {
            let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 1, 1), true, &t, 60);
            assert!(out.is_empty(), "{out:?}");
        }
        assert_eq!(s.charge, 31);
        assert_eq!(s.alt_cooldown[1], 0, "the cooldown waits for the release");
        // Letting go is the release (how long it was held), and the cooldown starts.
        let out = step_weapons(&mut s, &input(Buttons::NONE, 1, 1), true, &t, 60);
        let held = 31.0 / secs_to_ticks(t.bulwark.hold_time, 60, 1) as f32;
        assert_eq!(
            out,
            vec![FireIntent::Ability {
                weapon: WeaponKind::Bellows,
                ability: Ability::Bulwark,
                charge: held,
                phase: AbilityPhase::Release,
            }]
        );
        assert_eq!(s.charge, 0);
        assert_eq!(s.alt_cooldown[1], secs_to_ticks(t.bulwark.cooldown, 60, 1));
        // Held to its limit: released there (holding on doesn't raise it again).
        let tamper = Loadout::single(WeaponKind::Tamper.default_build());
        let mut s = WeaponState::new(tamper);
        let limit = secs_to_ticks(t.riposte.hold_time, 60, 1);
        let mut fired = Vec::new();
        for _ in 0..limit + 30 {
            fired.extend(step_weapons(
                &mut s,
                &input(Buttons::ALT_FIRE, 0, 1),
                true,
                &t,
                60,
            ));
        }
        assert_eq!(fired.len(), 2, "{fired:?}");
        assert!(matches!(
            fired[1],
            FireIntent::Ability { ability: Ability::Riposte, phase: AbilityPhase::Release, charge, .. }
                if charge == 1.0
        ));
        // Switching away lets go of it too (the server drops the guard).
        let mut s = WeaponState::default();
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 1, 1), true, &t, 60);
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 1), true, &t, 60);
        assert!(
            matches!(
                out.first(),
                Some(FireIntent::Ability {
                    ability: Ability::Bulwark,
                    phase: AbilityPhase::Release,
                    ..
                })
            ),
            "{out:?}"
        );
        assert!(s.alt_cooldown[1] > 0 && s.charge == 0);
        // The Tally replicates as TALLYING, a guard as neither CHARGING nor TALLYING (the
        // guard itself says GUARDING).
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Lancet.default_build()));
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        assert_eq!(s.ability(), Ability::Tally);
        assert!(s.charge > 0);
    }

    #[test]
    fn live_abilities_wait_recast_and_start_their_cooldown_at_the_end() {
        let t = CombatTuning::default();
        // The Arclash's Wisp (mode 0): a press, then live for its orbit.
        let mut s = WeaponState::default();
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 0), true, &t, 60);
        assert_eq!(
            out,
            vec![ability(
                WeaponKind::Arclash,
                Ability::Wisp,
                AbilityPhase::Use
            )]
        );
        let orbit = secs_to_ticks(t.arclash.wisp_orbit_time, 60, 1);
        assert_eq!((s.live[2], s.alt_cooldown[2]), (orbit, 0));
        assert_eq!(s.live_ability(2), Some(Ability::Wisp));
        // Holding on does nothing; the build's other ability stays usable meanwhile (on the
        // shared cooldown).
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 0), true, &t, 60);
        assert!(out.is_empty(), "{out:?}");
        for _ in 0..10 {
            step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 1), true, &t, 60);
        }
        let out = step_weapons(&mut s, &input(Buttons::NONE, 2, 1), true, &t, 60);
        assert!(
            matches!(
                out[..],
                [FireIntent::Ability {
                    ability: Ability::Discharge,
                    ..
                }]
            ),
            "{out:?}"
        );
        assert!(s.alt_cooldown[2] > 0 && s.live[2] > 0);
        // A second press (in its mode) flings it: the recast, and the cooldown counts the
        // flight.
        step_weapons(&mut s, &input(Buttons::NONE, 2, 0), true, &t, 60);
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 0), true, &t, 60);
        assert_eq!(
            out,
            vec![ability(
                WeaponKind::Arclash,
                Ability::Wisp,
                AbilityPhase::Recast
            )]
        );
        assert_eq!(s.live[2], 0);
        assert_eq!(
            s.alt_cooldown[2],
            secs_to_ticks(t.arclash.wisp_lifetime + t.arclash.wisp_cooldown, 60, 1)
        );
        // Left alone it runs out and the cooldown starts then.
        let mut s = WeaponState::default();
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 2, 0), true, &t, 60);
        for _ in 0..orbit {
            step_weapons(&mut s, &input(Buttons::NONE, 2, 0), true, &t, 60);
        }
        assert_eq!(s.live[2], 0);
        assert_eq!(
            s.alt_cooldown[2],
            secs_to_ticks(t.arclash.wisp_cooldown, 60, 1)
        );
        // The server can end one early (a Waxwork destroyed): the cooldown starts.
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Lancet.default_build()));
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 1), true, &t, 60);
        assert!(matches!(
            out[..],
            [FireIntent::Ability {
                ability: Ability::Waxwork,
                ..
            }]
        ));
        assert!(s.live[0] > 0);
        // No recast for the Waxwork: a second press does nothing.
        step_weapons(&mut s, &input(Buttons::NONE, 0, 1), true, &t, 60);
        assert!(step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 1), true, &t, 60).is_empty());
        s.end_live(0, &t, 60);
        assert_eq!(
            (s.live[0], s.alt_cooldown[0]),
            (0, secs_to_ticks(t.waxwork.cooldown, 60, 1))
        );
    }

    #[test]
    fn stoke_speeds_the_primary_then_overheats_it() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Riveter.default_build()));
        let count = |s: &mut WeaponState, b: Buttons, ticks: u32| -> usize {
            (0..ticks)
                .map(|_| {
                    step_weapons(s, &input(b, 0, 0), true, &t, 60)
                        .iter()
                        .filter(|f| matches!(f, FireIntent::Primary { .. }))
                        .count()
                })
                .sum()
        };
        let normal = count(&mut s.clone(), Buttons::FIRE, 60);
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        assert!(matches!(
            out[..],
            [FireIntent::Ability {
                ability: Ability::Stoke,
                ..
            }]
        ));
        assert!(s.is_stoked());
        let stoked = count(&mut s, Buttons::FIRE, 60);
        assert!(
            stoked as f32 >= normal as f32 * 1.5,
            "stoked {stoked} vs {normal} a second"
        );
        // Run the stoke out: then overheated, no primary, and it cools.
        let stoke = secs_to_ticks(t.stoke.duration, 60, 1) as u32;
        count(&mut s, Buttons::NONE, stoke - 60);
        assert!(s.is_overheated() && !s.is_stoked());
        let overheat = secs_to_ticks(t.stoke.overheat, 60, 1) as u32;
        assert_eq!(count(&mut s, Buttons::FIRE, overheat - 1), 0);
        assert!(count(&mut s, Buttons::FIRE, 10) > 0);
        assert!(!s.is_overheated());
        // From the use: 10 s.
        assert!(s.alt_cooldown[0] > 0);
    }

    #[test]
    fn the_lamplighter_focuses_and_fires_on_release() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Lamplighter.default_build()));
        let full = WeaponState::draw_ticks_of(WeaponKind::Lamplighter, &t, 60);
        assert_eq!(full, secs_to_ticks(t.lamplighter.focus_time, 60, 1));
        for _ in 0..full + 10 {
            assert!(step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60).is_empty());
        }
        assert_eq!((s.draw, s.charge_level(&t, 60)), (full, 255));
        let out = step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        assert_eq!(
            out,
            vec![FireIntent::Primary {
                weapon: WeaponKind::Lamplighter,
                charge: 1.0
            }]
        );
        // The next focus waits its interval; a tap fires at the minimum.
        let interval = secs_to_ticks(t.lamplighter.interval, 60, 1);
        for _ in 1..interval {
            step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60);
            assert_eq!(s.draw, 0);
        }
        step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60);
        match step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60).as_slice() {
            [FireIntent::Primary { charge, .. }] => assert!(*charge < 0.05),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn punch_opens_parry_window_and_thorns_recharge() {
        let t = CombatTuning::default();
        let mut s = WeaponState::default();
        let out = step_weapons(
            &mut s,
            &input(Buttons::PUNCH | Buttons::THROW, 0, 0),
            true,
            &t,
            60,
        );
        assert_eq!(
            out,
            vec![FireIntent::Punch, FireIntent::Knack(Knack::Thorn)]
        );
        assert_eq!(s.parry, 12);
        assert_eq!(s.knack_charges(&t), 2);
        // Three charges: the fourth press throws nothing.
        for _ in 0..2 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
            step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &t, 60);
        }
        assert_eq!(s.knack_charges(&t), 0);
        step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        assert!(step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &t, 60).is_empty());
        // One per 3 s.
        for _ in 0..180 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        }
        assert_eq!(s.knack_charges(&t), 1);
        assert_eq!(s.parry, 0);
        // Dead players fire nothing.
        assert!(
            step_weapons(
                &mut s,
                &input(Buttons::FIRE | Buttons::PUNCH, 0, 0),
                false,
                &t,
                60
            )
            .is_empty()
        );
        // No thorn charges in the tuning: THROW does nothing.
        let mut none = t.clone();
        none.thorn.charges = 0;
        let mut s = WeaponState::default();
        assert!(step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &none, 60).is_empty());
        assert_eq!(s.knack_charges(&none), 0);
    }

    #[test]
    fn every_knack_uses_its_own_charges_and_recharge() {
        let t = CombatTuning::default();
        for knack in Knack::ALL {
            let mut s = WeaponState::new(Loadout::default().with_knack(knack));
            let charges = knack.charges(&t);
            assert!(charges > 0, "{knack:?}");
            let mut used = 0;
            for _ in 0..charges + 2 {
                let out = step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &t, 60);
                used += out
                    .iter()
                    .filter(|f| **f == FireIntent::Knack(knack))
                    .count();
                step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
            }
            assert_eq!(used as u32, charges, "{knack:?}");
            assert_eq!(s.knack_charges(&t), 0);
            // One charge back after its recharge time.
            let ticks = secs_to_ticks(knack.recharge_time(&t), 60, 1);
            for _ in 0..ticks {
                step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
            }
            assert_eq!(s.knack_charges(&t), 1, "{knack:?}");
        }
        // A new knack starts full; the same one keeps its spent charges.
        let mut s = WeaponState::default();
        step_weapons(&mut s, &input(Buttons::THROW, 0, 0), true, &t, 60);
        assert_eq!(s.knack_used, 1);
        let same = s.with_loadout(Loadout::single(WeaponKind::Kiln.default_build()));
        assert_eq!(same.knack_used, 1);
        let other = s.with_loadout(Loadout::default().with_knack(Knack::Surge));
        assert_eq!((other.knack_used, other.knack_recharge), (0, 0));
    }

    #[test]
    fn slots_follow_the_loadout_and_skip_empty_ones() {
        let t = CombatTuning::default();
        let loadout = Loadout {
            slots: [
                None,
                Some(WeaponKind::Longwick.default_build()),
                None,
                Some(WeaponKind::Lancet.default_build()),
            ],
            ..Default::default()
        };
        let mut s = WeaponState::new(loadout);
        assert_eq!((s.slot, s.current()), (1, WeaponKind::Longwick));
        // Empty slots (and nonsense) keep the current weapon.
        for slot in [0, 2, 9] {
            step_weapons(&mut s, &input(Buttons::NONE, slot, 0), true, &t, 60);
            assert_eq!(s.slot, 1);
        }
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 3, 1), true, &t, 60);
        assert_eq!(s.current(), WeaponKind::Lancet);
        assert_eq!(s.ability(), Ability::Waxwork);
        assert_eq!(out, vec![primary(WeaponKind::Lancet)]);
        // Respawning with a loadout that lacks the slot falls back to the first weapon.
        let s = s.respawned(Loadout::single(WeaponKind::Kiln.default_build()));
        assert_eq!((s.slot, s.current()), (0, WeaponKind::Kiln));
    }

    #[test]
    fn every_weapon_and_ability_fires() {
        let t = CombatTuning::default();
        for w in WeaponKind::ALL {
            for &a in w.abilities() {
                let other = *w.abilities().iter().find(|o| **o != a).unwrap();
                let build = WeaponBuild {
                    weapon: w,
                    abilities: [a, other],
                };
                assert!(build.is_valid());
                let mut s = WeaponState::new(Loadout::single(build));
                let mut fired = Vec::new();
                // Hold both (draw / charge), release, press the ability alone, then tap the
                // trigger once everything has recovered.
                for i in 0..120 {
                    let b = match i {
                        0..60 => Buttons::FIRE | Buttons::ALT_FIRE,
                        62..64 => Buttons::ALT_FIRE,
                        90..92 => Buttons::FIRE,
                        _ => Buttons::NONE,
                    };
                    fired.extend(step_weapons(&mut s, &input(b, 0, 0), true, &t, 60));
                }
                assert!(
                    fired
                        .iter()
                        .any(|f| matches!(f, FireIntent::Primary { .. })),
                    "{w:?} primary"
                );
                assert!(
                    fired
                        .iter()
                        .any(|f| matches!(f, FireIntent::Ability { ability, .. } if *ability == a)),
                    "{w:?} {a:?}: {fired:?}"
                );
            }
        }
    }

    #[test]
    fn the_salvo_fires_over_several_ticks() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Cinderbox.default_build()));
        let mut ticks = Vec::new();
        for i in 0..30 {
            let b = if i == 0 { Buttons::FIRE } else { Buttons::NONE };
            if !step_weapons(&mut s, &input(b, 0, 0), true, &t, 60).is_empty() {
                ticks.push(i);
            }
        }
        // 3 rockets 0.08 s (5 ticks) apart.
        assert_eq!(ticks, vec![0, 5, 10]);
        // Switching weapon cancels the rest of a salvo.
        let mut s = WeaponState::new(Loadout {
            slots: [
                Some(WeaponKind::Cinderbox.default_build()),
                Some(WeaponKind::Quill.default_build()),
                None,
                None,
            ],
            ..Default::default()
        });
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60);
        assert_eq!(out.len(), 1);
        assert_eq!(s.burst.left, 2);
        step_weapons(&mut s, &input(Buttons::NONE, 1, 0), true, &t, 60);
        assert_eq!(s.burst, Burst::default());
    }

    #[test]
    fn the_bow_draws_and_armed_abilities_ride_the_release() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Longwick.default_build()));
        let full = WeaponState::draw_ticks(&t, 60);
        for _ in 0..full + 10 {
            assert!(step_weapons(&mut s, &input(Buttons::FIRE, 0, 1), true, &t, 60).is_empty());
        }
        assert_eq!(s.draw, full);
        assert_eq!(s.charge_level(&t, 60), 255);
        // Arm Pinning Arrow (mode 1) mid-draw, then release.
        step_weapons(
            &mut s,
            &input(Buttons::FIRE | Buttons::ALT_FIRE, 0, 1),
            true,
            &t,
            60,
        );
        assert_eq!(s.armed_ability(), Some(Ability::PinningArrow));
        let out = step_weapons(&mut s, &input(Buttons::NONE, 0, 1), true, &t, 60);
        assert_eq!(
            out,
            vec![FireIntent::Ability {
                weapon: WeaponKind::Longwick,
                ability: Ability::PinningArrow,
                charge: 1.0,
                phase: AbilityPhase::Use,
            }]
        );
        assert_eq!((s.draw, s.armed), (0, 0));
        assert!(s.alt_cooldown[0] > 0);
        // A tap after the recovery: a weak plain arrow.
        for _ in 0..30 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 1), true, &t, 60);
        }
        step_weapons(&mut s, &input(Buttons::FIRE, 0, 1), true, &t, 60);
        match step_weapons(&mut s, &input(Buttons::NONE, 0, 1), true, &t, 60).as_slice() {
            [FireIntent::Primary { charge, .. }] => assert!(*charge < 0.05),
            other => panic!("{other:?}"),
        }
        // The Blink Arrow (mode 0) rides a release, then stays live: ALT steps to it (the
        // press arms nothing meanwhile), and its cooldown starts then.
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Longwick.default_build()));
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        assert_eq!(s.armed_ability(), Some(Ability::BlinkArrow));
        for _ in 0..20 {
            step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60);
        }
        let out = step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        assert!(matches!(
            out.as_slice(),
            [FireIntent::Ability {
                ability: Ability::BlinkArrow,
                phase: AbilityPhase::Use,
                ..
            }]
        ));
        assert!(s.live[0] > 0 && s.alt_cooldown[0] == 0 && s.armed == 0);
        // The arrow stuck (the server shortens the window), then ALT: the step.
        s.shorten_live(0, t.blink_arrow.window, 60);
        assert_eq!(s.live[0], secs_to_ticks(t.blink_arrow.window, 60, 1));
        let out = step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        assert_eq!(
            out,
            vec![ability(
                WeaponKind::Longwick,
                Ability::BlinkArrow,
                AbilityPhase::Recast
            )]
        );
        assert_eq!(s.armed, 0);
        assert_eq!(
            s.alt_cooldown[0],
            secs_to_ticks(t.blink_arrow.cooldown, 60, 1)
        );
    }

    #[test]
    fn lancet_blooms_when_spammed() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Lancet.default_build()));
        let mut charges = Vec::new();
        for _ in 0..120 {
            for f in step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60) {
                if let FireIntent::Primary { charge, .. } = f {
                    charges.push(charge);
                }
            }
        }
        assert_eq!(charges[0], 0.0, "the first shot is exact");
        assert!(charges.windows(2).all(|w| w[1] >= w[0]));
        // Heat drains between shots: sustained fire settles at 75 % of the full bloom.
        assert_eq!(*charges.last().unwrap(), 0.75);
        // It cools down.
        for _ in 0..60 {
            step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        }
        assert_eq!(s.bloom, 0);
    }

    #[test]
    fn abilities_have_homes_and_power_is_relative() {
        for a in Ability::ALL {
            assert!(a.fits(a.home()), "{a:?}");
        }
        // The contract's homes (docs/rework.md): the first weapon whose default has it.
        let homes = [
            (Ability::Wisp, WeaponKind::Arclash),
            (Ability::Wickline, WeaponKind::Quill),
            (Ability::Tally, WeaponKind::Lancet),
            (Ability::Waxwork, WeaponKind::Lancet),
            (Ability::Bulwark, WeaponKind::Bellows),
            (Ability::Bolas, WeaponKind::Thresher),
            (Ability::Stoke, WeaponKind::Riveter),
            (Ability::Lantern, WeaponKind::Riveter),
            (Ability::Kite, WeaponKind::Cinderbox),
            (Ability::BlinkArrow, WeaponKind::Longwick),
            (Ability::BeaconArrow, WeaponKind::Longwick),
            (Ability::Riposte, WeaponKind::Tamper),
        ];
        for (a, home) in homes {
            assert_eq!(a.home(), home, "{a:?}");
        }
        let mut t = Tuning::default();
        // At home, and made of the weapon's own shots: unchanged.
        assert!(matches!(
            ability_tuning(&t, WeaponKind::Quill, Ability::Wickline),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            ability_tuning(&t, WeaponKind::Kiln, Ability::Lantern),
            Cow::Borrowed(_)
        ));
        t.combat.lancet.ability_power = 0.5;
        let scaled = ability_tuning(&t, WeaponKind::Lancet, Ability::Wickline);
        assert_eq!(scaled.combat.wickline.damage, 5.0);
        assert_eq!(scaled.combat.wickline.cooldown, 4.0);
        // The other way round: never above the listed numbers.
        t.combat.lancet.ability_power = 0.85;
        assert!(matches!(
            ability_tuning(&t, WeaponKind::Lamplighter, Ability::Tally),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn loadouts_pack_losslessly() {
        let mut loadouts = vec![
            Loadout::default(),
            Loadout::single(WeaponKind::Kiln.default_build()),
            Loadout::unarmed(),
        ];
        for (i, w) in WeaponKind::ALL.into_iter().enumerate() {
            for &a in w.abilities() {
                for &b in w.abilities() {
                    let build = WeaponBuild {
                        weapon: w,
                        abilities: [a, b],
                    };
                    loadouts.push(Loadout {
                        slots: [None, Some(build), None, Some(w.default_build())],
                        knack: Knack::ALL[i % Knack::COUNT],
                    });
                }
            }
        }
        // Every knack with every slot pattern (empty slots carry knack bits too).
        for knack in Knack::ALL {
            for mask in 0..16u8 {
                let mut l = Loadout::unarmed().with_knack(knack);
                for (i, slot) in l.slots.iter_mut().enumerate() {
                    if mask >> i & 1 == 1 {
                        *slot = Some(WeaponKind::ALL[i + 8].default_build());
                    }
                }
                loadouts.push(l);
            }
        }
        for l in loadouts {
            assert_eq!(packed_loadout::unpack(packed_loadout::pack(&l)), l);
        }
        // The packing of the old layout (no knack bits) is unchanged for the Thorn.
        let old = packed_loadout::pack(&Loadout::default());
        assert_eq!(old & 0x8000_8000_8000_8000, 0);
        let s = WeaponState::default();
        let bytes = crate::protocol::encode(&s);
        let back: WeaponState = crate::protocol::decode_server_message(&bytes).unwrap();
        assert_eq!(back, s);
        // Garbage decodes to something usable.
        let junk = packed_loadout::unpack(u64::MAX);
        assert_eq!(junk.slots, [None; 4]);
        assert_eq!(
            junk.knack,
            Knack::Thorn,
            "knack id 15 is unknown: the default"
        );
    }

    #[test]
    fn limpet_taps_throw_and_holds_detonate() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::single(WeaponKind::Cinderbox.default_build()));
        let hold = secs_to_ticks(t.limpet.detonate_hold, 60, 1);
        // A tap: fires on release, charge 0 (throw).
        step_weapons(&mut s, &input(Buttons::ALT_FIRE, 0, 0), true, &t, 60);
        let out = step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        assert!(matches!(
            out.as_slice(),
            [FireIntent::Ability { ability: Ability::Limpet, charge, .. }] if *charge == 0.0
        ));
        // A hold: fires once when the hold time is reached, charge 1 (detonate), nothing on
        // release.
        let mut fired = Vec::new();
        for _ in 0..hold + 20 {
            fired.extend(step_weapons(
                &mut s,
                &input(Buttons::ALT_FIRE, 0, 0),
                true,
                &t,
                60,
            ));
        }
        fired.extend(step_weapons(
            &mut s,
            &input(Buttons::NONE, 0, 0),
            true,
            &t,
            60,
        ));
        assert!(matches!(
            fired.as_slice(),
            [FireIntent::Ability { charge, .. }] if *charge == 1.0
        ));
    }

    #[test]
    fn unarmed_fires_only_the_fist_and_the_knack() {
        let t = CombatTuning::default();
        let mut s = WeaponState::new(Loadout::unarmed());
        assert!(s.is_unarmed() && s.held().is_none());
        let mut fired = Vec::new();
        for i in 0..120u8 {
            let mut b = Buttons::FIRE | Buttons::ALT_FIRE;
            b.set(Buttons::PUNCH, i % 30 == 0);
            b.set(Buttons::THROW, i == 5);
            // Every slot key and mode: there is nothing to select.
            fired.extend(step_weapons(&mut s, &input(b, i % 5, i % 3), true, &t, 60));
        }
        assert!(fired.contains(&FireIntent::Punch));
        assert!(fired.contains(&FireIntent::Knack(Knack::Thorn)));
        assert!(
            fired
                .iter()
                .all(|f| matches!(f, FireIntent::Punch | FireIntent::Knack(_))),
            "{fired:?}"
        );
        assert_eq!((s.charge, s.draw, s.burst.left), (0, 0, 0));
        let state = s.replicated_state(&t, 60);
        assert!(!crate::protocol::player_armed(state));
        // Armed again (a new loadout), everything works.
        let mut s = s.with_loadout(Loadout::default());
        assert!(!s.is_unarmed());
        let out = step_weapons(&mut s, &input(Buttons::NONE, 0, 0), true, &t, 60);
        assert!(out.is_empty());
        let out = step_weapons(&mut s, &input(Buttons::FIRE, 0, 0), true, &t, 60);
        assert_eq!(out, vec![primary(WeaponKind::Quill)]);
    }

    #[test]
    fn spread_stays_in_cone() {
        let mut rng = SpreadRng::new(5, PlayerId(1), 3);
        for _ in 0..200 {
            let d = spread_dir(Vec3::NEG_Z, 8.0, &mut rng);
            let angle = d.angle_between(Vec3::NEG_Z).to_degrees();
            assert!(angle <= 4.0 + 1e-3, "{angle}");
        }
    }
}
