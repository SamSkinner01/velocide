//! Blood bursts: close-range healing.
//!
//! Every damage instance dealt to an enemy (or, in PvP, to another player) spawns a burst at
//! the hit point holding a heal pool of `blood.heal_fraction` of the damage dealt
//! (`melee_heal_fraction` for punches and parries). Each tick the **closest** living player
//! inside the burst's radius who is below their effective max drains it (whoever is closest
//! gets it — players compete in PvP). An enemy death spawns a big burst. Bursts close
//! together in space and time merge (a shotgun blast makes one burst, not ten).
//!
//! Replicated as `kind::BLOOD_BURST`: `health` = remaining pool (HP, rounded up), `state` =
//! radius in decimetres, `aux` = spawn tick, flag `BIG` for death bursts. Heals are reported
//! as [`SimEvent::Healed`] (clients draw a red streak from the burst to the healed player).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::{DamageEvent, DamageFlags, Owner, secs_to_ticks};
use super::{PlayerId, SimEvent, SimWorld};
use crate::protocol::{EntityState, NetId, blood_flags, kind};
use crate::tuning::Tuning;

/// A live heal pool.
#[derive(Clone, Debug, PartialEq)]
pub struct BloodBurst {
    pub id: NetId,
    pub position: Vec3,
    /// Remaining heal (HP).
    pub pool: f32,
    pub radius: f32,
    pub spawn_tick: u32,
    pub expire_tick: u32,
    /// Enemy death burst.
    pub big: bool,
    /// PvP: the player who bled it can't heal from it.
    pub bleeder: Option<PlayerId>,
}

/// Where a heal came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealSource {
    Blood(NetId),
    Parry,
}

impl BloodBurst {
    pub fn entity_state(&self) -> EntityState {
        EntityState {
            id: self.id,
            kind: kind::BLOOD_BURST,
            position: self.position,
            flags: if self.big { blood_flags::BIG } else { 0 },
            state: (self.radius * 10.0).round().clamp(0.0, u16::MAX as f32) as u16,
            health: self.pool.ceil().clamp(0.0, u16::MAX as f32) as u16,
            aux: self.spawn_tick,
            ..Default::default()
        }
    }
}

impl SimWorld {
    /// Spawns (or merges into a recent nearby burst) a heal pool anyone may drain.
    pub fn spawn_blood(
        &mut self,
        t: &Tuning,
        position: Vec3,
        pool: f32,
        radius: f32,
        lifetime: f32,
        big: bool,
    ) -> Option<NetId> {
        self.spawn_blood_from(t, position, pool, (radius, lifetime), big, None)
    }

    /// Like [`SimWorld::spawn_blood`]; `bleeder` (a PvP victim) can't drain it.
    pub fn spawn_blood_from(
        &mut self,
        t: &Tuning,
        position: Vec3,
        pool: f32,
        (radius, lifetime): (f32, f32),
        big: bool,
        bleeder: Option<PlayerId>,
    ) -> Option<NetId> {
        if pool <= 0.0 || !position.is_finite() {
            return None;
        }
        let b = &t.combat.blood;
        let now = self.tick;
        let window = secs_to_ticks(b.merge_window, self.tick_rate, 0) as u32;
        let life = secs_to_ticks(lifetime, self.tick_rate, 1) as u32;
        if !big
            && let Some(existing) = self.blood.values_mut().find(|e| {
                !e.big
                    && e.bleeder == bleeder
                    && now.saturating_sub(e.spawn_tick) <= window
                    && e.position.distance(position) <= b.merge_distance
            })
        {
            existing.pool += pool;
            existing.expire_tick = existing.expire_tick.max(now + life);
            return Some(existing.id);
        }
        let id = self.alloc_net_id()?;
        self.blood.insert(
            id,
            BloodBurst {
                id,
                position,
                pool,
                radius,
                spawn_tick: now,
                expire_tick: now + life,
                big,
                bleeder,
            },
        );
        Some(id)
    }

    /// Burst for a damage instance dealt by a player.
    pub(crate) fn spawn_blood_for_damage(&mut self, t: &Tuning, event: &DamageEvent) {
        let b = &t.combat.blood;
        if !matches!(event.info.source, Owner::Player(_)) || event.dealt <= 0.0 {
            return;
        }
        let melee = event.info.flags.contains(DamageFlags::MELEE)
            || (event.info.flags.contains(DamageFlags::PARRIED) && event.info.projectile.is_none());
        let fraction = if melee {
            b.melee_heal_fraction
        } else {
            b.heal_fraction
        };
        let position = if event.info.point.length_squared() > 0.0 {
            event.info.point
        } else {
            self.target_origin(event.target).unwrap_or(Vec3::ZERO) + Vec3::Y
        };
        self.spawn_blood_from(
            t,
            position,
            event.dealt * fraction,
            (b.radius, b.lifetime),
            false,
            event.target.player(),
        );
    }

    /// Heals the closest player in range of each burst; removes empty/expired bursts.
    pub(crate) fn step_blood(&mut self, t: &Tuning) {
        let now = self.tick;
        let ids: Vec<NetId> = self.blood.keys().copied().collect();
        for id in ids {
            let Some(burst) = self.blood.get(&id) else {
                continue;
            };
            if now >= burst.expire_tick || burst.pool <= 1e-3 {
                self.blood.remove(&id);
                continue;
            }
            let (pos, radius, bleeder) = (burst.position, burst.radius, burst.bleeder);
            let closest: Option<(PlayerId, f32)> = self
                .players
                .values()
                .filter(|p| !p.combat.dead && p.combat.missing() > 1e-3 && Some(p.id) != bleeder)
                .map(|p| {
                    let (center, _, _) = p.movement.capsule(&t.movement);
                    (p.id, center.distance(pos))
                })
                .filter(|(_, d)| *d <= radius)
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let Some((player, _)) = closest else {
                continue;
            };
            let pool = self.blood.get(&id).map_or(0.0, |b| b.pool);
            let healed = self
                .players
                .get_mut(&player)
                .map_or(0.0, |p| p.combat.heal(pool));
            if healed <= 0.0 {
                continue;
            }
            if let Some(b) = self.blood.get_mut(&id) {
                b.pool -= healed;
            }
            self.events.push(SimEvent::Healed {
                player,
                amount: healed,
                source: HealSource::Blood(id),
                position: pos,
            });
        }
    }
}
