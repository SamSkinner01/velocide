//! Tally (Precision, set up; `docs/rework.md` §1): hold ALT for up to `tally.hold_time`
//! (`AbilityStyle::Channel`); every input it is held, every hostile the crosshair passes over —
//! within `tally.range`, in line of sight, as the caster saw it (lag compensation) and with
//! `tally.assist_degrees` of assist around the crosshair — is tallied, up to
//! `tally.max_targets` (PvP 3), remembering the part it passed over (a weak point / head wins
//! over the body). The release (or the limit) shoots each tallied target still in line of
//! sight: one ray from the eye at that part, `tally.damage` each (`Attack::Tally`, a hitscan
//! hit: weak points and heads count). Released with nothing tallied: no shot, and only
//! `tally.empty_cooldown` of the cooldown (server-side: the caster's client re-bases).
//!
//! The server keeps the tally; the caster sees it as `PlayerCombat::tallied` (their own state:
//! the marks on their screen). `SimEvent::TallyFired` → `Fx::TallyFired` draws the volley.

use std::collections::BTreeMap;

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::super::combat::Owner;
use super::super::hitscan::ShotSpec;
use super::super::{HitPads, HitPart, HitTarget, HitboxHistory, PlayerId, SimEvent, SimWorld};
use super::{AbilityCast, AbilityPhase};
use crate::input::PlayerInput;
use crate::loadout::Ability;
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// Most marks a tally holds (the owner's `PlayerCombat::tallied`).
pub const MAX_TALLY: usize = 4;

/// One tallied target: what the crosshair passed over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub target: HitTarget,
    pub part: HitPart,
}

/// A tally being held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tally {
    pub marks: Vec<Mark>,
}

/// Tallies being held, per player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tallies {
    pub held: BTreeMap<PlayerId, Tally>,
}

impl Tallies {
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.held.remove(&id);
    }
}

/// The replicated id of a target (players as `NetId::player`).
fn net_id(target: HitTarget) -> NetId {
    match target {
        HitTarget::Player(p) => NetId::player(p),
        HitTarget::Entity(id) => id,
    }
}

impl SimWorld {
    /// The press starts a tally, the release fires it.
    pub(crate) fn cast_tally(&mut self, c: &AbilityCast) {
        match c.phase {
            AbilityPhase::Use => {
                self.abilities.tally.held.insert(c.id, Tally::default());
                self.write_tally(c.id);
                // The press tallies what is under the crosshair already.
                self.tally_sweep(c.t, c.history, c.id, c.input);
            }
            AbilityPhase::Release | AbilityPhase::Recast => self.fire_tally(c),
        }
    }

