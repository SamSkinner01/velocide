//! The guard (`docs/rework.md` §1, "The guard"): the player-side state Bulwark and Riposte
//! raise while their ability is held.
//!
//! A [`Guard`] faces along its owner's aim (the Bulwark's slab with the pitch clamped) and
//! covers `half_angle` around it. The damage pipeline asks [`SimWorld::guard_covers`] for every
//! hit on a player: a hostile hit with a direction (a ray, a projectile, an explosion — its
//! centre —, a blow, an enemy attack; anything flagged `HITSCAN`, `PROJECTILE`, `EXPLOSIVE`,
//! `MELEE` or `SLAM`) coming from inside the cone is absorbed whole: no damage, no knockback
//! (`Blocked::Guarded`), and [`SimWorld::guard_absorbed`] books it (the Bulwark's strength
//! runs out, the Riposte counts its blocks) and emits [`SimEvent::GuardBlock`]. Burn ticks,
//! the world, floor hazards and your own blasts are never blocked.
//!
//! Raised by the ability's press ([`SimWorld::raise_guard`]), dropped by its release
//! ([`SimWorld::drop_guard`]); [`SimWorld::step_guards`] drops a guard whose owner died, left,
//! stopped holding its ability or ran out of `until`. While it is up the owner is slowed
//! (`MoveState::guard_time` / `guard_scale`, server-set: the owner reconciles once at the raise
//! and once at the drop) and replicated with `player_flags::GUARDING` (`aux` bits 12–15: the
//! strength left in sixteenths).

use std::collections::BTreeMap;

use glam::Vec3;

use super::super::combat::{DamageFlags, DamageInfo, Owner};
use super::super::projectiles::Projectile;
use super::super::{HitTarget, PlayerId, SimEvent, SimWorld};
use crate::input::look_dir;
use crate::loadout::{Ability, WeaponKind};
use crate::protocol::{EntityState, player_flags};
use crate::tuning::Tuning;

/// A raised guard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guard {
    /// The ability holding it (Bulwark or Riposte).
    pub ability: Ability,
    /// The weapon carrying it, and its slot.
    pub weapon: WeaponKind,
    pub slot: u8,
    /// Where it faces (unit; follows the owner's aim every tick, [`SimWorld::guard_facing`]).
    pub facing: Vec3,
    /// Half the aperture it covers (radians).
    pub half_angle: f32,
    /// Damage it can still absorb (`f32::INFINITY`: no limit) ...
    pub absorb_left: f32,
    /// ... of this much at full strength.
    pub absorb: f32,
    /// It drops by itself at this tick.
    pub until: u32,
    /// Hits it caught, and their damage.
    pub blocks: u32,
    pub absorbed: f32,
    /// The ability's own pitch limit (radians; `None`: the aim's pitch).
    pub pitch_limit: Option<f32>,
}

impl Guard {
    /// Strength left, 0..=1.
    pub fn strength(&self) -> f32 {
        if self.absorb_left.is_finite() && self.absorb > 0.0 {
            (self.absorb_left / self.absorb).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

/// Raised guards ([`super::AbilityWorld::guards`]), one per player at most.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guards {
    pub raised: BTreeMap<PlayerId, Guard>,
}

impl Guards {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.raised.remove(&id);
    }
}

/// Damage a guard can catch: something hostile that travels from somewhere (not a Burn tick,
/// the world or a floor hazard).
fn guardable(info: &DamageInfo) -> bool {
    let kinds = [
        DamageFlags::HITSCAN,
        DamageFlags::PROJECTILE,
        DamageFlags::EXPLOSIVE,
        DamageFlags::MELEE,
        DamageFlags::SLAM,
    ];
    !info.flags.contains(DamageFlags::BURN)
        && !info.flags.contains(DamageFlags::ENVIRONMENT)
        && kinds.iter().any(|k| info.flags.contains(*k))
}

impl SimWorld {
    /// Player `id`'s raised guard.
    pub fn guard(&self, id: PlayerId) -> Option<&Guard> {
        self.abilities.guards.raised.get(&id)
    }

