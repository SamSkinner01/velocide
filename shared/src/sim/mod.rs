//! Authoritative game simulation state and tick.
//!
//! [`SimWorld`] is the whole game state the server simulates (and the client mirrors for
//! prediction): the level and its collision world, the players, and — as they are added —
//! enemies, projectiles, pickups, prisms and blood orbs.
//!
//! One server tick ([`SimWorld::step`]) is structured as a fixed pipeline so new systems slot
//! in without touching the networking:
//!
//! 1. **inputs → movement**: each queued [`PlayerInput`] is exactly one fixed movement step
//!    ([`player_move_step`], the same function client prediction runs).
//! 2. *(future)* weapons fired by those inputs (lag-compensated hitscan via the server's
//!    history), projectiles, enemy AI, pickups / blood orbs, prisms, encounters.
//! 3. **world rules**: kill volumes / falling out of the level.
//! 4. **events**: everything that happened is returned as [`SimEvent`]s; the server forwards
//!    the ones clients need reliably.
//!
//! Replication: [`SimWorld::replicate`] writes every replicated entity into a
//! [`NetFrame`]; new entity types add themselves there with their own [`kind`] constant.

mod hitbox;

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub use hitbox::{HitPart, HitShape, HitTarget, Hitbox};

use crate::input::PlayerInput;
use crate::level::{Brush, Door, JumpPad, LevelCollision, LevelDef, LevelKind, SpawnPoint, Volume};
use crate::movement::{self, MoveEnv, MoveEvents, MoveState};
use crate::protocol::{EntityState, NetFrame, NetId, door_flags, kind, player_flags};
use crate::tuning::{MovementTuning, Tuning};

/// Server-assigned player slot (small, stable for the session). Distinct from the transport's
/// connection id.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct PlayerId(pub u8);

impl std::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "P{}", self.0)
    }
}

/// Default simulation rate (ticks per second).
pub const DEFAULT_TICK_RATE: u32 = 60;

/// Seconds per tick for a tick rate. Client and server must use this exact expression so
/// prediction is bit-identical.
pub fn tick_dt(tick_rate: u32) -> f32 {
    1.0 / tick_rate.max(1) as f32
}

/// One movement step for one player. Used by the server simulation *and* client prediction
/// (anything that changes how an input moves a player belongs here, never in only one of them).
pub fn player_move_step(
    state: &mut MoveState,
    input: &PlayerInput,
    env: &MoveEnv,
    tuning: &MovementTuning,
    dt: f32,
) -> MoveEvents {
    movement::step(state, input, env, tuning, dt)
}

/// Inputs to apply this tick, per player, ascending `seq`. Each input is one movement step.
pub type TickInputs = BTreeMap<PlayerId, Vec<PlayerInput>>;

/// Past hitboxes for lag-compensated hit tests. Implemented by the server's
/// `server::lagcomp::LagCompHistory` and passed to [`SimWorld::step`]; weapons validating a
/// shot from `input` call `rewind(input.view_time_ticks(), tuning.net.lag_comp_max_ms)` to
/// test against what that player saw.
pub trait HitboxHistory {
    /// Every hitbox as it was at `view_time_ticks` (clamped to `max_rewind_ms` and to the
    /// recorded range), interpolated between recorded ticks.
    fn rewind_hitboxes(&self, view_time_ticks: f64, max_rewind_ms: f32) -> Vec<Hitbox>;
}

/// A player in the simulation.
#[derive(Clone, Debug)]
pub struct SimPlayer {
    pub id: PlayerId,
    pub name: String,
    pub movement: MoveState,
    /// View angles (radians) from the last processed input.
    pub yaw: f32,
    pub pitch: f32,
    /// Last processed input (buttons, weapon selection, view time for lag compensation).
    pub last_input: PlayerInput,
    /// Sequence number of the last processed input (0 = none yet).
    pub last_input_seq: u32,
    pub inputs_processed: u64,
}

impl SimPlayer {
    pub fn new(id: PlayerId, name: String, spawn: SpawnPoint) -> Self {
        Self {
            id,
            name,
            movement: MoveState::spawn(spawn.position),
            yaw: spawn.yaw.to_radians(),
            pitch: 0.0,
            last_input: PlayerInput::default(),
            last_input_seq: 0,
            inputs_processed: 0,
        }
    }

