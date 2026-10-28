//! Melee weapons (`WeaponClass::Melee`: the Trimmer and the Tamper) and their abilities
//! (Lunge, Whirl, Hurl; the Riposte is `abilities/riposte.rs`), docs/arsenal.md "Melee".
//!
//! **Swings** (the primaries, the Riposte's counter) hit the hostiles inside an arc in front of
//! the eye
//! ([`in_arc`]: a (padded) shape whose surface is within `reach` of the eye and that the
//! `arc_degrees` cone around the aim touches — so a low Mould right at your feet is in it even
//! while you look ahead), as the swinger saw them (lag compensation, like hitscan), with hit
//! assist and line of sight from the eye. Melee ignores weak points
//! (no headshots): a blade or a hammer is aimed at the whole body, and the arc already
//! forgives where a ray would not. The Trimmer hits up to `max_targets`, nearest first, and
//! its swing deflects the parryable hostile projectiles in its arc like a punch parry (same
//! reward, `parry.rs`); the Tamper hits everything in its arc, hard.
//!
//! The abilities hit for a multiple of the carrier's primary damage (`damage_scale`):
//!
//! * **Lunge**: an impulse along the aim (mostly horizontal, lifted off the ground so it
//!   glides); for `lunge.window` the first
//!   hostile the lunging body comes within `lunge.reach` of is struck ([`SimEvent::LungeStruck`]).
//!   It tests the current hit shapes each tick (like projectiles: the body is really there).
//! * **Whirl**: everything within `whirl.radius` of the body (lag-compensated), knocked
//!   outward.
//! * **Hurl**: a spinning blade / hammer-head projectile (`ProjectileKind::Hurl`) that comes
//!   back (`abilities/hurl.rs`).
//!
//! A live thorn inside a swing's arc (or a Whirl's reach) is batted on at the hostile its relay
//! picks, as a ray of the swing's damage (`thorn.rs`).
//!
//! Every swing, Whirl, Riposte counter and the start of a Lunge is a [`SimEvent::Swing`]
//! (`Fx::Swing` for the other clients and the replay). All melee damage carries
//! [`DamageFlags::MELEE`].

use std::collections::BTreeMap;

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::projectiles::ProjectileKind;
use super::{HitPads, HitPart, HitTarget, Hitbox, HitboxHistory, PlayerId, SimEvent, SimWorld};
use crate::input::PlayerInput;
use crate::loadout::{Ability, WeaponKind};
use crate::tuning::{CombatTuning, Tuning};

/// A Lunge in progress: until `until` the first hostile the lunging body reaches takes
/// `damage`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LungeState {
    pub until: u32,
    pub damage: f32,
    pub knockback: f32,
    /// How close (beyond the hit shapes) counts as reached.
    pub reach: f32,
}

/// Melee state the server keeps between ticks ([`SimWorld::melee`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeleeWorld {
    pub lunges: BTreeMap<PlayerId, LungeState>,
}

impl MeleeWorld {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Forgets player `id`'s lunge (they left).
    pub(crate) fn forget(&mut self, id: PlayerId) {
        self.lunges.remove(&id);
    }
}

impl WeaponKind {
    /// A melee weapon's swing: (reach from the eye, arc in degrees). Other weapons (they can't
    /// carry melee abilities) get the Trimmer's.
    pub fn melee_arc(self, t: &CombatTuning) -> (f32, f32) {
        match self {
            WeaponKind::Tamper => (t.tamper.reach, t.tamper.arc_degrees),
            _ => (t.trimmer.reach, t.trimmer.arc_degrees),
        }
    }
}

