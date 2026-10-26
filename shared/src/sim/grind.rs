//! Grind (`GameMode::Grind`): endless numbered waves of Moulds on an arena for 1–4 players,
//! until everyone is down or dead at once. Numbers: the `grind` section of the tuning
//! ([`GrindTuning`]); full rules in `docs/style_modes.md`.
//!
//! * **Waves**: a breather (`first_delay`, then `breather` between waves), then wave *w*
//!   spawns. Its contents come from a budget in enemy cost units ([`wave_budget`]) times the
//!   difficulty × player count factor ([`PveScaling::count`]): Moulds are picked by weight
//!   from the roster entries unlocked by wave *w* (names the server's registry doesn't know
//!   are skipped). Health grows with the wave on top of the difficulty. Every `heavy_every`th
//!   wave is a **heavy wave** (bigger budget, guaranteed heavies, more elites), every
//!   `boss_every`th brings a **boss-grade elite**. A wave opens with a burst and trickles the
//!   rest in, never more than `max_alive` (+ per player) at once.
//! * **Spawn points**: the level's `grind_spawns` (preferring those away from the players),
//!   else walkable nav nodes that can reach the player spawns, at least `min_spawn_distance`
//!   from every living player when the arena allows.
//! * **Deaths**: co-op deaths are *downed* (revivable, as in the campaign); a downed player
//!   nobody revives, a kill-volume death or a solo death is *out* until the next wave starts:
//!   everyone dead or downed comes back at the start of every wave.
//! * **The end**: everyone down or dead at once → [`GrindEvent::Overrun`], results after
//!   `end_delay` (`EndReason::Overrun`): wave reached, kills, time, grades.
//!
//! Network: [`ModeEvent::Grind`] events for the banners and sounds, and the run's
//! [`GrindStatus`] on every scoreboard (HUD, late joiners).

use std::collections::VecDeque;

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::secs_to_ticks;
use super::difficulty::{ModeRng, PveScaling, name_seed};
use super::enemy::Locomotion;
use super::modes::{EndReason, LifeState, ModeEvent};
use super::nav::{self, NavClass, node_flags};
use super::{PlayerId, SimWorld};
use crate::level::{EnemySpawn, SpawnPoint};
use crate::protocol::NetId;
use crate::tuning::{GrindTuning, Tuning};

use engine::physics::MASK_MOVEMENT;

/// How high above a spawn point's floor flying Moulds appear (feet, m).
const FLYER_SPAWN_HEIGHT: f32 = 4.5;
/// Random nav nodes tried per spawn before settling for the best one seen.
const NAV_SPAWN_TRIES: usize = 32;

/// Where a grind run is: on every scoreboard (HUD, late joiners).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrindStatus {
    /// Current wave (1-based; 0 before the first).
    pub wave: u32,
    /// Moulds of the wave still to kill (alive + waiting to spawn).
    pub left: u32,
    /// Moulds the wave holds.
    pub total: u32,
    /// Breather: the next wave starts at this tick.
    pub next_wave_tick: Option<u32>,
    pub heavy: bool,
    pub boss: bool,
    /// Everyone is down: the run ends at this tick.
    pub overrun_tick: Option<u32>,
}

/// Grind events (broadcast as `ModeEvent::Grind`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrindEvent {
    /// Wave `wave` starts now with `enemies` Moulds (everyone who was out is back).
    WaveStarted {
        wave: u32,
        enemies: u32,
        heavy: bool,
        boss: bool,
    },
    /// The last Mould of wave `wave` died; the next wave starts at `next_tick`.
    WaveCleared { wave: u32, next_tick: u32 },
    /// Everyone is down or dead: the run ends at `end_tick`.
    Overrun { end_tick: u32 },
}

/// A grind run's result (`MatchResults::grind`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrindResult {
    /// The wave the run ended in.
    pub wave: u32,
    /// Waves fully cleared.
    pub cleared: u32,
    /// Grade for the wave reached (`grind.grade_wave`).
    pub grade: super::Grade,
}