    /// Replicated state of this player.
    pub fn entity_state(&self, t: &MovementTuning) -> EntityState {
        let mut flags = 0;
        if self.movement.height(t) < t.standing_height - 1e-3 {
            flags |= player_flags::LOW_PROFILE;
        }
        if self.movement.is_invulnerable() {
            flags |= player_flags::INVULNERABLE;
        }
        let m = &self.movement;
        for (on, bit) in [
            (m.on_ground, player_flags::ON_GROUND),
            (m.is_sliding(), player_flags::SLIDING),
            (m.is_dashing(), player_flags::DASHING),
            (m.is_slamming(), player_flags::SLAMMING),
        ] {
            if on {
                flags |= bit;
            }
        }
        let mut aux = 0;
        if m.wall_clinging && m.wall_normal != Vec3::ZERO {
            flags |= player_flags::WALL_CLING;
            aux = crate::protocol::quantize_yaw((-m.wall_normal.x).atan2(-m.wall_normal.z)) as u32;
        }
        EntityState {
            id: NetId::player(self.id),
            kind: kind::PLAYER,
            owner: self.id.0 as u16,
            position: self.movement.position,
            velocity: self.movement.velocity,
            yaw: self.yaw,
            pitch: self.pitch,
            flags,
            state: self.last_input.weapon as u16 | (self.last_input.weapon_mode as u16) << 8,
            aux,
            ..Default::default()
        }
    }

    /// Body capsule + head sphere.
    pub fn hitbox(&self, t: &MovementTuning) -> Hitbox {
        let (center, half_height, radius) = self.movement.capsule(t);
        let head = self.movement.eye_position(t);
        Hitbox {
            target: HitTarget::Player(self.id),
            shapes: vec![
                HitShape::capsule(
                    HitPart::Body,
                    center - Vec3::Y * half_height,
                    center + Vec3::Y * half_height,
                    radius,
                ),
                HitShape::sphere(HitPart::Head, head, radius * 0.6),
            ],
        }
    }
}

/// Something that happened during a tick.
#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    Jumped {
        player: PlayerId,
    },
    Landed {
        player: PlayerId,
        speed: f32,
    },
    /// Teleported to a spawn point (fell out of the level, or later: died and respawned).
    Respawned {
        player: PlayerId,
        position: Vec3,
        yaw: f32,
    },
}

/// The simulation state.
pub struct SimWorld {
    /// Number of ticks simulated so far.
    pub tick: u32,
    pub tick_rate: u32,
    pub level_name: String,
    pub level: Arc<LevelDef>,
    pub collision: LevelCollision,
    pub players: BTreeMap<PlayerId, SimPlayer>,
    next_spawn: usize,
    next_dynamic_id: u32,
    /// Anything below this height is out of the level.
    kill_floor: f32,
}

impl SimWorld {
    pub fn new(level_name: &str, level: Arc<LevelDef>, tick_rate: u32) -> Self {
        let collision = level.build_collision();
        let lowest = level
            .brushes
            .iter()
            .map(|b| b.min.y.min(b.max.y))
            .fold(f32::INFINITY, f32::min);
        let kill_floor = if lowest.is_finite() {
            lowest - 50.0
        } else {
            -100.0
        };
        Self {
            tick: 0,
            tick_rate: tick_rate.max(1),
            level_name: level_name.to_string(),
            level,
            collision,
            players: BTreeMap::new(),
            next_spawn: 0,
            next_dynamic_id: NetId::DYNAMIC_START,
            kill_floor,
        }
    }

    /// Seconds per tick.
    pub fn dt(&self) -> f32 {
        tick_dt(self.tick_rate)
    }

