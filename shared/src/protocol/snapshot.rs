//! Replicated entity state, quantisation and snapshot delta compression.
//!
//! Every replicated thing (players, doors, and later enemies, projectiles, pickups, thorns,
//! ember bursts...) is a generic [`NetEntity`] identified by a [`NetId`]. The server builds one
//! [`NetFrame`] (all entities, quantised) per snapshot and encodes it for each client as a
//! **delta** against the last frame that client acknowledged ([`encode_delta`]); the client
//! applies it to its copy of the same baseline ([`decode_delta`]). A full snapshot is simply a
//! delta against the empty frame.
//!
//! ## Wire format of the entity block
//!
//! ```text
//! varint  changed_count
//! repeat changed_count times (ascending id):
//!     varint  id gap        (first: id, then id - previous_id - 1)
//!     varint  field mask    (FIELD_* bits; low 7 bits fit in one byte)
//!     fields in mask order:
//!         POS     3 x zigzag varint   (quantised delta to baseline, 1/256 m)
//!         VEL     3 x zigzag varint   (quantised delta to baseline, 1/64 m/s)
//!         YAW     zigzag varint       (wrapping u16 delta, 2pi/65536 rad)
//!         PITCH   zigzag varint       (i16 delta)
//!         FLAGS   varint              (absolute)
//!         STATE   varint              (absolute)
//!         HEALTH  varint              (absolute)
//!         AUX     varint              (absolute)
//!         KIND    varint              (absolute)
//!         OWNER   varint              (absolute)
//! varint  removed_count
//! repeat removed_count times: varint id gap (as above)
//! ```
//!
//! Entities that did not change are omitted entirely; a new entity is encoded against
//! [`NetEntity::default()`] (and always listed, even if all its fields are default).

use std::collections::BTreeMap;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::fmt;

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Network id of a replicated entity.
///
/// Id ranges: players use `0..256` (= their [`crate::sim::PlayerId`]), dynamic gameplay
/// entities use [`NetId::DYNAMIC_START`]`..`[`NetId::STATIC_START`] (allocated by the
/// simulation, never reused within a session), level-static entities (doors...) use
/// [`NetId::STATIC_START`]`..`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct NetId(pub u32);

impl NetId {
    /// First id handed out to dynamic entities (enemies, projectiles, pickups...).
    pub const DYNAMIC_START: u32 = 1024;
    /// First id of level-static entities (doors use `STATIC_START + door id`).
    pub const STATIC_START: u32 = 1 << 24;

    pub fn player(id: crate::sim::PlayerId) -> NetId {
        NetId(id.0 as u32)
    }

    pub fn door(door_id: u32) -> NetId {
        NetId(Self::STATIC_START.saturating_add(door_id))
    }

    /// Arena encounter `index` of the level (static; see `kind::ENCOUNTER`).
    pub fn encounter(index: u32) -> NetId {
        NetId(Self::STATIC_START + 0x0010_0000 + index.min(0xFFFF))
    }

    /// The player this id belongs to, if it is in the player range.
    pub fn as_player(self) -> Option<crate::sim::PlayerId> {
        (self.0 < 256).then_some(crate::sim::PlayerId(self.0 as u8))
    }
}

/// Entity kinds (the `kind` field of [`NetEntity`]). Gameplay code adds its own constants here
/// (keep them stable: they are part of the protocol).
pub mod kind {
    /// A player (`owner` = player id). `flags` = [`super::player_flags`]; `state` = weapon
    /// (bits 0–3) | alt mode (bits 4–5) | Discharge charge (bits 8–15); `health` = health;
    /// `aux` = hard damage (bits 0–7) | thorn charges (8–11) | (12–15 unused) | wall-cling
    /// normal as a quantised yaw (16–31, while `WALL_CLING`). See `docs/combat.md`.
    pub const PLAYER: u16 = 0;
    /// A level door (`aux` = door id, `flags` bit 0 = open).
    pub const DOOR: u16 = 1;
    /// An arena encounter (`NetId::encounter(index)`, docs/enemies.md): `position` = arena
    /// centre, `flags` = [`super::encounter_flags`], `state` = current wave (1-based, 0 before
    /// the first), `health` = enemies left in the wave, `aux` = waves | wave size << 16.
    pub const ENCOUNTER: u16 = 2;

