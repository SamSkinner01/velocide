//! Helpers for the SimWorld-level combat tests: a flat test level, players placed and aimed
//! precisely, inputs carrying a simulated view lag, a hitbox history like the server's lag
//! compensation (so shots are tested against what a lagging player saw), and a fixed target
//! dummy so weapon numbers don't depend on the enemy roster.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::{Brush, LevelDef, LevelKind, SpawnPoint};
use shared::protocol::NetId;
use shared::sim::{
    EnemyDef, HitTarget, Hitbox, HitboxHistory, PlayerId, SimEvent, SimWorld, TickInputs,
};
use shared::tuning::Tuning;

/// Name of the [`target_dummy`] definition in every test arena.
pub const DUMMY: &str = "husk";

/// The weapon tests' target: a 40 HP, r 0.45 x 1.9 m walker with a r 0.26 head weak point
/// (x2) 1.66 m up and a telegraphed 15 damage swipe (the first roster's fodder numbers, which
/// the weapon tests were written against). Registered as [`DUMMY`] ("husk") in every
/// [`Arena`]; it is not a game enemy (net kind 63 is unused by the roster).
pub fn target_dummy() -> EnemyDef {
    EnemyDef::parse(
        r#"(
            name: "Target dummy",
            net_kind: 63,
            health: 40.0,
            speed: 9.0,
            acceleration: 60.0,
            turn_rate: 600.0,
            radius: 0.45,
            height: 1.9,
            body_top: Some(1.45),
            eye_height: 1.65,
            knockback_scale: 1.3,
            weak_points: [
                (name: "head", kind: Head, offset: (0.0, 1.66, -0.05), radius: 0.26, multiplier: 2.0),
            ],
            attacks: [
                (name: "swipe", kind: Melee, range: 2.2, arc: 120.0, damage: 15.0, knockback: 7.0,
                 windup: 0.45, strike: 0.12, recover: 0.45, cooldown: 0.5, windup_move_scale: 0.2),
            ],
        )"#,
    )
    .expect("the target dummy parses")
}

pub fn brush(min: [f32; 3], max: [f32; 3]) -> Brush {
    Brush {
        min: Vec3::from(min),
        max: Vec3::from(max),
        rotation: Vec3::ZERO,
        material: "concrete".into(),
        tint: Vec3::ONE,
        clip_only: false,
        no_collide: false,
    }
}

/// 200 m square floor at y = 0 plus `extra` brushes.
pub fn flat_level(kind: LevelKind, extra: Vec<Brush>) -> LevelDef {
    let mut brushes = vec![brush([-100.0, -1.0, -100.0], [100.0, 0.0, 100.0])];
    brushes.extend(extra);
    LevelDef {
        name: "combat test".into(),
        kind,
        brushes,
        spawns: vec![SpawnPoint {
            position: Vec3::new(0.0, 0.0, 30.0),
            yaw: 0.0,
        }],
        ..Default::default()
    }
}

/// Records hitboxes per tick and rewinds like `server::lagcomp::LagCompHistory`.
#[derive(Default)]
pub struct TestHistory {
    frames: VecDeque<(u32, Vec<Hitbox>)>,
    pub rate: u32,
}

impl TestHistory {
    pub fn record(&mut self, tick: u32, boxes: Vec<Hitbox>) {
        self.frames.push_back((tick, boxes));
        while self.frames.len() > 120 {
            self.frames.pop_front();
        }
    }
}

impl HitboxHistory for TestHistory {
    fn rewind_hitboxes(&self, view_time_ticks: f64, max_rewind_ms: f32) -> Vec<Hitbox> {
        let (Some(first), Some(last)) = (self.frames.front(), self.frames.back()) else {
            return Vec::new();
        };
        let latest = last.0 as f64;
        let earliest =
            (latest - max_rewind_ms as f64 / 1000.0 * self.rate as f64).max(first.0 as f64);
        let t = view_time_ticks.clamp(earliest, latest);
        let idx = self.frames.partition_point(|f| (f.0 as f64) <= t);
        let a = &self.frames[idx.saturating_sub(1)];
        match self.frames.get(idx) {
            Some(b) if idx > 0 => {
                let frac = ((t - a.0 as f64) / (b.0 - a.0) as f64) as f32;
                a.1.iter()
                    .map(|ha| match b.1.iter().find(|hb| hb.target == ha.target) {
                        Some(hb) => ha.lerp(hb, frac),
                        None => ha.clone(),
                    })
                    .collect()
            }
            _ => a.1.clone(),
        }
    }
}

/// A SimWorld with helpers. Inputs get sequence numbers and a view time `view_lag` ticks in
/// the past (what a client with that much latency + interpolation delay would send).
pub struct Arena {
    pub world: SimWorld,
    pub tuning: Tuning,
    pub history: TestHistory,
    pub use_history: bool,
    seqs: BTreeMap<PlayerId, u32>,
    /// Held input per player (repeated every tick unless overridden).
    pub held: BTreeMap<PlayerId, PlayerInput>,
    pub view_lag: BTreeMap<PlayerId, f64>,
    pub events: Vec<SimEvent>,
}

