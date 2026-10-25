//! Brands: the Quill's alt A marks what it hits. A branded target takes
//! `quill.brand_multiplier` × damage from every source for `quill.brand_duration` (applied in
//! the damage pipeline, [`SimWorld::apply_damage`]), and Thorn ricochets and Arclash chains
//! pick branded targets first.
//!
//! Brands live in a side table ([`SimWorld::brands`], keyed by the target) so neither players
//! nor enemies carry brand state. One live brand per player: a new brand replaces the
//! player's old one, and a target holds the newest brand put on it. [`SimWorld::replicate`]
//! ORs the `BRANDED` flag (bit 15 of `player_flags` / `enemy_flags`) into the target's entity
//! so every client draws the sigil.

use glam::Vec3;

use super::combat::secs_to_ticks;
use super::{HitTarget, PlayerId, SimEvent, SimWorld};
use crate::tuning::Tuning;

/// One live brand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brand {
    /// The player who branded the target.
    pub owner: PlayerId,
    /// The brand is gone at this tick.
    pub expire_tick: u32,
}

impl SimWorld {
    /// Is `target` branded right now?
    pub fn is_branded(&self, target: HitTarget) -> bool {
        self.brands
            .get(&target)
            .is_some_and(|b| self.tick < b.expire_tick)
    }

    /// `owner` brands `target` (hit at `point`) for `quill.brand_duration`, replacing the
    /// owner's previous brand. Targets that are dead (or that the owner may not hurt) are not
    /// branded. Emits [`SimEvent::Branded`].
    pub fn brand_target(&mut self, t: &Tuning, owner: PlayerId, target: HitTarget, point: Vec3) {
        if !self.can_hit(super::Owner::Player(owner), target) {
            return;
        }
        let ticks = secs_to_ticks(t.combat.quill.brand_duration, self.tick_rate, 1) as u32;
        self.brands.retain(|_, b| b.owner != owner);
        self.brands.insert(
            target,
            Brand {
                owner,
                expire_tick: self.tick + ticks,
            },
        );
        self.events.push(SimEvent::Branded {
            owner,
            target,
            position: point,
        });
    }

    /// Drops expired brands and brands on targets that are gone.
    pub(crate) fn step_brands(&mut self) {
        let now = self.tick;
        let players = &self.players;
        let enemies = &self.enemies;
        self.brands.retain(|target, b| {
            let alive = match target {
                HitTarget::Player(id) => players.get(id).is_some_and(|p| !p.combat.dead),
                HitTarget::Entity(id) => enemies.get(id).is_some_and(|e| e.is_alive()),
            };
            alive && now < b.expire_tick
        });
    }
}