    // 16..64: enemies (`EnemyDef::net_kind`). `owner` = target player (OWNER_NONE), `state` =
    // `AiState`, `health` = health, `aux` = progress | attack << 8 | max health << 16.
    pub const ENEMY_FIRST: u16 = 16;
    pub const ENEMY_LAST: u16 = 63;
    // 16..=19: retired (the first roster). docs/roster.md.
    pub const GNAWER: u16 = 20;
    pub const CENSER: u16 = 21;
    pub const HARRIER: u16 = 22;
    pub const GAOLER: u16 = 23;
    pub const HERON: u16 = 24;

    // 64..128: projectiles, replicated by trajectory: `position`/`velocity` at launch tick
    // `aux`; `owner` = player id / OWNER_ENEMY; `state` = bounces left. 64–69 are retired
    // (the first arsenal's rivet, sawblade, rocket, flak, sticky, mortar).
    pub const PROJECTILE_FIRST: u16 = 64;
    pub const PROJECTILE_LAST: u16 = 127;
    /// Tumbler bomb (`state` = world bounces left before it explodes).
    pub const BOMB: u16 = 72;
    /// Tumbler Gravewell canister (`STUCK` while it pulls).
    pub const GRAVEWELL: u16 = 73;
    /// Arclash Wisp (ball of lightning).
    pub const WISP: u16 = 74;
    // 70, 71: retired enemy projectiles. 80..: enemy projectiles.
    /// The Censer's lobbed ember bomb (explodes on impact, parryable).
    pub const EMBER_BOMB: u16 = 80;
    /// The Heron's needle (fast, straight, parryable).
    pub const NEEDLE: u16 = 81;

    // 128..160: pickups / ember bursts.
    /// Ember burst (heal pool): `health` = pool left, `state` = radius (dm), `aux` = spawn tick.
    pub const EMBER_BURST: u16 = 128;

    // 160..176: thorns.
    /// Thrown relay spike, replicated by trajectory like projectiles (gravity
    /// `thorn.gravity`); `owner` = thrower; `flags` = [`super::thorn_flags`]. While `PLANTED`,
    /// `position` is where it sits and `velocity` its orientation (the flight direction it
    /// planted along, unit length).
    pub const THORN: u16 = 160;

    pub fn is_enemy(kind: u16) -> bool {
        (ENEMY_FIRST..=ENEMY_LAST).contains(&kind)
    }

    pub fn is_projectile(kind: u16) -> bool {
        (PROJECTILE_FIRST..=PROJECTILE_LAST).contains(&kind)
    }
}

/// `owner` of an entity owned by no player.
pub const OWNER_NONE: u16 = 0xFFFF;
/// `owner` of a projectile fired by an enemy.
pub const OWNER_ENEMY: u16 = 0xFFFE;

/// Wire form of an [`crate::sim::Owner`] in `NetEntity::owner`.
pub fn owner_code(owner: crate::sim::Owner) -> u16 {
    match owner {
        crate::sim::Owner::Player(p) => p.0 as u16,
        crate::sim::Owner::Enemy(_) => OWNER_ENEMY,
        crate::sim::Owner::World => OWNER_NONE,
    }
}