    /// Raises (or replaces) player `id`'s guard and slows them while it lasts.
    pub(crate) fn raise_guard(&mut self, id: PlayerId, mut guard: Guard, move_scale: f32) {
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        guard.facing = Self::facing_of(p.yaw, p.pitch, guard.pitch_limit);
        let secs = guard.until.saturating_sub(self.tick) as f32 / self.tick_rate as f32;
        p.movement.guard_time = secs;
        p.movement.guard_scale = move_scale;
        self.abilities.guards.raised.insert(id, guard);
    }

    /// Drops player `id`'s guard (if it is up) and lifts its slow; returns it.
    pub(crate) fn drop_guard(&mut self, id: PlayerId) -> Option<Guard> {
        let guard = self.abilities.guards.raised.remove(&id)?;
        if let Some(p) = self.players.get_mut(&id) {
            p.movement.guard_time = 0.0;
        }
        Some(guard)
    }

    /// Where a guard faces for a view (`pitch_limit`: the Bulwark's clamp).
    fn facing_of(yaw: f32, pitch: f32, pitch_limit: Option<f32>) -> Vec3 {
        let pitch = match pitch_limit {
            Some(limit) => pitch.clamp(-limit, limit),
            None => pitch,
        };
        look_dir(yaw, pitch)
    }

    /// Where player `id`'s guard faces now (their current view).
    pub fn guard_facing(&self, id: PlayerId) -> Option<Vec3> {
        let g = self.guard(id)?;
        let p = self.players.get(&id)?;
        Some(Self::facing_of(p.yaw, p.pitch, g.pitch_limit))
    }

    /// Does player `id`'s guard catch `info`? (See the module docs; `apply_damage` asks.)
    pub fn guard_covers(&self, _t: &Tuning, id: PlayerId, info: &DamageInfo) -> bool {
        let Some(g) = self.guard(id) else {
            return false;
        };
        if self.tick >= g.until
            || g.absorb_left <= 0.0
            || matches!(info.source, Owner::World)
            || info.source == Owner::Player(id)
            || !self.is_hostile(info.source, HitTarget::Player(id))
            || !guardable(info)
        {
            return false;
        }
        let Some(facing) = self.guard_facing(id) else {
            return false;
        };
        // Towards where it came from: back along its travel, else at its source.
        let from = if info.dir.length_squared() > 1e-6 {
            -info.dir
        } else {
            let at = match info.source {
                Owner::Player(p) => self.players.get(&p).map(|p| p.movement.position),
                Owner::Enemy(e) => self.enemies.get(&e).map(|e| e.center()),
                Owner::World => None,
            };
            let Some(at) = at else {
                return false;
            };
            (at - info.point).normalize_or_zero()
        };
        from.length_squared() > 1e-6 && from.angle_between(facing) <= g.half_angle
    }

    /// Player `id`'s guard caught `info` (`amount` = the damage it would have dealt): books it,
    /// emits [`SimEvent::GuardBlock`] and tells the ability (the Bulwark wears, the Riposte
    /// counts and staggers).
    pub(crate) fn guard_absorbed(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        info: &DamageInfo,
        amount: f32,
    ) {
        let Some(g) = self.abilities.guards.raised.get_mut(&id) else {
            return;
        };
        g.blocks += 1;
        g.absorbed += amount;
        g.absorb_left -= amount;
        let guard = *g;
        let broke = match guard.ability {
            Ability::Bulwark => self.bulwark_blocked(t, id, &guard, info),
            Ability::Riposte => self.riposte_blocked(t, id, &guard, info),
            _ => false,
        };
        self.events.push(SimEvent::GuardBlock {
            player: id,
            ability: guard.ability,
            position: info.point,
            broke,
        });
    }

    /// A projectile `p` (flying along `dir`) is about to hit player `target`: their guard may
    /// meet it first (the Riposte deflects a parryable one, the Bulwark's slab stops it).
    /// `Some(keep)`: the guard dealt with it; `None`: the normal hit (whose damage the
    /// pipeline still absorbs if the guard covers it). `projectiles.rs` asks.
    pub(crate) fn guard_meets_projectile(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        target: HitTarget,
        dir: Vec3,
    ) -> Option<bool> {
        let HitTarget::Player(id) = target else {
            return None;
        };
        let guard = *self.guard(id)?;
        match guard.ability {
            Ability::Riposte => self.riposte_meets_projectile(t, id, &guard, p, dir),
            Ability::Bulwark => self.bulwark_meets_projectile(t, id, &guard, p, dir),
            _ => None,
        }
    }

