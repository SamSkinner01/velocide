//! Status effects of the arsenal (`docs/arsenal.md`, "Status effects"): **Burn** (embers from
//! the Kiln, a stoked weapon, the Waxwork's burst) and **Pinned** (Pinning Arrow, Snare,
//! Bolas), on enemies and, in PvP, on players.
//!
//! * Burn: damage over time, `burn.dps` per stack, up to `burn.max_stacks`, for
//!   `burn.duration` (every new stack refreshes it), dealt every `burn.tick_interval` through
//!   the damage pipeline as the attack that set it (flag [`DamageFlags::BURN`]). The latest
//!   source owns the burn (kill credit).
//! * Pinned: an enemy is rooted where it stood (`pinning_arrow.enemy_time`, heavies
//!   `heavy_time`; it can still attack); a player is slowed to `pinning_arrow.player_slow` of
//!   their speeds for `player_time` (`MoveState::slow_time`, predicted by the owner).
//!
//! Like brands, statuses live in a side table ([`SimWorld::statuses`]) and
//! [`SimWorld::replicate`] ORs `BURNING` / `PINNED` into the target's entity flags. (The
//! Flare's burning patches retired with the rework, `docs/rework.md`.)

use engine::physics::MASK_MOVEMENT;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::{HitPart, HitTarget, SimWorld};
use crate::tuning::Tuning;

/// A target's burn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burn {
    pub stacks: u8,
    /// It goes out at this tick.
    pub until: u32,
    /// Who set it (the latest one) and with what: the damage it deals is theirs.
    pub source: Owner,
    pub attack: Attack,
    /// Next damage tick.
    pub next: u32,
}

/// An enemy pinned in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub until: u32,
    /// Where it is held (feet, horizontal).
    pub anchor: Vec3,
}

/// Status effects of one target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    pub burn: Option<Burn>,
    /// Enemies only (players are slowed through their movement state).
    pub pin: Option<Pin>,
}

impl SimWorld {
    /// Is `target` burning right now?
    pub fn is_burning(&self, target: HitTarget) -> bool {
        self.statuses
            .get(&target)
            .and_then(|s| s.burn)
            .is_some_and(|b| self.tick < b.until)
    }

    /// Is `target` pinned right now (an enemy rooted, a player slowed)?
    pub fn is_pinned(&self, target: HitTarget) -> bool {
        match target {
            HitTarget::Player(id) => self
                .players
                .get(&id)
                .is_some_and(|p| !p.combat.dead && p.movement.slow_time > 0.0),
            HitTarget::Entity(_) => self
                .statuses
                .get(&target)
                .and_then(|s| s.pin)
                .is_some_and(|p| self.tick < p.until),
        }
    }

    /// `source` sets `target` alight with `stacks` more Burn stacks (capped), refreshing it.
    /// Only hostile, living targets burn.
    pub fn apply_burn(
        &mut self,
        t: &Tuning,
        source: Owner,
        attack: Attack,
        target: HitTarget,
        stacks: u32,
    ) {
        if stacks == 0 || !self.can_hit(source, target) {
            return;
        }
        let b = &t.combat.burn;
        let now = self.tick;
        let until = now + secs_to_ticks(b.duration, self.tick_rate, 1) as u32;
        let interval = secs_to_ticks(b.tick_interval, self.tick_rate, 1) as u32;
        let max = b.max_stacks.min(u8::MAX as u32) as u8;
        let status = self.statuses.entry(target).or_default();
        let (old, next) = match status.burn.filter(|old| now < old.until) {
            Some(old) => (old.stacks, old.next),
            None => (0, now + interval),
        };
        status.burn = Some(Burn {
            stacks: (old as u32 + stacks).min(max as u32) as u8,
            until,
            source,
            attack,
            next,
        });
    }

    /// `source` pins `target` with a Pinning Arrow: an enemy is rooted
    /// (`pinning_arrow.enemy_time`, heavies `heavy_time`), a player slowed
    /// (`player_slow` for `player_time`). Only hostile, living targets.
    pub fn apply_pin(&mut self, t: &Tuning, source: Owner, target: HitTarget) {
        if !self.can_hit(source, target) {
            return;
        }
        let pa = &t.combat.pinning_arrow;
        match target {
            HitTarget::Player(id) => {
                if let Some(p) = self.players.get_mut(&id) {
                    let m = &mut p.movement;
                    m.slow_time = m.slow_time.max(pa.player_time);
                    m.slow_scale = pa.player_slow;
                    // The pin bites at once (the owner's client is corrected like knockback).
                    m.velocity.x *= pa.player_slow;
                    m.velocity.z *= pa.player_slow;
                }
            }
            HitTarget::Entity(id) => {
                let Some(e) = self.enemies.get(&id) else {
                    return;
                };
                let secs = if e.def.health >= pa.heavy_health {
                    pa.heavy_time
                } else {
                    pa.enemy_time
                };
                let until = self.tick + secs_to_ticks(secs, self.tick_rate, 1) as u32;
                let anchor = e.position;
                let status = self.statuses.entry(target).or_default();
                let until = status
                    .pin
                    .filter(|p| p.until > until)
                    .map_or(until, |p| p.until);
                status.pin = Some(Pin { until, anchor });
            }
        }
    }

