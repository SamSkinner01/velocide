//! Stoke (Rapid, damage; `docs/rework.md` §1): for `stoke.duration` the weapon's primary fires
//! at `stoke.rate` × its rate and every hit sets `stoke.burn_stacks` Burn stacks (the Kiln,
//! which already burns: `stoke.kiln_reach` × its cone's reach instead); then it overheats for
//! `stoke.overheat` (no primary; the other ability, the fist and the knack still work).
//!
//! The rate and the overheat are predicted: `WeaponState::stoke` / `overheat` per slot,
//! `step_weapons` (`player_flags::STOKED` / `OVERHEATED`). The server side is what a stoked hit
//! does: [`SimWorld::stoke_after_damage`] sets the Burn (as `Attack::Stoke`, kill credit to
//! the shooter) on whatever a hit of the stoked weapon's primary reached — a ray, a rivet, a
//! zap and its chain, a relayed shot — and [`SimWorld::kiln_reach`] stretches the Kiln's spray.
//! It is judged when the hit lands: a rivet still flying when the Stoke runs out burns nothing.

use super::super::combat::{Attack, DamageFlags, Owner};
use super::super::{DamageEvent, PlayerId, SimWorld};
use super::AbilityCast;
use crate::loadout::WeaponKind;
use crate::tuning::Tuning;

/// Server-side Stoke state (none: the timers are the weapon state's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stokes {}

impl Stokes {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// A Stoke use (`step_weapons` already stoked the weapon).
    pub(crate) fn cast_stoke(&mut self, _c: &AbilityCast) {}

    /// Is player `id`'s `weapon` stoked right now (its slot's `WeaponState::stoke`)?
    pub fn is_stoked(&self, id: PlayerId, weapon: WeaponKind) -> bool {
        let Some(p) = self.players.get(&id) else {
            return false;
        };
        let w = &p.combat.weapons;
        w.loadout
            .slots
            .iter()
            .zip(w.stoke.iter())
            .any(|(slot, left)| *left > 0 && slot.is_some_and(|b| b.weapon == weapon))
    }

    /// The Kiln's spray reach for player `id`: `kiln.range`, × `stoke.kiln_reach` stoked.
    pub(crate) fn kiln_reach(&self, t: &Tuning, id: PlayerId) -> f32 {
        let c = &t.combat;
        if self.is_stoked(id, WeaponKind::Kiln) {
            c.kiln.range * c.stoke.kiln_reach.max(0.0)
        } else {
            c.kiln.range
        }
    }

    /// After every damage instance: a hit of a stoked weapon's primary (not the Kiln's, which
    /// burns anyway, and never a Burn tick) sets `stoke.burn_stacks` Burn stacks.
    pub(crate) fn stoke_after_damage(&mut self, t: &Tuning, event: &DamageEvent) {
        let Owner::Player(id) = event.info.source else {
            return;
        };
        // Not a Burn tick, nor a Bolas cord's share of a hit (`bolas.rs`).
        if event.info.flags.contains(DamageFlags::BURN)
            || event.killed
            || self.abilities.bolas.sharing
        {
            return;
        }
        let Some(weapon) = event.info.attack.primary_of() else {
            return;
        };
        if weapon == WeaponKind::Kiln || !self.is_stoked(id, weapon) {
            return;
        }
        let stacks = t.combat.stoke.burn_stacks;
        self.apply_burn(t, Owner::Player(id), Attack::Stoke, event.target, stacks);
    }

    /// Per tick.
    pub(crate) fn step_stoke(&mut self, _t: &Tuning) {}
}
