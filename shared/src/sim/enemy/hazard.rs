//! Hazards left by enemies: the Smelter's **slag lines** (`kind::SLAG_LINE`, docs/enemies.md).
//!
//! A pour lays its line when the wind-up starts: a strip `width` wide along the floor from just
//! in front of the Smelter towards (and past) where its target is heading, stopping at walls,
//! ledges and steps it can't flow over. The line is replicated at once — every client draws the
//! glowing telegraph strip during the wind-up — and **ignites** on the strike tick: from then
//! on, for `lifetime` seconds, every `interval` it burns each targetable player whose feet are
//! on it (within half the width of the strip, horizontally, and within `ON_LINE_HEIGHT` of the
//! floor there: jumping over it or dashing through it on i-frames is safe). A pour cancelled
//! before the strike (the Smelter stunned or killed) takes its unlit line with it. Enemies are
//! never hurt by slag.

use engine::physics::{CollisionWorld, MASK_MOVEMENT};
use glam::Vec3;

use crate::protocol::{EntityState, NetId, OWNER_ENEMY, kind, slag_flags};
use crate::sim::combat::{Attack, DamageInfo, Owner};
use crate::sim::{HitTarget, SimWorld};
use crate::tuning::Tuning;

/// Spacing (m) of the floor samples along a line.
const STEP: f32 = 0.75;
/// The floor may rise / fall this much between two samples (ramps, small steps).
const MAX_STEP: f32 = 0.8;
/// Feet more than this above the floor under them are clear of the slag.
const ON_LINE_HEIGHT: f32 = 0.5;
/// The last second of a line: cooling (replicated, the client fades it).
const COOLING: f32 = 1.0;
/// A pour needs at least this much line (else the Smelter doesn't pour).
pub(crate) const MIN_LENGTH: f32 = 3.0;

/// One slag line.
#[derive(Clone, Debug)]
pub struct SlagLine {
    pub id: NetId,
    /// The Smelter that poured it (damage source, kill feed; it may be dead by now).
    pub owner: NetId,
    pub attack: Attack,
    /// Floor points from the start to the end (every [`STEP`] m), following the floor.
    pub points: Vec<Vec3>,
    pub half_width: f32,
    /// Burns from this tick on (the telegraph shows before it).
    pub ignite_tick: u32,
    /// Gone at this tick.
    pub expire_tick: u32,
    /// Damage of one burn tick (already scaled by the pourer's fury).
    pub damage: f32,
    /// Ticks between burn ticks.
    pub interval: u32,
    cooling_ticks: u32,
}

impl SlagLine {
    pub fn start(&self) -> Vec3 {
        self.points.first().copied().unwrap_or_default()
    }

    pub fn end(&self) -> Vec3 {
        self.points.last().copied().unwrap_or_default()
    }

    /// Burning (hurts) at `tick`.
    pub fn live(&self, tick: u32) -> bool {
        tick >= self.ignite_tick && tick < self.expire_tick
    }