/// One Mould of a planned wave.
#[derive(Clone, Debug, PartialEq)]
pub struct GrindSpawn {
    /// Enemy definition name.
    pub enemy: String,
    /// Health multiplier (difficulty × players × wave, × `boss_health` for the boss).
    pub health: f32,
    pub elite: bool,
    pub boss: bool,
}

/// What a wave holds before placement ([`plan_wave`]).
#[derive(Clone, Debug, PartialEq)]
pub struct WavePlan {
    pub wave: u32,
    pub spawns: Vec<GrindSpawn>,
    pub heavy: bool,
    pub boss: bool,
    /// Budget after difficulty, players and the heavy-wave bonus.
    pub budget: f32,
}

/// Wave budget before scaling: `budget_base + budget_per_wave (w − 1) + budget_per_wave_sq
/// (w − 1)²`.
pub fn wave_budget(g: &GrindTuning, wave: u32) -> f32 {
    let n = wave.max(1) as f32 - 1.0;
    g.budget_base + g.budget_per_wave * n + g.budget_per_wave_sq * n * n
}

/// Every `heavy_every`th wave is a heavy wave.
pub fn is_heavy_wave(g: &GrindTuning, wave: u32) -> bool {
    g.heavy_every > 0 && wave > 0 && wave.is_multiple_of(g.heavy_every)
}

/// Every `boss_every`th wave brings a boss-grade elite.
pub fn is_boss_wave(g: &GrindTuning, wave: u32) -> bool {
    g.boss_every > 0 && wave > 0 && wave.is_multiple_of(g.boss_every)
}

/// Builds wave `wave`: the boss (boss waves), the guaranteed heavies (heavy waves), then
/// Moulds picked by weight from the unlocked roster until the budget is spent, shuffled
/// (the boss leads). `known`: whether the server has a definition of that name. Pure.
pub fn plan_wave(
    t: &Tuning,
    wave: u32,
    scaling: &PveScaling,
    known: &dyn Fn(&str) -> bool,
    rng: &mut ModeRng,
) -> WavePlan {
    let g = &t.grind;
    let wave = wave.max(1);
    let heavy = is_heavy_wave(g, wave);
    let boss = is_boss_wave(g, wave);
    let mut budget =
        wave_budget(g, wave) * scaling.count * if heavy { g.heavy_budget } else { 1.0 };
    let plan_budget = budget;
    let health = scaling.health * (1.0 + g.health_per_wave * (wave - 1) as f32);
    let elite_chance =
        (scaling.elite_chance + if heavy { g.heavy_elite_chance } else { 0.0 }).clamp(0.0, 1.0);
    let usable: Vec<_> = g
        .roster
        .iter()
        .filter(|e| e.weight > 0.0 && known(&e.enemy))
        .collect();
    // Heavies make their debut on the heavy wave before their own unlock.
    let lead = if g.heavy_every > 0 { g.heavy_every } else { 0 };
    let heavies: Vec<_> = usable
        .iter()
        .filter(|e| e.heavy && e.from_wave <= wave + lead)
        .copied()
        .collect();
    let pool: Vec<_> = usable
        .iter()
        .filter(|e| e.from_wave <= wave)
        .copied()
        .collect();
    let max = g.max_wave_enemies.max(1) as usize;
    let mut spawns = Vec::new();
    let mut lead_spawn = None;
    if boss && let Some(e) = heavies.get(rng.below(heavies.len())) {
        budget -= e.cost;
        lead_spawn = Some(GrindSpawn {
            enemy: e.enemy.clone(),
            health: health * g.boss_health,
            elite: true,
            boss: true,
        });
    }
    if heavy && !heavies.is_empty() {
        let n = 1 + (wave - 1) / g.boss_every.max(1);
        for _ in 0..n {
            let e = heavies[rng.below(heavies.len())];
            budget -= e.cost;
            spawns.push(GrindSpawn {
                enemy: e.enemy.clone(),
                health,
                elite: rng.chance(elite_chance),
                boss: false,
            });
        }
    }
    while spawns.len() + usize::from(lead_spawn.is_some()) < max {
        let affordable: Vec<_> = pool.iter().filter(|e| e.cost <= budget).collect();
        let total: f32 = affordable.iter().map(|e| e.weight).sum();
        if affordable.is_empty() || total <= 0.0 {
            break;
        }
        let mut x = rng.next_f32() * total;
        let mut pick = affordable[affordable.len() - 1];
        for e in &affordable {
            if x < e.weight {
                pick = e;
                break;
            }
            x -= e.weight;
        }
        budget -= pick.cost;
        spawns.push(GrindSpawn {
            enemy: pick.enemy.clone(),
            health,
            elite: rng.chance(elite_chance),
            boss: false,
        });
    }
    // Mixed arrivals (Fisher-Yates), the boss first.
    for i in (1..spawns.len()).rev() {
        let j = rng.below(i + 1);
        spawns.swap(i, j);
    }
    if let Some(b) = lead_spawn {
        spawns.insert(0, b);
    }
    WavePlan {
        wave,
        spawns,
        heavy,
        boss,
        budget: plan_budget,
    }
}

