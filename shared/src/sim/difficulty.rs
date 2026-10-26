//! PvE difficulty and co-op scaling: what the match's [`Difficulty`] (EASY .. INFERNO) and the
//! number of players do to the Moulds and to the co-op rules, in campaign encounters and grind
//! waves alike. Numbers: the `difficulty` section of the tuning ([`DifficultyTuning`]).
//!
//! With `d` the difficulty's table entries and `p` the co-op player count
//! ([`SimWorld::coop_player_count`]):
//!
//! | what | value |
//! |---|---|
//! | enemy health (at spawn) | `health[d] × (1 + player_health (p − 1))` |
//! | enemies per wave | `count[d] × (1 + player_count (p − 1))` (rounded, ≥ 1 per authored group) |
//! | damage, aggression | `damage[d] / aggression[d] × (1 + player_… (p − 1))` → [`SimWorld::enemy_scale`] |
//! | projectile speed | `projectile_speed[d]` → [`SimWorld::enemy_scale`] |
//! | elite chance | `elite_chance[d] + player_elite_chance (p − 1)` |
//! | downed time | `modes.downed_time × downed_time[d] × (1 − player_downed_time (p − 2))`, ≥ `min_downed_time` |
//! | revive health | `modes.revive_health × revive_health[d]` |
//!
//! [`SimWorld::step_modes`] recomputes `enemy_scale` every tick (players joining or leaving
//! change it at once); spawns read [`SimWorld::pve_scaling`] when they happen. PvP modes get no
//! scaling. The enemies area applies `enemy_scale` inside the AI and provides elites; until its
//! elite spawn lands, an elite roll spawns a normal Mould ([`SimWorld::spawn_pve_enemy`]).

use glam::Vec3;

use super::SimWorld;
use super::combat::secs_to_ticks;
use super::enemy::EnemyScale;
use crate::lobby::Difficulty;
use crate::protocol::NetId;
use crate::tuning::{DifficultyTuning, Tuning};

/// Everything the difficulty and the player count decide, for spawns happening now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PveScaling {
    pub difficulty: Difficulty,
    /// Players it was computed for.
    pub players: u32,
    /// Enemy health multiplier (at spawn).
    pub health: f32,
    /// Enemies per authored spawn / grind budget multiplier.
    pub count: f32,
    /// Chance (0..1) that a spawned Mould is an elite.
    pub elite_chance: f32,
    /// Behaviour multipliers for the AI (`SimWorld::enemy_scale`).
    pub enemy: EnemyScale,
    /// Co-op: seconds a downed player waits for a revive.
    pub downed_time: f32,
    /// Co-op: health after a revive.
    pub revive_health: f32,
}

impl PveScaling {
    /// No scaling at all (PvP modes): the enemy definitions and the `modes` values as written.
    pub fn none(t: &Tuning) -> Self {
        Self {
            difficulty: Difficulty::Normal,
            players: 1,
            health: 1.0,
            count: 1.0,
            elite_chance: 0.0,
            enemy: EnemyScale::default(),
            downed_time: t.modes.downed_time,
            revive_health: t.modes.revive_health,
        }
    }

    /// The scaling for `difficulty` with `players` players (see the module docs).
    pub fn compute(t: &Tuning, difficulty: Difficulty, players: u32) -> Self {
        let d: &DifficultyTuning = &t.difficulty;
        let i = difficulty.index().min(4);
        let extra = players.max(1).saturating_sub(1) as f32;
        let per = |x: f32| 1.0 + x * extra;
        let beyond_two = players.saturating_sub(2) as f32;
        let downed = t.modes.downed_time
            * d.downed_time[i]
            * (1.0 - d.player_downed_time * beyond_two).max(0.0);
        Self {
            difficulty,
            players: players.max(1),
            health: d.health[i] * per(d.player_health),
            count: d.count[i] * per(d.player_count),
            elite_chance: (d.elite_chance[i] + d.player_elite_chance * extra).clamp(0.0, 1.0),
            enemy: EnemyScale {
                damage: d.damage[i] * per(d.player_damage),
                aggression: d.aggression[i] * per(d.player_aggression),
                projectile_speed: d.projectile_speed[i],
            },
            downed_time: downed.max(d.min_downed_time.min(t.modes.downed_time)),
            revive_health: (t.modes.revive_health * d.revive_health[i])
                .clamp(1.0, t.combat.max_health),
        }
    }

    /// Enemies for a wave of `n` authored spawns: `round(n × count)`, at least one when the
    /// wave has any.
    pub fn scaled_count(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        ((n as f32 * self.count).round() as usize).max(1)
    }
}

/// Deterministic pseudo-random numbers for the mode rules (splitmix64): elite rolls, grind
/// wave contents and spawn points. Seeded from the tick and a salt, so the server is the only
/// one that needs to run it and replays stay identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRng(u64);

impl ModeRng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0xD1B5_4A32_D192_ED03)
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

    /// Uniform in `0..n` (0 for `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        p > 0.0 && self.next_f32() < p
    }
}