/// Flag bits of player entities.
pub mod player_flags {
    /// Using the low (sliding/crouched) capsule.
    pub const LOW_PROFILE: u32 = 1 << 0;
    /// Dash invulnerability frames.
    pub const INVULNERABLE: u32 = 1 << 1;
    /// Dead (corpse; waiting to respawn).
    pub const DEAD: u32 = 1 << 2;
    /// Parry window open (punch pose).
    pub const PARRY: u32 = 1 << 3;
    /// Charging the Arclash Discharge (charge level in `state` bits 8–15).
    pub const CHARGING: u32 = 1 << 4;
    /// Spawn protection / god mode: can't be hurt.
    pub const PROTECTED: u32 = 1 << 5;
    /// Reserved for co-op downed state.
    pub const DOWNED: u32 = 1 << 6;
    // Movement state for third-person animation (clients can't run other players' movement).
    pub const ON_GROUND: u32 = 1 << 7;
    pub const SLIDING: u32 = 1 << 8;
    pub const DASHING: u32 = 1 << 9;
    pub const SLAMMING: u32 = 1 << 10;
    /// Clinging to a wall; `aux` bits 16–31 hold the wall normal's yaw (`quantize_yaw`).
    pub const WALL_CLING: u32 = 1 << 11;
    /// Branded by a Quill Brand shot: takes extra damage, draws a sigil (weapons area).
    pub const BRANDED: u32 = 1 << 15;
}

/// Flag bits of enemy entities.
pub mod enemy_flags {
    /// Has a target (`owner`).
    pub const ALERT: u32 = 1 << 0;
    /// In the parryable phase of a telegraphed attack (flash the parry colour).
    pub const PARRYABLE: u32 = 1 << 1;
    pub const STUNNED: u32 = 1 << 2;
    /// Dead (corpse, no hitboxes).
    pub const DYING: u32 = 1 << 3;
    /// Not on the ground (knocked up / flying).
    pub const AIRBORNE: u32 = 1 << 4;
    /// Has directional armour.
    pub const ARMOURED: u32 = 1 << 5;
    // Bits 6..=14: per-type presentation state (enemies area); bit 15 is `BRANDED`.
    /// Aim laser on (a laser wind-up): `yaw` / `pitch` are the exact aim from the attack's
    /// muzzle (unquantised while this is set).
    pub const AIMING: u32 = 1 << 6;
    /// The aim laser has locked (the last moments before the shot: dodge now).
    pub const AIM_LOCKED: u32 = 1 << 7;
    /// Branded by a Quill Brand shot: takes extra damage, draws a sigil (weapons area; the
    /// brand lives in a `SimWorld` side table and is OR-ed in by `SimWorld::replicate`).
    pub const BRANDED: u32 = 1 << 15;
}

/// Flag bits of projectile entities.
pub mod projectile_flags {
    /// Deflected by a punch or a Gust: flies straight (no gravity), now owned by the
    /// deflector.
    pub const PARRIED: u32 = 1 << 0;
    /// Stuck to the level (a Gravewell pulling): `position` is current (no trajectory).
    pub const STUCK: u32 = 1 << 1;
    /// Touched a player and waits out the parry tolerance at `position`.
    pub const PENDING: u32 = 1 << 2;
    /// Originally fired by an enemy.
    pub const ENEMY: u32 = 1 << 3;
}

/// Flag bits of ember bursts.
pub mod ember_flags {
    /// Enemy death burst.
    pub const BIG: u32 = 1 << 0;
}

/// Flag bits of thorns.
pub mod thorn_flags {
    /// Not moving (planted in a surface or lying on the floor): `position` is current, no
    /// trajectory; `velocity` is its orientation.
    pub const PLANTED: u32 = 1 << 0;
    /// Lying on the floor after hitting a target (drawn flat, not stuck in).
    pub const LYING: u32 = 1 << 1;
    /// Dropping to the floor after hitting a target (tumbling, trajectory with gravity).
    pub const DROPPING: u32 = 1 << 2;
}

/// Flag bits of door entities.
pub mod door_flags {
    pub const OPEN: u32 = 1 << 0;
}