/// Server-side state of a grind run (`ModeState::grind`).
#[derive(Clone, Debug, Default)]
pub struct GrindState {
    /// Current wave (1-based; 0 before the first).
    pub wave: u32,
    /// Waves fully cleared.
    pub cleared: u32,
    /// Breather: the next wave starts at this tick (`None` while a wave is fought).
    pub next_wave: Option<u32>,
    /// Living Moulds of the current wave.
    pub alive: Vec<NetId>,
    /// Moulds of the current wave still waiting to spawn.
    pub queue: VecDeque<GrindSpawn>,
    /// Moulds the current wave holds.
    pub total: u32,
    pub heavy: bool,
    pub boss: bool,
    /// Everyone is down: the run ends at this tick.
    pub overrun: Option<u32>,
    next_spawn_tick: u32,
    rng: Option<ModeRng>,
    /// Nav nodes Moulds may appear on (nav fallback), found once per run.
    spawn_nodes: Option<Vec<u32>>,
}

impl GrindState {
    /// The run as the HUD sees it.
    pub fn status(&self) -> GrindStatus {
        GrindStatus {
            wave: self.wave,
            left: (self.alive.len() + self.queue.len()) as u32,
            total: self.total,
            next_wave_tick: self.next_wave,
            heavy: self.heavy,
            boss: self.boss,
            overrun_tick: self.overrun,
        }
    }

    /// Moulds of the current wave still to kill.
    pub fn left(&self) -> u32 {
        (self.alive.len() + self.queue.len()) as u32
    }
}

impl SimWorld {
    /// Grind step (pipeline step 6, while the match is playing): the overrun, wave clears and
    /// starts, trickled spawns.
    pub(crate) fn step_grind(&mut self, t: &Tuning) {
        let tick = self.tick;
        if self.mode.grind.next_wave.is_none() && self.mode.grind.wave == 0 {
            // A fresh run: the first breather.
            let at = tick.wrapping_add(self.grind_ticks(t.grind.first_delay));
            self.mode.grind.next_wave = Some(at);
            self.mode.touch();
        }
        if let Some(end) = self.mode.grind.overrun {
            if tick >= end {
                self.end_match(t, EndReason::Overrun);
            }
            return;
        }
        if !self.players.is_empty() && self.players.values().all(|p| p.combat.dead) {
            let end_tick = tick.wrapping_add(self.grind_ticks(t.grind.end_delay));
            for p in self.players.values_mut() {
                // Nobody is coming back: no respawn countdowns on the death screens.
                p.combat.respawn_in = u16::MAX;
            }
            self.mode.grind.overrun = Some(end_tick);
            self.mode.touch();
            self.push_mode(ModeEvent::Grind(GrindEvent::Overrun { end_tick }));
            return;
        }
        let enemies = &self.enemies;
        let before = self.mode.grind.alive.len();
        self.mode
            .grind
            .alive
            .retain(|id| enemies.get(id).is_some_and(|e| e.is_alive()));
        if self.mode.grind.alive.len() != before {
            self.mode.touch();
        }
        match self.mode.grind.next_wave {
            Some(at) if tick >= at => self.start_grind_wave(t),
            Some(_) => {}
            None => {
                self.trickle_grind_spawns(t);
                let g = &self.mode.grind;
                if g.alive.is_empty() && g.queue.is_empty() {
                    self.clear_grind_wave(t);
                }
            }
        }
    }

