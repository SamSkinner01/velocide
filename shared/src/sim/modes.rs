//! Game modes: the death / respawn rule, campaign checkpoints, co-op downed & revive, the level
//! exit, deathmatch frags and limits, per-player stats, the scoreboard and end-of-match results
//! with grades. Everything is server-authoritative and lives in [`ModeState`]
//! (`SimWorld::mode`); `SimWorld::step` runs [`SimWorld::step_modes`] at the end of every tick.
//!
//! * **Campaign, solo**: death → `modes.sp_death_time` of death cam → back at the last
//!   checkpoint (the level resets to the checkpoint state, see [`SimWorld::reset_to_checkpoint`]).
//! * **Campaign, co-op** (2+ players): death → *downed* for `modes.downed_time` (can look around,
//!   can't move or shoot; replicated as `player_flags::DOWNED`). A teammate holding `USE`
//!   within `revive_range` for `revive_time` revives at `revive_health`; otherwise back at the
//!   checkpoint. Kill-volume deaths aren't revivable (`fall_respawn_time`). Everyone down at
//!   once → team wipe: all back at the checkpoint after `wipe_respawn_time`, level reset.
//! * **Campaign exit**: solo → reaching it ends the level. Co-op → the first player in starts an
//!   extraction countdown (`exit_countdown`); it ends early once every living player is in.
//!   Whoever is still outside when it runs out is *left behind* (shown on the results).
//! * **Deathmatch**: +1 frag per kill, −1 per suicide / kill-volume death (a kill-volume death
//!   shortly after being hit frags the hitter instead: `knockoff_credit`), respawn after
//!   `combat.respawn_delay` at the spawn point farthest from other players with
//!   `combat.spawn_protection`; frag / time limit → match end.
//!
//! Network: [`ModeEvent`]s go to everyone (`GameEvent::Mode`), the scoreboard is re-broadcast
//! (throttled) whenever something on it changes, and downed players carry `DOWNED` in their
//! entity flags. Full rules, thresholds and the protocol: `docs/style_modes.md`.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::{
    Attack, CombatRules, DamageFlags, KillInfo, Owner, PlayerCombat, secs_to_ticks,
};
use super::encounter::EncounterPhase;
use super::style::{StyleBonus, StyleEvent};
use super::{HitTarget, PlayerId, SimEvent, SimWorld, WeaponState};
use crate::input::Buttons;
use crate::level::{LevelDef, SpawnPoint};
use crate::lobby::{DEFAULT_FRAG_LIMIT, DEFAULT_TIME_LIMIT_SECS, GameMode, MatchSettings};
use crate::movement::MoveState;
use crate::protocol::{EntityState, player_flags};
use crate::tuning::{ModeTuning, Tuning};

/// The match configuration the mode rules run with: from the lobby's match settings
/// (`SimWorld::apply_match_settings` → [`ModeRules::from_settings`]); worlds that never get any
/// (tests, client copies) derive it from the level kind ([`ModeRules::for_level`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModeRules {
    pub kind: GameMode,
    /// Deathmatch: first to this many frags wins (0 = no limit).
    pub frag_limit: u32,
    /// Deathmatch: the match ends after this many seconds (0 = no limit).
    pub time_limit_secs: f32,
}

impl ModeRules {
    pub fn campaign() -> Self {
        Self {
            kind: GameMode::Campaign,
            frag_limit: 0,
            time_limit_secs: 0.0,
        }
    }

    pub fn deathmatch(frag_limit: u32, time_limit_secs: f32) -> Self {
        Self {
            kind: GameMode::Deathmatch,
            frag_limit,
            time_limit_secs: time_limit_secs.max(0.0),
        }
    }

    /// Campaign levels → campaign rules; arenas → deathmatch with the default limits.
    pub fn for_level(level: &LevelDef) -> Self {
        match GameMode::for_level_kind(level.kind) {
            GameMode::Campaign => Self::campaign(),
            GameMode::Deathmatch => {
                Self::deathmatch(DEFAULT_FRAG_LIMIT, DEFAULT_TIME_LIMIT_SECS as f32)
            }
        }
    }

    /// The rules of the lobby's match settings (mode, frag and time limits).
    pub fn from_settings(settings: &MatchSettings) -> Self {
        match settings.mode {
            GameMode::Campaign => Self::campaign(),
            GameMode::Deathmatch => {
                Self::deathmatch(settings.frag_limit, settings.time_limit_secs as f32)
            }
        }
    }

    /// The combat rules that go with the mode (deathmatch = PvP).
    pub fn combat_rules(&self) -> CombatRules {
        match self.kind {
            GameMode::Campaign => CombatRules::coop(),
            GameMode::Deathmatch => CombatRules::pvp(),
        }
    }
}

/// Where a match is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchPhase {
    /// Nobody is in the game yet (the clock starts with the first player).
    Waiting,
    Playing,
    /// Results are out; nothing scores any more and everyone is protected.
    Ended,
}

/// A player's state as far as the mode is concerned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifeState {
    #[default]
    Alive,
    /// Co-op: down, waiting for a revive (`combat.dead` is set too).
    Downed,
    Dead,
}

/// Per-player match statistics (scoreboard and results).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerStats {
    /// Campaign: enemies killed. Deathmatch: players killed.
    pub kills: u32,
    pub deaths: u32,
    /// Deathmatch score: kills − suicides / kill-volume deaths.
    pub frags: i32,
    /// Health lost (all sources).
    pub damage_taken: f32,
    /// Teammates revived.
    pub revives: u32,
}

