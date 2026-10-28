//! Bulwark (Scatter, defend; `docs/rework.md` §1): hold ALT to raise a slab of hardened wax
//! in front of you (`bulwark.width` × `height`, `distance` out, turning with the aim's yaw,
//! pitch clamped to `pitch_limit_degrees`), up to `bulwark.hold_time`. The shared guard
//! (`guard.rs`) absorbs hostile hits from its `cone_degrees` front until it has absorbed
//! `bulwark.absorb`, then it shatters. Your own shots pass; you move at `move_scale`. The
//! cooldown runs from the drop (`broken_cooldown` if it shattered).
//!
//! Groundwork (arsenal-a M1): the guard goes up on the press, down on the release, and
//! shatters when it is spent. The slab stopping rays and projectiles physically (so they don't
//! reach what is behind you either) is arsenal-b's (`SimWorld::guard_meets_projectile`, the
//! ray hook in `hitscan.rs`).

use glam::Vec3;

use super::super::combat::{DamageInfo, secs_to_ticks};
use super::super::projectiles::Projectile;
use super::super::{PlayerId, SimWorld};
use super::guard::Guard;
use super::{AbilityCast, AbilityPhase};
use crate::tuning::Tuning;

/// Slabs being held (their guard is `guard.rs`'s).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bulwarks {}

impl Bulwarks {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// The press raises the slab, the release drops it.
    pub(crate) fn cast_bulwark(&mut self, c: &AbilityCast) {
        let b = &c.t.combat.bulwark;
        match c.phase {
            AbilityPhase::Use => {
                let Some(p) = self.players.get(&c.id) else {
                    return;
                };
                let until = self.tick + secs_to_ticks(b.hold_time, self.tick_rate, 1) as u32;
                let guard = Guard {
                    ability: c.ability,
                    weapon: c.weapon,
                    slot: p.combat.weapons.slot,
                    facing: c.input.look_dir(),
                    half_angle: (b.cone_degrees * 0.5).to_radians(),
                    absorb_left: b.absorb,
                    absorb: b.absorb,
                    until,
                    blocks: 0,
                    absorbed: 0.0,
                    pitch_limit: Some(b.pitch_limit_degrees.to_radians()),
                };
                self.raise_guard(c.id, guard, b.move_scale);
            }
            AbilityPhase::Release | AbilityPhase::Recast => {
                self.drop_guard(c.id);
            }
        }
    }

    /// The slab caught a hit (`guard.rs` booked it). Returns whether it shattered: spent, it
    /// drops and the longer cooldown starts (a server-side change of the owner's weapon state:
    /// its client re-bases).
    pub(crate) fn bulwark_blocked(
        &mut self,
        t: &Tuning,
        id: PlayerId,
        guard: &Guard,
        _info: &DamageInfo,
    ) -> bool {
        if guard.absorb_left > 0.0 {
            return false;
        }
        self.drop_guard(id);
        let rate = self.tick_rate;
        if let Some(p) = self.players.get_mut(&id) {
            let w = &mut p.combat.weapons;
            let slot = guard.slot as usize;
            if w.slot == guard.slot {
                w.charge = 0;
            }
            if let Some(cd) = w.alt_cooldown.get_mut(slot) {
                *cd = secs_to_ticks(t.combat.bulwark.broken_cooldown, rate, 1);
            }
        }
        true
    }

    /// A projectile is about to hit player `id` behind a raised slab (`guard.rs` asks):
    /// `Some(keep)` if the slab deals with it.
    pub(crate) fn bulwark_meets_projectile(
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
    pub(crate) fn step_bulwark(&mut self, _t: &Tuning) {}
}