    pub fn move_env(&self) -> MoveEnv<'_> {
        MoveEnv {
            world: &self.collision.world,
            jump_pads: &self.level.jump_pads,
        }
    }

    /// Allocates a fresh id for a dynamic replicated entity (never reused in a session).
    pub fn alloc_net_id(&mut self) -> NetId {
        let id = NetId(self.next_dynamic_id);
        self.next_dynamic_id = (self.next_dynamic_id + 1).min(NetId::STATIC_START - 1);
        id
    }

    /// Picks the next spawn point (round robin).
    pub fn next_spawn_point(&mut self) -> SpawnPoint {
        let spawns = &self.level.spawns;
        if spawns.is_empty() {
            return SpawnPoint {
                position: Vec3::new(0.0, 1.0, 0.0),
                yaw: 0.0,
            };
        }
        let spawn = spawns[self.next_spawn % spawns.len()];
        self.next_spawn = self.next_spawn.wrapping_add(1);
        spawn
    }

    /// Adds (or replaces) a player at the next spawn point.
    pub fn add_player(&mut self, id: PlayerId, name: &str) -> &mut SimPlayer {
        let spawn = self.next_spawn_point();
        self.players
            .insert(id, SimPlayer::new(id, name.to_string(), spawn));
        self.players.get_mut(&id).expect("just inserted")
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Option<SimPlayer> {
        self.players.remove(&id)
    }

    /// Restarts the level: every player is put back at the level start (spawn points handed
    /// out from the first one again, in player order) with fresh movement state. Returns a
    /// `Respawned` event per player. Systems that keep level state (encounters, enemies,
    /// pickups, doors) reset themselves here as they are added.
    pub fn restart_level(&mut self) -> Vec<SimEvent> {
        self.next_spawn = 0;
        let ids: Vec<PlayerId> = self.players.keys().copied().collect();
        let mut events = Vec::with_capacity(ids.len());
        for id in ids {
            let spawn = self.next_spawn_point();
            if let Some(p) = self.players.get_mut(&id) {
                p.movement = MoveState::spawn(spawn.position);
                p.yaw = spawn.yaw.to_radians();
                p.pitch = 0.0;
                events.push(SimEvent::Respawned {
                    player: id,
                    position: spawn.position,
                    yaw: p.yaw,
                });
            }
        }
        events
    }

    /// External impulse (explosions, knockback): changes velocity between movement steps.
    /// Clients pick it up through reconciliation.
    pub fn apply_impulse(&mut self, id: PlayerId, delta_velocity: Vec3) {
        if let Some(p) = self.players.get_mut(&id) {
            p.movement.velocity += delta_velocity;
        }
    }

    /// Advances one tick. See the module docs for the pipeline. `history` gives weapons
    /// lag-compensated hitboxes (the server passes its history; `None` where unavailable).
    pub fn step(
        &mut self,
        inputs: &TickInputs,
        tuning: &Tuning,
        history: Option<&dyn HitboxHistory>,
    ) -> Vec<SimEvent> {
        let _ = history; // used by weapons (step 2) once they exist
        self.tick = self.tick.wrapping_add(1);
        let mut events = Vec::new();
        let dt = self.dt();

        // 1. Inputs -> movement.
        let env = MoveEnv {
            world: &self.collision.world,
            jump_pads: &self.level.jump_pads,
        };
        for (id, list) in inputs {
            let Some(player) = self.players.get_mut(id) else {
                continue;
            };
            for input in list {
                player.yaw = input.yaw;
                player.pitch = input.pitch;
                let ev = player_move_step(&mut player.movement, input, &env, &tuning.movement, dt);
                player.last_input = *input;
                player.last_input_seq = input.seq;
                player.inputs_processed += 1;
                if ev.jumped {
                    events.push(SimEvent::Jumped { player: *id });
                }
                if let Some(speed) = ev.landed {
                    events.push(SimEvent::Landed { player: *id, speed });
                }
            }
        }

        // 2. Future systems: weapons (fired by the inputs above; hitscan uses `history`),
        //    projectiles, enemies, pickups, blood orbs, prisms, encounters (doors via
        //    `self.collision.set_door_open`).

        // 3. World rules.
        self.apply_world_rules(&mut events);

        events
    }

    fn apply_world_rules(&mut self, events: &mut Vec<SimEvent>) {
        let ids: Vec<PlayerId> = self
            .players
            .values()
            .filter(|p| {
                let pos = p.movement.position;
                !pos.is_finite()
                    || pos.y < self.kill_floor
                    || self.level.kill_volumes.iter().any(|v| v.contains(pos))
            })
            .map(|p| p.id)
            .collect();
        for id in ids {
            let spawn = self.next_spawn_point();
            if let Some(p) = self.players.get_mut(&id) {
                p.movement = MoveState::spawn(spawn.position);
                p.yaw = spawn.yaw.to_radians();
                p.pitch = 0.0;
                events.push(SimEvent::Respawned {
                    player: id,
                    position: spawn.position,
                    yaw: p.yaw,
                });
            }
        }
    }

    /// Writes every replicated entity into `frame` (cleared first).
    pub fn replicate(&self, tuning: &Tuning, frame: &mut NetFrame) {
        frame.entities.clear();
        for p in self.players.values() {
            frame.insert(&p.entity_state(&tuning.movement));
        }
        for door in &self.level.doors {
            let mut flags = 0;
            if self.collision.is_door_open(door.id) {
                flags |= door_flags::OPEN;
            }
            frame.insert(&EntityState {
                id: NetId::door(door.id),
                kind: kind::DOOR,
                position: door.brush.center(),
                flags,
                aux: door.id,
                ..Default::default()
            });
        }
        // Future: enemies, projectiles, pickups, prisms, blood orbs.
    }

    /// Applies replicated level state (door open/closed) received in a snapshot so client
    /// prediction collides against the same geometry as the server.
    pub fn apply_replicated_level_state(&mut self, frame: &NetFrame) {
        for (id, e) in &frame.entities {
            if e.kind == kind::DOOR && id.0 >= NetId::STATIC_START {
                self.collision
                    .set_door_open(e.aux, e.flags & door_flags::OPEN != 0);
            }
        }
    }

    /// Current hitboxes of everything that can be shot (players now, enemies later).
    pub fn hitboxes(&self, tuning: &Tuning, out: &mut Vec<Hitbox>) {
        out.clear();
        for p in self.players.values() {
            out.push(p.hitbox(&tuning.movement));
        }
    }
}