/// A revive in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revive {
    pub reviver: PlayerId,
    pub start_tick: u32,
    pub done_tick: u32,
}

/// Mode state of one player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModePlayer {
    pub stats: PlayerStats,
    pub life: LifeState,
    /// Being revived (downed players only).
    pub revive: Option<Revive>,
    /// Last other player who hurt them, and when (deathmatch knock-off credit).
    pub last_hit_by: Option<(PlayerId, u32)>,
}

/// One scoreboard line.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoreEntry {
    pub player: PlayerId,
    pub kills: u32,
    pub deaths: u32,
    pub frags: i32,
    /// Current style rank and the best one reached.
    pub rank: u8,
    pub best_rank: u8,
    /// Style points earned.
    pub style: u32,
    pub life: LifeState,
}

/// The scoreboard and match clock (broadcast whenever something on it changes, throttled).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scoreboard {
    pub rules: ModeRules,
    pub phase: MatchPhase,
    /// Tick the match clock started (time left = limit − (now − start) / tick rate).
    pub start_tick: u32,
    /// Campaign: index of the last checkpoint reached.
    pub checkpoint: Option<u8>,
    pub rows: Vec<ScoreEntry>,
}

/// A results grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Grade {
    D,
    C,
    B,
    A,
    S,
}

impl Grade {
    pub const ALL: [Grade; 5] = [Grade::D, Grade::C, Grade::B, Grade::A, Grade::S];

    pub fn letter(self) -> &'static str {
        match self {
            Grade::D => "D",
            Grade::C => "C",
            Grade::B => "B",
            Grade::A => "A",
            Grade::S => "S",
        }
    }

    /// 0 (D) ..= 4 (S).
    pub fn value(self) -> u8 {
        self as u8
    }

    /// Rounded (halves up) and clamped.
    pub fn from_value(v: f32) -> Grade {
        let i = if v.is_finite() {
            v.round().clamp(0.0, 4.0)
        } else {
            0.0
        } as usize;
        Self::ALL[i]
    }

    /// Higher is better: `value ≥ thresholds[0]` → S, `[1]` → A, `[2]` → B, `[3]` → C, else D.
    pub fn at_least(value: f32, thresholds: &[f32; 4]) -> Grade {
        let best = [Grade::S, Grade::A, Grade::B, Grade::C];
        for (g, th) in best.iter().zip(thresholds) {
            if value >= *th {
                return *g;
            }
        }
        Grade::D
    }

    /// Lower is better: `value ≤ thresholds[0]` → S, ... else D.
    pub fn at_most(value: f32, thresholds: &[f32; 4]) -> Grade {
        let best = [Grade::S, Grade::A, Grade::B, Grade::C];
        for (g, th) in best.iter().zip(thresholds) {
            if value <= *th {
                return *g;
            }
        }
        Grade::D
    }

    /// Average grade (rounded, halves up); `None` for no grades.
    pub fn average(grades: impl IntoIterator<Item = Grade>) -> Option<Grade> {
        let (sum, n) = grades
            .into_iter()
            .fold((0u32, 0u32), |(s, n), g| (s + g.value() as u32, n + 1));
        (n > 0).then(|| Grade::from_value(sum as f32 / n as f32))
    }
}

/// A player's campaign grades.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grades {
    /// Level time vs par (`None` without a par time).
    pub time: Option<Grade>,
    /// Kills vs the player's share of the level's enemies (`None` without enemies).
    pub kills: Option<Grade>,
    pub style: Grade,
    pub damage: Grade,
    /// Average of the above.
    pub overall: Grade,
}

/// Grades one player's campaign run. `players`: how many share the level's enemies.
#[allow(clippy::too_many_arguments)]
pub fn grade_player(
    t: &ModeTuning,
    secs: f32,
    par_secs: f32,
    kills: u32,
    style_points: f32,
    damage_taken: f32,
    total_enemies: u32,
    players: usize,
) -> Grades {
    let share = total_enemies as f32 / players.max(1) as f32;
    let per = share.max(1.0);
    let time = (par_secs > 0.0).then(|| Grade::at_most(secs / par_secs, &t.grade_time));
    let kills = (total_enemies > 0).then(|| Grade::at_least(kills as f32 / share, &t.grade_kills));
    let style = Grade::at_least(style_points / per, &t.grade_style);
    let damage = Grade::at_most(damage_taken / per, &t.grade_damage);
    let overall =
        Grade::average(time.into_iter().chain(kills).chain([style, damage])).unwrap_or(Grade::D);
    Grades {
        time,
        kills,
        style,
        damage,
        overall,
    }
}

/// Why a match ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndReason {
    /// Campaign: the exit was reached.
    Exit,
    FragLimit,
    TimeLimit,
    /// Ended by the host / lobby.
    Host,
}

/// One player's line on the results screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerResult {
    pub player: PlayerId,
    pub name: String,
    pub stats: PlayerStats,
    /// Style points earned and the best rank reached.
    pub style_points: u32,
    pub best_rank: u8,
    /// Campaign: alive and inside the exit when the level ended.
    pub extracted: bool,
    /// Campaign grades.
    pub grades: Option<Grades>,
}