    /// Length along the floor (m).
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// Are feet at `feet` (with a body `radius`) standing on the burning strip?
    pub fn touches(&self, feet: Vec3, radius: f32) -> bool {
        let reach = self.half_width + radius * 0.5;
        self.points.windows(2).any(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
            let ap = Vec3::new(feet.x - a.x, 0.0, feet.z - a.z);
            let len2 = ab.length_squared();
            let t = if len2 > 1e-6 {
                (ap.dot(ab) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let floor = a.y + (b.y - a.y) * t;
            (ap - ab * t).length() <= reach
                && feet.y - floor <= ON_LINE_HEIGHT
                && feet.y - floor >= -1.0
        })
    }

    /// Replicated state (see `kind::SLAG_LINE`).
    pub fn entity_state(&self, tick: u32) -> EntityState {
        let mut flags = 0;
        if self.live(tick) {
            flags |= slag_flags::LIVE;
            if tick + self.cooling_ticks >= self.expire_tick {
                flags |= slag_flags::COOLING;
            }
        }
        let start = self.start();
        EntityState {
            id: self.id,
            kind: kind::SLAG_LINE,
            owner: OWNER_ENEMY,
            position: start,
            velocity: self.end() - start,
            flags,
            state: (self.half_width * 200.0).round().clamp(0.0, 65535.0) as u16,
            aux: self.ignite_tick,
            ..Default::default()
        }
    }
}

/// The floor path of a line from `from` (feet level) along horizontal `dir` for up to `len`
/// metres: a point every [`STEP`] m on the floor, stopping where a wall is in the way (knee
/// height), at a ledge or a step it can't flow over, or off the level. Clients trace the same path
/// from the replicated start and end to draw the line.
pub fn trace_floor(world: &CollisionWorld, from: Vec3, dir: Vec3, len: f32) -> Vec<Vec3> {
    let dir = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    let floor_at = |p: Vec3, y: f32| {
        world
            .cast_ray(
                Vec3::new(p.x, y + MAX_STEP + 0.2, p.z),
                Vec3::NEG_Y,
                2.0 * MAX_STEP + 0.4,
                MASK_MOVEMENT,
            )
            .filter(|h| h.normal.y > 0.7)
            .map(|h| h.point.y)
    };
    let mut points = Vec::new();
    let Some(y) = floor_at(from, from.y) else {
        return points;
    };
    if dir == Vec3::ZERO {
        return points;
    }
    let mut prev = Vec3::new(from.x, y, from.z);
    points.push(prev);
    let steps = (len / STEP).floor() as usize;
    for _ in 0..steps {
        // A wall (or anything knee-high) ends the flow.
        if world
            .cast_ray(prev + Vec3::Y * 0.35, dir, STEP, MASK_MOVEMENT)
            .is_some()
        {
            break;
        }
        let next = prev + dir * STEP;
        match floor_at(next, prev.y) {
            Some(y) if (y - prev.y).abs() <= MAX_STEP => {
                prev = Vec3::new(next.x, y, next.z);
                points.push(prev);
            }
            _ => break,
        }
    }
    points
}

impl SimWorld {
    /// Lays a slag line (unlit until `ignite_tick`) and returns its id; `None` (nothing laid)
    /// when the floor path is shorter than [`MIN_LENGTH`] or the world is out of ids.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lay_slag_line(
        &mut self,
        owner: NetId,
        attack: Attack,
        from: Vec3,
        dir: Vec3,
        len: f32,
        width: f32,
        ignite_tick: u32,
        lifetime: f32,
        damage: f32,
        interval: f32,
    ) -> Option<NetId> {
        let points = trace_floor(&self.collision.world, from, dir, len);
        let length: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        if length < MIN_LENGTH {
            return None;
        }
        let id = self.alloc_net_id()?;
        let rate = self.tick_rate;
        let ticks = |s: f32| crate::sim::combat::secs_to_ticks(s, rate, 1) as u32;
        self.enemy_hazards.insert(
            id,
            SlagLine {
                id,
                owner,
                attack,
                points,
                half_width: width * 0.5,
                ignite_tick,
                expire_tick: ignite_tick + ticks(lifetime),
                damage,
                interval: ticks(interval),
                cooling_ticks: ticks(COOLING),
            },
        );
        Some(id)
    }

    /// Removes a line that hasn't ignited yet (its pour was cancelled).
    pub(crate) fn drop_unlit_line(&mut self, id: NetId) {
        let tick = self.tick;
        if self
            .enemy_hazards
            .get(&id)
            .is_some_and(|h| tick < h.ignite_tick)
        {
            self.enemy_hazards.remove(&id);
        }
    }

    /// Burns whoever stands on a live line (every line's `interval`, from its ignition) and
    /// removes expired lines. Nothing burns while the world is frozen.
    pub(crate) fn step_enemy_hazards(&mut self, t: &Tuning) {
        if self.enemy_hazards.is_empty() {
            return;
        }
        let tick = self.tick;
        self.enemy_hazards.retain(|_, h| tick < h.expire_tick);
        if self.frozen {
            return;
        }
        let mut hits = Vec::new();
        for h in self.enemy_hazards.values() {
            if !h.live(tick) || !(tick - h.ignite_tick).is_multiple_of(h.interval.max(1)) {
                continue;
            }
            for p in self.players.values() {
                if !super::is_targetable(p) {
                    continue;
                }
                let feet = p.movement.position;
                let (_, _, radius) = p.movement.capsule(&t.movement);
                if h.touches(feet, radius) {
                    let info = DamageInfo::new(h.damage, Owner::Enemy(h.owner), h.attack)
                        .at(feet + Vec3::Y * 0.2, Vec3::Y);
                    hits.push((p.id, info));
                }
            }
        }
        for (victim, info) in hits {
            self.apply_damage(t, HitTarget::Player(victim), info);
        }
    }
}
