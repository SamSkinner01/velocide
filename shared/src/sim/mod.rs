//! Authoritative game simulation state and tick.
//!
//! [`SimWorld`] is the whole game state the server simulates (and the client mirrors for
//! prediction): the level and its collision world, the players (movement + combat state),
//! enemies, projectiles, thorns, brands and ember bursts.
//!
//! One server tick ([`SimWorld::step`]) is a fixed pipeline:
//!
//! 1. **inputs → movement → weapons**: each queued [`PlayerInput`] is exactly one fixed
//!    movement step ([`player_move_step`], the same function client prediction runs; dead
//!    players move with [`neutral_input`]), followed by [`weapons::step_weapons`] (also
//!    predictable) whose fire intents are resolved right away: lag-compensated hitscan via the
//!    server's history, area attacks, projectile spawns, punches/parries, thorn throws, slam
//!    shockwaves.
//! 2. **projectiles** (sweeps, impacts, explosions, parry-delayed impacts), then open parry
//!    windows are re-checked against the moved projectiles.
//! 3. **enemies** (AI, attacks, movement), parry-delayed enemy hits.
//! 4. **thorns**, **brands**, **status effects** (Burn, Pinned, burning patches),
//!    **ember bursts**, **vitals** (hard damage recovery, respawns).
//! 5. **world rules**: kill volumes / falling out of the level.
//! 6. **game mode + style** ([`modes`], [`style`]): stats and style scored from the tick's
//!    events, checkpoints, revives, the exit, deathmatch limits, the scoreboard.
//! 7. **events**: everything that happened is returned as [`SimEvent`]s; the server forwards
//!    them to clients (reliably or batched in snapshots).
//!
//! All damage goes through [`SimWorld::apply_damage`] (`combat.rs`). See `docs/combat.md`.
//!
//! Replication: [`SimWorld::replicate`] writes every replicated entity into a
//! [`NetFrame`]; new entity types add themselves there with their own [`kind`] constant.

pub mod area; // Gust, Discharge, Breach
pub mod brand;
pub mod combat;
pub mod embers;
pub mod encounter; // arena encounters (waves, doors, co-op scaling): docs/enemies.md
pub mod enemy;
mod hitbox;
pub mod hitscan;
pub mod loadouts; // requested / forced loadouts and when they apply: docs/arsenal.md
// Game modes (death rule, checkpoints, downed/revive, deathmatch, results) and the style
// meter: docs/style_modes.md.
pub mod modes;
pub mod nav; // nav grid for ground enemies: docs/enemies.md
pub mod parry;
mod payload; // what the expansion's projectiles do on contact: docs/arsenal.md
pub mod projectiles;
pub mod status; // Burn, Pinned, burning patches: docs/arsenal.md
pub mod style;
pub mod thorn;
pub mod trail;
pub mod weapons;

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub use brand::Brand;
pub use combat::{
    Attack, Blocked, CombatRules, DamageEvent, DamageFlags, DamageInfo, KillInfo, Owner,
    PlayerCombat,
};
pub use embers::{EmberBurst, HealSource};
pub use enemy::{AiState, Enemy, EnemyDef, EnemyRegistry, EnemyScale, lob_velocity};
pub use hitbox::{HitPads, HitPart, HitShape, HitTarget, Hitbox};
pub use modes::{
    EndReason, Grade, Grades, LifeState, MatchPhase, MatchResults, ModeEvent, ModeRules, ModeState,
    PlayerResult, PlayerStats, ScoreEntry, Scoreboard,
};
pub use parry::{ParryEvent, ParryTarget};
pub use projectiles::{Projectile, ProjectileKind};
pub use style::{PlayerStyle, StyleBonus, StyleEvent, StyleKey, StyleSystem, StyleView};
pub use thorn::{Thorn, ThornState};
pub use weapons::{
    Ability, FireIntent, Loadout, WeaponBuild, WeaponClass, WeaponKind, WeaponState, step_weapons,
};

use crate::input::PlayerInput;
use crate::level::{Brush, Door, JumpPad, LevelCollision, LevelDef, LevelKind, SpawnPoint, Volume};
use crate::lobby::MatchSettings;
use crate::movement::{self, MoveEnv, MoveEvents, MoveState, SlamLanded};
use crate::protocol::{
    EntityState, NetFrame, NetId, door_flags, kind, pack_player_weapon, player_flags,
};
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

/// Most enemies (corpses included) in the world at once: further spawns (dev `/spawn`, waves)
/// are refused, and an encounter wave simply has fewer enemies.
pub const MAX_ENEMIES: usize = 256;
/// Most projectiles in flight at once: further shots / enemy attacks spawn nothing.
pub const MAX_PROJECTILES: usize = 1024;

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

/// The input a dead player's body is simulated with: no movement axes or buttons, view angles
/// kept (the corpse keeps falling/sliding naturally). Client prediction must use the same
/// while its `OwnState::combat.dead` is set.
pub fn neutral_input(input: &PlayerInput) -> PlayerInput {
    PlayerInput {
        seq: input.seq,
        yaw: input.yaw,
        pitch: input.pitch,
        weapon: input.weapon,
        weapon_mode: input.weapon_mode,
        view_tick: input.view_tick,
        view_frac: input.view_frac,
        ..Default::default()
    }
}