/// `flags` of `kind::ENCOUNTER` entities.
pub mod encounter_flags {
    /// Doors locked, waves in progress.
    pub const ACTIVE: u32 = 1 << 0;
    /// Won (doors open for good).
    pub const CLEARED: u32 = 1 << 1;
    /// Active, the next wave is about to spawn.
    pub const BETWEEN_WAVES: u32 = 1 << 2;
}

/// Quantisation scales.
pub const POS_SCALE: f32 = 256.0;
pub const VEL_SCALE: f32 = 64.0;
pub const ANGLE_SCALE: f32 = 65536.0 / TAU;

/// Hard cap on entities per frame (decoder safety limit).
pub const MAX_NET_ENTITIES: usize = 8192;

/// Field mask bits, ordered by how often they change so common deltas fit a 1-byte mask.
pub const FIELD_POS: u32 = 1 << 0;
pub const FIELD_VEL: u32 = 1 << 1;
pub const FIELD_YAW: u32 = 1 << 2;
pub const FIELD_PITCH: u32 = 1 << 3;
pub const FIELD_FLAGS: u32 = 1 << 4;
pub const FIELD_STATE: u32 = 1 << 5;
pub const FIELD_HEALTH: u32 = 1 << 6;
pub const FIELD_AUX: u32 = 1 << 7;
pub const FIELD_KIND: u32 = 1 << 8;
pub const FIELD_OWNER: u32 = 1 << 9;
const FIELD_ALL: u32 = (1 << 10) - 1;

/// Quantised replicated state of one entity. Generic on purpose: each `kind` documents what
/// `flags`, `state`, `health` and `aux` mean for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NetEntity {
    pub kind: u16,
    /// Owning player (players: themselves; projectiles: shooter...). Meaning is per kind.
    pub owner: u16,
    /// Position in 1/[`POS_SCALE`] metres.
    pub pos: [i32; 3],
    /// Velocity in 1/[`VEL_SCALE`] m/s.
    pub vel: [i32; 3],
    /// Yaw in 2pi/65536 rad (wrapping).
    pub yaw: u16,
    /// Pitch in 2pi/65536 rad, within +-pi/2.
    pub pitch: i16,
    pub flags: u32,
    /// Animation / AI state (per kind).
    pub state: u16,
    /// Health or any 16-bit gauge (per kind).
    pub health: u16,
    /// Free-form per-kind data (door id, enemy definition index, timer...).
    pub aux: u32,
}

/// Dequantised (float) entity state, as used by gameplay and rendering code.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityState {
    pub id: NetId,
    pub kind: u16,
    pub owner: u16,
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub flags: u32,
    pub state: u16,
    pub health: u16,
    pub aux: u32,
}

pub fn quantize_vec(v: Vec3, scale: f32) -> [i32; 3] {
    // `as` saturates and maps NaN to 0, so this never panics.
    [
        (v.x * scale).round() as i32,
        (v.y * scale).round() as i32,
        (v.z * scale).round() as i32,
    ]
}

pub fn dequantize_vec(q: [i32; 3], scale: f32) -> Vec3 {
    Vec3::new(q[0] as f32, q[1] as f32, q[2] as f32) / scale
}

pub fn quantize_yaw(yaw: f32) -> u16 {
    let y = yaw.rem_euclid(TAU);
    ((y * ANGLE_SCALE).round() as u32 & 0xFFFF) as u16
}

pub fn dequantize_yaw(q: u16) -> f32 {
    q as f32 / ANGLE_SCALE
}

pub fn quantize_pitch(pitch: f32) -> i16 {
    (pitch.clamp(-FRAC_PI_2, FRAC_PI_2) * ANGLE_SCALE).round() as i16
}

pub fn dequantize_pitch(q: i16) -> f32 {
    q as f32 / ANGLE_SCALE
}

