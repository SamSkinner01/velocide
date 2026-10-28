//! The rework's abilities (`docs/rework.md` §1), one file each, and the shared **guard**.
//!
//! [`SimWorld::resolve_intent`] (`weapons.rs`) hands every new ability's intents to
//! [`SimWorld::cast_ability`], which dispatches to its file with an [`AbilityCast`]: the
//! tuning already scaled for the carrying weapon (`weapons::ability_tuning`), the
//! lag-compensation history, the player, the input that fired it, the weapon, the charge /
//! draw and the [`AbilityPhase`] (a use, a held ability's release, the second press of a live
//! one). How each ability is *pressed* (held, armed on the bow, live and recast...) is decided
//! by the predicted `step_weapons`; what it *does* is decided here, on the server.
//!
//! State an ability keeps between ticks lives in [`AbilityWorld`] ([`SimWorld::abilities`]),
//! one field per ability whose type its own file defines; [`SimWorld::step_abilities`] runs
//! every file's per-tick step (pipeline step 4, after the knacks), and replication adds their
//! entities and flags ([`SimWorld::replicate_abilities`], [`SimWorld::decorate_player_abilities`],
//! [`SimWorld::decorate_enemy_abilities`]).
//!
//! | file | ability | class |
//! |---|---|---|
//! | `wickline.rs` | Wickline | Precision |
//! | `tally.rs` | Tally | Precision |
//! | `waxwork.rs` | Waxwork (the decoy; `select_target` prefers it) | Precision |
//! | `bolas.rs` | Bolas | Scatter |
//! | `bulwark.rs` | Bulwark (the guard's slab) | Scatter |
//! | `wisp.rs` | Wisp (orbiting, flung) | Rapid |
//! | `stoke.rs` | Stoke (its rate and overheat are `WeaponState`'s) | Rapid |
//! | `lantern.rs` | Lantern | Rapid |
//! | `kite.rs` | Kite | Explosive |
//! | `blink_arrow.rs` | Blink Arrow | Draw |
//! | `beacon_arrow.rs` | Beacon Arrow (`REVEALED`) | Draw |
//! | `hurl.rs` | Hurl's return leg | Melee |
//! | `riposte.rs` | Riposte (the guard's counter) | Melee |
//! | `guard.rs` | the guard Bulwark and Riposte raise | |

pub mod beacon_arrow;
pub mod blink_arrow;
pub mod bolas;
pub mod bulwark;
pub mod guard;
pub mod hurl;
pub mod kite;
pub mod lantern;
pub mod riposte;
pub mod stoke;
pub mod tally;
pub mod waxwork;
pub mod wickline;
pub mod wisp;

use serde::{Deserialize, Serialize};

use super::{DamageEvent, HitTarget, HitboxHistory, PlayerId, SimWorld};
use crate::input::PlayerInput;
use crate::loadout::{Ability, WeaponKind};
use crate::protocol::{EntityState, NetFrame, NetId};
use crate::tuning::Tuning;

/// Which part of an ability an intent is (`FireIntent::Ability::phase`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AbilityPhase {
    /// Its use: every ability's one intent, a held one's press (Tally, Bulwark, Riposte), the
    /// bow's release of an armed arrow.
    #[default]
    Use,
    /// A held ability let go, or held to its limit (or the weapon / ability switched away):
    /// `charge` = how far it was held (0..=1).
    Release,
    /// A live ability pressed again (`Ability::recasts`): the Wisp flung, the Blink Arrow's
    /// step.
    Recast,
}

/// One ability intent being resolved (see the module docs).
pub struct AbilityCast<'a> {
    /// Already scaled for the carrying weapon (`weapons::ability_tuning`).
    pub t: &'a Tuning,
    pub history: Option<&'a dyn HitboxHistory>,
    pub id: PlayerId,
    pub input: &'a PlayerInput,
    pub weapon: WeaponKind,
    pub ability: Ability,
    /// The bow's draw for an arrow, how long a held ability was held; 0 otherwise.
    pub charge: f32,
    pub phase: AbilityPhase,
}

/// State the rework's abilities keep between ticks ([`SimWorld::abilities`]), one field per
/// ability (each type lives in its file).
#[derive(Clone, Debug, Default)]
pub struct AbilityWorld {
    pub guards: guard::Guards,
    pub wickline: wickline::Wicklines,
    pub tally: tally::Tallies,
    pub waxwork: waxwork::Waxworks,
    pub bolas: bolas::BolasWorld,
    pub bulwark: bulwark::Bulwarks,
    pub wisp: wisp::Wisps,
    pub stoke: stoke::Stokes,
    pub lantern: lantern::Lanterns,
    pub kite: kite::Kites,
    pub blink: blink_arrow::BlinkArrows,
    pub beacon: beacon_arrow::Beacons,
    pub hurl: hurl::Hurls,
    pub riposte: riposte::Ripostes,
}