/// Inputs to apply this tick, per player, ascending `seq`. Each input is one movement step.
pub type TickInputs = BTreeMap<PlayerId, Vec<PlayerInput>>;

/// Most `player`'s view may be rewound (ticks): `net.lag_comp_max_ms`, tightened by the
/// server's RTT-based [`SimPlayer::view_lag_limit_ms`].
fn max_rewind_ticks(net: &crate::tuning::NetTuning, player: &SimPlayer, tick_rate: u32) -> f64 {
    let mut ms = net.lag_comp_max_ms.max(0.0);
    if let Some(limit) = player.view_lag_limit_ms.filter(|l| !l.is_nan()) {
        ms = ms.min(limit.max(0.0));
    }
    ms as f64 / 1000.0 * tick_rate as f64
}

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
    /// Health, hard damage, death/respawn, weapons (sent to the owner in `OwnState`).
    pub combat: PlayerCombat,
    /// How far in the past (ticks) this player sees remote entities: server tick minus the
    /// view time of its last input, clamped to [`SimWorld::max_rewind_ticks`]. Used for parry
    /// tolerance (0 for bots).
    pub view_lag_ticks: f32,
    /// Server-side bound (ms) on how old this player's view may claim to be, from their
    /// measured round-trip time (set by the server every tick; `None` = unknown, only
    /// `net.lag_comp_max_ms` applies). The view time travels in the client's inputs, so
    /// without it a low-ping client could always claim the maximum rewind.
    pub view_lag_limit_ms: Option<f32>,
    /// Dev `/notarget`: enemies ignore this player.
    pub notarget: bool,
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
            combat: PlayerCombat::default(),
            view_lag_ticks: 0.0,
            view_lag_limit_ms: None,
            notarget: false,
        }
    }

    /// Replicated state including combat presentation fields (see `docs/combat.md`):
    /// `flags` += `DEAD`, `PARRY`, `CHARGING`, `DRAWING`, `PROTECTED`; `state` = held weapon,
    /// selected ability and charge / draw level ([`crate::protocol::player_weapon`]);
    /// `health` = health (rounded up); `aux` = hard damage (bits 0–7, rounded up) | thorn
    /// charges (bits 8–11) | wall-cling yaw (bits 16–31). (`BRANDED` is added by
    /// [`SimWorld::replicate`].)
    pub fn replicated_state(&self, t: &Tuning, tick_rate: u32) -> EntityState {
        let mut e = self.entity_state(&t.movement);
        let c = &self.combat;
        let w = &c.weapons;
        if c.dead {
            e.flags |= player_flags::DEAD;
            e.flags &= !player_flags::INVULNERABLE;
        }
        if w.parry > 0 {
            e.flags |= player_flags::PARRY;
        }
        if w.charge > 0 {
            e.flags |= player_flags::CHARGING;
        }
        if w.draw > 0 {
            e.flags |= player_flags::DRAWING;
        }
        if c.protection > 0 || c.god {
            e.flags |= player_flags::PROTECTED;
        }
        e.state = pack_player_weapon(
            w.current(),
            w.ability(),
            w.charge_level(&t.combat, tick_rate),
        );
        e.health = c.health(&t.combat).ceil().clamp(0.0, 65535.0) as u16;
        let hard = c.hard.ceil().clamp(0.0, 255.0) as u32;
        let thorns = w.thorns(&t.combat).min(15);
        // Keep the wall-cling yaw from `entity_state` in the high 16 bits.
        e.aux = (e.aux & 0xFFFF_0000) | hard | (thorns << 8);
        e
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
            let yaw = crate::protocol::quantize_yaw((-m.wall_normal.x).atan2(-m.wall_normal.z));
            aux = (yaw as u32) << 16;
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
            state: pack_player_weapon(
                self.combat.weapons.current(),
                self.combat.weapons.ability(),
                0,
            ),
            aux,
            ..Default::default()
        }
    }

    /// Body capsule (feet to shoulders) + head sphere at the eyes. The body stops below the
    /// head so the head is exposed from every side (headshots).
    pub fn hitbox(&self, t: &MovementTuning) -> Hitbox {
        let (center, half_height, radius) = self.movement.capsule(t);
        let head = self.movement.eye_position(t);
        let head_radius = radius * 0.6;
        let bottom = center - Vec3::Y * half_height;
        let top_y = (center.y + half_height)
            .min(head.y - head_radius - radius * 0.875)
            .max(bottom.y);
        Hitbox {
            target: HitTarget::Player(self.id),
            shapes: vec![
                HitShape::capsule(
                    HitPart::Body,
                    bottom,
                    Vec3::new(center.x, top_y, center.z),
                    radius,
                ),
                HitShape::sphere(HitPart::Head, head, head_radius),
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
    /// Teleported to a spawn point (fell out of the level, or died and respawned).
    Respawned {
        player: PlayerId,
        position: Vec3,
        yaw: f32,
    },

    // ---- combat (see docs/combat.md for who receives what) ----
    /// Every damage instance (also blocked ones: i-frames, friendly fire...).
    Damage(DamageEvent),
    /// A player or enemy died.
    Killed(KillInfo),
    Parried(ParryEvent),
    Healed {
        player: PlayerId,
        amount: f32,
        source: HealSource,
        /// Where the heal came from (burst position / the parrier).
        position: Vec3,
    },
    /// A player fired (hitscan: `ends` = end of each ray; projectile weapons: empty).
    Shot(ShotEvent),
    /// A Quill shot bounced off thorns: thorn positions, then the final end point.
    Ricochet {
        shooter: Owner,
        attack: Attack,
        points: Vec<Vec3>,
    },
    /// A Splinter shot burst into secondary traces at `origin` (end of each).
    Splinters {
        shooter: PlayerId,
        origin: Vec3,
        ends: Vec<Vec3>,
    },
    /// A projectile burst into secondary rays at `origin` (the Flak ring): the end of each.
    Burst {
        shooter: Owner,
        attack: Attack,
        origin: Vec3,
        ends: Vec<Vec3>,
    },
    /// An Arclash zap jumped between targets: the first target's hit point, then the centre
    /// of each target it chained to.
    ArcChain {
        shooter: Owner,
        points: Vec<Vec3>,
    },
    /// A Wisp zapped a target.
    WispZap {
        id: NetId,
        from: Vec3,
        to: Vec3,
    },
    /// A Brand shot branded `target` (hit at `position`).
    Branded {
        owner: PlayerId,
        target: HitTarget,
        position: Vec3,
    },
    /// A Gust turned a hostile projectile around (it now belongs to `player`).
    Deflected {
        player: PlayerId,
        id: NetId,
        kind: ProjectileKind,
        position: Vec3,
    },
    /// An Arclash Discharge nova around `position` (`charge` 0..=1).
    Discharge {
        player: PlayerId,
        position: Vec3,
        radius: f32,
        charge: f32,
    },
    /// A Gravewell canister stuck to the level and starts pulling.
    GravewellStuck {
        id: NetId,
        position: Vec3,
    },
    Explosion {
        position: Vec3,
        radius: f32,
        attack: Attack,
        owner: Owner,
    },
    Punched {
        player: PlayerId,
        /// Hit something or parried.
        hit: bool,
    },
    ThornThrown {
        player: PlayerId,
        id: NetId,
    },
    /// A flying thorn planted itself in the level.
    ThornPlanted {
        id: NetId,
        position: Vec3,
    },
    /// A shot ricocheted off a thorn (it is gone).
    ThornShattered {
        id: NetId,
        position: Vec3,
    },
    ThornExpired {
        id: NetId,
    },
    /// A slam landed: shockwave around `position` (`power` 0..=1).
    SlamShockwave {
        player: PlayerId,
        position: Vec3,
        power: f32,
        radius: f32,
    },
    ProjectileBounced {
        id: NetId,
        kind: ProjectileKind,
        position: Vec3,
    },
    /// A non-explosive projectile hit the level and vanished (enemy projectiles).
    ProjectileImpact {
        id: NetId,
        kind: ProjectileKind,
        position: Vec3,
        normal: Vec3,
    },
    EnemySpawned {
        id: NetId,
        kind: u16,
        position: Vec3,
    },
    /// The corpse is gone (entity removed).
    EnemyRemoved {
        id: NetId,
    },
    EnemyAlerted {
        id: NetId,
        target: Option<PlayerId>,
    },
    EnemyAttack {
        id: NetId,
        attack: u8,
        phase: EnemyAttackPhase,
        parryable: bool,
    },
    EnemyFired {
        id: NetId,
        attack: u8,
        position: Vec3,
    },
    EnemyShockwave {
        id: NetId,
        position: Vec3,
        radius: f32,
    },

    // ---- style & game modes (docs/style_modes.md) ----
    /// A style bonus / rank change (sent to that player only).
    Style(StyleEvent),
    /// Checkpoints, downed / revive, exit countdown, scoreboard, match end (everyone).
    Mode(ModeEvent),
    // ---- arena encounters (sim::encounter, docs/enemies.md) ----
    /// Encounter `index` locked its doors (`waves` waves to come).
    EncounterStarted {
        index: u32,
        waves: u32,
    },
    /// Wave `wave` (1-based) of `waves` spawned `enemies` enemies (co-op scaled).
    WaveStarted {
        index: u32,
        wave: u32,
        waves: u32,
        enemies: u32,
    },
    /// The last wave died: the exit doors are open.
    EncounterCleared {
        index: u32,
    },
}

/// A player's shot (for remote tracers, muzzle flashes and sounds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShotEvent {
    pub shooter: PlayerId,
    pub attack: Attack,
    /// Eye position the shot left from.
    pub origin: Vec3,
    /// End point of each hitscan ray (empty for projectile weapons).
    pub ends: Vec<Vec3>,
}

/// Phase of an enemy attack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyAttackPhase {
    /// Telegraph starts.
    Windup,
    /// The hit lands / projectiles leave.
    Strike,
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
    /// Mode, level and limits of the running match, set by the server at match start (the
    /// lobby's settings; docs/lobby.md). Game-mode rules (frag / time limits, respawns) read
    /// them here; `rules.friendly_fire` follows `match_settings` (see
    /// [`SimWorld::apply_match_settings`]). Client prediction copies keep the default.
    pub match_settings: MatchSettings,
    /// The match is over (results screen): enemies stop thinking and attacking and encounters
    /// don't trigger; players can still move. Set by the server's `end_match`; game modes may
    /// set it too (one shared flag). A fresh world starts unfrozen.
    pub frozen: bool,
    /// Game-mode combat rules (PvP friendly fire, co-op scaling). From the level kind; the
    /// server applies the lobby's match settings on top.
    pub rules: CombatRules,
    /// Difficulty scaling of enemy behaviour (damage, aggression), set by the game mode.
    pub enemy_scale: enemy::EnemyScale,
    /// Enemy definitions (built-ins; the server loads `assets/enemies/`).
    pub enemy_defs: EnemyRegistry,
    pub enemies: BTreeMap<NetId, Enemy>,
    pub projectiles: BTreeMap<NetId, Projectile>,
    pub thorns: BTreeMap<NetId, Thorn>,
    /// Live Quill brands, keyed by the branded target (`brand.rs`).
    pub brands: BTreeMap<HitTarget, Brand>,
    /// Live ember bursts (heal pools, `embers.rs`).
    pub embers: BTreeMap<NetId, EmberBurst>,
    /// Burn / Pinned status effects, keyed by the target (`status.rs`).
    pub statuses: BTreeMap<HitTarget, status::Status>,
    /// Burning patches on the ground (Flare, `status.rs`).
    pub fire_patches: BTreeMap<NetId, status::FirePatch>,
    /// The loadout each player asked for (`loadouts.rs`). The server carries it over to every
    /// new world, so it outlives matches.
    pub loadout_requests: BTreeMap<PlayerId, Loadout>,
    /// Loadouts a game mode forces on players (gun game), for this world only.
    pub forced_loadouts: BTreeMap<PlayerId, Loadout>,
    /// Warm-up lobby world (set by the server's lobby): loadout changes apply at once.
    pub warmup: bool,
    /// Parry-delayed enemy melee damage.
    pub pending_hits: Vec<parry::PendingHit>,
    /// Game-mode state: rules, stats, checkpoint, downed players, match phase (`modes.rs`).
    pub mode: ModeState,
    /// Every player's style meter (`style.rs`).
    pub style: StyleSystem,
    /// Enemy AI shared state: nav grid (lazy), A* buffers, stats (`enemy/steer.rs`).
    pub ai: enemy::AiWorld,
    /// Arena encounter states (`encounter.rs`); persist until `restart_level`.
    pub encounters: encounter::Encounters,
    /// Events of the tick being simulated.
    pub(crate) events: Vec<SimEvent>,
    /// The projectile step of the current tick has run.
    pub(crate) projectiles_stepped: bool,
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
        let rules = match level.kind {
            LevelKind::Arena => CombatRules::pvp(),
            LevelKind::Campaign | LevelKind::Hub => CombatRules::coop(),
        };
        let mode = ModeState::new(ModeRules::for_level(&level));
        Self {
            tick: 0,
            tick_rate: tick_rate.max(1),
            level_name: level_name.to_string(),
            level,
            collision,
            players: BTreeMap::new(),
            match_settings: MatchSettings::default(),
            frozen: false,
            rules,
            enemy_scale: enemy::EnemyScale::default(),
            enemy_defs: EnemyRegistry::builtin(),
            enemies: BTreeMap::new(),
            projectiles: BTreeMap::new(),
            thorns: BTreeMap::new(),
            brands: BTreeMap::new(),
            embers: BTreeMap::new(),
            statuses: BTreeMap::new(),
            fire_patches: BTreeMap::new(),
            loadout_requests: BTreeMap::new(),
            forced_loadouts: BTreeMap::new(),
            warmup: false,
            pending_hits: Vec::new(),
            mode,
            style: StyleSystem::default(),
            ai: enemy::AiWorld::default(),
            encounters: encounter::Encounters::default(),
            events: Vec::new(),
            projectiles_stepped: false,
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

    /// Allocates an id for a new dynamic replicated entity. Ids count up from
    /// `NetId::DYNAMIC_START`; after the last one (2²⁴ − 1) they wrap around, skipping ids
    /// still in use by an enemy, projectile, thorn or ember burst. `None` if no id is free:
    /// the spawn is refused.
    pub fn alloc_net_id(&mut self) -> Option<NetId> {
        let span = (NetId::STATIC_START - NetId::DYNAMIC_START) as usize;
        let live = self.enemies.len()
            + self.projectiles.len()
            + self.thorns.len()
            + self.embers.len()
            + self.fire_patches.len();
        // Of `live + 1` consecutive candidates at least one is free.
        for _ in 0..=live.min(span - 1) {
            let id = NetId(self.next_dynamic_id);
            self.next_dynamic_id = if self.next_dynamic_id + 1 >= NetId::STATIC_START {
                NetId::DYNAMIC_START
            } else {
                self.next_dynamic_id + 1
            };
            if !self.net_id_in_use(id) {
                return Some(id);
            }
        }
        log::warn!("no free entity id ({live} live entities): spawn refused");
        None
    }

    /// Is `id` held by a live dynamic entity?
    fn net_id_in_use(&self, id: NetId) -> bool {
        self.enemies.contains_key(&id)
            || self.projectiles.contains_key(&id)
            || self.thorns.contains_key(&id)
            || self.embers.contains_key(&id)
            || self.fire_patches.contains_key(&id)
    }

    /// Most player `id`'s view may be rewound for lag compensation and parries (ticks):
    /// `net.lag_comp_max_ms`, tightened by their RTT-based [`SimPlayer::view_lag_limit_ms`].
    pub fn max_rewind_ticks(&self, t: &Tuning, id: PlayerId) -> f64 {
        match self.players.get(&id) {
            Some(p) => max_rewind_ticks(&t.net, p, self.tick_rate),
            None => t.net.lag_comp_max_ms.max(0.0) as f64 / 1000.0 * self.tick_rate as f64,
        }
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

    /// Adds (or replaces) a player at the next spawn point, carrying their loadout
    /// ([`SimWorld::spawn_loadout`]).
    pub fn add_player(&mut self, id: PlayerId, name: &str) -> &mut SimPlayer {
        let spawn = self.next_spawn_point();
        let mut player = SimPlayer::new(id, name.to_string(), spawn);
        player.combat.weapons = WeaponState::new(self.spawn_loadout(id));
        self.players.insert(id, player);
        self.players.get_mut(&id).expect("just inserted")
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Option<SimPlayer> {
        self.forget_loadouts(id);
        let player = self.players.remove(&id)?;
        // Their thorns and brands vanish; their pending hits and projectiles' targets are moot.
        self.thorns.retain(|_, th| th.owner != id);
        self.brands
            .retain(|target, b| b.owner != id && *target != HitTarget::Player(id));
        self.statuses.remove(&HitTarget::Player(id));
        self.fire_patches
            .retain(|_, p| p.owner != Owner::Player(id));
        self.cancel_pending_for_victim(id);
        for e in self.enemies.values_mut() {
            if e.target == Some(id) {
                e.target = None;
            }
            e.aggro.retain(|a| a.0 != id);
        }
        Some(player)
    }

    /// Restarts the level: every player is put back at the level start (spawn points handed
    /// out from the first one again, in player order) with fresh movement state. Returns a
    /// `Respawned` event per player. Systems that keep level state (encounters, enemies,
    /// pickups, doors) reset themselves here as they are added.
    /// Stores the match settings (docs/lobby.md) and derives the combat rules from them:
    /// deathmatch is PvP, co-op scales enemies and hurts teammates only with
    /// `friendly_fire`. Called by the server on every new match world.
    pub fn apply_match_settings(&mut self, settings: MatchSettings) {
        self.rules = if settings.mode.is_pvp() {
            CombatRules::pvp()
        } else {
            CombatRules {
                friendly_fire: settings.friendly_fire,
                ..CombatRules::coop()
            }
        };
        // Game-mode rules (frag / time limits) start afresh with the settings (modes.rs).
        self.mode = ModeState::new(ModeRules::from_settings(&settings));
        self.style.clear();
        self.match_settings = settings;
    }

    pub fn restart_level(&mut self) -> Vec<SimEvent> {
        self.next_spawn = 0;
        // Combat state goes back to the level start too.
        self.enemies.clear();
        self.projectiles.clear();
        self.thorns.clear();
        self.brands.clear();
        self.embers.clear();
        self.statuses.clear();
        self.fire_patches.clear();
        self.pending_hits.clear();
        // Encounters go back to idle (doors are reset just below).
        self.reset_encounters();
        let level = Arc::clone(&self.level);
        for door in &level.doors {
            self.collision.set_door_open(door.id, door.starts_open);
        }
        let loadouts: Vec<Loadout> = self
            .players
            .keys()
            .map(|id| self.spawn_loadout(*id))
            .collect();
        for (p, loadout) in self.players.values_mut().zip(loadouts) {
            p.combat = PlayerCombat {
                god: p.combat.god,
                weapons: p.combat.weapons.respawned(loadout),
                ..Default::default()
            };
        }
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
        // Game mode: stats, style, checkpoint and the match clock start over (modes.rs).
        self.reset_modes();
        events
    }

    /// Moves a living player to `position` (feet) with a fresh movement state, like an
    /// encounter rally: pending hits on them are dropped and `Respawned` (emitted with the
    /// next step's events) snaps their client's view. Dev `/tp`, `/goto`. False if there is
    /// no such living player.
    pub fn teleport_player(&mut self, id: PlayerId, position: Vec3, yaw: Option<f32>) -> bool {
        let Some(p) = self.players.get_mut(&id) else {
            return false;
        };
        if p.combat.dead {
            return false;
        }
        p.movement = MoveState::spawn(position);
        p.yaw = yaw.unwrap_or(p.yaw);
        p.pitch = 0.0;
        let yaw = p.yaw;
        self.cancel_pending_for_victim(id);
        self.events.push(SimEvent::Respawned {
            player: id,
            position,
            yaw,
        });
        true
    }

    /// External impulse (explosions, recoil, knockback): changes velocity between movement
    /// steps.
    /// Clients pick it up through reconciliation.
    pub fn apply_impulse(&mut self, id: PlayerId, delta_velocity: Vec3) {
        if let Some(p) = self.players.get_mut(&id) {
            p.movement.velocity += delta_velocity;
        }
    }

    /// Advances one tick. See the module docs for the pipeline. `history` gives weapons
    /// lag-compensated hitboxes (the server passes its history; `None` where unavailable:
    /// shots then test current hitboxes).
    pub fn step(
        &mut self,
        inputs: &TickInputs,
        tuning: &Tuning,
        history: Option<&dyn HitboxHistory>,
    ) -> Vec<SimEvent> {
        self.tick = self.tick.wrapping_add(1);
        self.projectiles_stepped = false;
        let dt = self.dt();
        let tick_rate = self.tick_rate;

        // 1. Inputs -> movement -> weapons.
        for (id, list) in inputs {
            for input in list {
                let tick = self.tick;
                let env = MoveEnv {
                    world: &self.collision.world,
                    jump_pads: &self.level.jump_pads,
                };
                let Some(player) = self.players.get_mut(id) else {
                    break;
                };
                player.yaw = input.yaw;
                player.pitch = input.pitch;
                let alive = !player.combat.dead;
                let move_input = if alive { *input } else { neutral_input(input) };
                let ev = player_move_step(
                    &mut player.movement,
                    &move_input,
                    &env,
                    &tuning.movement,
                    dt,
                );
                player.last_input = *input;
                player.last_input_seq = input.seq;
                player.inputs_processed += 1;
                let view = input.view_time_ticks();
                let max_lag = max_rewind_ticks(&tuning.net, player, tick_rate);
                player.view_lag_ticks = if view.is_finite() {
                    (tick as f64 - view).clamp(0.0, max_lag) as f32
                } else {
                    0.0
                };
                let intents = step_weapons(
                    &mut player.combat.weapons,
                    input,
                    alive,
                    &tuning.combat,
                    tick_rate,
                );
                if ev.jumped {
                    self.events.push(SimEvent::Jumped { player: *id });
                }
                if let Some(speed) = ev.landed {
                    self.events.push(SimEvent::Landed { player: *id, speed });
                }
                if let Some(slam) = ev.slam_landed
                    && alive
                {
                    self.slam_shockwave(tuning, *id, slam);
                }
                for intent in intents {
                    self.resolve_intent(tuning, history, *id, input, intent);
                }
            }
        }

        // 2. Projectiles, then parry windows against their new positions.
        self.step_projectiles(tuning);
        self.projectiles_stepped = true;
        self.tick_parry_windows(tuning);

        // 3. Enemies and their parry-delayed hits (none land once the match is over).
        self.step_enemies(tuning);
        if !self.frozen {
            self.apply_due_pending_hits(tuning);
        }

        // 4. Thorns, brands, status effects, healing, vitals.
        self.step_thorns(tuning);
        self.step_brands();
        self.step_statuses(tuning);
        self.step_embers(tuning);
        self.tick_vitals(tuning);

        // 5. World rules, then arena encounters (triggers, waves, doors).
        self.apply_world_rules(tuning);
        self.step_encounters(tuning);

        // 6. Game mode + style: scores this tick's events, checkpoints, revives, exit,
        //    deathmatch limits, scoreboard (modes.rs / style.rs).
        self.step_modes(tuning);

        std::mem::take(&mut self.events)
    }

    /// Slam landing shockwave: damage + knockback (mostly upwards) to everything hostile
    /// within `movement.slam_shockwave_radius`, scaled by the slam's power.
    fn slam_shockwave(&mut self, t: &Tuning, id: PlayerId, slam: SlamLanded) {
        let c = &t.combat;
        let radius = t.movement.slam_shockwave_radius;
        let scale =
            c.slam_min_power_scale + (1.0 - c.slam_min_power_scale) * slam.power.clamp(0.0, 1.0);
        self.events.push(SimEvent::SlamShockwave {
            player: id,
            position: slam.position,
            power: slam.power,
            radius,
        });
        let me = Owner::Player(id);
        let mut hits = Vec::new();
        for h in self.current_hitboxes(t) {
            if !self.can_hit(me, h.target) {
                continue;
            }
            let Some(center) = h.center() else {
                continue;
            };
            let to = center - slam.position;
            let flat = Vec3::new(to.x, 0.0, to.z);
            if flat.length() > radius || to.y.abs() > 3.0 {
                continue;
            }
            let dir = flat.normalize_or(Vec3::X);
            let kb = (dir * 0.5 + Vec3::Y).normalize() * (c.slam_knockback * scale);
            let info = DamageInfo::new(c.slam_damage * scale, me, Attack::Slam)
                .at(center, dir)
                .knockback(kb)
                .flags(DamageFlags::SLAM);
            hits.push((h.target, info));
        }
        for (target, info) in hits {
            self.apply_damage(t, target, info);
        }
    }

    fn apply_world_rules(&mut self, t: &Tuning) {
        let kill_floor = self.kill_floor;
        let mut reset = Vec::new();
        let mut kill = Vec::new();
        for p in self.players.values_mut() {
            let pos = p.movement.position;
            let out = !pos.is_finite() || pos.y < kill_floor;
            if p.combat.dead {
                // Corpses stop at the kill floor (they respawn on their own).
                if out {
                    p.movement.velocity = Vec3::ZERO;
                    if !pos.is_finite() {
                        p.movement.position = Vec3::ZERO;
                    }
                    p.movement.position.y = p.movement.position.y.max(kill_floor);
                }
                continue;
            }
            if out {
                reset.push(p.id);
            } else if self.level.kill_volumes.iter().any(|v| v.contains(pos)) {
                kill.push(p.id);
            }
        }
        // Out of the level entirely: put back immediately (safety net, not a death).
        for id in reset {
            self.respawn_player(t, id);
        }
        for id in kill {
            if self.frozen {
                // Nobody dies on the results screen: back to a spawn instead.
                self.respawn_player(t, id);
            } else {
                self.kill_player(t, id, Attack::World);
            }
        }
    }

    /// Writes every replicated entity into `frame` (cleared first).
    pub fn replicate(&self, tuning: &Tuning, frame: &mut NetFrame) {
        frame.entities.clear();
        for p in self.players.values() {
            let mut e = p.replicated_state(tuning, self.tick_rate);
            // Game mode flags (co-op DOWNED), modes.rs.
            self.decorate_player_entity(&mut e);
            let target = HitTarget::Player(p.id);
            if self.is_branded(target) {
                e.flags |= player_flags::BRANDED;
            }
            if self.is_burning(target) {
                e.flags |= player_flags::BURNING;
            }
            if self.is_pinned(target) {
                e.flags |= player_flags::PINNED;
            }
            frame.insert(&e);
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
        for e in self.enemies.values() {
            let mut state = e.entity_state(self.tick);
            // Brands and statuses live in side tables (brand.rs, status.rs), not on the enemy.
            let target = HitTarget::Entity(e.id);
            if self.is_branded(target) {
                state.flags |= crate::protocol::enemy_flags::BRANDED;
            }
            if self.is_burning(target) {
                state.flags |= crate::protocol::enemy_flags::BURNING;
            }
            if self.is_pinned(target) {
                state.flags |= crate::protocol::enemy_flags::PINNED;
            }
            frame.insert(&state);
        }
        for p in self.projectiles.values() {
            frame.insert(&p.entity_state());
        }
        for th in self.thorns.values() {
            frame.insert(&th.entity_state());
        }
        for b in self.embers.values() {
            frame.insert(&b.entity_state());
        }
        for p in self.fire_patches.values() {
            frame.insert(&p.entity_state());
        }
        // Encounter state (HUD: wave, enemies left).
        self.replicate_encounters(frame);
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

    /// Current hitboxes of everything that can be shot: living players (body + head) and
    /// living enemies (body + weak points). The server records these every tick for lag
    /// compensation.
    pub fn hitboxes(&self, tuning: &Tuning, out: &mut Vec<Hitbox>) {
        out.clear();
        for p in self.players.values().filter(|p| !p.combat.dead) {
            out.push(p.hitbox(&tuning.movement));
        }
        for e in self.enemies.values().filter(|e| e.is_alive()) {
            out.push(e.hitbox());
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
    fn claimed_view_lag_is_bounded_per_player() {
        let t = Tuning::default(); // lag_comp_max_ms 350 = 21 ticks
        let mut w = world();
        let id = PlayerId(0);
        w.add_player(id, "A");
        for _ in 0..60 {
            w.step(&TickInputs::new(), &t, None);
        }
        let now = w.tick;
        let claim = PlayerInput {
            seq: 1,
            view_tick: now - 21,
            ..Default::default()
        };
        // No RTT known (single-player, tests): only `lag_comp_max_ms` applies.
        assert_eq!(w.view_time(&t, id, &claim), (now - 21) as f64);
        let unbounded_delay = w.parry_delay_ticks(&t, id);
        // The server measured a fast connection: at most 100 ms.
        w.players.get_mut(&id).unwrap().view_lag_limit_ms = Some(100.0);
        assert_eq!(w.max_rewind_ticks(&t, id), 6.0);
        assert_eq!(w.view_time(&t, id, &claim), now as f64 - 6.0);
        let mut inputs = TickInputs::new();
        inputs.insert(id, vec![claim]);
        w.step(&inputs, &t, None);
        assert_eq!(w.players[&id].view_lag_ticks, 6.0);
        // The parry delay (tolerance + view lag) shrinks with it: 4.8 + 6 ticks.
        assert_eq!(w.parry_delay_ticks(&t, id), 11);
        assert_eq!(unbounded_delay, 5, "no view lag before the first input");
        // A limit above `lag_comp_max_ms` changes nothing; garbage is ignored.
        for limit in [1000.0, f32::NAN] {
            w.players.get_mut(&id).unwrap().view_lag_limit_ms = Some(limit);
            assert_eq!(w.max_rewind_ticks(&t, id), 21.0);
        }
    }

    fn wisp(w: &mut SimWorld, t: &Tuning) -> Option<NetId> {
        let at = Vec3::new(0.0, 50.0, 0.0);
        w.spawn_projectile(t, ProjectileKind::Wisp, Owner::World, at, Vec3::ZERO)
    }

    #[test]
    fn net_ids_wrap_around_without_reusing_live_ones() {
        let t = Tuning::default();
        let mut w = world();
        let start = NetId::DYNAMIC_START;
        // One live entity of every kind at the start of the dynamic range.
        let projectile = wisp(&mut w, &t).unwrap();
        let enemy = w
            .spawn_enemy("gnawer", Vec3::new(20.0, 0.0, 20.0), 0.0)
            .unwrap();
        let embers = w
            .spawn_embers(&t, Vec3::new(-20.0, 1.0, 0.0), 10.0, 1.0, 5.0, true)
            .unwrap();
        let thorn = w
            .spawn_thorn(&t, PlayerId(9), Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO)
            .unwrap();
        assert_eq!(
            [projectile, enemy, embers, thorn].map(|id| id.0),
            [start, start + 1, start + 2, start + 3]
        );
        // A very long session reaches the end of the id space ...
        w.next_dynamic_id = NetId::STATIC_START - 1;
        let last = wisp(&mut w, &t).unwrap();
        assert_eq!(last.0, NetId::STATIC_START - 1);
        // ... and wraps around, skipping the ids still alive (it used to hand out the last id
        // forever, each new entity silently replacing the previous one).
        let wrapped = wisp(&mut w, &t).unwrap();
        assert_eq!(wrapped.0, start + 4);
        let next = wisp(&mut w, &t).unwrap();
        assert_eq!(next.0, start + 5);
        assert_eq!(w.projectiles.len(), 4);
        for id in [projectile, enemy, embers, thorn, last, wrapped, next] {
            assert!(id.0 >= start && id.0 < NetId::STATIC_START);
        }
        assert!(w.projectiles.contains_key(&projectile) && w.enemies.contains_key(&enemy));
        assert!(w.embers.contains_key(&embers) && w.thorns.contains_key(&thorn));
    }

    #[test]
    fn spawns_beyond_the_world_caps_are_refused() {
        let t = Tuning::default();
        let mut w = world();
        for _ in 0..MAX_PROJECTILES {
            assert!(wisp(&mut w, &t).is_some());
        }
        assert!(wisp(&mut w, &t).is_none());
        assert_eq!(w.projectiles.len(), MAX_PROJECTILES);
        for i in 0..MAX_ENEMIES {
            let at = Vec3::new(
                (i % 16) as f32 * 3.0 - 24.0,
                0.0,
                (i / 16) as f32 * 3.0 - 24.0,
            );
            assert!(w.spawn_enemy("gnawer", at, 0.0).is_some(), "enemy {i}");
        }
        assert!(w.spawn_enemy("gnawer", Vec3::ZERO, 0.0).is_none());
        assert_eq!(w.enemies.len(), MAX_ENEMIES);
        // A crowded world keeps ticking.
        w.step(&TickInputs::new(), &t, None);
    }

    #[test]
    fn encounter_waves_fill_up_to_the_enemy_cap_and_still_clear() {
        use crate::level::{Encounter, EnemySpawn, Wave};
        let t = Tuning::default();
        let mut level = builtin_level(BUILTIN_LEVEL).unwrap();
        level.kind = LevelKind::Campaign;
        let gnawer = EnemySpawn {
            enemy: "gnawer".into(),
            position: Vec3::new(10.0, 0.0, 10.0),
            yaw: 0.0,
        };
        level.encounters.push(Encounter {
            trigger: Volume {
                min: Vec3::splat(-300.0),
                max: Vec3::splat(-299.0),
            },
            arena: None,
            rally: None,
            lock_doors: vec![],
            unlock_doors: vec![],
            waves: vec![Wave {
                delay: 0.0,
                spawns: vec![gnawer; 3],
            }],
        });
        let mut w = SimWorld::new("capped", Arc::new(level), DEFAULT_TICK_RATE);
        w.add_player(PlayerId(0), "A").notarget = true;
        // Room for two more enemies.
        let filler: Vec<NetId> = (0..MAX_ENEMIES - 2)
            .map(|i| {
                let at = Vec3::new(
                    (i % 16) as f32 * 3.0 - 50.0,
                    0.0,
                    (i / 16) as f32 * 3.0 - 50.0,
                );
                w.spawn_enemy("gnawer", at, 0.0).unwrap()
            })
            .collect();
        w.start_encounter(0);
        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(w.step(&TickInputs::new(), &t, None));
        }
        let wave = events.iter().find_map(|e| match e {
            SimEvent::WaveStarted { enemies, .. } => Some(*enemies),
            _ => None,
        });
        assert_eq!(wave, Some(2), "the wave spawns what fits");
        assert_eq!(w.enemies.len(), MAX_ENEMIES);
        let wave_enemies = w.encounters.states[0].alive.clone();
        assert_eq!(wave_enemies.len(), 2);
        assert!(wave_enemies.iter().all(|id| !filler.contains(id)));
        for id in wave_enemies {
            w.kill_enemy(&t, id);
        }
        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(w.step(&TickInputs::new(), &t, None));
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SimEvent::EncounterCleared { index: 0 })),
            "{events:?}"
        );
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
