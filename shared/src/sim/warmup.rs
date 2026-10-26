//! The warm-up (docs/lobby.md "Warm-up"): the world a lobby server runs on its hub level
//! ([`WARMUP_LEVEL`], `LevelKind::Hub`) while the players wait for the next match.
//!
//! * Players run around and shoot. A death respawns after [`WARMUP_RESPAWN_SECS`] at the hub
//!   spawn farthest from everyone (no checkpoints, no downed state, no checkpoint reset).
//! * Nothing is scored: no stats, scoreboard, limits or results (`SimWorld::step_modes` only
//!   runs the style meter, so style still reads while practising).
//! * Players hurt each other only when the *next* match is PvP ([`SimWorld::set_warmup_next`],
//!   called whenever the host changes the mode); otherwise their hits only knock back.
//!
//! The hub level's **encounters** are not locked-room fights here (`step_encounters` only runs
//! campaign levels); the warm-up reads them as practice stations:
//!
//! * A **fixture** is an encounter whose trigger has no volume ([`is_fixture`]): the spawns of
//!   its first wave stand there from the start (the firing range's Effigy dummies) and each
//!   comes back [`FIXTURE_RESPAWN_SECS`] after it dies. Spawns naming an enemy the server's
//!   registry doesn't know are skipped, so a hub works with or without them.
//! * Any other encounter is a **console**: a living player pressing `USE` inside its trigger
//!   summons its next wave (cycling through the waves; the previous pack is removed first) as a
//!   practice pack in its `arena` (the pen). A pack Mould that gets more than [`PEN_MARGIN`]
//!   out of the pen (a pounce over the rim) is put back where it was summoned. Each summon
//!   reports `SimEvent::WaveStarted` (index = the console's encounter, wave = which pack) so
//!   clients can name it.

use std::sync::Arc;

use glam::Vec3;

use super::combat::secs_to_ticks;
use super::modes::{ModeRules, ModeState};
use super::{CombatRules, Owner, PlayerId, SimEvent, SimWorld};
use crate::input::Buttons;
use crate::level::{Encounter, Volume};
use crate::lobby::MatchSettings;
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// The hub level lobby servers run while they wait (`assets/levels/vestry.ron`).
pub const WARMUP_LEVEL: &str = "vestry";
/// A warm-up death respawns after this long (seconds).
pub const WARMUP_RESPAWN_SECS: f32 = 1.0;
/// A fixture's dummy comes back this long after it died (seconds).
pub const FIXTURE_RESPAWN_SECS: f32 = 3.0;
/// A console accepts a new summon this long after the last one (seconds).
pub const CONSOLE_COOLDOWN_SECS: f32 = 1.0;
/// Pack Moulds this far outside their pen (m) are put back in it.
pub const PEN_MARGIN: f32 = 2.0;

/// Warm-up state of a world (`SimWorld::warmup`, `None` outside the warm-up).
#[derive(Clone, Debug, Default)]
pub struct Warmup {
    /// The next match is PvP: players' hits hurt each other (otherwise knockback only).
    pub pvp: bool,
    /// One per hub encounter.
    stations: Vec<Station>,
    /// Players holding `USE` last tick (a console reacts to the press, not the hold).
    use_held: Vec<PlayerId>,
}

#[derive(Clone, Debug, Default)]
struct Station {
    /// Fixture: one slot per spawn of the first wave.
    slots: Vec<Slot>,
    /// Console: the living pack and where each was summoned.
    pack: Vec<(NetId, Vec3)>,
    /// Console: the wave the next summon spawns.
    next_wave: usize,
    /// Console: no summon before this tick.
    ready_at: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    /// The standing enemy.
    id: Option<NetId>,
    /// When it may (re)spawn.
    respawn_at: u32,
    /// The registry doesn't know this enemy: never spawned.
    unknown: bool,
}

impl Warmup {
    /// Living enemies of console `index`'s current pack (tests, tools).
    pub fn pack(&self, index: usize) -> Vec<NetId> {
        self.stations
            .get(index)
            .map_or_else(Vec::new, |s| s.pack.iter().map(|(id, _)| *id).collect())
    }
}

/// A hub encounter whose trigger has no volume: its first wave stands there from the start
/// (firing-range dummies) instead of being summoned at a console.
pub fn is_fixture(e: &Encounter) -> bool {
    let size = (e.trigger.max - e.trigger.min).abs();
    size.min_element() <= 0.0
}