    fn grind_ticks(&self, secs: f32) -> u32 {
        secs_to_ticks(secs, self.tick_rate, 1) as u32
    }

    /// Ticks a player who is out waits: until the next wave in a breather, else "a long time"
    /// (the wave start brings them back; `respawn_in` must stay > 0 so their client keeps
    /// predicting a corpse).
    pub(crate) fn grind_wait_ticks(&self) -> u16 {
        match self.mode.grind.next_wave {
            Some(at) => at
                .saturating_sub(self.tick)
                .saturating_add(1)
                .min(u16::MAX as u32) as u16,
            None => u16::MAX,
        }
    }

    /// A dead (or downed, timed out) grind player is out until the next wave.
    pub(crate) fn grind_hold(&mut self, id: PlayerId) {
        let wait = self.grind_wait_ticks();
        if let Some(p) = self.players.get_mut(&id) {
            p.combat.dead = true;
            p.combat.respawn_in = wait;
        }
        let m = self.mode.player(id);
        if m.life != LifeState::Dead {
            m.life = LifeState::Dead;
            m.revive = None;
            self.mode.touch();
        }
    }

    /// Wave `w + 1`: everyone who is out comes back, the wave is planned and its first burst
    /// spawns.
    fn start_grind_wave(&mut self, t: &Tuning) {
        let out: Vec<PlayerId> = self
            .players
            .values()
            .filter(|p| p.combat.dead)
            .map(|p| p.id)
            .collect();
        for id in out {
            self.place_at_spawn(t, id);
        }
        let wave = self.mode.grind.wave + 1;
        let scaling = self.pve_scaling(t);
        let mut rng = self.grind_rng();
        let defs = &self.enemy_defs;
        let plan = plan_wave(
            t,
            wave,
            &scaling,
            &|name| defs.get(name).is_some(),
            &mut rng,
        );
        self.mode.grind.rng = Some(rng);
        let g = &mut self.mode.grind;
        g.wave = wave;
        g.next_wave = None;
        g.total = plan.spawns.len() as u32;
        g.heavy = plan.heavy;
        g.boss = plan.boss;
        g.queue = plan.spawns.into();
        g.next_spawn_tick = self.tick;
        log::info!(
            "grind wave {wave}: {} Moulds (budget {:.1}, {} player(s), {}{}{})",
            g.total,
            plan.budget,
            scaling.players,
            scaling.difficulty.title(),
            if plan.heavy { ", heavy" } else { "" },
            if plan.boss { ", boss" } else { "" }
        );
        let burst = t.grind.spawn_burst.max(1);
        for _ in 0..burst {
            if !self.spawn_next_grind_mould(t) {
                break;
            }
        }
        let interval = self.grind_ticks(t.grind.spawn_interval);
        self.mode.grind.next_spawn_tick = self.tick.wrapping_add(interval);
        self.mode.touch();
        self.push_mode(ModeEvent::Grind(GrindEvent::WaveStarted {
            wave,
            enemies: self.mode.grind.total,
            heavy: plan.heavy,
            boss: plan.boss,
        }));
    }

