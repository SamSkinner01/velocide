//! Arena encounters (campaign levels): locked rooms with enemy waves. See `docs/enemies.md`.
//!
//! Flow per [`crate::level::Encounter`]:
//!
//! 1. **Idle** until a living player's feet enter its `trigger`. Then it **starts**: the
//!    `lock_doors` close, every living player outside `arena` is teleported to `rally` (co-op
//!    lock-out fix; a `Respawned` event makes their client snap the camera), and
//!    `SimEvent::EncounterStarted` fires.
//! 2. Wave *n* spawns `delay` seconds after the encounter started (wave 1) or after the previous
//!    wave was wiped out (`SimEvent::WaveStarted`). Co-op scaling: with `p` players (connected,
//!    non-spectating, counted when the wave spawns) a wave of `n` authored spawns has
//!    `round(n × (1 + 0.5 (p − 1)))` enemies — the extra copies are placed around the authored
//!    points (validated against the nav grid and the collision world) — each with health ×
//!    `(1 + 0.5 (p − 1))`. Wave enemies are *aware* (they hunt players they can't see).
//! 3. When the last wave is dead it is **cleared** for good: `unlock_doors` (and the entry
//!    `lock_doors`) open, `SimEvent::EncounterCleared`.
//!
//! State lives in `SimWorld::encounters`; `restart_level` resets everything, and the game modes
//! use [`SimWorld::reset_encounter`] / [`SimWorld::reset_uncleared`] when the team respawns at
//! a checkpoint. Each encounter is replicated as a static `kind::ENCOUNTER` entity (phase, wave,
//! enemies left) for the HUD and late joiners. Deathmatch (`LevelKind::Arena`) levels have no
//! encounters.

use glam::Vec3;

use crate::level::{EnemySpawn, LevelKind};
use crate::movement::MoveState;
use crate::protocol::{EntityState, NetFrame, NetId, OWNER_NONE, encounter_flags, kind};
use crate::sim::nav::NavClass;
use crate::sim::{Owner, SimEvent, SimPlayer, SimWorld};
use crate::tuning::Tuning;

use engine::physics::{MASK_MOVEMENT, MASK_SHOTS};

/// Where an encounter is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EncounterPhase {
    #[default]
    Idle,
    Active,
    /// Won; never restarts (until the level restarts or the modes reset it).
    Cleared,
}

/// Runtime state of one encounter.
#[derive(Clone, Debug, Default)]
pub struct EncounterState {
    pub phase: EncounterPhase,
    /// Current wave (0-based; the one alive, or the one about to spawn).
    pub wave: usize,
    /// Tick the pending wave spawns at (`None` once it is out).
    pub spawn_at: Option<u32>,
    /// Living enemies of the current wave.
    pub alive: Vec<NetId>,
    /// Enemies the current wave spawned.
    pub spawned: u32,
    /// Player count the current wave was scaled for.
    pub players: u32,
    pub started_tick: u32,
    pub cleared_tick: u32,
}

/// All encounters of the level (`SimWorld::encounters`).
#[derive(Clone, Debug, Default)]
pub struct Encounters {
    pub states: Vec<EncounterState>,
}

/// Does this player count as "in the game" for encounters (triggering, rally teleport)?
/// **Hook for the game modes**: downed / spectating players should return false.
pub fn counts_for_encounters(p: &SimPlayer) -> bool {
    !p.combat.dead
}

/// `round(n × (1 + 0.5 (players − 1)))`: enemies in a wave of `n` authored spawns.
pub fn scaled_count(n: usize, players: u32) -> usize {
    let extra = 0.5 * (players.max(1) - 1) as f32;
    (n as f32 * (1.0 + extra)).round() as usize
}

impl SimWorld {
    /// Players the co-op scaling counts (connected, non-spectating). **Hook for the game
    /// modes / lobby** (spectators).
    pub fn coop_player_count(&self) -> u32 {
        self.players.len().max(1) as u32
    }

    /// Health multiplier and enemy-count factor for a wave spawning now.
    fn wave_scaling(&self) -> (u32, f32) {
        if !self.rules.coop_enemy_scaling {
            return (1, 1.0);
        }
        let players = self.coop_player_count();
        (players, 1.0 + 0.5 * (players - 1) as f32)
    }