    /// Every input of a player holding a Tally: what the crosshair passes over is tallied.
    pub(crate) fn tally_sweep(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) {
        let Some(tally) = self.abilities.tally.held.get(&id) else {
            return;
        };
        let k = &t.combat.tally;
        let max = (k.max_targets as usize).min(MAX_TALLY);
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let me = Owner::Player(id);
        let view = self.shot_view(t, history, id, input);
        let world_t = self
            .collision
            .world
            .cast_ray(eye, aim, k.range, MASK_SHOTS)
            .map_or(k.range, |h| h.distance);
        let assist = (k.assist_degrees * 0.5).to_radians().tan();
        let mut marks = tally.marks.clone();
        let mut passed: Vec<(f32, Mark)> = view
            .hitboxes
            .iter()
            .filter(|h| self.can_hit(me, h.target))
            .filter_map(|h| {
                // The assist cone: the shapes grow with their distance.
                let d = h.center()?.distance(eye);
                let base = self.hit_pads(t, me, h.target);
                let pads = HitPads {
                    body: base.body + d * assist,
                    weak: base.weak + d * assist,
                };
                let (dist, part) = h.ray_hit_padded(eye, aim, k.range, pads)?;
                // In sight: nothing of the level between the eye and the target.
                let aim_at = h.part_center(part).unwrap_or(eye + aim * dist);
                let blocked = dist > world_t + 0.5
                    || self.collision.world.line_blocked(eye, aim_at, MASK_SHOTS);
                (!blocked).then_some((
                    dist,
                    Mark {
                        target: h.target,
                        part,
                    },
                ))
            })
            .collect();
        passed.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, mark) in passed {
            match marks.iter().position(|m| m.target == mark.target) {
                // A weak point / head passed over later wins over the body.
                Some(i) => {
                    if marks[i].part == HitPart::Body && mark.part != HitPart::Body {
                        marks[i].part = mark.part;
                    }
                }
                None if marks.len() < max => marks.push(mark),
                None => {}
            }
        }
        if let Some(tally) = self.abilities.tally.held.get_mut(&id) {
            tally.marks = marks;
        }
        self.write_tally(id);
    }

    /// The release: a ray at each tallied target still in sight (or, with nothing tallied, a
    /// quarter of the cooldown).
    fn fire_tally(&mut self, c: &AbilityCast) {
        let t = c.t;
        let Some(tally) = self.abilities.tally.held.remove(&c.id) else {
            return;
        };
        self.write_tally(c.id);
        let k = &t.combat.tally;
        if tally.marks.is_empty() {
            let secs = Ability::Tally.cooldown(&t.combat) * k.empty_cooldown;
            if let Some(p) = self.players.get_mut(&c.id) {
                let slot = p.combat.weapons.slot;
                p.combat
                    .weapons
                    .set_alt_cooldown(slot, secs, self.tick_rate);
            }
            return;
        }
        let Some(p) = self.players.get(&c.id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let me = Owner::Player(c.id);
        let attack = c.ability.attack();
        let view = self.shot_view(t, c.history, c.id, c.input);
        let world = &self.collision.world;
        let shots: Vec<ShotSpec> = tally
            .marks
            .iter()
            .filter(|m| self.can_hit(me, m.target))
            .filter_map(|m| {
                let h = view.hitboxes.iter().find(|h| h.target == m.target)?;
                let at = h.part_center(m.part).or_else(|| h.center())?;
                if eye.distance(at) > k.range + 2.0 || world.line_blocked(eye, at, MASK_SHOTS) {
                    return None;
                }
                let dir = (at - eye).normalize_or(Vec3::NEG_Z);
                Some(ShotSpec::new(eye, dir, k.range + 2.0, k.damage, attack))
            })
            .collect();
        let mut ends = Vec::with_capacity(shots.len());
        for shot in &shots {
            let r = self.trace_shot(t, me, &view.hitboxes, Some(view.time), shot, 0);
            ends.push(r.end);
        }
        self.events.push(SimEvent::TallyFired {
            shooter: c.id,
            origin: eye,
            ends,
        });
    }

    /// The tally's marks into its owner's own state (`PlayerCombat::tallied`).
    fn write_tally(&mut self, id: PlayerId) {
        let marks: Vec<NetId> = self
            .abilities
            .tally
            .held
            .get(&id)
            .map(|t| t.marks.iter().map(|m| net_id(m.target)).collect())
            .unwrap_or_default();
        if let Some(p) = self.players.get_mut(&id) {
            let mut tallied = [None; MAX_TALLY];
            for (slot, mark) in tallied.iter_mut().zip(marks) {
                *slot = Some(mark);
            }
            p.combat.tallied = tallied;
        }
    }

    /// Per tick: a tally whose owner died, left or let go without a release (switching away
    /// releases it) is dropped.
    pub(crate) fn step_tally(&mut self, _t: &Tuning) {
        let ids: Vec<PlayerId> = self.abilities.tally.held.keys().copied().collect();
        for id in ids {
            let holding = self.players.get(&id).is_some_and(|p| {
                let w = &p.combat.weapons;
                !p.combat.dead
                    && w.charge > 0
                    && w.held().is_some()
                    && w.ability() == Ability::Tally
            });
            if !holding {
                self.abilities.tally.held.remove(&id);
                self.write_tally(id);
            }
        }
    }
}