/// End-of-match results (`ModeEvent::Ended`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatchResults {
    pub rules: ModeRules,
    pub reason: EndReason,
    /// Level display name.
    pub level: String,
    pub duration_secs: f32,
    /// Campaign par time (0 = none).
    pub par_secs: f32,
    /// Enemies in the level (authored, or spawned if more: co-op scaling).
    pub total_enemies: u32,
    /// Enemies killed by players.
    pub team_kills: u32,
    /// Campaign: player id order. Deathmatch: final standings (best first).
    pub players: Vec<PlayerResult>,
    /// Campaign: average of the players' overall grades.
    pub team_grade: Option<Grade>,
    /// Deathmatch winner.
    pub winner: Option<PlayerId>,
}

/// Game-mode events (all broadcast as `GameEvent::Mode`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModeEvent {
    /// The team's respawn point is now checkpoint `index` (`player` reached it).
    Checkpoint {
        index: u8,
        player: PlayerId,
    },
    /// A co-op player is down; respawns at the checkpoint at `respawn_tick` unless revived.
    Downed {
        player: PlayerId,
        respawn_tick: u32,
    },
    /// A revive of `player` started (`reviver: Some`, done at `done_tick`) or stopped (`None`).
    Reviving {
        player: PlayerId,
        reviver: Option<PlayerId>,
        start_tick: u32,
        done_tick: u32,
    },
    Revived {
        player: PlayerId,
        by: PlayerId,
    },
    /// Everyone was down at once: back to the checkpoint at `respawn_tick`, the fight resets.
    TeamWiped {
        respawn_tick: u32,
    },
    /// Co-op: `player` reached the exit; the level ends at `end_tick` (or once everyone alive
    /// is in).
    ExitCountdown {
        player: PlayerId,
        end_tick: u32,
    },
    Scoreboard(Scoreboard),
    /// The match is over.
    Ended(Box<MatchResults>),
}

/// The whole mode state (`SimWorld::mode`).
#[derive(Clone, Debug)]
pub struct ModeState {
    pub rules: ModeRules,
    pub phase: MatchPhase,
    /// Tick the match clock started.
    pub start_tick: u32,
    /// Tick it ended.
    pub end_tick: Option<u32>,
    /// Campaign: last checkpoint reached (index into `LevelDef::checkpoints`).
    pub checkpoint: Option<usize>,
    pub players: BTreeMap<PlayerId, ModePlayer>,
    /// Co-op exit countdown: who started it and the tick it ends.
    pub exit: Option<(PlayerId, u32)>,
    /// Team wipe in progress: the tick everyone respawns.
    pub wipe: Option<u32>,
    pub team_kills: u32,
    pub enemies_spawned: u32,
    /// Killers of each encounter's enemies and how many it spawned (a checkpoint restart that
    /// resets an encounter takes them back, so re-fighting it doesn't count twice).
    encounter_kills: BTreeMap<u16, Vec<PlayerId>>,
    encounter_spawns: BTreeMap<u16, u32>,
    pub results: Option<MatchResults>,
    scoreboard_dirty: bool,
    last_scoreboard: Option<u32>,
}

impl ModeState {
    pub fn new(rules: ModeRules) -> Self {
        Self {
            rules,
            phase: MatchPhase::Waiting,
            start_tick: 0,
            end_tick: None,
            checkpoint: None,
            players: BTreeMap::new(),
            exit: None,
            wipe: None,
            team_kills: 0,
            enemies_spawned: 0,
            encounter_kills: BTreeMap::new(),
            encounter_spawns: BTreeMap::new(),
            results: None,
            scoreboard_dirty: true,
            last_scoreboard: None,
        }
    }

    pub fn player(&mut self, id: PlayerId) -> &mut ModePlayer {
        self.players.entry(id).or_default()
    }

    pub fn life(&self, id: PlayerId) -> LifeState {
        self.players.get(&id).map_or(LifeState::Alive, |p| p.life)
    }

    pub fn stats(&self, id: PlayerId) -> PlayerStats {
        self.players
            .get(&id)
            .map_or_else(PlayerStats::default, |p| p.stats)
    }

    pub fn is_playing(&self) -> bool {
        self.phase == MatchPhase::Playing
    }

    /// The match ended (results): AI and damage are frozen, nothing scores.
    pub fn is_over(&self) -> bool {
        self.phase == MatchPhase::Ended
    }

    /// Something on the scoreboard changed: it is re-broadcast soon.
    pub fn touch(&mut self) {
        self.scoreboard_dirty = true;
    }
}

impl SimWorld {
    /// Enemies this level's encounters spawn for the current player count (co-op scaling).
    pub fn expected_enemies(&self) -> u32 {
        if !self.rules.coop_enemy_scaling {
            return authored_enemies(&self.level);
        }
        let players = self.coop_player_count();
        self.level
            .encounters
            .iter()
            .flat_map(|e| &e.waves)
            .map(|w| super::encounter::scaled_count(w.spawns.len(), players) as u32)
            .sum()
    }
}

/// Number of enemies the level's encounters spawn (before co-op scaling).
pub fn authored_enemies(level: &LevelDef) -> u32 {
    level
        .encounters
        .iter()
        .flat_map(|e| &e.waves)
        .map(|w| w.spawns.len() as u32)
        .sum()
}

impl SimWorld {
    /// Replaces the mode rules (lobby match settings) and starts the mode afresh. Also sets the
    /// matching combat rules (deathmatch = PvP).
    pub fn set_mode_rules(&mut self, rules: ModeRules) {
        self.rules = rules.combat_rules();
        self.mode = ModeState::new(rules);
        self.style.clear();
    }