impl Arena {
    pub fn new(level: LevelDef) -> Self {
        let mut world = SimWorld::new("combat_test", Arc::new(level), 60);
        world.enemy_defs.insert(DUMMY, target_dummy());
        Self {
            world,
            tuning: Tuning::default(),
            history: TestHistory {
                rate: 60,
                ..Default::default()
            },
            use_history: true,
            seqs: BTreeMap::new(),
            held: BTreeMap::new(),
            view_lag: BTreeMap::new(),
            events: Vec::new(),
        }
    }

    pub fn coop() -> Self {
        Self::new(flat_level(LevelKind::Campaign, Vec::new()))
    }

    pub fn pvp() -> Self {
        Self::new(flat_level(LevelKind::Arena, Vec::new()))
    }

    /// Adds a player standing at `pos` (feet) looking along yaw (radians).
    pub fn player(&mut self, id: u8, pos: Vec3, yaw: f32) -> PlayerId {
        let id = PlayerId(id);
        let p = self.world.add_player(id, "p");
        p.movement.position = pos;
        p.movement.on_ground = true;
        p.movement.ground_normal = Vec3::Y;
        p.movement.mode = shared::movement::MoveMode::Walk;
        p.yaw = yaw;
        self.held.insert(
            id,
            PlayerInput {
                yaw,
                ..Default::default()
            },
        );
        id
    }

    /// Spawns a [`target_dummy`].
    pub fn husk(&mut self, pos: Vec3, yaw: f32) -> NetId {
        self.world
            .spawn_enemy(DUMMY, pos, yaw)
            .expect("the target dummy exists")
    }

    pub fn eye(&self, id: PlayerId) -> Vec3 {
        self.world.players[&id]
            .movement
            .eye_position(&self.tuning.movement)
    }

    /// Points player `id`'s held input at `target`.
    pub fn aim(&mut self, id: PlayerId, target: Vec3) {
        let dir = (target - self.eye(id)).normalize();
        let input = self.held.get_mut(&id).expect("player");
        input.yaw = (-dir.x).atan2(-dir.z);
        input.pitch = dir.y.clamp(-1.0, 1.0).asin();
    }

    pub fn set_buttons(&mut self, id: PlayerId, buttons: Buttons) {
        self.held.get_mut(&id).expect("player").buttons = buttons;
    }

    pub fn select(&mut self, id: PlayerId, weapon: u8, mode: u8) {
        let input = self.held.get_mut(&id).expect("player");
        input.weapon = weapon;
        input.weapon_mode = mode;
    }

    /// One tick with every player's held input (players listed in `idle` send nothing).
    pub fn tick(&mut self) -> Vec<SimEvent> {
        let mut inputs = TickInputs::new();
        let next_tick = self.world.tick + 1;
        for (id, input) in &self.held {
            let seq = self.seqs.entry(*id).or_insert(0);
            *seq += 1;
            let lag = self.view_lag.get(id).copied().unwrap_or(1.0);
            let view = (next_tick as f64 - lag).max(0.0);
            let mut input = *input;
            input.seq = *seq;
            input.view_tick = view.floor() as u32;
            input.view_frac = ((view.fract()) * 256.0) as u8;
            inputs.insert(*id, vec![input]);
        }
        let history: Option<&dyn HitboxHistory> = if self.use_history {
            Some(&self.history)
        } else {
            None
        };
        let events = self.world.step(&inputs, &self.tuning, history);
        let mut boxes = Vec::new();
        self.world.hitboxes(&self.tuning, &mut boxes);
        self.history.record(self.world.tick, boxes);
        self.events.extend(events.iter().cloned());
        events
    }

    pub fn ticks(&mut self, n: u32) -> Vec<SimEvent> {
        let mut all = Vec::new();
        for _ in 0..n {
            all.extend(self.tick());
        }
        all
    }

    /// Presses `buttons` for one tick, then releases them for `after` ticks.
    pub fn tap(&mut self, id: PlayerId, buttons: Buttons, after: u32) -> Vec<SimEvent> {
        self.set_buttons(id, buttons);
        let mut ev = self.tick();
        self.set_buttons(id, Buttons::NONE);
        ev.extend(self.ticks(after));
        ev
    }

    pub fn health(&self, id: PlayerId) -> f32 {
        self.world.players[&id].combat.health(&self.tuning.combat)
    }

    pub fn enemy_health(&self, id: NetId) -> f32 {
        self.world.enemies.get(&id).map_or(0.0, |e| e.health)
    }

    pub fn enemy_alive(&self, id: NetId) -> bool {
        self.world.enemies.get(&id).is_some_and(|e| e.is_alive())
    }

    pub fn husk_head(&self, id: NetId) -> Vec3 {
        let e = &self.world.enemies[&id];
        e.local_to_world(e.def.weak_points[0].offset)
    }

    pub fn husk_body(&self, id: NetId) -> Vec3 {
        self.world.enemies[&id].position + Vec3::Y * 0.9
    }
}

/// Damage dealt to `target` in `events`.
pub fn damage_to(events: &[SimEvent], target: HitTarget) -> f32 {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) if d.target == target => Some(d.dealt),
            _ => None,
        })
        .sum()
}

pub fn kills(events: &[SimEvent]) -> Vec<shared::sim::KillInfo> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Killed(k) => Some(*k),
            _ => None,
        })
        .collect()
}
