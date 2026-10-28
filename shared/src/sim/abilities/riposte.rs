//! Riposte (Melee, defend; `docs/rework.md` §1): hold ALT to guard for up to
//! `riposte.hold_time`, moving at `riposte.move_scale`. The shared guard (`guard.rs`) blocks
//! blows and shots from the front `riposte.cone_degrees` (enemy parryable projectiles are
//! deflected like a parry, others destroyed; melee attackers are staggered). The release (or
//! the end) counter-swings in the carrier's arc at `riposte.damage_scale` × its swing
//! (+`per_block` per further block, at most `max_scale`) if anything was blocked, and the
//! first block heals like a parry; nothing blocked: no swing. Cooldown from the release.
//!
//! Groundwork (arsenal-a M1): the guard goes up and down; the counter, the stagger, the
//! deflect and the heal come with M2.

use glam::Vec3;

use super::super::combat::{DamageInfo, secs_to_ticks};
use super::super::projectiles::Projectile;
use super::super::{PlayerId, SimWorld};
use super::guard::Guard;
use super::{AbilityCast, AbilityPhase};
use crate::tuning::Tuning;

/// Riposte state between ticks (the guard itself is `guard.rs`'s).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ripostes {}

impl Ripostes {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The press raises the guard, the release drops it (and counters).
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
                self.drop_guard(c.id);
            }
        }
    }

    /// The guard caught a hit (`guard.rs` booked it). Never breaks.
    pub(crate) fn riposte_blocked(
        &mut self,
        _t: &Tuning,
        _id: PlayerId,
        _guard: &Guard,
        _info: &DamageInfo,
    ) -> bool {
        false
    }

    /// A projectile is about to hit player `id` behind a raised Riposte (`guard.rs` asks):
    /// `Some(keep)` if the guard deals with it.
    pub(crate) fn riposte_meets_projectile(
        &mut self,
        _t: &Tuning,
        _id: PlayerId,
        _guard: &Guard,
        _p: &mut Projectile,
        _dir: Vec3,
    ) -> Option<bool> {
        None
    }

    /// Per tick.
    pub(crate) fn step_riposte(&mut self, _t: &Tuning) {}
}