/// A 64-bit FNV-1a hash of a name (level names seed the grind's random numbers).
pub fn name_seed(name: &str) -> u64 {
    name.bytes().fold(0xCBF2_9CE4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

impl SimWorld {
    /// The difficulty × player count scaling of the running match, for spawns happening now.
    /// PvP modes: none. Players count only with `rules.coop_enemy_scaling`.
    pub fn pve_scaling(&self, t: &Tuning) -> PveScaling {
        if !self.mode.rules.kind.has_difficulty() {
            return PveScaling::none(t);
        }
        let players = if self.rules.coop_enemy_scaling {
            self.coop_player_count()
        } else {
            1
        };
        PveScaling::compute(t, self.mode.rules.difficulty, players)
    }

    /// Puts the current scaling into [`SimWorld::enemy_scale`] (every tick: a player joining or
    /// leaving changes the AI's damage / aggression right away).
    pub(crate) fn update_enemy_scale(&mut self, t: &Tuning) {
        self.enemy_scale = self.pve_scaling(t).enemy;
    }

    /// Ticks a downed co-op player waits for a revive (difficulty × players).
    pub(crate) fn downed_ticks(&self, t: &Tuning) -> u16 {
        secs_to_ticks(self.pve_scaling(t).downed_time, self.tick_rate, 1)
    }

    /// Spawns a Mould for an encounter or grind wave with `health` × its definition's health;
    /// it knows where the players are (`aware`). `elite` is the difficulty's elite roll:
    /// elites belong to the enemies area (docs/expansion.md). Until their elite spawn lands an
    /// elite roll spawns a normal Mould (the roll is still made, so wave contents don't shift
    /// when it does). `None` if the name is unknown or the world is full.
    pub(crate) fn spawn_pve_enemy(
        &mut self,
        name: &str,
        position: Vec3,
        yaw: f32,
        health: f32,
        elite: bool,
    ) -> Option<NetId> {
        let id = self.spawn_enemy_scaled(name, position, yaw, health)?;
        if elite {
            log::debug!("elite {name} rolled (spawned as a normal Mould for now)");
        }
        if let Some(e) = self.enemies.get_mut(&id) {
            e.aware = true;
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn normal_solo_is_the_reference() {
        let t = Tuning::default();
        let s = PveScaling::compute(&t, Difficulty::Normal, 1);
        assert_eq!((s.health, s.count), (1.0, 1.0));
        assert_eq!(s.enemy, EnemyScale::default());
        assert_eq!(s.downed_time, t.modes.downed_time);
        assert_eq!(s.revive_health, t.modes.revive_health);
        assert_eq!(s.scaled_count(4), 4);
    }

    #[test]
    fn players_scale_harder_than_before() {
        let t = Tuning::default();
        let four = PveScaling::compute(&t, Difficulty::Normal, 4);
        // The old rule was health x2.5 and count x2.5 for four players, nothing else.
        assert!(close(four.health, 2.8), "{}", four.health);
        assert!(close(four.count, 2.8), "{}", four.count);
        assert!(close(four.enemy.aggression, 1.24));
        assert!(close(four.enemy.damage, 1.3));
        assert_eq!(four.scaled_count(4), 11);
        // Downed time: unchanged for two, shorter from the third player on.
        let two = PveScaling::compute(&t, Difficulty::Normal, 2);
        assert_eq!(two.downed_time, t.modes.downed_time);
        assert!(close(four.downed_time, 12.0), "{}", four.downed_time);
    }

    #[test]
    fn difficulties_are_ordered() {
        let t = Tuning::default();
        let all: Vec<PveScaling> = Difficulty::ALL
            .iter()
            .map(|d| PveScaling::compute(&t, *d, 2))
            .collect();
        for w in all.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            assert!(b.health > a.health && b.count > a.count);
            assert!(b.enemy.damage > a.enemy.damage && b.enemy.aggression > a.enemy.aggression);
            assert!(b.elite_chance > a.elite_chance);
            assert!(b.downed_time < a.downed_time && b.revive_health < a.revive_health);
        }
        let easy = PveScaling::compute(&t, Difficulty::Easy, 1);
        assert_eq!(easy.elite_chance, 0.0);
        assert_eq!(
            easy.scaled_count(1),
            1,
            "a lone authored Mould never disappears"
        );
        let inferno = PveScaling::compute(&t, Difficulty::Inferno, 4);
        assert!(inferno.downed_time >= t.difficulty.min_downed_time);
    }

    #[test]
    fn rng_is_deterministic_and_uniform_enough() {
        let mut a = ModeRng::new(7);
        let mut b = ModeRng::new(7);
        let xs: Vec<f32> = (0..1000).map(|_| a.next_f32()).collect();
        assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
        assert_eq!(xs[17], {
            for _ in 0..17 {
                b.next_f32();
            }
            b.next_f32()
        });
        let mean = xs.iter().sum::<f32>() / xs.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        assert_eq!(ModeRng::new(1).below(0), 0);
        assert_ne!(name_seed("cistern_nine"), name_seed("knell_yard"));
    }
}