    /// One more Mould of the queue every `spawn_interval`, while fewer than the cap are alive.
    fn trickle_grind_spawns(&mut self, t: &Tuning) {
        if self.mode.grind.queue.is_empty() || self.tick < self.mode.grind.next_spawn_tick {
            return;
        }
        let players = self.coop_player_count().saturating_sub(1);
        let cap = (t.grind.max_alive + t.grind.alive_per_player * players) as usize;
        if self.mode.grind.alive.len() >= cap {
            return;
        }
        self.spawn_next_grind_mould(t);
        let interval = self.grind_ticks(t.grind.spawn_interval);
        self.mode.grind.next_spawn_tick = self.tick.wrapping_add(interval);
    }

    /// Spawns the next queued Mould. False when there is none or it can't spawn now (the world
    /// is full: it stays queued).
    fn spawn_next_grind_mould(&mut self, t: &Tuning) -> bool {
        let Some(s) = self.mode.grind.queue.pop_front() else {
            return false;
        };
        if self.enemy_defs.get(&s.enemy).is_none() {
            // Unknown after all (the registry changed): the wave is one Mould smaller.
            self.mode.grind.total = self.mode.grind.total.saturating_sub(1);
            self.mode.touch();
            return true;
        }
        let Some((position, yaw)) = self.grind_spawn_point(t, &s.enemy) else {
            self.mode.grind.queue.push_front(s);
            return false;
        };
        match self.spawn_pve_enemy(&s.enemy, position, yaw, s.health, s.elite) {
            Some(id) => {
                self.mode.grind.alive.push(id);
                // Counted here: the mode step already accounted this tick's events.
                self.mode.enemies_spawned += 1;
                self.mode.touch();
                true
            }
            None => {
                self.mode.grind.queue.push_front(s);
                false
            }
        }
    }

    /// The wave's last Mould died: breather, and the players who are out see the countdown.
    fn clear_grind_wave(&mut self, t: &Tuning) {
        let next_tick = self.tick.wrapping_add(self.grind_ticks(t.grind.breather));
        let g = &mut self.mode.grind;
        g.cleared = g.wave;
        g.next_wave = Some(next_tick);
        g.heavy = false;
        g.boss = false;
        let wave = g.wave;
        let wait = self.grind_wait_ticks();
        let out: Vec<PlayerId> = self
            .mode
            .players
            .iter()
            .filter(|(_, m)| m.life == LifeState::Dead)
            .map(|(id, _)| *id)
            .collect();
        for id in out {
            if let Some(p) = self.players.get_mut(&id) {
                p.combat.respawn_in = wait;
            }
        }
        self.mode.touch();
        log::info!("grind wave {wave} cleared");
        self.push_mode(ModeEvent::Grind(GrindEvent::WaveCleared {
            wave,
            next_tick,
        }));
    }

    fn grind_rng(&self) -> ModeRng {
        self.mode.grind.rng.unwrap_or_else(|| {
            ModeRng::new(name_seed(&self.level_name) ^ ((self.mode.start_tick as u64) << 20))
        })
    }