impl AbilityWorld {
    /// Everything goes (level restart).
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Forgets everything of player `id` (they left).
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.guards.forget(id);
        self.wickline.forget(id);
        self.tally.forget(id);
        self.waxwork.forget(id);
        self.bolas.forget(id);
        self.bulwark.forget(id);
        self.wisp.forget(id);
        self.stoke.forget(id);
        self.lantern.forget(id);
        self.kite.forget(id);
        self.blink.forget(id);
        self.beacon.forget(id);
        self.hurl.forget(id);
        self.riposte.forget(id);
    }

    /// Replicated entities they hold (for `SimWorld::alloc_net_id`).
    pub(crate) fn len(&self) -> usize {
        self.waxwork.len() + self.lantern.len()
    }

    /// Does one of them hold entity id `id`?
    pub(crate) fn holds(&self, id: NetId) -> bool {
        self.waxwork.holds(id) || self.lantern.holds(id)
    }
}

impl SimWorld {
    /// Resolves one of the rework's ability intents (`weapons.rs` sends every ability whose
    /// [`Ability::is_rework`] is true here).
    pub(crate) fn cast_ability(&mut self, c: &AbilityCast) {
        match c.ability {
            Ability::Wickline => self.cast_wickline(c),
            Ability::Tally => self.cast_tally(c),
            Ability::Waxwork => self.cast_waxwork(c),
            Ability::Bolas => self.cast_bolas(c),
            Ability::Bulwark => self.cast_bulwark(c),
            Ability::Wisp => self.cast_wisp(c),
            Ability::Stoke => self.cast_stoke(c),
            Ability::Lantern => self.cast_lantern(c),
            Ability::Kite => self.cast_kite(c),
            Ability::BlinkArrow => self.cast_blink_arrow(c),
            Ability::BeaconArrow => self.cast_beacon_arrow(c),
            Ability::Riposte => self.cast_riposte(c),
            // The Hurl's throw is `melee.rs`'s; its return leg is `hurl.rs`'s projectile step.
            _ => {}
        }
    }

    /// Per tick (pipeline step 4): every ability's own step.
    pub(crate) fn step_abilities(&mut self, t: &Tuning) {
        self.step_guards(t);
        self.step_wickline(t);
        self.step_tally(t);
        self.step_waxwork(t);
        self.step_bolas(t);
        self.step_bulwark(t);
        self.step_wisp(t);
        self.step_stoke(t);
        self.step_lantern(t);
        self.step_kite(t);
        self.step_blink_arrow(t);
        self.step_beacon_arrow(t);
        self.step_hurl(t);
        self.step_riposte(t);
    }

    /// After every input of player `id` (pipeline step 1, with the lag-compensation history):
    /// the held abilities that look at what the player aims at every tick (the Tally).
    pub(crate) fn abilities_after_input(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) {
        self.tally_sweep(t, history, id, input);
    }

    /// After every damage instance that went through (`apply_damage`): the Bolas' cord shares
    /// it, a stoked weapon's hit sets Burn.
    pub(crate) fn abilities_after_damage(&mut self, t: &Tuning, event: &DamageEvent) {
        self.bolas_after_damage(t, event);
        self.stoke_after_damage(t, event);
    }

    /// Writes the abilities' own entities (the Waxwork's double, the Lantern) into `frame`.
    pub(crate) fn replicate_abilities(&self, t: &Tuning, frame: &mut NetFrame) {
        self.replicate_waxworks(t, frame);
        self.replicate_lanterns(t, frame);
    }

    /// Ability flags of a player entity: `GUARDING` (and its strength in `aux`), `REVEALED`,
    /// `BOUND`.
    pub(crate) fn decorate_player_abilities(&self, t: &Tuning, e: &mut EntityState) {
        let Some(id) = e.id.as_player() else {
            return;
        };
        self.decorate_guard(t, id, e);
        let target = HitTarget::Player(id);
        if self.is_revealed(target) {
            e.flags |= crate::protocol::player_flags::REVEALED;
        }
        if self.is_bound(target) {
            e.flags |= crate::protocol::player_flags::BOUND;
        }
    }

    /// Ability flags of an enemy entity: `REVEALED`, `BOUND`, `LURED`.
    pub(crate) fn decorate_enemy_abilities(&self, id: NetId, e: &mut EntityState) {
        use crate::protocol::enemy_flags;
        let target = HitTarget::Entity(id);
        if self.is_revealed(target) {
            e.flags |= enemy_flags::REVEALED;
        }
        if self.is_bound(target) {
            e.flags |= enemy_flags::BOUND;
        }
        if self.is_lured(id) {
            e.flags |= enemy_flags::LURED;
        }
    }
}
