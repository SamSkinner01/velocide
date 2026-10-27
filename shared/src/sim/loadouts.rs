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
//! forced loadout applies at once and wins over the request until it is lifted. Requests are
//! always sanitised to at least one weapon; only a forced loadout may be
//! [`Loadout::unarmed`] (fists only).
//!
//! The knack: whatever applies goes through [`SimWorld::fit_loadout`] first, so in a PvP world
//! (a PvP match or the warm-up before one) a knack that isn't `Knack::pvp_allowed` (the Thorn)
//! is replaced by `Knack::PVP_FALLBACK`; the stored request keeps the player's pick.

use super::{PlayerId, SimWorld, WeaponState};
use crate::loadout::{Knack, Loadout};

impl SimWorld {
    /// Stores the player's chosen loadout (sanitised); applies it per the rules above. Works
    /// for players without a body too (lobby): it is what they spawn with.
    pub fn request_loadout(&mut self, id: PlayerId, loadout: Loadout) {
        let loadout = loadout.sanitized();
        self.loadout_requests.insert(id, loadout);
        if self.forced_loadouts.contains_key(&id) {
            return;
        }
        let loadout = self.fit_loadout(loadout);
        let warmup = self.warmup;
        if let Some(p) = self.players.get_mut(&id)
            && (p.combat.dead || warmup || p.inputs_processed == 0)
            && p.combat.weapons.loadout != loadout
        {
            p.combat.weapons = p.combat.weapons.with_loadout(loadout);
        }
    }

    /// Forces a loadout now (gun game tier changes): replaces the weapons immediately, selects
    /// the first slot, keeps nothing of the old weapons' state. [`Loadout::unarmed`] is kept
    /// as it is (fists only); anything else is sanitised. `None` lifts the override (the
    /// requested loadout applies at the next spawn).
    pub fn force_loadout(&mut self, id: PlayerId, loadout: Option<Loadout>) {
        let sanitize = |l: Loadout| if l.is_unarmed() { l } else { l.sanitized() };
        let Some(loadout) = loadout.map(sanitize) else {
            self.forced_loadouts.remove(&id);
            return;
        };
        self.forced_loadouts.insert(id, loadout);
        let loadout = self.fit_loadout(loadout);
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
    /// else the default (fitted to this world, [`SimWorld::fit_loadout`]).
    pub fn spawn_loadout(&self, id: PlayerId) -> Loadout {
        let loadout = self
            .forced_loadouts
            .get(&id)
            .or_else(|| self.loadout_requests.get(&id))
            .copied()
            .unwrap_or_default();
        self.fit_loadout(loadout)
    }

    /// The knack `id` picked (their request's; the default without one). Game modes that
    /// force weapons keep it (gun game).
    pub fn requested_knack(&self, id: PlayerId) -> Knack {
        self.loadout_requests
            .get(&id)
            .map(|l| l.knack)
            .unwrap_or_default()
    }

    /// A PvP world: a PvP match, or the warm-up before one.
    pub fn pvp_world(&self) -> bool {
        match &self.warmup_rules {
            Some(w) => w.pvp,
            None => self.mode.rules.kind.is_pvp(),
        }
    }

    /// `loadout` as it applies in this world: in a PvP world a knack that isn't
    /// `Knack::pvp_allowed` becomes `Knack::PVP_FALLBACK`.
    pub fn fit_loadout(&self, loadout: Loadout) -> Loadout {
        loadout.with_knack(loadout.knack.for_world(self.pvp_world()))
    }

    /// Applies the knack rule again to everyone carrying a loadout (the world turned PvP or
    /// PvE: the warm-up's next mode changed). A changed knack starts with full charges.
    pub(crate) fn refit_knacks(&mut self) {
        let ids: Vec<PlayerId> = self.players.keys().copied().collect();
        for id in ids {
            let want = self
                .forced_loadouts
                .get(&id)
                .or_else(|| self.loadout_requests.get(&id))
                .map_or_else(|| self.requested_knack(id), |l| l.knack);
            let knack = want.for_world(self.pvp_world());
            if let Some(p) = self.players.get_mut(&id)
                && p.combat.weapons.knack() != knack
            {
                let w = &mut p.combat.weapons;
                *w = w.with_loadout(w.loadout.with_knack(knack));
            }
        }
    }

    /// Forgets `id`'s requested and forced loadouts (the player left).
    pub(crate) fn forget_loadouts(&mut self, id: PlayerId) {
        self.loadout_requests.remove(&id);
        self.forced_loadouts.remove(&id);
    }
}