    fn encounter_ticks(&self, secs: f32) -> u32 {
        (secs.max(0.0) * self.tick_rate as f32).round() as u32
    }

    /// Runs the encounter rules (start on trigger, waves, clear). Called once per tick after
    /// the world rules.
    pub(crate) fn step_encounters(&mut self, t: &Tuning) {
        if self.level.kind != LevelKind::Campaign || self.level.encounters.is_empty() || self.frozen
        {
            return;
        }
        let n = self.level.encounters.len();
        if self.encounters.states.len() != n {
            self.encounters
                .states
                .resize_with(n, EncounterState::default);
        }
        let now = self.tick;
        for i in 0..n {
            match self.encounters.states[i].phase {
                EncounterPhase::Idle => {
                    let trigger = self.level.encounters[i].trigger;
                    let entered = self
                        .players
                        .values()
                        .any(|p| counts_for_encounters(p) && trigger.contains(p.movement.position));
                    if entered {
                        self.start_encounter(i);
                    }
                }
                EncounterPhase::Active => {
                    let enemies = &self.enemies;
                    let st = &mut self.encounters.states[i];
                    st.alive
                        .retain(|id| enemies.get(id).is_some_and(|e| e.is_alive()));
                    match st.spawn_at {
                        Some(at) if now >= at => self.spawn_wave(t, i),
                        Some(_) => {}
                        None if st.alive.is_empty() => {
                            let waves = self.level.encounters[i].waves.len();
                            if st.wave + 1 < waves {
                                st.wave += 1;
                                let delay = self.level.encounters[i].waves[st.wave].delay;
                                let at = now + self.encounter_ticks(delay).max(1);
                                self.encounters.states[i].spawn_at = Some(at);
                            } else {
                                self.clear_encounter(i);
                            }
                        }
                        None => {}
                    }
                }
                EncounterPhase::Cleared => {}
            }
        }
    }

    /// Starts encounter `index` now (doors lock, stragglers rally, wave 1 scheduled). Also used
    /// by tests and dev tools. No-op unless it is idle.
    pub fn start_encounter(&mut self, index: usize) {
        let Some(enc) = self.level.encounters.get(index).cloned() else {
            return;
        };
        if self.encounters.states.len() < self.level.encounters.len() {
            self.encounters
                .states
                .resize_with(self.level.encounters.len(), EncounterState::default);
        }
        if self.encounters.states[index].phase != EncounterPhase::Idle {
            return;
        }
        let now = self.tick;
        let first_delay = enc.waves.first().map_or(0.0, |w| w.delay);
        self.encounters.states[index] = EncounterState {
            phase: EncounterPhase::Active,
            wave: 0,
            spawn_at: Some(now + self.encounter_ticks(first_delay).max(1)),
            started_tick: now,
            ..Default::default()
        };
        for &door in &enc.lock_doors {
            self.collision.set_door_open(door, false);
        }
        // Co-op lock-out fix: living players outside the locked room join the fight.
        let mut rallied = Vec::new();
        if let (Some(arena), Some(rally)) = (enc.arena, enc.rally) {
            let yaw = rally.yaw.to_radians();
            for p in self.players.values_mut() {
                if counts_for_encounters(p) && !arena.contains(p.movement.position) {
                    p.movement = MoveState::spawn(rally.position);
                    p.yaw = yaw;
                    p.pitch = 0.0;
                    rallied.push(p.id);
                }
            }
            for &player in &rallied {
                self.cancel_pending_for_victim(player);
                self.events.push(SimEvent::Respawned {
                    player,
                    position: rally.position,
                    yaw,
                });
            }
        }
        log::info!(
            "encounter {} started ({} waves, doors {:?} locked{})",
            index + 1,
            enc.waves.len(),
            enc.lock_doors,
            if rallied.is_empty() {
                String::new()
            } else {
                format!(", {} player(s) pulled to the rally point", rallied.len())
            }
        );
        self.events.push(SimEvent::EncounterStarted {
            index: index as u32,
            waves: enc.waves.len() as u32,
        });
    }

