//! The expansion arsenal's presentation (`docs/expansion.md`, area **arsenal-client**): which
//! build the local player is holding, and the visuals of the new projectiles, hazards and
//! status effects, keyed on small enums ([`ArsenalProjectile`], [`Hazard`], [`Status`]) so the
//! replicated kinds / flags map onto them in one place.

use shared::loadout::{Ability, Loadout, WeaponBuild, WeaponKind};

use super::super::InGame;

impl InGame {
    /// The loadout the local player carries right now. Until the simulation carries loadouts
    /// (`OwnState`'s weapon state), that is the fixed default four on slots 1–4.
    pub(crate) fn carried_loadout(&self) -> Loadout {
        Loadout::default()
    }

    /// The build in the selected slot (an empty slot shows the first weapon carried).
    pub(crate) fn held_build(&self) -> WeaponBuild {
        let loadout = self.carried_loadout();
        loadout
            .slot(self.controls.weapons.current)
            .or_else(|| loadout.builds().next())
            .copied()
            .unwrap_or_default()
    }

    /// The held weapon and its selected ability.
    pub(crate) fn held_weapon(&self) -> (WeaponKind, Ability) {
        let build = self.held_build();
        (build.weapon, build.ability(self.controls.weapons.mode()))
    }
}