/// Is `h` inside a swing from `eye` along `aim` (`reach`, half aperture `half_arc` radians)?
/// Along the aim within reach; or one of its shapes (inflated by `pads`) has its surface within
/// reach and the cone touches it: the angle from the aim to the point of the shape's axis
/// nearest the eye, less the angle the shape's radius covers from there. Returns the surface
/// distance and the point struck.
pub fn in_arc(
    h: &Hitbox,
    pads: HitPads,
    eye: Vec3,
    aim: Vec3,
    reach: f32,
    half_arc: f32,
) -> Option<(f32, Vec3)> {
    if let Some((d, _)) = h.ray_hit_padded(eye, aim, reach, pads) {
        return Some((d, eye + aim * d));
    }
    h.shapes
        .iter()
        .filter_map(|s| {
            let radius = s.radius + pads.for_part(s.part).max(0.0);
            let ab = s.b - s.a;
            let along = if ab.length_squared() < 1e-12 {
                0.0
            } else {
                ((eye - s.a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            };
            let nearest = s.a + ab * along;
            let to = nearest - eye;
            let d = to.length();
            let surface = (d - radius).max(0.0);
            if surface > reach {
                return None;
            }
            let covers = if d > radius {
                (radius / d).asin()
            } else {
                std::f32::consts::PI
            };
            let angle = to.normalize_or(aim).angle_between(aim);
            (angle - covers <= half_arc).then_some((surface, nearest))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// Melee damage by player `id`: `amount` as `attack`, struck at `point` along `dir` (on the
/// body: no weak points), `MELEE`.
pub(crate) fn melee_damage(
    id: PlayerId,
    (amount, attack): (f32, Attack),
    point: Vec3,
    dir: Vec3,
    knockback: Vec3,
) -> DamageInfo {
    DamageInfo::new(amount, Owner::Player(id), attack)
        .at(point, dir)
        .part(HitPart::Body)
        .knockback(knockback)
        .flags(DamageFlags::MELEE)
}

/// Knockback of a swing along the aim: mostly horizontal, a little up.
pub(crate) fn swing_knockback(aim: Vec3, speed: f32) -> Vec3 {
    let flat = Vec3::new(aim.x, 0.0, aim.z).normalize_or(aim);
    (flat + Vec3::Y * 0.3).normalize_or(Vec3::Y) * speed
}

impl SimWorld {
    /// The hostiles inside player `id`'s arc as they saw them, nearest first: (distance,
    /// target, where it is struck).
    pub(crate) fn arc_targets(
        &self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        (reach, arc_degrees): (f32, f32),
    ) -> Vec<(f32, HitTarget, Vec3)> {
        let Some(p) = self.players.get(&id) else {
            return Vec::new();
        };
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let half_arc = (arc_degrees * 0.5).to_radians();
        let me = Owner::Player(id);
        let view = self.shot_view(t, history, id, input);
        let world = &self.collision.world;
        let mut hits: Vec<(f32, HitTarget, Vec3)> = view
            .hitboxes
            .iter()
            .filter(|h| self.can_hit(me, h.target))
            .filter_map(|h| {
                let pads = self.hit_pads(t, me, h.target);
                let (d, point) = in_arc(h, pads, eye, aim, reach, half_arc)?;
                (!world.line_blocked(eye, point, MASK_SHOTS)).then_some((d, h.target, point))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits
    }

    /// A melee weapon's primary swing.
    pub(crate) fn melee_swing(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        weapon: WeaponKind,
    ) {
        let c = &t.combat;
        let aim = input.look_dir();
        let arc = weapon.melee_arc(c);
        let mut targets = self.arc_targets(t, history, id, input, arc);
        let (damage, knockback) = match weapon {
            WeaponKind::Tamper => (c.tamper.damage, c.tamper.knockback),
            _ => {
                targets.truncate(c.trimmer.max_targets.max(1) as usize);
                (c.trimmer.damage, c.trimmer.knockback)
            }
        };
        // The shears catch what flies into them (before the cut lands: a deflected ember bomb
        // can't hurt the swinger any more).
        if weapon == WeaponKind::Trimmer {
            self.deflect_in_arc(t, id, arc.0, arc.1);
        }
        let attack = weapon.primary();
        let kb = swing_knockback(aim, knockback);
        let hits = targets.len().min(u8::MAX as usize) as u8;
        for (_, target, point) in targets {
            let info = melee_damage(id, (damage, attack), point, aim, kb);
            self.apply_damage(t, target, info);
        }
        // A thorn in the arc is batted on at the swing's damage (thorn.rs).
        self.relay_cone(t, id, input, arc, damage, attack);
        self.events.push(SimEvent::Swing {
            player: id,
            attack,
            hits,
        });
    }

    /// A melee ability (`t` is already scaled for the carrying weapon, `weapons::ability_tuning`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn melee_ability(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        weapon: WeaponKind,
        ability: Ability,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let eye = p.movement.eye_position(&t.movement);
        let (center, _, _) = p.movement.capsule(&t.movement);
        let aim = input.look_dir();
        let c = &t.combat;
        let primary = weapon.primary_damage(c);
        let attack = ability.attack();
        let hits = match ability {
            Ability::Lunge => {
                let l = &c.lunge;
                let dir = Vec3::new(aim.x, aim.y * l.vertical, aim.z).normalize_or(aim);
                self.apply_impulse(id, dir * l.speed);
                // Off the ground (a hop, not stacked on a jump): ground friction would stop it.
                if let Some(p) = self.players.get_mut(&id) {
                    let v = &mut p.movement.velocity;
                    v.y = v.y.max(l.lift);
                }
                let until = self.tick + secs_to_ticks(l.window, self.tick_rate, 1) as u32;
                self.melee.lunges.insert(
                    id,
                    LungeState {
                        until,
                        damage: primary * l.damage_scale,
                        knockback: l.knockback,
                        reach: l.reach,
                    },
                );
                0
            }
            Ability::Whirl => {
                let w = &c.whirl;
                let me = Owner::Player(id);
                let view = self.shot_view(t, history, id, input);
                let world = &self.collision.world;
                let mut struck = Vec::new();
                for h in &view.hitboxes {
                    if !self.can_hit(me, h.target) {
                        continue;
                    }
                    let pads = self.hit_pads(t, me, h.target);
                    let Some((d, _)) = h.distance_to_point_padded(center, pads) else {
                        continue;
                    };
                    let at = h.center().unwrap_or(center);
                    if d > w.radius || world.line_blocked(center, at, MASK_SHOTS) {
                        continue;
                    }
                    let to = at - center;
                    let out = Vec3::new(to.x, 0.0, to.z).normalize_or(Vec3::X);
                    let kb = (out + Vec3::Y * w.up).normalize_or(Vec3::Y) * w.knockback;
                    struck.push((h.target, at, to.normalize_or(out), kb));
                }
                let n = struck.len();
                for (target, at, dir, kb) in struck {
                    let info = melee_damage(id, (primary * w.damage_scale, attack), at, dir, kb);
                    self.apply_damage(t, target, info);
                }
                // The thorns around are batted on (thorn.rs).
                let damage = primary * w.damage_scale;
                self.relay_nova(t, id, input, center, w.radius, damage, attack);
                n
            }
            Ability::Hurl => {
                let h = &c.hurl;
                let start = self.safe_launch_point(eye, eye + aim * 0.5);
                let owner = Owner::Player(id);
                let spawned =
                    self.spawn_projectile(t, ProjectileKind::Hurl, owner, start, aim * h.speed);
                if let Some(proj) = spawned.and_then(|pid| self.projectiles.get_mut(&pid)) {
                    proj.damage = primary * h.damage_scale;
                    // Replicated as the projectile's `state`: a blade or a hammer-head.
                    proj.bounces_left = weapon.id() as u32;
                }
                self.push_fired(id, attack, eye);
                return;
            }
            // Other classes' abilities never reach here (`weapons.rs`).
            _ => return,
        };
        self.events.push(SimEvent::Swing {
            player: id,
            attack,
            hits: hits.min(u8::MAX as usize) as u8,
        });
    }

    /// Per tick (pipeline step 4): every Lunge strikes the first hostile its body reaches
    /// (current hit shapes, hit assist, line of sight), then ends; unspent ones run out.
    pub(crate) fn step_melee(&mut self, t: &Tuning) {
        let now = self.tick;
        self.melee.lunges.retain(|_, l| now < l.until);
        if self.melee.lunges.is_empty() {
            return;
        }
        let boxes = self.current_hitboxes(t);
        let lunges: Vec<(PlayerId, LungeState)> =
            self.melee.lunges.iter().map(|(id, l)| (*id, *l)).collect();
        for (id, lunge) in lunges {
            let Some(p) = self.players.get(&id).filter(|p| !p.combat.dead) else {
                self.melee.lunges.remove(&id);
                continue;
            };
            let (center, half_height, radius) = p.movement.capsule(&t.movement);
            let along = p.movement.velocity;
            // The body as three points along its axis.
            let samples = [-0.8, 0.0, 0.8].map(|f| center + Vec3::Y * (half_height * f));
            let me = Owner::Player(id);
            let world = &self.collision.world;
            let first = boxes
                .iter()
                .filter(|h| self.can_hit(me, h.target))
                .filter_map(|h| {
                    let pads = self.hit_pads(t, me, h.target);
                    let d = samples
                        .iter()
                        .filter_map(|s| h.distance_to_point_padded(*s, pads))
                        .map(|(d, _)| d)
                        .min_by(f32::total_cmp)?;
                    let at = h.center()?;
                    (d - radius <= lunge.reach && !world.line_blocked(center, at, MASK_SHOTS))
                        .then_some((d, h.target, at))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((_, target, at)) = first else {
                continue;
            };
            self.melee.lunges.remove(&id);
            let dir = (at - center).normalize_or(along.normalize_or(Vec3::NEG_Z));
            let kb = swing_knockback(dir, lunge.knockback);
            let info = melee_damage(id, (lunge.damage, Attack::Lunge), at, dir, kb);
            self.apply_damage(t, target, info);
            self.events.push(SimEvent::LungeStruck {
                player: id,
                target,
                position: at,
            });
        }
    }
}