    /// Spawns the current wave of encounter `index` (with co-op scaling).
    fn spawn_wave(&mut self, _t: &Tuning, index: usize) {
        let Some(enc) = self.level.encounters.get(index).cloned() else {
            return;
        };
        let wave_index = self.encounters.states[index].wave;
        let Some(wave) = enc.waves.get(wave_index) else {
            self.encounters.states[index].spawn_at = None;
            return;
        };
        let (players, health) = self.wave_scaling();
        let total = scaled_count(wave.spawns.len(), players);
        let points = self.wave_spawn_points(&wave.spawns, total);
        let mut ids = Vec::with_capacity(points.len());
        for (spawn, mut position) in points {
            let yaw = spawn.yaw.to_radians();
            // Flyers are authored at their hover centre; enemies are placed by their feet.
            if let Some(def) = self.enemy_defs.get(&spawn.enemy)
                && matches!(def.locomotion, super::enemy::Locomotion::Flying { .. })
            {
                position.y -= def.height.max(2.0 * def.radius) * 0.5;
            }
            match self.spawn_enemy_scaled(&spawn.enemy, position, yaw, health) {
                Some(id) => {
                    if let Some(e) = self.enemies.get_mut(&id) {
                        e.encounter = Some((index as u16, wave_index.min(255) as u8));
                        e.aware = true;
                    }
                    ids.push(id);
                }
                // The wave just has one enemy fewer (it still clears when the rest die).
                None => log::warn!(
                    "encounter {} wave {}: '{}' not spawned ({})",
                    index + 1,
                    wave_index + 1,
                    spawn.enemy,
                    if self.enemy_defs.get(&spawn.enemy).is_some() {
                        "enemy limit reached"
                    } else {
                        "unknown enemy"
                    }
                ),
            }
        }
        let st = &mut self.encounters.states[index];
        st.spawn_at = None;
        st.spawned = ids.len() as u32;
        st.players = players;
        st.alive = ids;
        log::info!(
            "encounter {} wave {}/{}: {} enemies ({} authored, {} player(s), health x{:.2})",
            index + 1,
            wave_index + 1,
            enc.waves.len(),
            st.spawned,
            wave.spawns.len(),
            players,
            health
        );
        self.events.push(SimEvent::WaveStarted {
            index: index as u32,
            wave: wave_index as u32 + 1,
            waves: enc.waves.len() as u32,
            enemies: self.encounters.states[index].spawned,
        });
    }

    fn clear_encounter(&mut self, index: usize) {
        let Some(enc) = self.level.encounters.get(index).cloned() else {
            return;
        };
        let st = &mut self.encounters.states[index];
        st.phase = EncounterPhase::Cleared;
        st.spawn_at = None;
        st.alive.clear();
        st.cleared_tick = self.tick;
        for &door in enc.unlock_doors.iter().chain(&enc.lock_doors) {
            self.collision.set_door_open(door, true);
        }
        log::info!(
            "encounter {} cleared, doors {:?} open",
            index + 1,
            enc.unlock_doors
        );
        self.events.push(SimEvent::EncounterCleared {
            index: index as u32,
        });
    }

    /// Resets encounter `index` to idle: its enemies (and their projectiles / pending hits)
    /// vanish and its doors go back to their level-start state, so entering the trigger starts
    /// it again. **Hook for the game modes** (team wiped → respawn at the checkpoint). Works on
    /// cleared encounters too.
    pub fn reset_encounter(&mut self, index: usize) {
        let Some(enc) = self.level.encounters.get(index).cloned() else {
            return;
        };
        let ids: Vec<NetId> = self
            .enemies
            .values()
            .filter(|e| e.encounter.is_some_and(|(i, _)| i as usize == index))
            .map(|e| e.id)
            .collect();
        for id in ids {
            self.remove_enemy(id);
        }
        for &door in enc.lock_doors.iter().chain(&enc.unlock_doors) {
            let start_open = self
                .level
                .doors
                .iter()
                .find(|d| d.id == door)
                .is_none_or(|d| d.starts_open);
            self.collision.set_door_open(door, start_open);
        }
        if let Some(st) = self.encounters.states.get_mut(index) {
            *st = EncounterState::default();
        }
        log::info!("encounter {} reset", index + 1);
    }