    /// Feet and yaw for a new grind Mould of type `enemy`: a `grind_spawns` point (else a nav
    /// node) away from the players, on free ground (flyers: in the air above it), facing the
    /// nearest player. `None` if nothing fits right now.
    fn grind_spawn_point(&mut self, t: &Tuning, enemy: &str) -> Option<(Vec3, f32)> {
        let def = self.enemy_defs.get(enemy)?;
        let flying = matches!(def.locomotion, Locomotion::Flying { .. });
        let (r, h) = (def.radius, def.height.max(2.0 * def.radius));
        let class = NavClass::for_size(def.radius, def.height);
        let players: Vec<Vec3> = {
            let living: Vec<Vec3> = self
                .players
                .values()
                .filter(|p| !p.combat.dead)
                .map(|p| p.movement.position)
                .collect();
            if living.is_empty() {
                self.level.spawns.iter().map(|s| s.position).collect()
            } else {
                living
            }
        };
        let away = |p: Vec3| {
            players
                .iter()
                .map(|q| Vec3::new(q.x - p.x, 0.0, q.z - p.z).length())
                .fold(f32::INFINITY, f32::min)
        };
        let min_dist = t.grind.min_spawn_distance;
        let taken: Vec<(f32, Vec3)> = self
            .enemies
            .values()
            .filter(|e| e.is_alive())
            .map(|e| (e.radius(), e.feet()))
            .collect();
        let mut rng = self.grind_rng();
        let ground = if !self.level.grind_spawns.is_empty() {
            // Authored points: a random one of those far enough away, else the farthest.
            let points = self.level.grind_spawns.clone();
            let far: Vec<Vec3> = points
                .iter()
                .copied()
                .filter(|p| away(*p) >= min_dist)
                .collect();
            let point = if far.is_empty() {
                points
                    .iter()
                    .copied()
                    .max_by(|a, b| away(*a).total_cmp(&away(*b)))?
            } else {
                far[rng.below(far.len())]
            };
            let free = !taken.iter().any(|(or, q)| {
                Vec3::new(q.x - point.x, 0.0, q.z - point.z).length() < r + or + 0.3
                    && (q.y - point.y).abs() < 2.5
            });
            if free {
                Some(point)
            } else {
                let spawn = EnemySpawn {
                    enemy: enemy.to_string(),
                    position: point,
                    yaw: 0.0,
                };
                // Flyers are placed above the point below, so search the ground ring for them.
                let copy = rng.below(10);
                if flying {
                    Some(point + Vec3::new(rng.next_f32() - 0.5, 0.0, rng.next_f32() - 0.5) * 4.0)
                } else {
                    self.find_copy_spot(&spawn, copy, &taken)
                }
            }
        } else {
            self.grind_nav_spawn(class, r, h, &taken, &away, min_dist, &mut rng)
        };
        self.mode.grind.rng = Some(rng);
        let mut feet = ground?;
        if flying {
            let world = &self.collision.world;
            let lift = [FLYER_SPAWN_HEIGHT, 2.5, 1.0].into_iter().find(|up| {
                let center = feet + Vec3::Y * (up + h * 0.5);
                !world.intersects_capsule(center, (h * 0.5 - r).max(0.0), r + 0.2, MASK_MOVEMENT)
            })?;
            feet.y += lift;
        }
        let nearest = players
            .iter()
            .copied()
            .min_by(|a, b| {
                a.distance_squared(feet)
                    .total_cmp(&b.distance_squared(feet))
            })
            .unwrap_or(Vec3::ZERO);
        let to = nearest - feet;
        Some((feet, (-to.x).atan2(-to.z)))
    }

