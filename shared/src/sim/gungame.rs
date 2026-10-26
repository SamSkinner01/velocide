//! Gun game (`GameMode::GunGame`): a PvP free-for-all on an arena (up to 8 players) where
//! everyone climbs the same fixed ladder of weapon builds (`gun_game.ladder` in the tuning,
//! [`GunGameTuning`]), then the **Fist**. Full rules: `docs/style_modes.md`.
//!
//! * Each player carries only their tier's build: a forced loadout
//!   (`SimWorld::force_loadout`, the arsenal's API), kept through respawns.
//! * A kill with the weapon of your tier (`KillInfo::weapon`: the weapon that fired the
//!   killing blow, abilities included) moves you up a tier at once. Kills with anything else
//!   (a bomb of your previous weapon still bouncing, a slam, a knock into a kill volume) don't.
//! * A punch kill (`Attack::Fist`) knocks the victim down `punch_demotes` tiers, whatever tier
//!   either of you is on. A punch kill on the Fist tier wins the match (`EndReason::Ladder`).
//! * Respawns, spawn protection and the time limit are deathmatch's; when time runs out the
//!   most tiers wins (then kills, fewer deaths, style). Late joiners start on the lowest tier
//!   anyone holds. Thorns are off (the PvP balance profile).
//!
//! Tiers ride on the scoreboard (`ScoreEntry::tier`) and the results (`PlayerStats::tier`,
//! [`GunGameResult`]); promotions and demotions are [`GunGameEvent`]s to everyone.

use serde::{Deserialize, Serialize};

use super::combat::{Attack, KillInfo, Owner};
use super::modes::{EndReason, ModeEvent};
use super::{HitTarget, PlayerId, SimEvent, SimWorld};
use crate::loadout::{Loadout, WeaponBuild, WeaponKind};
use crate::tuning::{GunGameTuning, Tuning};

/// Gun game events (broadcast as `ModeEvent::GunGame`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GunGameEvent {
    /// `player` moved up to `tier` by killing `victim` (`tier` = ladder length: the Fist).
    Promoted {
        player: PlayerId,
        tier: u8,
        victim: PlayerId,
    },
    /// `player` was punched down to `tier` by `by`.
    Demoted {
        player: PlayerId,
        tier: u8,
        by: PlayerId,
    },
}

/// A gun game's ladder for the results screen (`MatchResults::gun_game`); each player's tier
/// is in their stats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GunGameResult {
    /// The weapon of every tier, first to last (the Fist comes after).
    pub ladder: Vec<WeaponKind>,
}

/// What a tier of the ladder is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Weapon(WeaponBuild),
    /// After the last weapon: a punch kill wins.
    Fist,
    /// Past the Fist: the match winner.
    Won,
}

/// Index of the Fist tier (the ladder length).
pub fn fist_tier(g: &GunGameTuning) -> u8 {
    g.ladder.len().min(u8::MAX as usize - 1) as u8
}

/// Tier `tier` of `g`'s ladder.
pub fn tier_of(g: &GunGameTuning, tier: u8) -> Tier {
    let fist = fist_tier(g);
    match g.ladder.get(tier as usize) {
        Some(build) if tier < fist => Tier::Weapon(*build),
        _ if tier == fist => Tier::Fist,
        _ => Tier::Won,
    }
}

/// The loadout of tier `tier`: its build alone. On the Fist tier the last build stays in hand
/// (an unarmed loadout needs support from the arsenal: until then only a punch counts there).
pub fn tier_loadout(g: &GunGameTuning, tier: u8) -> Loadout {
    let build = match tier_of(g, tier) {
        Tier::Weapon(build) => build,
        Tier::Fist | Tier::Won => g.ladder.last().copied().unwrap_or_default(),
    };
    Loadout::single(build)
}

impl SimWorld {
    /// `id`'s gun game tier.
    pub fn gun_game_tier(&self, id: PlayerId) -> u8 {
        self.mode.stats(id).tier
    }

    /// Gun game step (pipeline step 6, while the match is playing): promotions and demotions
    /// from this tick's kills, a ladder win, everyone's loadout kept on their tier.
    pub(crate) fn step_gun_game(&mut self, t: &Tuning, events: &[SimEvent]) {
        for event in events {
            if let SimEvent::Killed(k) = event {
                self.gun_game_kill(t, k);
                if !self.mode.is_playing() {
                    return;
                }
            }
        }
        // Everyone carries their tier's build (joins, restarts, a hot-reloaded ladder).
        let ids: Vec<PlayerId> = self.players.keys().copied().collect();
        for id in ids {
            let want = tier_loadout(&t.gun_game, self.gun_game_tier(id));
            if self.forced_loadouts.get(&id) != Some(&want) {
                self.force_loadout(id, Some(want));
            }
        }
    }

    fn gun_game_kill(&mut self, t: &Tuning, k: &KillInfo) {
        let g = &t.gun_game;
        let (HitTarget::Player(victim), Owner::Player(killer)) = (k.victim, k.killer) else {
            return;
        };
        if killer == victim || !self.players.contains_key(&killer) {
            return;
        }
        let tier = self.gun_game_tier(killer);
        if k.attack == Attack::Fist {
            // A punch kill knocks the victim down, whatever tier either is on ...
            let before = self.gun_game_tier(victim);
            let after = before.saturating_sub(g.punch_demotes.min(u8::MAX as u32) as u8);
            if after != before {
                self.set_gun_game_tier(t, victim, after);
                self.push_mode(ModeEvent::GunGame(GunGameEvent::Demoted {
                    player: victim,
                    tier: after,
                    by: killer,
                }));
            }
            // ... and on the Fist tier it wins.
            if tier_of(g, tier) == Tier::Fist {
                self.mode.player(killer).stats.tier = tier.saturating_add(1);
                self.mode.touch();
                self.end_match(t, EndReason::Ladder);
            }
            return;
        }
        if let Tier::Weapon(build) = tier_of(g, tier)
            && k.weapon == Some(build.weapon)
        {
            let up = tier + 1;
            self.set_gun_game_tier(t, killer, up);
            self.push_mode(ModeEvent::GunGame(GunGameEvent::Promoted {
                player: killer,
                tier: up,
                victim,
            }));
        }
    }

    /// Moves `id` to `tier` and hands them its build at once.
    fn set_gun_game_tier(&mut self, t: &Tuning, id: PlayerId, tier: u8) {
        self.mode.player(id).stats.tier = tier;
        self.mode.touch();
        self.force_loadout(id, Some(tier_loadout(&t.gun_game, tier)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ladder_has_every_weapon_once_then_the_fist() {
        let g = GunGameTuning::default();
        assert_eq!(g.ladder.len(), WeaponKind::COUNT);
        for w in WeaponKind::ALL {
            assert_eq!(
                g.ladder.iter().filter(|b| b.weapon == w).count(),
                1,
                "{w:?}"
            );
        }
        assert!(g.ladder.iter().all(WeaponBuild::is_valid));
        let fist = fist_tier(&g);
        assert_eq!(fist, 10);
        assert_eq!(tier_of(&g, 0), Tier::Weapon(g.ladder[0]));
        assert_eq!(tier_of(&g, fist), Tier::Fist);
        assert_eq!(tier_of(&g, fist + 1), Tier::Won);
        assert_eq!(tier_loadout(&g, 3), Loadout::single(g.ladder[3]));
        assert!(tier_loadout(&g, 3).is_valid());
        assert_eq!(tier_loadout(&g, fist), Loadout::single(g.ladder[9]));
    }
}