    /// Resets every encounter that is in progress (not idle, not cleared). **Hook for the game
    /// modes**: call it when everyone respawns at a checkpoint.
    pub fn reset_uncleared(&mut self) {
        let active: Vec<usize> = self
            .encounters
            .states
            .iter()
            .enumerate()
            .filter(|(_, s)| s.phase == EncounterPhase::Active)
            .map(|(i, _)| i)
            .collect();
        for i in active {
            self.reset_encounter(i);
        }
    }

    /// Back to the level start (`restart_level` clears enemies and doors itself).
    pub(crate) fn reset_encounters(&mut self) {
        self.encounters.states.clear();
        self.encounters
            .states
            .resize_with(self.level.encounters.len(), EncounterState::default);
    }

    /// Removes an enemy right away (no death, no heat released): its projectiles and pending melee
    /// hits go with it.
    pub fn remove_enemy(&mut self, id: NetId) {
        if self.enemies.remove(&id).is_none() {
            return;
        }
        // Its projectiles vanish too (parried ones now belong to a player and stay).
        self.projectiles.retain(|_, p| p.owner != Owner::Enemy(id));
        self.pending_hits
            .retain(|h| !matches!(h.key, super::parry::ParryKey::Melee(e, _) if e == id));
        self.events.push(SimEvent::EnemyRemoved { id });
    }

    /// The encounter in progress, if any: (index, wave 1-based, waves, enemies left in the
    /// wave). For HUDs and tools.
    pub fn active_encounter(&self) -> Option<(usize, u32, u32, u32)> {
        self.encounters
            .states
            .iter()
            .enumerate()
            .find(|(_, s)| s.phase == EncounterPhase::Active)
            .map(|(i, s)| {
                let waves = self.level.encounters.get(i).map_or(0, |e| e.waves.len());
                (i, s.wave as u32 + 1, waves as u32, s.alive.len() as u32)
            })
    }

    /// Positions for `total` enemies of a wave: the authored spawns, then extra copies of
    /// them (in order) placed around the originals on free, safe ground (flyers: free air).
    fn wave_spawn_points(&self, spawns: &[EnemySpawn], total: usize) -> Vec<(EnemySpawn, Vec3)> {
        let mut out: Vec<(EnemySpawn, Vec3)> =
            spawns.iter().map(|s| (s.clone(), s.position)).collect();
        if spawns.is_empty() {
            return out;
        }
        for k in 0..total.saturating_sub(spawns.len()) {
            let s = &spawns[k % spawns.len()];
            let copy = k / spawns.len();
            let pos = self.find_copy_spot(s, copy, &out).unwrap_or(s.position);
            out.push((s.clone(), pos));
        }
        out
    }