    /// Back to the start of the match (level restart): stats, style, checkpoint, clock.
    pub(crate) fn reset_modes(&mut self) {
        self.frozen = false;
        self.mode = ModeState::new(self.mode.rules);
        self.style.clear();
        self.sync_mode_players(None);
    }

    /// Is this a single-player campaign right now (solo rules)?
    pub fn is_solo(&self) -> bool {
        self.players.len() <= 1
    }

    /// End-of-tick game-mode step (pipeline step 6): stats and style from this tick's events,
    /// checkpoints, revives, team wipes, the exit, deathmatch limits, the scoreboard.
    pub(crate) fn step_modes(&mut self, t: &Tuning) {
        // This tick's events so far; everything emitted below is appended after them.
        let events = std::mem::take(&mut self.events);
        self.sync_mode_players(Some(t));
        if self.frozen && self.mode.is_playing() {
            // Frozen from outside (the server's results phase): the match is over.
            self.end_match(t, EndReason::Host);
        }
        if self.mode.is_playing() {
            self.account_events(t, &events);
            for e in self.score_style(t, &events) {
                self.push_style(e);
            }
            for e in self.decay_style(t) {
                self.push_style(e);
            }
            match self.mode.rules.kind {
                GameMode::Campaign => {
                    self.tick_checkpoints();
                    self.tick_revives(t);
                    self.check_team_wipe(t);
                    self.tick_exit(t);
                }
                GameMode::Deathmatch => self.tick_deathmatch(t),
            }
        }
        if self.mode.is_over() || self.frozen {
            // Nothing can hurt anyone on the results screen.
            for p in self.players.values_mut() {
                p.combat.protection = p.combat.protection.max(2);
            }
        }
        self.maybe_broadcast_scoreboard(t);
        let new = std::mem::replace(&mut self.events, events);
        self.events.extend(new);
    }

    fn push_style(&mut self, e: StyleEvent) {
        if matches!(e, StyleEvent::Rank { .. }) {
            self.mode.touch();
        }
        self.events.push(SimEvent::Style(e));
    }

    fn push_mode(&mut self, e: ModeEvent) {
        self.events.push(SimEvent::Mode(e));
    }

    /// Registers players who joined and forgets those who left. With `t` (a running match), a
    /// late joiner is moved to where the mode respawns players (the campaign checkpoint, the
    /// deathmatch spawn farthest from everyone).
    fn sync_mode_players(&mut self, t: Option<&Tuning>) {
        let ids: Vec<PlayerId> = self.players.keys().copied().collect();
        let before = self.mode.players.len();
        self.mode.players.retain(|id, _| ids.contains(id));
        self.style.players.retain(|id, _| ids.contains(id));
        if self.mode.players.len() != before {
            self.mode.touch();
        }
        if ids.is_empty() {
            if self.mode.phase != MatchPhase::Waiting {
                // Everyone left: the next player starts a fresh match.
                self.mode = ModeState::new(self.mode.rules);
                self.style.clear();
            }
            return;
        }
        // Players who are there when the match starts stay where the level put them.
        let late = self.mode.phase == MatchPhase::Playing && t.is_some();
        if self.mode.phase == MatchPhase::Waiting {
            self.mode.phase = MatchPhase::Playing;
            self.mode.start_tick = self.tick;
            self.mode.touch();
        }
        for id in ids {
            if self.mode.players.contains_key(&id) {
                continue;
            }
            self.mode.player(id);
            self.style.player(id);
            self.mode.touch();
            let move_them = match self.mode.rules.kind {
                GameMode::Campaign => self.mode.checkpoint.is_some(),
                GameMode::Deathmatch => self.players.len() > 1,
            };
            if late
                && move_them
                && let Some(t) = t
                && self.players.get(&id).is_some_and(|p| !p.combat.dead)
            {
                self.place_at_spawn(t, id);
            }
        }
    }