/// A hub encounter summoned with `USE` inside its trigger.
pub fn is_console(e: &Encounter) -> bool {
    !is_fixture(e)
}

/// `volume` grown by `margin` on every side.
fn grown(volume: &Volume, margin: f32) -> Volume {
    Volume {
        min: volume.min.min(volume.max) - Vec3::splat(margin),
        max: volume.min.max(volume.max) + Vec3::splat(margin),
    }
}

impl SimWorld {
    /// Turns this (hub) world into the lobby's warm-up: warm-up death / respawn rules, nothing
    /// scored, practice stations, and PvP damage only if `next` (the lobby's settings for the
    /// coming match) is a PvP mode. Called by the server on every fresh warm-up world.
    pub fn start_warmup(&mut self, next: &MatchSettings) {
        self.mode = ModeState::new(ModeRules::campaign());
        self.style.clear();
        self.warmup = Some(Warmup::default());
        self.set_warmup_next(next);
    }

    /// The host changed the next match's settings: players hurt each other in the warm-up only
    /// when that match is PvP. No-op outside the warm-up.
    pub fn set_warmup_next(&mut self, next: &MatchSettings) {
        let Some(w) = &mut self.warmup else {
            return;
        };
        w.pvp = next.mode.is_pvp();
        self.rules = CombatRules {
            friendly_fire: w.pvp,
            coop_enemy_scaling: false,
        };
    }

    /// This world is the lobby's warm-up.
    pub fn in_warmup(&self) -> bool {
        self.warmup.is_some()
    }

    /// Practice stations (pipeline step 5, after the world rules). No-op outside the warm-up.
    pub(crate) fn step_warmup(&mut self, t: &Tuning) {
        let Some(mut w) = self.warmup.take() else {
            return;
        };
        let level = Arc::clone(&self.level);
        if w.stations.len() != level.encounters.len() {
            w.stations
                .resize_with(level.encounters.len(), Station::default);
        }
        let now = self.tick;
        // Living players who pressed USE this tick (and where their feet are).
        let holding: Vec<(PlayerId, Vec3)> = self
            .players
            .values()
            .filter(|p| !p.combat.dead && p.last_input.buttons.contains(Buttons::USE))
            .map(|p| (p.id, p.movement.position))
            .collect();
        let pressed: Vec<Vec3> = holding
            .iter()
            .filter(|(id, _)| !w.use_held.contains(id))
            .map(|(_, at)| *at)
            .collect();
        w.use_held = holding.iter().map(|(id, _)| *id).collect();

        for (index, enc) in level.encounters.iter().enumerate() {
            if is_fixture(enc) {
                self.step_fixture(t, &mut w.stations[index], enc, now);
            } else {
                let used = pressed.iter().any(|&feet| {
                    enc.trigger.contains(feet) || enc.trigger.contains(feet + Vec3::Y)
                });
                self.step_console(&mut w.stations[index], index, enc, used, now);
            }
        }
        self.warmup = Some(w);
    }

    /// Keeps a fixture's dummies standing (each back a while after it dies).
    fn step_fixture(&mut self, _t: &Tuning, st: &mut Station, enc: &Encounter, now: u32) {
        let spawns = enc.waves.first().map_or(&[][..], |w| &w.spawns[..]);
        st.slots.resize(spawns.len(), Slot::default());
        let rate = self.tick_rate;
        for (slot, spawn) in st.slots.iter_mut().zip(spawns) {
            if slot.unknown {
                continue;
            }
            if let Some(id) = slot.id {
                if self.enemies.get(&id).is_some_and(|e| e.is_alive()) {
                    continue;
                }
                // Down: back after a while (the corpse melts meanwhile).
                slot.id = None;
                slot.respawn_at =
                    now.wrapping_add(secs_to_ticks(FIXTURE_RESPAWN_SECS, rate, 1) as u32);
            }
            if now < slot.respawn_at {
                continue;
            }
            if self.enemy_defs.get(&spawn.enemy).is_none() {
                slot.unknown = true;
                continue;
            }
            // `None` here means the world is full: try again next tick.
            slot.id = self.spawn_enemy(&spawn.enemy, spawn.position, spawn.yaw.to_radians());
        }
    }