    /// A free spot near spawn `s` for its `copy`-th extra copy, not overlapping `taken`.
    fn find_copy_spot(
        &self,
        s: &EnemySpawn,
        copy: usize,
        taken: &[(EnemySpawn, Vec3)],
    ) -> Option<Vec3> {
        let def = self.enemy_defs.get(&s.enemy)?;
        let flying = matches!(def.locomotion, super::enemy::Locomotion::Flying { .. });
        let r = def.radius;
        let h = def.height.max(2.0 * r);
        let class = NavClass::for_size(r, def.height);
        let world = &self.collision.world;
        let nav = self.ai.nav.as_deref();
        let radius_of = |name: &str| self.enemy_defs.get(name).map_or(0.5, |d| d.radius);
        let start = copy as f32 * 1.3 + s.position.x * 0.17 + s.position.z * 0.11;
        for ring in [2.0 * r + 0.8, 2.0 * r + 1.8, 2.0 * r + 3.0, 2.0 * r + 4.5] {
            for k in 0..10 {
                let a = start + k as f32 * std::f32::consts::TAU / 10.0;
                let mut p = s.position + Vec3::new(a.cos(), 0.0, a.sin()) * ring;
                if taken.iter().any(|(o, q)| {
                    Vec3::new(q.x - p.x, 0.0, q.z - p.z).length() < r + radius_of(&o.enemy) + 0.3
                        && (q.y - p.y).abs() < 2.5
                }) {
                    continue;
                }
                // In sight of the authored spot (same room, same side of any wall).
                let eye = Vec3::Y * if flying { 0.0 } else { 1.0 };
                if world.line_blocked(s.position + eye, p + eye, MASK_SHOTS) {
                    continue;
                }
                if flying {
                    let lift = h * 0.5;
                    if world.intersects_capsule(p, (h * 0.5 - r).max(0.0), r + 0.2, MASK_MOVEMENT) {
                        continue;
                    }
                    let below = world
                        .cast_ray(p, Vec3::NEG_Y, 12.0, MASK_MOVEMENT)
                        .map(|hit| hit.distance);
                    if !below.is_some_and(|d| d >= lift + 1.0) {
                        continue;
                    }
                } else {
                    // Safe floor at about the same height (nav grid), capsule free there.
                    if let Some(nav) = nav.filter(|n| !n.is_empty()) {
                        let Some(n) = nav.node_at(p, 0.8, class, &[]) else {
                            continue;
                        };
                        let node = &nav.nodes[n as usize];
                        if (node.pos.y - s.position.y).abs() > 0.7
                            || node.is(crate::sim::nav::node_flags::HAZARD)
                            || Vec3::new(node.pos.x - p.x, 0.0, node.pos.z - p.z).length() > 1.0
                            || !nav.ground_ok(p, s.position.y, 0.3)
                        {
                            continue;
                        }
                        p.y = node.pos.y + 0.02;
                    } else {
                        let Some(hit) =
                            world.cast_ray(p + Vec3::Y * 1.0, Vec3::NEG_Y, 1.8, MASK_MOVEMENT)
                        else {
                            continue;
                        };
                        p.y = hit.point.y + 0.02;
                    }
                    let center = p + Vec3::Y * (h * 0.5 + 0.03);
                    if world.intersects_capsule(center, (h * 0.5 - r).max(0.0), r, MASK_MOVEMENT) {
                        continue;
                    }
                    if self.level.kill_volumes.iter().any(|v| v.contains(p)) {
                        continue;
                    }
                }
                return Some(p);
            }
        }
        None
    }

    /// Replicates every encounter as a static `kind::ENCOUNTER` entity: `position` = arena (or
    /// trigger) centre, `flags` = [`encounter_flags`], `state` = current wave (1-based, 0 before
    /// the first), `health` = enemies left in the wave, `aux` = waves | enemies the wave
    /// spawned << 16.
    pub(crate) fn replicate_encounters(&self, frame: &mut NetFrame) {
        for (i, enc) in self.level.encounters.iter().enumerate() {
            let st = self.encounters.states.get(i);
            let mut flags = 0;
            let (mut wave, mut left, mut spawned) = (0, 0, 0);
            if let Some(st) = st {
                match st.phase {
                    EncounterPhase::Idle => {}
                    EncounterPhase::Active => {
                        flags |= encounter_flags::ACTIVE;
                        if st.spawn_at.is_some() {
                            flags |= encounter_flags::BETWEEN_WAVES;
                        }
                        wave = st.wave as u16 + 1;
                        left = st.alive.len() as u16;
                        spawned = st.spawned;
                    }
                    EncounterPhase::Cleared => {
                        flags |= encounter_flags::CLEARED;
                        wave = enc.waves.len() as u16;
                    }
                }
            }
            let center = enc.arena.unwrap_or(enc.trigger).center();
            frame.insert(&EntityState {
                id: NetId::encounter(i as u32),
                kind: kind::ENCOUNTER,
                owner: OWNER_NONE,
                position: center,
                flags,
                state: wave,
                health: left,
                aux: (enc.waves.len() as u32 & 0xFFFF) | (spawned.min(0xFFFF) << 16),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coop_counts() {
        assert_eq!(scaled_count(4, 1), 4);
        assert_eq!(scaled_count(4, 2), 6);
        assert_eq!(scaled_count(4, 3), 8);
        assert_eq!(scaled_count(4, 4), 10);
        assert_eq!(scaled_count(3, 2), 5); // 4.5 rounds up
        assert_eq!(scaled_count(0, 4), 0);
    }
}
