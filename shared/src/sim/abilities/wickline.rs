//! Wickline (Precision, move; `docs/rework.md` §1): a burning wick-thread, hitscan to
//! `wickline.range`, tested against what the caster saw (lag compensation, hit assist, like
//! any ray).
//!
//! * **Anchored in the level** (it reached a surface first): one impulse along the line of
//!   `wickline.zip_speed` (what of the caster's velocity ran against the line is dropped
//!   first, so a fall doesn't eat the zip) with at least `zip_lift` of it upwards — a
//!   grappling zip.
//! * **On a hostile**: a light one (an enemy whose base health is below
//!   `wickline.heavy_health`, any PvP player) is yanked towards the caster (`yank_speed`;
//!   enemies × their knockback scale through the damage pipeline, PvP players ×
//!   `player_yank_scale`) and takes `wickline.damage` (`Attack::Wickline`, a hitscan hit); a
//!   heavy one yanks the *caster* to it (the zip, anchored where the line struck).
//! * **A miss** (nothing within range): nothing happens and only `miss_cooldown` of the
//!   cooldown is served (a server-side change of the caster's weapon state: their client
//!   re-bases).
//!
//! The impulses are server-side like the Lunge's: the caster's client reconciles. The line is
//! `SimEvent::Wickline` → `Fx::Wickline` (everyone, the caster too) for `wickline.line_time`.

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::super::combat::{DamageInfo, Owner};
use super::super::{HitTarget, PlayerId, SimEvent, SimWorld};
use super::AbilityCast;
use crate::tuning::Tuning;

/// Wickline state between ticks (none: the line is an effect).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wicklines {}

impl Wicklines {
    pub(crate) fn forget(&mut self, _id: PlayerId) {}
}

impl SimWorld {
    /// A Wickline cast: the thread, then the zip or the yank (or a miss).
    pub(crate) fn cast_wickline(&mut self, c: &AbilityCast) {
        let t = c.t;
        let w = &t.combat.wickline;
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let aim = c.input.look_dir();
        let me = Owner::Player(c.id);
        let view = self.shot_view(t, c.history, c.id, c.input);
        let world = self
            .collision
            .world
            .cast_ray(eye, aim, w.range, MASK_SHOTS)
            .map(|h| h.distance);
        let world_t = world.unwrap_or(w.range);
        let target = self.first_target_on_ray(t, me, &view.hitboxes, eye, aim, world_t, None);
        let (end, anchored) = match target {
            Some((d, target, part)) => {
                let at = eye + aim * d;
                if self.wickline_heavy(t, target) {
                    // A heavy one holds: the thread pulls the caster to it.
                    self.wickline_zip(t, c.id, at);
                } else {
                    let to_caster = -aim;
                    let flat = Vec3::new(to_caster.x, 0.0, to_caster.z).normalize_or(to_caster);
                    let mut speed = w.yank_speed;
                    if matches!(target, HitTarget::Player(_)) {
                        speed *= w.player_yank_scale;
                    }
                    let kb = (flat + Vec3::Y * 0.25).normalize_or(Vec3::Y) * speed;
                    let info = DamageInfo::new(w.damage, me, c.ability.attack())
                        .at(at, aim)
                        .part(part)
                        .knockback(kb)
                        .flags(Self::hitscan_flags(c.ability.attack()));
                    self.apply_damage(t, target, info);
                }
                (at, false)
            }
            None => match world {
                Some(d) => {
                    let at = eye + aim * d;
                    self.wickline_zip(t, c.id, at);
                    (at, true)
                }
                None => {
                    // A miss: half the cooldown (the caster's client re-bases).
                    let secs = c.ability.cooldown(&t.combat) * w.miss_cooldown;
                    if let Some(p) = self.players.get_mut(&c.id) {
                        let slot = p.combat.weapons.slot;
                        p.combat
                            .weapons
                            .set_alt_cooldown(slot, secs, self.tick_rate);
                    }
                    (eye + aim * w.range, false)
                }
            },
        };
        self.events.push(SimEvent::Wickline {
            player: c.id,
            from: eye,
            to: end,
            anchored,
        });
    }

    /// Whether the thread holds `target` (it pulls the caster instead): a heavy enemy.
    fn wickline_heavy(&self, t: &Tuning, target: HitTarget) -> bool {
        match target {
            HitTarget::Entity(id) => self
                .enemies
                .get(&id)
                .is_some_and(|e| e.def.health >= t.combat.wickline.heavy_health),
            HitTarget::Player(_) => false,
        }
    }

    /// The zip: player `id` is pulled along the line towards `anchor`.
    fn wickline_zip(&mut self, t: &Tuning, id: PlayerId, anchor: Vec3) {
        let w = &t.combat.wickline;
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let dir = (anchor - eye).normalize_or(Vec3::Y);
        let v = &mut p.movement.velocity;
        // What runs against the line goes (a fall, a run the other way), then the impulse.
        let along = v.dot(dir);
        if along < 0.0 {
            *v -= dir * along;
        }
        *v += dir * w.zip_speed;
        v.y = v.y.max(w.zip_lift);
    }

    /// Per tick.
    pub(crate) fn step_wickline(&mut self, _t: &Tuning) {}
}