/// Name of the level built into the binary (always available, used by tests and as a
/// fallback when `assets/levels/` has no levels yet).
pub const BUILTIN_LEVEL: &str = "proving_ground";

/// Built-in levels by name.
pub fn builtin_level(name: &str) -> Option<LevelDef> {
    (name == BUILTIN_LEVEL).then(proving_ground)
}

/// Loads `assets/levels/<name>.ron`, falling back to a built-in level of that name.
pub fn load_level(name: &str) -> anyhow::Result<LevelDef> {
    let path = LevelDef::path_for(name);
    if path.exists() {
        return LevelDef::load(&path);
    }
    builtin_level(name).ok_or_else(|| {
        anyhow::anyhow!(
            "level '{name}' not found ({} does not exist and there is no built-in level of that name)",
            path.display()
        )
    })
}

fn brush(min: Vec3, max: Vec3, material: &str, tint: Vec3) -> Brush {
    Brush {
        min,
        max,
        rotation: Vec3::ZERO,
        material: material.to_string(),
        tint,
        clip_only: false,
        no_collide: false,
    }
}

/// A walled 120 m square test arena with a few blocks, a ramp, a door and a jump pad.
fn proving_ground() -> LevelDef {
    let grey = Vec3::splat(0.8);
    let rust = Vec3::new(0.8, 0.45, 0.35);
    let mut brushes = vec![
        brush(
            Vec3::new(-60.0, -1.0, -60.0),
            Vec3::new(60.0, 0.0, 60.0),
            "concrete",
            grey,
        ),
        brush(
            Vec3::new(-61.0, 0.0, -61.0),
            Vec3::new(61.0, 8.0, -60.0),
            "brick",
            rust,
        ),
        brush(
            Vec3::new(-61.0, 0.0, 60.0),
            Vec3::new(61.0, 8.0, 61.0),
            "brick",
            rust,
        ),
        brush(
            Vec3::new(-61.0, 0.0, -60.0),
            Vec3::new(-60.0, 8.0, 60.0),
            "brick",
            rust,
        ),
        brush(
            Vec3::new(60.0, 0.0, -60.0),
            Vec3::new(61.0, 8.0, 60.0),
            "brick",
            rust,
        ),
        brush(
            Vec3::new(24.0, 0.0, 24.0),
            Vec3::new(30.0, 3.0, 30.0),
            "metal",
            grey,
        ),
        brush(
            Vec3::new(-34.0, 0.0, -30.0),
            Vec3::new(-31.0, 7.0, -27.0),
            "metal",
            grey,
        ),
        brush(
            Vec3::new(30.0, 0.0, -40.0),
            Vec3::new(45.0, 1.5, -30.0),
            "grate",
            grey,
        ),
    ];
    let mut ramp = brush(
        Vec3::new(-10.0, -1.0, -45.0),
        Vec3::new(-2.0, 1.0, -35.0),
        "metal",
        grey,
    );
    ramp.rotation = Vec3::new(15.0, 0.0, 0.0);
    brushes.push(ramp);

    let spawns = (0..8)
        .map(|i| {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let position = Vec3::new(a.sin() * 6.0, 0.0, a.cos() * 6.0);
            // Face the centre (see `input::look_dir`: yaw 0 looks down -Z).
            SpawnPoint {
                position,
                yaw: position.x.atan2(position.z).to_degrees(),
            }
        })
        .collect();

    LevelDef {
        name: "Proving Ground".into(),
        kind: LevelKind::Arena,
        brushes,
        spawns,
        doors: vec![Door {
            id: 1,
            brush: brush(
                Vec3::new(-50.0, 0.0, 40.0),
                Vec3::new(-49.5, 4.0, 46.0),
                "metal",
                rust,
            ),
            starts_open: true,
        }],
        jump_pads: vec![JumpPad {
            trigger: Volume {
                min: Vec3::new(-46.0, 0.0, -46.0),
                max: Vec3::new(-44.0, 0.5, -44.0),
            },
            velocity: Vec3::new(0.0, 22.0, 6.0),
        }],
        kill_volumes: vec![Volume {
            min: Vec3::new(-500.0, -200.0, -500.0),
            max: Vec3::new(500.0, -30.0, 500.0),
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Buttons;

    fn world() -> SimWorld {
        SimWorld::new(
            BUILTIN_LEVEL,
            Arc::new(builtin_level(BUILTIN_LEVEL).unwrap()),
            DEFAULT_TICK_RATE,
        )
    }

    #[test]
    fn players_move_with_inputs_and_replicate() {
        let mut w = world();
        let tuning = Tuning::default();
        let id = PlayerId(0);
        let start = w.add_player(id, "A").movement.position;
        let mut inputs = TickInputs::new();
        for seq in 1..=30 {
            inputs.insert(
                id,
                vec![PlayerInput {
                    seq,
                    forward: 127,
                    ..Default::default()
                }],
            );
            w.step(&inputs, &tuning, None);
        }
        let p = &w.players[&id];
        assert_eq!(p.last_input_seq, 30);
        assert!((p.movement.position - start).length() > 1.0);

        let mut frame = NetFrame::new();
        w.replicate(&tuning, &mut frame);
        let e = frame.get(NetId::player(id)).unwrap();
        assert!((e.position - p.movement.position).length() < 0.01);
        assert!(frame.get(NetId::door(1)).is_some());
    }

    #[test]
    fn simulation_is_deterministic() {
        let tuning = Tuning::default();
        let run = || {
            let mut w = world();
            w.add_player(PlayerId(1), "A");
            for seq in 1..=200u32 {
                let mut inputs = TickInputs::new();
                let mut input = PlayerInput {
                    seq,
                    forward: 127,
                    right: if seq % 50 < 25 { 60 } else { -60 },
                    yaw: seq as f32 * 0.03,
                    ..Default::default()
                };
                input.buttons.set(Buttons::JUMP, seq % 40 == 0);
                inputs.insert(PlayerId(1), vec![input]);
                w.step(&inputs, &tuning, None);
            }
            w.players[&PlayerId(1)].movement
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn falling_out_respawns() {
        let mut w = world();
        let tuning = Tuning::default();
        w.add_player(PlayerId(0), "A").movement.position = Vec3::new(0.0, -1000.0, 0.0);
        let events = w.step(&TickInputs::new(), &tuning, None);
        assert!(matches!(events[0], SimEvent::Respawned { .. }));
        assert!(w.players[&PlayerId(0)].movement.position.y > -1.0);
    }

    #[test]
    fn restart_puts_players_back_at_the_start() {
        let mut w = world();
        w.add_player(PlayerId(0), "A").movement.position = Vec3::new(20.0, 3.0, 20.0);
        w.add_player(PlayerId(1), "B").movement.velocity = Vec3::new(5.0, 0.0, 0.0);
        let events = w.restart_level();
        assert_eq!(events.len(), 2);
        let spawns = &w.level.spawns;
        assert_eq!(
            w.players[&PlayerId(0)].movement.position,
            spawns[0].position
        );
        assert_eq!(
            w.players[&PlayerId(1)].movement.position,
            spawns[1].position
        );
        assert_eq!(w.players[&PlayerId(1)].movement.velocity, Vec3::ZERO);
    }

    #[test]
    fn level_survives_bincode() {
        let level = builtin_level(BUILTIN_LEVEL).unwrap();
        let bytes = bincode::serde::encode_to_vec(&level, bincode::config::standard()).unwrap();
        let (back, _): (LevelDef, usize) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(back, level);
    }
}