impl EntityState {
    pub fn quantize(&self) -> NetEntity {
        NetEntity {
            kind: self.kind,
            owner: self.owner,
            pos: quantize_vec(self.position, POS_SCALE),
            vel: quantize_vec(self.velocity, VEL_SCALE),
            yaw: quantize_yaw(self.yaw),
            pitch: quantize_pitch(self.pitch),
            flags: self.flags,
            state: self.state,
            health: self.health,
            aux: self.aux,
        }
    }
}

impl NetEntity {
    pub fn dequantize(&self, id: NetId) -> EntityState {
        EntityState {
            id,
            kind: self.kind,
            owner: self.owner,
            position: dequantize_vec(self.pos, POS_SCALE),
            velocity: dequantize_vec(self.vel, VEL_SCALE),
            yaw: dequantize_yaw(self.yaw),
            pitch: dequantize_pitch(self.pitch),
            flags: self.flags,
            state: self.state,
            health: self.health,
            aux: self.aux,
        }
    }

    /// Bitmask of fields that differ between `self` and `base`.
    pub fn diff_mask(&self, base: &NetEntity) -> u32 {
        let mut mask = 0;
        if self.pos != base.pos {
            mask |= FIELD_POS;
        }
        if self.vel != base.vel {
            mask |= FIELD_VEL;
        }
        if self.yaw != base.yaw {
            mask |= FIELD_YAW;
        }
        if self.pitch != base.pitch {
            mask |= FIELD_PITCH;
        }
        if self.flags != base.flags {
            mask |= FIELD_FLAGS;
        }
        if self.state != base.state {
            mask |= FIELD_STATE;
        }
        if self.health != base.health {
            mask |= FIELD_HEALTH;
        }
        if self.aux != base.aux {
            mask |= FIELD_AUX;
        }
        if self.kind != base.kind {
            mask |= FIELD_KIND;
        }
        if self.owner != base.owner {
            mask |= FIELD_OWNER;
        }
        mask
    }
}

/// All replicated entities at one tick, quantised.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetFrame {
    pub entities: BTreeMap<NetId, NetEntity>,
}

impl NetFrame {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, state: &EntityState) {
        self.entities.insert(state.id, state.quantize());
    }

    pub fn get(&self, id: NetId) -> Option<EntityState> {
        self.entities.get(&id).map(|e| e.dequantize(id))
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Dequantised view of every entity.
    pub fn states(&self) -> impl Iterator<Item = EntityState> + '_ {
        self.entities.iter().map(|(id, e)| e.dequantize(*id))
    }
}

/// Error decoding a delta-compressed entity block (malformed or hostile data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaError {
    UnexpectedEnd,
    VarintOverflow,
    TooManyEntities,
    BadId,
    BadMask,
    ValueOutOfRange,
    TrailingBytes,
}

impl fmt::Display for DeltaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DeltaError::UnexpectedEnd => "unexpected end of data",
            DeltaError::VarintOverflow => "varint too long",
            DeltaError::TooManyEntities => "too many entities",
            DeltaError::BadId => "entity ids not ascending",
            DeltaError::BadMask => "invalid field mask",
            DeltaError::ValueOutOfRange => "value out of range",
            DeltaError::TrailingBytes => "trailing bytes",
        };
        write!(f, "snapshot delta: {s}")
    }
}

impl std::error::Error for DeltaError {}