    /// A random walkable nav node (that can reach the player spawns) for an agent of `class`,
    /// at least `min_dist` from every living player and clear of other bodies; the farthest
    /// usable one tried if none is that far. Feet on the floor.
    #[allow(clippy::too_many_arguments)]
    fn grind_nav_spawn(
        &mut self,
        class: NavClass,
        r: f32,
        h: f32,
        taken: &[(f32, Vec3)],
        away: &dyn Fn(Vec3) -> f32,
        min_dist: f32,
        rng: &mut ModeRng,
    ) -> Option<Vec3> {
        if self.ai.nav.is_none() {
            self.build_nav();
        }
        if self.mode.grind.spawn_nodes.is_none() {
            let nodes = self.grind_spawn_nodes();
            log::info!("grind: {} nav nodes usable as spawn points", nodes.len());
            self.mode.grind.spawn_nodes = Some(nodes);
        }
        let nav = self.ai.nav.as_deref()?;
        let nodes = self.mode.grind.spawn_nodes.as_deref()?;
        if nodes.is_empty() {
            return None;
        }
        let world = &self.collision.world;
        let mut best: Option<(f32, Vec3)> = None;
        for _ in 0..NAV_SPAWN_TRIES {
            let n = nodes[rng.below(nodes.len())];
            if !nav.passable(n, class, &[]) {
                continue;
            }
            let feet = nav.nodes[n as usize].pos + Vec3::Y * 0.02;
            if taken.iter().any(|(or, q)| {
                Vec3::new(q.x - feet.x, 0.0, q.z - feet.z).length() < r + or + 0.3
                    && (q.y - feet.y).abs() < 2.5
            }) {
                continue;
            }
            let center = feet + Vec3::Y * (h * 0.5 + 0.03);
            if world.intersects_capsule(center, (h * 0.5 - r).max(0.0), r, MASK_MOVEMENT) {
                continue;
            }
            let d = away(feet);
            if d >= min_dist {
                return Some(feet);
            }
            if best.is_none_or(|(b, _)| d > b) {
                best = Some((d, feet));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Walkable, safe nav nodes from which the player spawns can be reached (a Mould spawned
    /// there can always come for the players): a breadth-first search backwards along the
    /// links from the nodes under the level's spawn points.
    fn grind_spawn_nodes(&self) -> Vec<u32> {
        let Some(nav) = self.ai.nav.as_deref() else {
            return Vec::new();
        };
        let class = NavClass::SMALL;
        let n = nav.nodes.len();
        let mut back: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (i, node) in nav.nodes.iter().enumerate() {
            for &l in &node.links {
                if l != nav::NONE && (l as usize) < n {
                    back[l as usize].push(i as u32);
                }
            }
        }
        let mut seen = vec![false; n];
        let mut queue = VecDeque::new();
        let seeds = self.level.spawns.iter().map(|s| s.position);
        for p in seeds.chain(self.level.grind_spawns.iter().copied()) {
            if let Some(k) = nav.nearest_node(p, 4, class, &[])
                && !seen[k as usize]
            {
                seen[k as usize] = true;
                queue.push_back(k);
            }
        }
        while let Some(k) = queue.pop_front() {
            for &from in &back[k as usize] {
                if !seen[from as usize] && nav.passable(from, class, &[]) {
                    seen[from as usize] = true;
                    queue.push_back(from);
                }
            }
        }
        (0..n as u32)
            .filter(|&k| {
                let node = &nav.nodes[k as usize];
                seen[k as usize]
                    && nav.passable(k, class, &[])
                    && !node.is(node_flags::HAZARD)
                    && !node.is(node_flags::DEADLY)
            })
            .collect()
    }

    /// Grind respawns: the level spawn point whose nearest living Mould is farthest away (ties:
    /// the player's own slot), so nobody comes back inside a pack.
    pub(crate) fn grind_spawn_for(&self, id: PlayerId) -> Option<SpawnPoint> {
        let spawns = &self.level.spawns;
        if spawns.is_empty() {
            return None;
        }
        let index = self.players.keys().position(|p| *p == id).unwrap_or(0) % spawns.len();
        let enemies: Vec<Vec3> = self
            .enemies
            .values()
            .filter(|e| e.is_alive())
            .map(|e| e.feet())
            .collect();
        if enemies.is_empty() {
            return Some(spawns[index]);
        }
        let nearest = |s: &SpawnPoint| {
            enemies
                .iter()
                .map(|e| e.distance_squared(s.position))
                .fold(f32::INFINITY, f32::min)
        };
        let mut best = (nearest(&spawns[index]), spawns[index]);
        for s in spawns {
            let d = nearest(s);
            if d > best.0 + 1.0 {
                best = (d, *s);
            }
        }
        Some(best.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::Difficulty;

    fn plan(
        wave: u32,
        difficulty: Difficulty,
        players: u32,
        known: &dyn Fn(&str) -> bool,
    ) -> WavePlan {
        let t = Tuning::default();
        let s = PveScaling::compute(&t, difficulty, players);
        plan_wave(&t, wave, &s, known, &mut ModeRng::new(wave as u64))
    }

    fn roster_now(name: &str) -> bool {
        ["gnawer", "censer", "harrier", "gaoler", "heron"].contains(&name)
    }

    #[test]
    fn budgets_grow_with_the_wave() {
        let g = GrindTuning::default();
        let b: Vec<f32> = (1..=30).map(|w| wave_budget(&g, w)).collect();
        assert_eq!(b[0], g.budget_base);
        assert!(b.windows(2).all(|w| w[1] > w[0]));
        // Faster than linear: wave 20 holds more than 3x wave 5.
        assert!(b[19] > 3.0 * b[4], "{} vs {}", b[19], b[4]);
    }

    #[test]
    fn wave_one_is_gnawers_and_types_unlock_later() {
        let w1 = plan(1, Difficulty::Normal, 1, &roster_now);
        assert_eq!(w1.spawns.len(), 6, "{w1:?}");
        assert!(w1.spawns.iter().all(|s| s.enemy == "gnawer"));
        assert!(!w1.heavy && !w1.boss);
        let w9 = plan(9, Difficulty::Normal, 1, &roster_now);
        let kinds: std::collections::BTreeSet<&str> =
            w9.spawns.iter().map(|s| s.enemy.as_str()).collect();
        assert!(kinds.len() >= 3, "{kinds:?}");
        assert!(
            w9.spawns.iter().all(|s| s.health > 1.0),
            "health grows per wave"
        );
    }

    #[test]
    fn unknown_moulds_are_skipped_and_new_ones_join_when_known() {
        let w = plan(14, Difficulty::Normal, 2, &roster_now);
        assert!(w.spawns.iter().all(|s| roster_now(&s.enemy)));
        let everything = |_: &str| true;
        let all: std::collections::BTreeSet<String> = (12..40)
            .flat_map(|wave| plan(wave, Difficulty::Hard, 4, &everything).spawns)
            .map(|s| s.enemy)
            .collect();
        for name in ["bellwether", "snuffer", "smelter"] {
            assert!(all.contains(name), "{name} never shows up: {all:?}");
        }
        assert!(
            plan(3, Difficulty::Normal, 1, &|_: &str| false)
                .spawns
                .is_empty()
        );
    }

    #[test]
    fn heavy_and_boss_waves() {
        let w5 = plan(5, Difficulty::Normal, 1, &roster_now);
        assert!(w5.heavy && !w5.boss);
        assert!(
            w5.spawns.iter().any(|s| s.enemy == "gaoler"),
            "the Gaoler debuts"
        );
        let w10 = plan(10, Difficulty::Normal, 1, &roster_now);
        assert!(w10.heavy && w10.boss);
        let boss = &w10.spawns[0];
        assert!(boss.boss && boss.elite && boss.enemy == "gaoler");
        assert!(boss.health > 3.9 * w10.spawns[1].health);
        assert_eq!(w10.spawns.iter().filter(|s| s.boss).count(), 1);
        assert!(!plan(11, Difficulty::Normal, 1, &roster_now).heavy);
    }

    #[test]
    fn difficulty_and_players_make_waves_bigger() {
        let size = |d, p| plan(8, d, p, &roster_now).budget;
        assert!(size(Difficulty::Easy, 1) < size(Difficulty::Normal, 1));
        assert!(size(Difficulty::Normal, 1) < size(Difficulty::Inferno, 1));
        assert!(size(Difficulty::Normal, 4) > 2.5 * size(Difficulty::Normal, 1));
        let huge = plan(400, Difficulty::Inferno, 4, &roster_now);
        assert_eq!(
            huge.spawns.len(),
            GrindTuning::default().max_wave_enemies as usize
        );
    }
}