    /// A practice console: summons its next pack on `used`, keeps the pack in its pen.
    fn step_console(
        &mut self,
        st: &mut Station,
        index: usize,
        enc: &Encounter,
        used: bool,
        now: u32,
    ) {
        st.pack
            .retain(|(id, _)| self.enemies.get(id).is_some_and(|e| e.is_alive()));
        if let Some(pen) = enc.arena.map(|a| grown(&a, PEN_MARGIN)) {
            // Out of the pit: back where it was summoned (health and target kept).
            for (id, home) in &st.pack {
                if let Some(e) = self.enemies.get_mut(id)
                    && !pen.contains(e.position)
                {
                    e.position = *home;
                    e.velocity = Vec3::ZERO;
                    e.nav = Default::default();
                }
            }
        }
        if !used || now < st.ready_at || enc.waves.is_empty() {
            return;
        }
        st.ready_at =
            now.wrapping_add(secs_to_ticks(CONSOLE_COOLDOWN_SECS, self.tick_rate, 1) as u32);
        for (id, _) in std::mem::take(&mut st.pack) {
            self.remove_practice_enemy(id);
        }
        let wave = st.next_wave % enc.waves.len();
        st.next_wave = wave + 1;
        for spawn in &enc.waves[wave].spawns {
            if let Some(id) = self.spawn_enemy(&spawn.enemy, spawn.position, spawn.yaw.to_radians())
            {
                if let Some(e) = self.enemies.get_mut(&id) {
                    // They know where the players are (no waiting to be found in the pen).
                    e.aware = true;
                }
                st.pack.push((id, spawn.position));
            }
        }
        self.events.push(SimEvent::WaveStarted {
            index: index as u32,
            wave: wave as u32 + 1,
            waves: enc.waves.len() as u32,
            enemies: st.pack.len() as u32,
        });
    }