    /// Stats from this tick's events: damage taken, kills, deaths, frags, enemies spawned.
    fn account_events(&mut self, t: &Tuning, events: &[SimEvent]) {
        let dm = self.mode.rules.kind == GameMode::Deathmatch;
        let credit_ticks = secs_to_ticks(t.modes.knockoff_credit, self.tick_rate, 0) as u32;
        for event in events {
            match event {
                SimEvent::Damage(d) => {
                    let HitTarget::Player(victim) = d.target else {
                        continue;
                    };
                    if d.blocked.is_none() && d.dealt > 0.0 {
                        self.mode.player(victim).stats.damage_taken += d.dealt;
                    }
                    if let Owner::Player(attacker) = d.info.source
                        && attacker != victim
                        && (d.blocked.is_none() || d.info.knockback.length_squared() > 0.0)
                    {
                        self.mode.player(victim).last_hit_by = Some((attacker, d.tick));
                    }
                }
                SimEvent::Killed(k) => self.account_kill(t, k, dm, credit_ticks),
                SimEvent::EnemySpawned { id, .. } => {
                    self.mode.enemies_spawned += 1;
                    if let Some((enc, _)) = self.enemies.get(id).and_then(|e| e.encounter) {
                        *self.mode.encounter_spawns.entry(enc).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
    }

    fn account_kill(&mut self, t: &Tuning, k: &KillInfo, dm: bool, credit_ticks: u32) {
        match k.victim {
            HitTarget::Entity(id) => {
                if let Owner::Player(killer) = k.killer {
                    self.mode.player(killer).stats.kills += 1;
                    self.mode.team_kills += 1;
                    self.mode.touch();
                    if let Some((enc, _)) = self.enemies.get(&id).and_then(|e| e.encounter) {
                        self.mode
                            .encounter_kills
                            .entry(enc)
                            .or_default()
                            .push(killer);
                    }
                }
            }
            HitTarget::Player(victim) => {
                self.mode.touch();
                let last_hit = {
                    let v = self.mode.player(victim);
                    v.stats.deaths += 1;
                    v.last_hit_by.take()
                };
                match k.killer {
                    Owner::Player(killer) if killer != victim => {
                        let s = &mut self.mode.player(killer).stats;
                        s.kills += 1;
                        if dm {
                            s.frags += 1;
                        }
                    }
                    Owner::Player(_) => {
                        if dm {
                            self.mode.player(victim).stats.frags -= 1;
                        }
                    }
                    Owner::World if dm => {
                        // Knocked into the coolant: the last hitter gets the frag and the
                        // victim no suicide penalty; an uncredited world death costs one.
                        if let Some((hitter, at)) = last_hit
                            && k.tick.wrapping_sub(at) <= credit_ticks
                            && self.players.contains_key(&hitter)
                        {
                            let s = &mut self.mode.player(hitter).stats;
                            s.kills += 1;
                            s.frags += 1;
                            let mut out = Vec::new();
                            let items = [(StyleBonus::Frag, t.style.frag, 0)];
                            let tick = self.tick;
                            self.style
                                .player(hitter)
                                .award(&t.style, hitter, None, &items, false, tick, &mut out);
                            for e in out {
                                self.push_style(e);
                            }
                        } else {
                            self.mode.player(victim).stats.frags -= 1;
                        }
                    }
                    Owner::World | Owner::Enemy(_) => {}
                }
            }
        }
    }

    // ---- the death / respawn rule (called by `combat.rs`) ------------------------------------

    /// THE death rule: a player just died ([`SimWorld::on_player_died`] delegates here).
    /// Picks dead / downed and the respawn delay by mode, empties the style meter.
    pub(crate) fn mode_player_died(&mut self, t: &Tuning, id: PlayerId, kill: &KillInfo) {
        let rate = self.tick_rate;
        let void = kill.attack == Attack::World || kill.flags.contains(DamageFlags::ENVIRONMENT);
        let solo = self.is_solo();
        let (secs, life) = match (self.mode.rules.kind, self.mode.phase) {
            (_, MatchPhase::Ended) | (GameMode::Deathmatch, _) => {
                (t.combat.respawn_delay, LifeState::Dead)
            }
            (GameMode::Campaign, _) if solo => (t.modes.sp_death_time, LifeState::Dead),
            (GameMode::Campaign, _) if void => (t.modes.fall_respawn_time, LifeState::Dead),
            (GameMode::Campaign, _) => (t.modes.downed_time, LifeState::Downed),
        };
        let ticks = secs_to_ticks(secs, rate, 1);
        if let Some(p) = self.players.get_mut(&id) {
            p.combat.dead = true;
            p.combat.respawn_in = ticks;
            p.combat.weapons.on_death();
        }
        // Their pending parryable hits are moot.
        self.cancel_pending_for_victim(id);
        let m = self.mode.player(id);
        m.life = life;
        m.revive = None;
        self.mode.touch();
        if life == LifeState::Downed {
            let respawn_tick = self.tick.wrapping_add(ticks as u32);
            self.push_mode(ModeEvent::Downed {
                player: id,
                respawn_tick,
            });
        }
        if t.style.reset_on_death && self.style.player(id).reset() {
            self.push_style(StyleEvent::Rank {
                player: id,
                rank: 0,
                up: false,
            });
        }
    }

    /// THE respawn rule ([`SimWorld::respawn_player`] delegates here): back into the game where
    /// the mode says, with full health, fresh weapons and spawn protection. A solo death or a
    /// team wipe first puts the level back to its checkpoint state.
    pub(crate) fn mode_respawn_player(&mut self, t: &Tuning, id: PlayerId) {
        let was_dead = self.players.get(&id).is_some_and(|p| p.combat.dead);
        if self.mode.rules.kind == GameMode::Campaign
            && self.mode.phase == MatchPhase::Playing
            && ((was_dead && self.is_solo()) || self.mode.wipe.take().is_some())
        {
            self.reset_to_checkpoint(t);
        }
        self.place_at_spawn(t, id);
    }

    /// Moves a player to the mode's spawn point as a fresh, protected body and emits
    /// `Respawned`.
    fn place_at_spawn(&mut self, t: &Tuning, id: PlayerId) {
        let spawn = self.mode_spawn_point(id);
        let rate = self.tick_rate;
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        let god = p.combat.god;
        p.movement = MoveState::spawn(spawn.position);
        p.yaw = spawn.yaw.to_radians();
        p.pitch = 0.0;
        p.combat = PlayerCombat {
            god,
            protection: secs_to_ticks(t.combat.spawn_protection, rate, 0),
            weapons: WeaponState::fresh(p.combat.weapons.weapon, p.combat.weapons.mode),
            ..Default::default()
        };
        let m = self.mode.player(id);
        m.life = LifeState::Alive;
        m.revive = None;
        m.last_hit_by = None;
        self.mode.touch();
        self.events.push(SimEvent::Respawned {
            player: id,
            position: spawn.position,
            yaw: spawn.yaw.to_radians(),
        });
    }

    /// Where the mode puts `id` when it (re)spawns: deathmatch → the spawn point farthest from
    /// the other living players; campaign → the last checkpoint, or the player's level start.
    pub fn mode_spawn_point(&self, id: PlayerId) -> SpawnPoint {
        let level = &self.level;
        match self.mode.rules.kind {
            GameMode::Deathmatch => self.farthest_spawn(id),
            GameMode::Campaign => {
                if let Some(cp) = self.mode.checkpoint.and_then(|i| level.checkpoints.get(i)) {
                    return cp.spawn;
                }
                let index = self.players.keys().position(|p| *p == id).unwrap_or(0);
                level
                    .spawns
                    .get(index % level.spawns.len().max(1))
                    .copied()
                    .unwrap_or(SpawnPoint {
                        position: Vec3::new(0.0, 1.0, 0.0),
                        yaw: 0.0,
                    })
            }
        }
    }

    /// The level spawn point whose nearest living opponent is farthest away (ties: the first).
    pub fn farthest_spawn(&self, id: PlayerId) -> SpawnPoint {
        let spawns = &self.level.spawns;
        if spawns.is_empty() {
            return SpawnPoint {
                position: Vec3::new(0.0, 1.0, 0.0),
                yaw: 0.0,
            };
        }
        let others: Vec<Vec3> = self
            .players
            .values()
            .filter(|p| p.id != id && !p.combat.dead)
            .map(|p| p.movement.position)
            .collect();
        if others.is_empty() {
            return spawns[(id.0 as usize + self.tick as usize) % spawns.len()];
        }
        let mut best = (f32::NEG_INFINITY, spawns[0]);
        for s in spawns {
            let nearest = others
                .iter()
                .map(|o| o.distance_squared(s.position))
                .fold(f32::INFINITY, f32::min);
            if nearest > best.0 {
                best = (nearest, *s);
            }
        }
        best.1
    }

    /// Puts the level back to the state of the last checkpoint (solo death, team wipe): every
    /// encounter in progress goes back to idle (its enemies removed, its doors back to their
    /// level-start state, its trigger re-armed: `SimWorld::reset_uncleared`), and nothing left
    /// in flight (projectiles, parry-delayed hits) greets the respawning players.
    pub fn reset_to_checkpoint(&mut self, _t: &Tuning) {
        // Kills and spawns of the encounters about to reset don't count: they'll be fought again.
        let active: Vec<u16> = self
            .encounters
            .states
            .iter()
            .enumerate()
            .filter(|(_, s)| s.phase == EncounterPhase::Active)
            .map(|(i, _)| i.min(u16::MAX as usize) as u16)
            .collect();
        for enc in active {
            for killer in self.mode.encounter_kills.remove(&enc).unwrap_or_default() {
                let s = &mut self.mode.player(killer).stats;
                s.kills = s.kills.saturating_sub(1);
                self.mode.team_kills = self.mode.team_kills.saturating_sub(1);
            }
            let spawned = self.mode.encounter_spawns.remove(&enc).unwrap_or(0);
            self.mode.enemies_spawned = self.mode.enemies_spawned.saturating_sub(spawned);
            self.mode.touch();
        }
        self.reset_uncleared();
        self.projectiles.clear();
        self.pending_hits.clear();
        self.mode.exit = None;
    }

    // ---- campaign ---------------------------------------------------------------------------

    fn tick_checkpoints(&mut self) {
        let level = std::sync::Arc::clone(&self.level);
        if level.checkpoints.is_empty() {
            return;
        }
        let reached: Vec<(PlayerId, Vec3)> = self
            .players
            .values()
            .filter(|p| !p.combat.dead)
            .map(|p| (p.id, p.movement.position))
            .collect();
        for (id, feet) in reached {
            let current = self.mode.checkpoint;
            let hit = level.checkpoints.iter().enumerate().rev().find(|(i, cp)| {
                current.is_none_or(|c| *i > c)
                    && (cp.trigger.contains(feet) || cp.trigger.contains(feet + Vec3::Y * 0.9))
            });
            if let Some((index, _)) = hit {
                self.mode.checkpoint = Some(index);
                self.mode.touch();
                self.push_mode(ModeEvent::Checkpoint {
                    index: index.min(u8::MAX as usize) as u8,
                    player: id,
                });
            }
        }
    }

    fn tick_revives(&mut self, t: &Tuning) {
        let range2 = t.modes.revive_range * t.modes.revive_range;
        let duration = secs_to_ticks(t.modes.revive_time, self.tick_rate, 1) as u32;
        let tick = self.tick;
        let downed: Vec<(PlayerId, Vec3, Option<Revive>)> = self
            .mode
            .players
            .iter()
            .filter(|(_, m)| m.life == LifeState::Downed)
            .filter_map(|(id, m)| Some((*id, self.players.get(id)?.movement.position, m.revive)))
            .collect();
        for (id, pos, current) in downed {
            // Living teammates holding USE in range, nearest first.
            let mut helpers: Vec<(PlayerId, f32)> = self
                .players
                .values()
                .filter(|p| {
                    p.id != id
                        && !p.combat.dead
                        && self.mode.life(p.id) == LifeState::Alive
                        && p.last_input.buttons.contains(Buttons::USE)
                })
                .map(|p| (p.id, p.movement.position.distance_squared(pos)))
                .filter(|(_, d2)| *d2 <= range2)
                .collect();
            helpers.sort_by(|a, b| a.1.total_cmp(&b.1));
            if let Some(rv) = current
                && helpers.iter().any(|(r, _)| *r == rv.reviver)
            {
                if tick.wrapping_sub(rv.start_tick) >= rv.done_tick.wrapping_sub(rv.start_tick) {
                    self.revive(t, id, rv.reviver);
                }
                continue;
            }
            if current.is_some() {
                self.mode.player(id).revive = None;
                self.push_mode(ModeEvent::Reviving {
                    player: id,
                    reviver: None,
                    start_tick: tick,
                    done_tick: tick,
                });
            }
            if let Some(&(reviver, _)) = helpers.first() {
                let rv = Revive {
                    reviver,
                    start_tick: tick,
                    done_tick: tick.wrapping_add(duration),
                };
                self.mode.player(id).revive = Some(rv);
                self.push_mode(ModeEvent::Reviving {
                    player: id,
                    reviver: Some(reviver),
                    start_tick: rv.start_tick,
                    done_tick: rv.done_tick,
                });
            }
        }
    }

    /// Gets a downed player back up where they lie.
    fn revive(&mut self, t: &Tuning, id: PlayerId, by: PlayerId) {
        let rate = self.tick_rate;
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        let c = &mut p.combat;
        c.dead = false;
        c.respawn_in = 0;
        c.hard = 0.0;
        c.damage = (t.combat.max_health - t.modes.revive_health).max(0.0);
        c.protection = secs_to_ticks(t.combat.spawn_protection, rate, 0);
        c.last_attacker = Owner::World;
        let (position, yaw) = (p.movement.position, p.yaw);
        let m = self.mode.player(id);
        m.life = LifeState::Alive;
        m.revive = None;
        self.mode.player(by).stats.revives += 1;
        self.mode.touch();
        self.push_mode(ModeEvent::Revived { player: id, by });
        // Ends the death view on the revived player's client like any respawn.
        self.events.push(SimEvent::Respawned {
            player: id,
            position,
            yaw,
        });
    }

    fn check_team_wipe(&mut self, t: &Tuning) {
        if self.mode.wipe.is_some() || self.players.is_empty() {
            return;
        }
        let all_down = self.players.values().all(|p| p.combat.dead);
        let any_downed = self
            .mode
            .players
            .values()
            .any(|m| m.life == LifeState::Downed);
        // Solo deaths have their own rule; a lone downed player (teammates left) is a wipe.
        if !all_down || !(self.players.len() >= 2 || any_downed) {
            return;
        }
        let ticks = secs_to_ticks(t.modes.wipe_respawn_time, self.tick_rate, 1);
        for p in self.players.values_mut() {
            p.combat.respawn_in = ticks;
        }
        for m in self.mode.players.values_mut() {
            if m.life == LifeState::Downed {
                m.life = LifeState::Dead;
            }
            m.revive = None;
        }
        let respawn_tick = self.tick.wrapping_add(ticks as u32);
        self.mode.wipe = Some(respawn_tick);
        self.mode.touch();
        self.push_mode(ModeEvent::TeamWiped { respawn_tick });
    }

    fn tick_exit(&mut self, t: &Tuning) {
        let Some(exit) = self.level.exit else {
            return;
        };
        let living: Vec<(PlayerId, bool)> = self
            .players
            .values()
            .filter(|p| !p.combat.dead)
            .map(|p| {
                let f = p.movement.position;
                (p.id, exit.contains(f) || exit.contains(f + Vec3::Y * 0.9))
            })
            .collect();
        let first_in = living.iter().find(|(_, inside)| *inside).map(|(id, _)| *id);
        let all_in = !living.is_empty() && living.iter().all(|(_, inside)| *inside);
        if self.is_solo() {
            if first_in.is_some() {
                self.end_match(t, EndReason::Exit);
            }
            return;
        }
        if all_in {
            self.end_match(t, EndReason::Exit);
            return;
        }
        match self.mode.exit {
            Some((_, end)) if self.tick >= end => self.end_match(t, EndReason::Exit),
            Some(_) => {}
            None => {
                if let Some(player) = first_in {
                    let ticks = secs_to_ticks(t.modes.exit_countdown, self.tick_rate, 1);
                    let end_tick = self.tick.wrapping_add(ticks as u32);
                    self.mode.exit = Some((player, end_tick));
                    self.push_mode(ModeEvent::ExitCountdown { player, end_tick });
                }
            }
        }
    }

    // ---- deathmatch ---------------------------------------------------------------------------

    fn tick_deathmatch(&mut self, t: &Tuning) {
        let rules = self.mode.rules;
        if rules.frag_limit > 0
            && self
                .mode
                .players
                .values()
                .any(|m| m.stats.frags >= rules.frag_limit as i32)
        {
            self.end_match(t, EndReason::FragLimit);
            return;
        }
        if rules.time_limit_secs > 0.0 {
            let limit = (rules.time_limit_secs as f64 * self.tick_rate as f64).round() as u32;
            if self.tick.wrapping_sub(self.mode.start_tick) >= limit {
                self.end_match(t, EndReason::TimeLimit);
            }
        }
    }

    // ---- end of match -----------------------------------------------------------------------

    /// Ends the match now (exit, limits, or the host / lobby): builds the results and
    /// broadcasts them (`ModeEvent::Ended`). Does nothing if it already ended.
    pub fn end_match(&mut self, t: &Tuning, reason: EndReason) {
        if self.mode.phase == MatchPhase::Ended {
            return;
        }
        self.mode.phase = MatchPhase::Ended;
        self.mode.end_tick = Some(self.tick);
        // The shared results freeze: enemies stand still, encounters don't trigger, no enemy
        // hits land, kill volumes don't kill (docs/enemies.md).
        self.frozen = true;
        let results = self.build_results(t, reason);
        self.mode.results = Some(results.clone());
        self.mode.touch();
        self.push_mode(ModeEvent::Ended(Box::new(results)));
    }

    /// The results as they would be if the match ended now.
    pub fn build_results(&self, t: &Tuning, reason: EndReason) -> MatchResults {
        let rules = self.mode.rules;
        let end = self.mode.end_tick.unwrap_or(self.tick);
        let duration_secs =
            end.wrapping_sub(self.mode.start_tick) as f32 / self.tick_rate.max(1) as f32;
        let total_enemies = self.expected_enemies().max(self.mode.enemies_spawned);
        let exit = self.level.exit;
        let n = self.players.len();
        let mut players: Vec<PlayerResult> = self
            .players
            .values()
            .map(|p| {
                let stats = self.mode.stats(p.id);
                let (style_points, best_rank) = self
                    .style
                    .get(p.id)
                    .map_or((0.0, 0), |s| (s.total, s.best_rank));
                let feet = p.movement.position;
                let extracted = !p.combat.dead
                    && exit.is_some_and(|e| e.contains(feet) || e.contains(feet + Vec3::Y * 0.9));
                let grades = (rules.kind == GameMode::Campaign).then(|| {
                    grade_player(
                        &t.modes,
                        duration_secs,
                        self.level.par_time,
                        stats.kills,
                        style_points,
                        stats.damage_taken,
                        total_enemies,
                        n,
                    )
                });
                PlayerResult {
                    player: p.id,
                    name: p.name.clone(),
                    stats,
                    style_points: style_points.round().max(0.0) as u32,
                    best_rank,
                    extracted,
                    grades,
                }
            })
            .collect();
        let (team_grade, winner) = match rules.kind {
            GameMode::Campaign => (
                Grade::average(players.iter().filter_map(|p| Some(p.grades?.overall))),
                None,
            ),
            GameMode::Deathmatch => {
                players.sort_by(|a, b| {
                    b.stats
                        .frags
                        .cmp(&a.stats.frags)
                        .then(a.stats.deaths.cmp(&b.stats.deaths))
                        .then(b.style_points.cmp(&a.style_points))
                        .then(a.player.cmp(&b.player))
                });
                (None, players.first().map(|p| p.player))
            }
        };
        let level = if self.level.name.is_empty() {
            self.level_name.clone()
        } else {
            self.level.name.clone()
        };
        MatchResults {
            rules,
            reason,
            level,
            duration_secs,
            par_secs: self.level.par_time,
            total_enemies,
            team_kills: self.mode.team_kills,
            players,
            team_grade,
            winner,
        }
    }

    // ---- scoreboard / replication ---------------------------------------------------------------

    /// The current scoreboard.
    pub fn scoreboard(&self) -> Scoreboard {
        let rows = self
            .players
            .keys()
            .map(|&id| {
                let s = self.mode.stats(id);
                let (rank, best_rank, style) = self.style.get(id).map_or((0, 0, 0), |st| {
                    (st.rank, st.best_rank, st.total.round() as u32)
                });
                ScoreEntry {
                    player: id,
                    kills: s.kills,
                    deaths: s.deaths,
                    frags: s.frags,
                    rank,
                    best_rank,
                    style,
                    life: self.mode.life(id),
                }
            })
            .collect();
        Scoreboard {
            rules: self.mode.rules,
            phase: self.mode.phase,
            start_tick: self.mode.start_tick,
            checkpoint: self.mode.checkpoint.map(|c| c.min(u8::MAX as usize) as u8),
            rows,
        }
    }

    fn maybe_broadcast_scoreboard(&mut self, t: &Tuning) {
        if !self.mode.scoreboard_dirty || self.players.is_empty() {
            return;
        }
        let interval = secs_to_ticks(t.modes.scoreboard_interval, self.tick_rate, 1) as u32;
        if let Some(last) = self.mode.last_scoreboard
            && self.tick.wrapping_sub(last) < interval
        {
            return;
        }
        let board = self.scoreboard();
        self.push_mode(ModeEvent::Scoreboard(board));
        self.mode.scoreboard_dirty = false;
        self.mode.last_scoreboard = Some(self.tick);
    }

    /// Mode flags on a replicated player entity: downed players are `DOWNED` (not `DEAD`).
    pub(crate) fn decorate_player_entity(&self, e: &mut EntityState) {
        if let Some(id) = e.id.as_player()
            && self.mode.life(id) == LifeState::Downed
        {
            e.flags |= player_flags::DOWNED;
            e.flags &= !player_flags::DEAD;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_from_thresholds() {
        let th = [1.0, 0.9, 0.75, 0.5];
        assert_eq!(Grade::at_least(1.0, &th), Grade::S);
        assert_eq!(Grade::at_least(0.95, &th), Grade::A);
        assert_eq!(Grade::at_least(0.5, &th), Grade::C);
        assert_eq!(Grade::at_least(0.49, &th), Grade::D);
        let lower = [1.0, 3.0, 6.0, 10.0];
        assert_eq!(Grade::at_most(0.0, &lower), Grade::S);
        assert_eq!(Grade::at_most(3.0, &lower), Grade::A);
        assert_eq!(Grade::at_most(11.0, &lower), Grade::D);
        assert_eq!(Grade::average([Grade::S, Grade::A]), Some(Grade::S));
        assert_eq!(
            Grade::average([Grade::B, Grade::C, Grade::C]),
            Some(Grade::C)
        );
        assert_eq!(Grade::average([]), None);
    }
}
