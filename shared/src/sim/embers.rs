//! Ember bursts: close-range healing (*kindling*, see `docs/premise.md`).
//!
//! Every damage instance dealt to an enemy (or, in PvP, to another player) sheds a burst of
//! embers at the hit point holding a heal pool of `embers.heal_fraction` of the damage dealt
//! (`melee_heal_fraction` for punches and parries, `melee_weapon_heal_fraction` for the melee
//! weapons' strikes). Each tick the **closest** living player
//! inside the burst's radius who is below their effective max draws it in (whoever is closest
//! gets it — players compete in PvP). An enemy death releases its stored heat as a big burst.
//! Bursts close together in space and time merge (a shotgun blast makes one burst, not ten).
//!
//! Replicated as `kind::EMBER_BURST`: `health` = remaining pool (HP, rounded up), `state` =
//! radius in decimetres, `aux` = spawn tick, flag `BIG` for death bursts. Heals are reported
//! as [`SimEvent::Healed`] (clients draw a stream of sparks from the burst into the healed
//! player's chest lantern).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageEvent, DamageFlags, Owner, secs_to_ticks};
use super::{PlayerId, SimEvent, SimWorld};
use crate::protocol::{EntityState, NetId, ember_flags, kind};
use crate::tuning::Tuning;

/// A live heal pool.
#[derive(Clone, Debug, PartialEq)]
pub struct EmberBurst {
    pub id: NetId,
    pub position: Vec3,
    /// Remaining heal (HP).
    pub pool: f32,
    pub radius: f32,
    pub spawn_tick: u32,
    pub expire_tick: u32,
    /// Enemy death burst.
    pub big: bool,
    /// PvP: the player whose wick shed these embers can't draw them back in.
    pub shed_by: Option<PlayerId>,
}

/// Where a heal came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealSource {
    /// Drew in an ember burst (kindling).
    Embers(NetId),
    Parry,
    /// Stood in a Kindle Flask's pool (`knacks.rs`).
    KindleFlask(NetId),
}

impl EmberBurst {
    pub fn entity_state(&self) -> EntityState {
        EntityState {
            id: self.id,
            kind: kind::EMBER_BURST,
            position: self.position,
            flags: if self.big { ember_flags::BIG } else { 0 },
            state: (self.radius * 10.0).round().clamp(0.0, u16::MAX as f32) as u16,
            health: self.pool.ceil().clamp(0.0, u16::MAX as f32) as u16,
            aux: self.spawn_tick,
            ..Default::default()
        }
    }
}

impl SimWorld {
    /// Spawns (or merges into a recent nearby burst) a heal pool anyone may draw in.
    pub fn spawn_embers(
        &mut self,
        t: &Tuning,
        position: Vec3,
        pool: f32,
        radius: f32,
        lifetime: f32,
        big: bool,
    ) -> Option<NetId> {
        self.spawn_embers_from(t, position, pool, (radius, lifetime), big, None)
    }

    /// Like [`SimWorld::spawn_embers`]; `shed_by` (a PvP victim) can't draw it in.
    pub fn spawn_embers_from(
        &mut self,
        t: &Tuning,
        position: Vec3,
        pool: f32,
        (radius, lifetime): (f32, f32),
        big: bool,
        shed_by: Option<PlayerId>,
    ) -> Option<NetId> {
        if pool <= 0.0 || !position.is_finite() {
            return None;
        }
        let b = &t.combat.embers;
        let now = self.tick;
        let window = secs_to_ticks(b.merge_window, self.tick_rate, 0) as u32;
        let life = secs_to_ticks(lifetime, self.tick_rate, 1) as u32;
        if !big
            && let Some(existing) = self.embers.values_mut().find(|e| {
                !e.big
                    && e.shed_by == shed_by
                    && now.saturating_sub(e.spawn_tick) <= window
                    && e.position.distance(position) <= b.merge_distance
            })
        {
            existing.pool += pool;
            existing.expire_tick = existing.expire_tick.max(now + life);
            return Some(existing.id);
        }
        let id = self.alloc_net_id()?;
        self.embers.insert(
            id,
            EmberBurst {
                id,
                position,
                pool,
                radius,
                spawn_tick: now,
                expire_tick: now + life,
                big,
                shed_by,
            },
        );
        Some(id)
    }

    /// Burst for a damage instance dealt by a player.
    pub(crate) fn spawn_embers_for_damage(&mut self, t: &Tuning, event: &DamageEvent) {
        let b = &t.combat.embers;
        if !matches!(event.info.source, Owner::Player(_)) || event.dealt <= 0.0 {
            return;
        }
        let melee = event.info.flags.contains(DamageFlags::MELEE);
        let fist = (melee && event.info.attack == Attack::Fist)
            || (event.info.flags.contains(DamageFlags::PARRIED) && event.info.projectile.is_none());
        let fraction = if fist {
            b.melee_heal_fraction
        } else if melee {
            b.melee_weapon_heal_fraction
        } else {
            b.heal_fraction
        };
        let position = if event.info.point.length_squared() > 0.0 {
            event.info.point
        } else {
            self.target_origin(event.target).unwrap_or(Vec3::ZERO) + Vec3::Y
        };
        self.spawn_embers_from(
            t,
            position,
            event.dealt * fraction,
            (b.radius, b.lifetime),
            false,
            event.target.player(),
        );
    }

    /// Heals the closest player in range of each burst; removes empty/expired bursts.
    pub(crate) fn step_embers(&mut self, t: &Tuning) {
        let now = self.tick;
        let ids: Vec<NetId> = self.embers.keys().copied().collect();
        for id in ids {
            let Some(burst) = self.embers.get(&id) else {
                continue;
            };
            if now >= burst.expire_tick || burst.pool <= 1e-3 {
                self.embers.remove(&id);
                continue;
            }
            let (pos, radius, shed_by) = (burst.position, burst.radius, burst.shed_by);
            let closest: Option<(PlayerId, f32)> = self
                .players
                .values()
                .filter(|p| !p.combat.dead && p.combat.missing() > 1e-3 && Some(p.id) != shed_by)
                .map(|p| {
                    let (center, _, _) = p.movement.capsule(&t.movement);
                    (p.id, center.distance(pos))
                })
                .filter(|(_, d)| *d <= radius)
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            let Some((player, _)) = closest else {
                continue;
            };
            let pool = self.embers.get(&id).map_or(0.0, |b| b.pool);
            let healed = self
                .players
                .get_mut(&player)
                .map_or(0.0, |p| p.combat.heal(pool));
            if healed <= 0.0 {
                continue;
            }
            if let Some(b) = self.embers.get_mut(&id) {
                b.pool -= healed;
            }
            self.events.push(SimEvent::Healed {
                player,
                amount: healed,
                source: HealSource::Embers(id),
                position: pos,
            });
        }
    }
}
