//! Loadouts in the world (`docs/arsenal.md`, "Loadouts"): the loadout each player asked for,
//! the loadout a game mode forces on them (gun game) and when either takes effect.
//!
//! What a player carries right now is `WeaponState::loadout` (predicted state: the owner's
//! client sees it in `OwnState::combat.weapons`). Requests live in
//! [`SimWorld::loadout_requests`], which the server carries from one world to the next (lobby →
//! match → lobby), so a loadout chosen in the lobby is there when the match spawns everyone.
//! Forced loadouts belong to one world ([`SimWorld::forced_loadouts`]).
//!
//! Rules: a request applies at once while the player is dead (downed included), in the warm-up
//! ([`SimWorld::warmup`]), or has a fresh body that hasn't acted yet (just joined); otherwise at
//! the player's next (re)spawn (`WeaponState::respawned`, from [`SimWorld::spawn_loadout`]). A
//! forced loadout applies at once and wins over the request until it is lifted.

use super::{PlayerId, SimWorld, WeaponState};
use crate::loadout::Loadout;

impl SimWorld {
    /// Stores the player's chosen loadout (sanitised); applies it per the rules above. Works
    /// for players without a body too (lobby): it is what they spawn with.
    pub fn request_loadout(&mut self, id: PlayerId, loadout: Loadout) {
        let loadout = loadout.sanitized();
        self.loadout_requests.insert(id, loadout);
        if self.forced_loadouts.contains_key(&id) {
            return;
        }
        let warmup = self.warmup;
        if let Some(p) = self.players.get_mut(&id)
            && (p.combat.dead || warmup || p.inputs_processed == 0)
            && p.combat.weapons.loadout != loadout
        {
            p.combat.weapons = p.combat.weapons.with_loadout(loadout);
        }
    }

    /// Forces a loadout now (gun game tier changes): replaces the weapons immediately, selects
    /// the first slot, keeps nothing of the old weapons' state. `None` lifts the override (the
    /// requested loadout applies at the next spawn).
    pub fn force_loadout(&mut self, id: PlayerId, loadout: Option<Loadout>) {
        let Some(loadout) = loadout.map(Loadout::sanitized) else {
            self.forced_loadouts.remove(&id);
            return;
        };
        self.forced_loadouts.insert(id, loadout);
        if let Some(p) = self.players.get_mut(&id) {
            p.combat.weapons = WeaponState {
                slot: loadout.first_slot().unwrap_or(0),
                ..p.combat.weapons.with_loadout(loadout)
            };
        }
    }

    /// The loadout the player is carrying right now (without a body: the one they would
    /// spawn with).
    pub fn loadout(&self, id: PlayerId) -> Loadout {
        match self.players.get(&id) {
            Some(p) => p.combat.weapons.loadout,
            None => self.spawn_loadout(id),
        }
    }

    /// The loadout `id` gets at its next (re)spawn: the forced one, else the requested one,
    /// else the default.
    pub fn spawn_loadout(&self, id: PlayerId) -> Loadout {
        self.forced_loadouts
            .get(&id)
            .or_else(|| self.loadout_requests.get(&id))
            .copied()
            .unwrap_or_default()
    }

    /// Forgets `id`'s requested and forced loadouts (the player left).
    pub(crate) fn forget_loadouts(&mut self, id: PlayerId) {
        self.loadout_requests.remove(&id);
        self.forced_loadouts.remove(&id);
    }
}