    /// Per tick: guards follow their owner's aim; a guard whose owner died, left, stopped
    /// holding its ability or ran past `until` drops.
    pub(crate) fn step_guards(&mut self, _t: &Tuning) {
        let now = self.tick;
        let ids: Vec<PlayerId> = self.abilities.guards.raised.keys().copied().collect();
        for id in ids {
            let held = self.players.get(&id).and_then(|p| {
                let w = &p.combat.weapons;
                (!p.combat.dead && w.charge > 0 && w.held().is_some())
                    .then(|| (w.ability(), w.slot))
            });
            let Some(g) = self.abilities.guards.raised.get(&id).copied() else {
                continue;
            };
            if now >= g.until || held != Some((g.ability, g.slot)) {
                self.drop_guard(id);
                continue;
            }
            if let Some(facing) = self.guard_facing(id)
                && let Some(g) = self.abilities.guards.raised.get_mut(&id)
            {
                g.facing = facing;
            }
        }
    }

    /// `GUARDING` and the strength left (`aux` bits 12–15) on player `id`'s entity.
    pub(crate) fn decorate_guard(&self, _t: &Tuning, id: PlayerId, e: &mut EntityState) {
        let Some(g) = self.guard(id) else {
            return;
        };
        e.flags |= player_flags::GUARDING;
        let strength = (g.strength() * 15.0).round() as u32;
        e.aux = (e.aux & !0xF000) | (strength.min(15) << 12);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sim::combat::{Attack, Blocked, CombatRules};
    use crate::sim::{BUILTIN_LEVEL, builtin_level};

    /// A PvP world with `me` at the origin looking down −Z and a shooter 10 m in front.
    fn duel() -> (SimWorld, Tuning, PlayerId, PlayerId) {
        let level = builtin_level(BUILTIN_LEVEL).unwrap();
        let mut w = SimWorld::new(BUILTIN_LEVEL, Arc::new(level), 60);
        w.rules = CombatRules::pvp();
        let (me, foe) = (PlayerId(0), PlayerId(1));
        for (id, z) in [(me, 0.0), (foe, -10.0)] {
            let p = w.add_player(id, "p");
            p.movement.position = Vec3::new(0.0, 0.0, z);
            p.combat.protection = 0;
            p.yaw = 0.0;
            p.pitch = 0.0;
        }
        (w, Tuning::default(), me, foe)
    }

    fn raise(w: &mut SimWorld, id: PlayerId, ability: Ability, half_degrees: f32, absorb: f32) {
        let guard = Guard {
            ability,
            weapon: WeaponKind::Bellows,
            slot: 0,
            facing: Vec3::NEG_Z,
            half_angle: half_degrees.to_radians(),
            absorb_left: absorb,
            absorb,
            until: w.tick + 600,
            blocks: 0,
            absorbed: 0.0,
            pitch_limit: Some(30f32.to_radians()),
        };
        w.raise_guard(id, guard, 0.75);
    }

    /// A 20 damage hit on `me` from `foe`, travelling along `dir`.
    fn hit(foe: PlayerId, dir: Vec3, flags: DamageFlags) -> DamageInfo {
        DamageInfo::new(20.0, Owner::Player(foe), Attack::Quill)
            .at(Vec3::new(0.0, 1.2, 0.0), dir)
            .knockback(dir * 10.0)
            .flags(flags)
    }

    #[test]
    fn the_guard_catches_hostile_hits_from_its_front_only() {
        let (mut w, t, me, foe) = duel();
        raise(&mut w, me, Ability::Bulwark, 55.0, 150.0);
        assert!(
            w.players[&me].movement.guard_time > 0.0,
            "slowed while it is up"
        );
        let target = HitTarget::Player(me);
        // A ray from the front (travelling +Z, towards me): absorbed whole, no knockback.
        let ev = w.apply_damage(&t, target, hit(foe, Vec3::Z, DamageFlags::HITSCAN));
        assert_eq!(ev.blocked, Some(Blocked::Guarded));
        assert_eq!(ev.dealt, 0.0);
        assert_eq!(w.players[&me].movement.velocity, Vec3::ZERO);
        assert!(w.events.iter().any(|e| matches!(
            e,
            SimEvent::GuardBlock { player, ability: Ability::Bulwark, broke: false, .. }
                if *player == me
        )));
        assert_eq!(w.guard(me).unwrap().blocks, 1);
        assert_eq!(w.guard(me).unwrap().absorb_left, 130.0);
        // An explosion whose centre is in front (pushing me +Z): absorbed whole too, its
        // knockback included.
        let blast = hit(foe, Vec3::Z, DamageFlags::EXPLOSIVE);
        assert_eq!(
            w.apply_damage(&t, target, blast).blocked,
            Some(Blocked::Guarded)
        );
        assert_eq!(w.players[&me].movement.velocity, Vec3::ZERO);
        // From behind (travelling −Z) or the side: it lands.
        let ev = w.apply_damage(&t, target, hit(foe, Vec3::NEG_Z, DamageFlags::HITSCAN));
        assert_eq!((ev.blocked, ev.dealt), (None, 20.0));
        let ev = w.apply_damage(&t, target, hit(foe, Vec3::X, DamageFlags::PROJECTILE));
        assert_eq!(ev.blocked, None);
        // Burn ticks, the world and floor hazards (no kind with a direction) are never
        // blocked.
        let burn = hit(foe, Vec3::Z, DamageFlags::BURN);
        assert_eq!(w.apply_damage(&t, target, burn).blocked, None);
        let hazard =
            DamageInfo::new(5.0, Owner::Player(foe), Attack::Quill).at(Vec3::ZERO, Vec3::Y);
        assert_eq!(w.apply_damage(&t, target, hazard).blocked, None);
        let world = DamageInfo::new(5.0, Owner::World, Attack::World)
            .at(Vec3::ZERO, Vec3::Z)
            .flags(DamageFlags::HITSCAN);
        assert_eq!(w.apply_damage(&t, target, world).blocked, None);
        // Turning round turns the guard: the same ray from the front now lands.
        w.players.get_mut(&me).unwrap().yaw = std::f32::consts::PI;
        let ev = w.apply_damage(&t, target, hit(foe, Vec3::Z, DamageFlags::HITSCAN));
        assert_eq!(ev.blocked, None);
        let ev = w.apply_damage(&t, target, hit(foe, Vec3::NEG_Z, DamageFlags::HITSCAN));
        assert_eq!(ev.blocked, Some(Blocked::Guarded));
    }

    #[test]
    fn a_spent_bulwark_shatters_and_the_guard_drops_with_its_ability() {
        let (mut w, t, me, foe) = duel();
        raise(&mut w, me, Ability::Bulwark, 55.0, 30.0);
        let target = HitTarget::Player(me);
        let front = || hit(foe, Vec3::Z, DamageFlags::HITSCAN);
        assert_eq!(
            w.apply_damage(&t, target, front()).blocked,
            Some(Blocked::Guarded)
        );
        // The hit that spends it is still absorbed whole, then the slab is gone.
        assert_eq!(
            w.apply_damage(&t, target, front()).blocked,
            Some(Blocked::Guarded)
        );
        assert!(w.guard(me).is_none());
        assert!(
            w.events
                .iter()
                .any(|e| matches!(e, SimEvent::GuardBlock { broke: true, .. }))
        );
        assert_eq!(
            w.players[&me].movement.guard_time, 0.0,
            "the slow is lifted"
        );
        assert_eq!(
            w.players[&me].combat.weapons.alt_cooldown[0],
            crate::sim::combat::secs_to_ticks(t.combat.bulwark.broken_cooldown, 60, 1)
        );
        assert_eq!(w.apply_damage(&t, target, front()).blocked, None);
        // A teammate's shots aren't hostile: never caught (co-op: friendly fire first).
        let (mut w, t, me, foe) = duel();
        raise(&mut w, me, Ability::Riposte, 60.0, f32::INFINITY);
        w.rules = CombatRules::coop();
        let ev = w.apply_damage(
            &t,
            HitTarget::Player(me),
            hit(foe, Vec3::Z, DamageFlags::HITSCAN),
        );
        assert_eq!(ev.blocked, Some(Blocked::FriendlyFire));
        assert_eq!(w.guard(me).unwrap().blocks, 0);
        // Not holding its ability any more (the default build holds the Quill): it drops.
        w.step_guards(&t);
        assert!(w.guard(me).is_none());
    }
}