    /// Takes a practice Mould out of the world at once (its pack was replaced).
    fn remove_practice_enemy(&mut self, id: NetId) {
        if self.enemies.remove(&id).is_some() {
            // Its parry-delayed hits go with it.
            self.pending_hits
                .retain(|h| h.info.source != Owner::Enemy(id));
            self.events.push(SimEvent::EnemyRemoved { id });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{EnemySpawn, LevelKind, Wave};
    use crate::lobby::GameMode;
    use crate::sim::{BUILTIN_LEVEL, DEFAULT_TICK_RATE, TickInputs, builtin_level};

    fn hub() -> SimWorld {
        let mut level = builtin_level(BUILTIN_LEVEL).unwrap();
        level.kind = LevelKind::Hub;
        let spawn = |enemy: &str, x: f32| EnemySpawn {
            enemy: enemy.into(),
            position: Vec3::new(x, 0.0, -20.0),
            yaw: 0.0,
        };
        // A firing range (one known dummy, one unknown) and a console with two packs.
        level.encounters.push(Encounter {
            trigger: Volume {
                min: Vec3::new(0.0, 0.0, -30.0),
                max: Vec3::new(0.0, 0.0, -30.0),
            },
            arena: None,
            rally: None,
            lock_doors: vec![],
            unlock_doors: vec![],
            waves: vec![Wave {
                delay: 0.0,
                spawns: vec![spawn("gnawer", 12.0), spawn("no_such_mould", 14.0)],
            }],
        });
        level.encounters.push(Encounter {
            trigger: Volume {
                min: Vec3::new(-2.0, -1.0, -2.0),
                max: Vec3::new(2.0, 3.0, 2.0),
            },
            arena: Some(Volume {
                min: Vec3::new(-30.0, -5.0, -40.0),
                max: Vec3::new(-10.0, 10.0, -10.0),
            }),
            rally: None,
            lock_doors: vec![],
            unlock_doors: vec![],
            waves: vec![
                Wave {
                    delay: 0.0,
                    spawns: vec![spawn("gnawer", -20.0), spawn("gnawer", -15.0)],
                },
                Wave {
                    delay: 0.0,
                    spawns: vec![spawn("censer", -20.0)],
                },
            ],
        });
        let mut w = SimWorld::new(WARMUP_LEVEL, Arc::new(level), DEFAULT_TICK_RATE);
        w.start_warmup(&MatchSettings::new(GameMode::Campaign, "crucible_of_veins"));
        w
    }

    fn step(w: &mut SimWorld, inputs: &TickInputs, ticks: u32) -> Vec<SimEvent> {
        let t = Tuning::default();
        (0..ticks).flat_map(|_| w.step(inputs, &t, None)).collect()
    }

    fn input(seq: u32, use_held: bool) -> TickInputs {
        let mut input = crate::input::PlayerInput {
            seq,
            ..Default::default()
        };
        input.buttons.set(Buttons::USE, use_held);
        TickInputs::from([(PlayerId(0), vec![input])])
    }

    #[test]
    fn fixtures_stand_and_come_back() {
        let mut w = hub();
        w.add_player(PlayerId(0), "A").notarget = true;
        step(&mut w, &TickInputs::new(), 2);
        assert_eq!(
            w.enemies.len(),
            1,
            "the known dummy stands, the unknown one is skipped"
        );
        let id = *w.enemies.keys().next().unwrap();
        w.kill_enemy(&Tuning::default(), id);
        step(&mut w, &TickInputs::new(), 30);
        assert!(w.enemies.values().filter(|e| e.is_alive()).count() == 0);
        step(
            &mut w,
            &TickInputs::new(),
            (FIXTURE_RESPAWN_SECS * 60.0) as u32 + 5,
        );
        assert_eq!(
            w.enemies.values().filter(|e| e.is_alive()).count(),
            1,
            "back again"
        );
    }

    #[test]
    fn consoles_summon_packs_in_turn_and_keep_them_penned() {
        let mut w = hub();
        w.add_player(PlayerId(0), "A").notarget = true;
        let p = w.players.get_mut(&PlayerId(0)).unwrap();
        p.movement.position = Vec3::new(0.0, 0.0, 0.0);
        // Holding USE summons once; a fresh press after the cooldown summons the next pack.
        let events = step(&mut w, &input(1, true), 5);
        let waves: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                SimEvent::WaveStarted { index: 1, wave, .. } => Some(*wave),
                _ => None,
            })
            .collect();
        assert_eq!(waves, vec![1]);
        assert_eq!(w.warmup.as_ref().unwrap().pack(1).len(), 2);
        step(
            &mut w,
            &input(2, false),
            (CONSOLE_COOLDOWN_SECS * 60.0) as u32,
        );
        let events = step(&mut w, &input(3, true), 1);
        assert!(events.iter().any(|e| matches!(
            e,
            SimEvent::WaveStarted {
                index: 1,
                wave: 2,
                enemies: 1,
                ..
            }
        )));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, SimEvent::EnemyRemoved { .. }))
                .count(),
            2,
            "the old pack is replaced"
        );
        // A Mould that gets out of the pen is put back where it was summoned.
        let id = w.warmup.as_ref().unwrap().pack(1)[0];
        w.enemies.get_mut(&id).unwrap().position = Vec3::new(40.0, 0.0, 40.0);
        step(&mut w, &TickInputs::new(), 1);
        let back = w.enemies[&id].position;
        assert!(back.distance(Vec3::new(-20.0, 0.0, -20.0)) < 1.0, "{back}");
    }

    #[test]
    fn damage_between_players_follows_the_next_mode() {
        let mut w = hub();
        assert!(w.in_warmup() && !w.rules.friendly_fire && !w.rules.coop_enemy_scaling);
        w.set_warmup_next(&MatchSettings::new(GameMode::Deathmatch, "cistern_nine"));
        assert!(w.rules.friendly_fire);
        w.set_warmup_next(&MatchSettings::new(GameMode::Grind, "cistern_nine"));
        assert!(!w.rules.friendly_fire);
    }

    #[test]
    fn deaths_respawn_quickly_without_scores() {
        let t = Tuning::default();
        let mut w = hub();
        w.add_player(PlayerId(0), "A");
        w.add_player(PlayerId(1), "B");
        step(&mut w, &TickInputs::new(), 2);
        w.kill_player(&t, PlayerId(0), crate::sim::Attack::World);
        assert!(w.players[&PlayerId(0)].combat.dead);
        let events = step(
            &mut w,
            &TickInputs::new(),
            (WARMUP_RESPAWN_SECS * 60.0) as u32 + 2,
        );
        assert!(
            !w.players[&PlayerId(0)].combat.dead,
            "respawned within ~1 s"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            SimEvent::Respawned {
                player: PlayerId(0),
                ..
            }
        )));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SimEvent::Mode(crate::sim::ModeEvent::Scoreboard(_)))),
            "nothing is scored in the warm-up"
        );
        assert_eq!(w.mode.stats(PlayerId(0)).deaths, 0);
    }
}