    /// The floor under `point` within `depth` metres (a pool or a lantern lands there), if
    /// any.
    pub(crate) fn floor_below(&self, point: Vec3, depth: f32) -> Option<Vec3> {
        self.collision
            .world
            .cast_ray(
                point + Vec3::Y * 0.2,
                Vec3::NEG_Y,
                depth + 0.2,
                MASK_MOVEMENT,
            )
            .map(|h| h.point + Vec3::Y * 0.02)
    }

    /// Per tick: burns deal their damage, pinned enemies are held, and everything expired,
    /// dead or gone is dropped.
    pub(crate) fn step_statuses(&mut self, t: &Tuning) {
        let now = self.tick;

        // Drop what is over or whose target is gone.
        let players = &self.players;
        let enemies = &self.enemies;
        self.statuses.retain(|target, s| {
            let alive = match target {
                HitTarget::Player(id) => players.get(id).is_some_and(|p| !p.combat.dead),
                HitTarget::Entity(id) => enemies.get(id).is_some_and(|e| e.is_alive()),
            };
            if !alive {
                return false;
            }
            s.burn = s.burn.filter(|b| now < b.until && b.stacks > 0);
            s.pin = s.pin.filter(|p| now < p.until);
            s.burn.is_some() || s.pin.is_some()
        });

        // Pinned enemies stay where they were pinned (their own steering keeps trying).
        for (target, s) in &self.statuses {
            let (HitTarget::Entity(id), Some(pin)) = (target, s.pin) else {
                continue;
            };
            if let Some(e) = self.enemies.get_mut(id) {
                e.position.x = pin.anchor.x;
                e.position.z = pin.anchor.z;
                e.velocity.x = 0.0;
                e.velocity.z = 0.0;
            }
        }

        // Burn damage.
        let b = &t.combat.burn;
        let interval = secs_to_ticks(b.tick_interval, self.tick_rate, 1) as u32;
        let per_tick = b.dps * interval as f32 / self.tick_rate as f32;
        let mut hits = Vec::new();
        for (target, s) in self.statuses.iter_mut() {
            let Some(burn) = &mut s.burn else {
                continue;
            };
            if now < burn.next {
                continue;
            }
            burn.next = now + interval;
            hits.push((*target, burn.source, burn.attack, burn.stacks));
        }
        for (target, source, attack, stacks) in hits {
            let Some(center) = self.target_center(target) else {
                continue;
            };
            let info = DamageInfo::new(per_tick * stacks as f32, source, attack)
                .at(center, Vec3::NEG_Y)
                .part(HitPart::Body)
                .flags(DamageFlags::BURN);
            self.apply_damage(t, target, info);
        }
    }

    /// Living players and enemies whose feet are inside a disc on the ground (and at most 2 m
    /// above it): Kindle pools and snares (`knacks.rs`).
    pub(crate) fn targets_in_patch(&self, center: Vec3, radius: f32) -> Vec<HitTarget> {
        let inside = |feet: Vec3| {
            let d = feet - center;
            Vec3::new(d.x, 0.0, d.z).length() <= radius && (-0.5..=2.0).contains(&d.y)
        };
        let players = self
            .players
            .values()
            .filter(|p| !p.combat.dead && inside(p.movement.position))
            .map(|p| HitTarget::Player(p.id));
        let enemies = self
            .enemies
            .values()
            .filter(|e| e.is_alive() && inside(e.position))
            .map(|e| HitTarget::Entity(e.id));
        players.chain(enemies).collect()
    }

    /// Body centre of a living target.
    pub(crate) fn target_center(&self, target: HitTarget) -> Option<Vec3> {
        match target {
            HitTarget::Player(id) => self
                .players
                .get(&id)
                .filter(|p| !p.combat.dead)
                .map(|p| p.movement.position + Vec3::Y * 0.9),
            HitTarget::Entity(id) => self
                .enemies
                .get(&id)
                .filter(|e| e.is_alive())
                .map(|e| e.center()),
        }
    }
}
