//! Riposte (Melee, defend; `docs/rework.md` §1): hold ALT to guard for up to
//! `riposte.hold_time` (`AbilityStyle::Channel`), moving at `riposte.move_scale`. The shared
//! guard (`guard.rs`) blocks blows and shots from the front `riposte.cone_degrees`: no damage,
//! no knockback. A Mould's blow that it blocks staggers the Mould (`riposte.stagger`); an
//! enemy's parryable projectile reaching it is deflected like a parry (the punch's rules and
//! reward), any other projectile is destroyed. The first block heals like a parry
//! (`SimWorld::parry_reward`).
//!
//! The release (or the limit) counter-swings in the carrier's arc (`melee_arc`, lag-compensated
//! like any swing) at `riposte.damage_scale` × its swing, `per_block` more for every further
//! block, at most `max_scale` (`Attack::Riposte`, `MELEE`, a `SimEvent::Swing`) — if anything
//! was blocked; nothing blocked: no swing. The cooldown runs from the release either way.

use glam::Vec3;

use super::super::combat::{DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::super::melee::{melee_damage, swing_knockback};
use super::super::projectiles::Projectile;
use super::super::{PlayerId, SimEvent, SimWorld};
use super::guard::Guard;
use super::{AbilityCast, AbilityPhase};
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// Riposte state between ticks (the guard itself is `guard.rs`'s).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ripostes {
    /// Moulds whose blow a guard caught this tick, to stagger once the Moulds have moved
    /// (a Mould striking is out of `SimWorld::enemies` during its own step).
    pub staggers: Vec<NetId>,
}

impl Ripostes {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The press raises the guard, the release drops it and counters.
    pub(crate) fn cast_riposte(&mut self, c: &AbilityCast) {
        let r = &c.t.combat.riposte;
        match c.phase {
            AbilityPhase::Use => {
                let Some(p) = self.players.get(&c.id) else {
                    return;
                };
                let until = self.tick + secs_to_ticks(r.hold_time, self.tick_rate, 1) as u32;
                let guard = Guard {
                    ability: c.ability,
                    weapon: c.weapon,
                    slot: p.combat.weapons.slot,
                    facing: c.input.look_dir(),
                    half_angle: (r.cone_degrees * 0.5).to_radians(),
                    absorb_left: f32::INFINITY,
                    absorb: f32::INFINITY,
                    until,
                    blocks: 0,
                    absorbed: 0.0,
                    pitch_limit: None,
                };
                self.raise_guard(c.id, guard, r.move_scale);
            }
            AbilityPhase::Release | AbilityPhase::Recast => {
                let Some(guard) = self.drop_guard(c.id) else {
                    return;
                };
                if guard.blocks > 0 {
                    self.riposte_counter(c, guard.blocks);
                }
            }
        }
    }

    /// The counter-swing after `blocks` blocks.
    fn riposte_counter(&mut self, c: &AbilityCast, blocks: u32) {
        let t = c.t;
        let r = &t.combat.riposte;
        let scale = (r.damage_scale + r.per_block * (blocks - 1) as f32).min(r.max_scale);
        let damage = c.weapon.primary_damage(&t.combat) * scale;
        let aim = c.input.look_dir();
        let targets = self.arc_targets(t, c.history, c.id, c.input, c.weapon.melee_arc(&t.combat));
        let kb = swing_knockback(aim, r.knockback);
        let hits = targets.len().min(u8::MAX as usize) as u8;
        for (_, target, point) in targets {
            let info = melee_damage(c.id, (damage, c.ability.attack()), point, aim, kb);
            self.apply_damage(t, target, info);
        }
        self.events.push(SimEvent::Swing {
            player: c.id,
            attack: c.ability.attack(),
            hits,
        });
    }

    /// The guard caught a hit (`guard.rs` booked it): the first one heals like a parry, a
    /// Mould's blow staggers the Mould. Never breaks.
    pub(crate) fn riposte_blocked(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        guard: &Guard,
        info: &DamageInfo,
    ) -> bool {
        if guard.blocks == 1 {
            let healed = self.parry_reward(t, id);
            if healed > 0.0
                && let Some(p) = self.players.get(&id)
            {
                let position = p.movement.eye_position(&t.movement);
                self.events.push(SimEvent::Healed {
                    player: id,
                    amount: healed,
                    source: super::super::HealSource::Parry,
                    position,
                });
            }
        }
        if info.flags.contains(DamageFlags::MELEE)
            && let Owner::Enemy(e) = info.source
        {
            self.abilities.riposte.staggers.push(e);
        }
        false
    }

    /// A projectile is about to hit player `id` behind a raised Riposte (`guard.rs` asks):
    /// from the guarded front, an enemy's parryable one is deflected like a parry and any
    /// other destroyed (both count as a block).
    pub(crate) fn riposte_meets_projectile(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        _guard: &Guard,
        p: &mut Projectile,
        dir: Vec3,
    ) -> Option<bool> {
        let info = DamageInfo::new(p.damage, p.owner, p.attack)
            .at(p.position, dir)
            .flags(DamageFlags::PROJECTILE);
        if !self.guard_covers(t, id, &info) {
            return None;
        }
        let amount = p.explosion.map_or(p.damage, |e| e.damage);
        self.guard_absorbed(t, id, &info, amount);
        if p.kind.parryable() && matches!(p.original_owner, Owner::Enemy(_)) {
            // Turned around like a punch would (`redirect_projectile` works on the world's
            // copy: put it back for it, take it back after).
            self.projectiles.insert(p.id, p.clone());
            let c = &t.combat;
            let (speed, damage) = (c.parry_speed_multiplier, c.parry_damage_multiplier);
            self.redirect_projectile(t, id, p.id, speed, damage, true);
            // The deflected copy lives on in the map; this one is dropped by the caller.
            if let Some(turned) = self.projectiles.remove(&p.id) {
                *p = turned;
            }
            return Some(true);
        }
        Some(false)
    }

    /// Per tick (after the Moulds): the blows caught this tick stagger their Moulds.
    pub(crate) fn step_riposte(&mut self, t: &Tuning) {
        let ticks = secs_to_ticks(t.combat.riposte.stagger, self.tick_rate, 1);
        for id in std::mem::take(&mut self.abilities.riposte.staggers) {
            if let Some(e) = self.enemies.get_mut(&id) {
                e.stun(ticks);
            }
        }
    }
}