/// Encodes `current` as a delta against `baseline` (pass an empty frame for a full snapshot)
/// and appends it to `out`.
pub fn encode_delta(baseline: &NetFrame, current: &NetFrame, out: &mut Vec<u8>) {
    let default = NetEntity::default();
    let mut changed: Vec<(NetId, &NetEntity, &NetEntity, u32)> = Vec::new();
    for (id, entity) in &current.entities {
        match baseline.entities.get(id) {
            Some(base) => {
                let mask = entity.diff_mask(base);
                if mask != 0 {
                    changed.push((*id, base, entity, mask));
                }
            }
            None => changed.push((*id, &default, entity, entity.diff_mask(&default))),
        }
    }

    put_varint(out, changed.len() as u64);
    let mut prev: Option<u32> = None;
    for (id, base, entity, mask) in changed {
        put_id(out, &mut prev, id);
        put_varint(out, mask as u64);
        if mask & FIELD_POS != 0 {
            for i in 0..3 {
                put_zigzag(out, entity.pos[i] as i64 - base.pos[i] as i64);
            }
        }
        if mask & FIELD_VEL != 0 {
            for i in 0..3 {
                put_zigzag(out, entity.vel[i] as i64 - base.vel[i] as i64);
            }
        }
        if mask & FIELD_YAW != 0 {
            put_zigzag(out, entity.yaw.wrapping_sub(base.yaw) as i16 as i64);
        }
        if mask & FIELD_PITCH != 0 {
            put_zigzag(out, entity.pitch as i64 - base.pitch as i64);
        }
        if mask & FIELD_FLAGS != 0 {
            put_varint(out, entity.flags as u64);
        }
        if mask & FIELD_STATE != 0 {
            put_varint(out, entity.state as u64);
        }
        if mask & FIELD_HEALTH != 0 {
            put_varint(out, entity.health as u64);
        }
        if mask & FIELD_AUX != 0 {
            put_varint(out, entity.aux as u64);
        }
        if mask & FIELD_KIND != 0 {
            put_varint(out, entity.kind as u64);
        }
        if mask & FIELD_OWNER != 0 {
            put_varint(out, entity.owner as u64);
        }
    }

    let removed: Vec<NetId> = baseline
        .entities
        .keys()
        .filter(|id| !current.entities.contains_key(id))
        .copied()
        .collect();
    put_varint(out, removed.len() as u64);
    let mut prev = None;
    for id in removed {
        put_id(out, &mut prev, id);
    }
}

/// Convenience: full (non-delta) encoding of a frame.
pub fn encode_full(current: &NetFrame) -> Vec<u8> {
    let mut out = Vec::new();
    encode_delta(&NetFrame::default(), current, &mut out);
    out
}

/// Reconstructs a frame from `baseline` and a delta produced by [`encode_delta`] against the
/// same baseline. Never panics on malformed input.
pub fn decode_delta(baseline: &NetFrame, data: &[u8]) -> Result<NetFrame, DeltaError> {
    let mut r = Reader { data, pos: 0 };
    let mut frame = baseline.clone();

    let count = r.varint()? as usize;
    if count > MAX_NET_ENTITIES {
        return Err(DeltaError::TooManyEntities);
    }
    let mut prev = None;
    for _ in 0..count {
        let id = r.id(&mut prev)?;
        let mask = r.varint()?;
        if mask > FIELD_ALL as u64 {
            return Err(DeltaError::BadMask);
        }
        let mask = mask as u32;
        let mut e = frame.entities.get(&id).copied().unwrap_or_default();
        if mask & FIELD_POS != 0 {
            for i in 0..3 {
                e.pos[i] = add_i32(e.pos[i], r.zigzag()?)?;
            }
        }
        if mask & FIELD_VEL != 0 {
            for i in 0..3 {
                e.vel[i] = add_i32(e.vel[i], r.zigzag()?)?;
            }
        }
        if mask & FIELD_YAW != 0 {
            let d = r.zigzag()?;
            let d = i16::try_from(d).map_err(|_| DeltaError::ValueOutOfRange)?;
            e.yaw = e.yaw.wrapping_add(d as u16);
        }
        if mask & FIELD_PITCH != 0 {
            let v = e.pitch as i64 + r.zigzag()?;
            e.pitch = i16::try_from(v).map_err(|_| DeltaError::ValueOutOfRange)?;
        }
        if mask & FIELD_FLAGS != 0 {
            e.flags = r.u32()?;
        }
        if mask & FIELD_STATE != 0 {
            e.state = r.u16()?;
        }
        if mask & FIELD_HEALTH != 0 {
            e.health = r.u16()?;
        }
        if mask & FIELD_AUX != 0 {
            e.aux = r.u32()?;
        }
        if mask & FIELD_KIND != 0 {
            e.kind = r.u16()?;
        }
        if mask & FIELD_OWNER != 0 {
            e.owner = r.u16()?;
        }
        frame.entities.insert(id, e);
        if frame.entities.len() > MAX_NET_ENTITIES {
            return Err(DeltaError::TooManyEntities);
        }
    }

    let removed = r.varint()? as usize;
    if removed > MAX_NET_ENTITIES {
        return Err(DeltaError::TooManyEntities);
    }
    let mut prev = None;
    for _ in 0..removed {
        let id = r.id(&mut prev)?;
        frame.entities.remove(&id);
    }
    if r.pos != data.len() {
        return Err(DeltaError::TrailingBytes);
    }
    Ok(frame)
}

fn add_i32(base: i32, delta: i64) -> Result<i32, DeltaError> {
    i32::try_from(base as i64 + delta).map_err(|_| DeltaError::ValueOutOfRange)
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_zigzag(out: &mut Vec<u8>, v: i64) {
    put_varint(out, ((v << 1) ^ (v >> 63)) as u64);
}

fn put_id(out: &mut Vec<u8>, prev: &mut Option<u32>, id: NetId) {
    let gap = match *prev {
        None => id.0,
        Some(p) => id.0 - p - 1,
    };
    put_varint(out, gap as u64);
    *prev = Some(id.0);
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn varint(&mut self) -> Result<u64, DeltaError> {
        let mut value: u64 = 0;
        for i in 0..10 {
            let byte = *self.data.get(self.pos).ok_or(DeltaError::UnexpectedEnd)?;
            self.pos += 1;
            let bits = (byte & 0x7F) as u64;
            if i == 9 && bits > 1 {
                return Err(DeltaError::VarintOverflow);
            }
            value |= bits << (7 * i);
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(DeltaError::VarintOverflow)
    }

    fn zigzag(&mut self) -> Result<i64, DeltaError> {
        let v = self.varint()?;
        Ok(((v >> 1) as i64) ^ -((v & 1) as i64))
    }

    fn u32(&mut self) -> Result<u32, DeltaError> {
        u32::try_from(self.varint()?).map_err(|_| DeltaError::ValueOutOfRange)
    }

    fn u16(&mut self) -> Result<u16, DeltaError> {
        u16::try_from(self.varint()?).map_err(|_| DeltaError::ValueOutOfRange)
    }

    fn id(&mut self, prev: &mut Option<u32>) -> Result<NetId, DeltaError> {
        let gap = self.u32()?;
        let id = match *prev {
            None => gap,
            Some(p) => p
                .checked_add(1)
                .and_then(|v| v.checked_add(gap))
                .ok_or(DeltaError::BadId)?,
        };
        *prev = Some(id);
        Ok(NetId(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: u32, pos: Vec3, yaw: f32) -> EntityState {
        EntityState {
            id: NetId(id),
            kind: kind::PLAYER,
            owner: id as u16,
            position: pos,
            velocity: Vec3::new(1.0, -2.0, 3.0),
            yaw,
            pitch: -0.3,
            ..Default::default()
        }
    }

    fn frame(states: &[EntityState]) -> NetFrame {
        let mut f = NetFrame::new();
        for s in states {
            f.insert(s);
        }
        f
    }

    #[test]
    fn quantisation_is_close() {
        let s = player(3, Vec3::new(12.345, -0.5, 1000.25), 7.0);
        let back = s.quantize().dequantize(NetId(3));
        assert!((back.position - s.position).length() < 1.0 / POS_SCALE);
        assert!((back.velocity - s.velocity).length() < 1.0 / VEL_SCALE);
        let yaw_err = (back.yaw - 7.0f32.rem_euclid(TAU)).abs();
        assert!(yaw_err < 1e-3, "{yaw_err}");
        assert!((back.pitch + 0.3).abs() < 1e-3);
    }

    #[test]
    fn delta_round_trips() {
        let base = frame(&[
            player(0, Vec3::ZERO, 0.0),
            player(1, Vec3::new(5.0, 0.0, 5.0), 1.0),
            player(2, Vec3::new(-5.0, 1.0, 5.0), 6.2),
        ]);
        let mut moved = player(1, Vec3::new(5.5, 0.2, 5.0), 6.1);
        moved.flags = player_flags::LOW_PROFILE;
        let mut door = EntityState {
            id: NetId::door(7),
            kind: kind::DOOR,
            aux: 7,
            ..Default::default()
        };
        door.flags = door_flags::OPEN;
        let current = frame(&[player(0, Vec3::ZERO, 0.0), moved, door]);

        let mut delta = Vec::new();
        encode_delta(&base, &current, &mut delta);
        let decoded = decode_delta(&base, &delta).unwrap();
        assert_eq!(decoded, current);

        let full = encode_full(&current);
        assert_eq!(decode_delta(&NetFrame::default(), &full).unwrap(), current);
        assert!(
            delta.len() < full.len(),
            "delta {} full {}",
            delta.len(),
            full.len()
        );
    }

    #[test]
    fn unchanged_frame_is_tiny() {
        let f = frame(&[player(0, Vec3::ONE, 0.5), player(1, Vec3::ONE, 0.5)]);
        let mut delta = Vec::new();
        encode_delta(&f, &f, &mut delta);
        assert_eq!(delta, vec![0, 0]);
    }

    #[test]
    fn yaw_wraps_through_zero() {
        let a = frame(&[player(0, Vec3::ZERO, TAU - 0.01)]);
        let b = frame(&[player(0, Vec3::ZERO, 0.01)]);
        let mut delta = Vec::new();
        encode_delta(&a, &b, &mut delta);
        assert_eq!(decode_delta(&a, &delta).unwrap(), b);
    }

    #[test]
    fn extreme_values_round_trip() {
        let mut e = NetEntity {
            kind: u16::MAX,
            owner: u16::MAX,
            pos: [i32::MIN, i32::MAX, 0],
            vel: [i32::MAX, i32::MIN, -1],
            yaw: u16::MAX,
            pitch: i16::MIN,
            flags: u32::MAX,
            state: u16::MAX,
            health: u16::MAX,
            aux: u32::MAX,
        };
        let mut a = NetFrame::new();
        a.entities.insert(NetId(u32::MAX), e);
        e.pos = [i32::MAX, i32::MIN, 5];
        e.pitch = i16::MAX;
        let mut b = NetFrame::new();
        b.entities.insert(NetId(u32::MAX), e);
        b.entities.insert(NetId(0), NetEntity::default());
        let mut delta = Vec::new();
        encode_delta(&a, &b, &mut delta);
        assert_eq!(decode_delta(&a, &delta).unwrap(), b);
    }

    #[test]
    fn garbage_never_panics() {
        let base = frame(&[player(0, Vec3::ZERO, 0.0), player(1, Vec3::ONE, 1.0)]);
        let mut seed: u64 = 0x1234_5678_9abc_def0;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for len in 0..200 {
            for _ in 0..50 {
                let data: Vec<u8> = (0..len).map(|_| rand() as u8).collect();
                let _ = decode_delta(&base, &data);
            }
        }
        assert_eq!(
            decode_delta(&base, &[0xFF; 11]),
            Err(DeltaError::VarintOverflow)
        );
        assert_eq!(decode_delta(&base, &[]), Err(DeltaError::UnexpectedEnd));
        assert_eq!(
            decode_delta(&base, &[0, 0, 0]),
            Err(DeltaError::TrailingBytes)
        );
    }
}
