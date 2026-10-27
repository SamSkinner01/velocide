//! Area attacks without projectiles: the Bellows **Gust** (cone knockback that also turns
//! hostile projectiles around), the Arclash **Discharge** (radial nova around the shooter),
//! the Tumbler **Breach** (point-blank explosion) and the **Kiln**'s ember spray (a short
//! cone). All resolve instantly on the server from the input that fired them, against the
//! hitboxes the shooter saw (lag compensation), with hit assist.

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner};
use super::projectiles::Explosive;
use super::{HitPart, HitTarget, Hitbox, HitboxHistory, PlayerId, SimEvent, SimWorld};
use crate::input::PlayerInput;
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// A target inside a cone: `h` along the aim within `range` (ray), or its nearest (padded)
/// surface within `range` with its centre inside `half_cone` radians of the aim. Returns the
/// distance, the part and the point to apply the hit at. (Also the melee arc, `melee.rs`.)
pub(crate) fn in_cone(
    h: &Hitbox,
    pads: super::HitPads,
    eye: Vec3,
    aim: Vec3,
    range: f32,
    half_cone: f32,
) -> Option<(f32, HitPart, Vec3)> {
    if let Some((d, part)) = h.ray_hit_padded(eye, aim, range, pads) {
        return Some((d, part, eye + aim * d));
    }
    let center = h.center()?;
    let (d, part) = h.distance_to_point_padded(eye, pads)?;
    let to = center - eye;
    (d <= range && to.normalize_or_zero().angle_between(aim) <= half_cone).then_some((
        d.max(0.0),
        part,
        center,
    ))
}

impl SimWorld {
    /// Bellows Gust: every hostile in the cone (line of sight) takes `gust_damage` and is
    /// knocked back along the aim (+ up); every hostile projectile in the cone is deflected
    /// (it becomes the shooter's and flies along the aim, faster and stronger — no heal, no
    /// parry reward; [`SimEvent::Deflected`]); the shooter recoils.
    pub(crate) fn gust(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let b = &t.combat.bellows;
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let half_cone = (b.gust_cone_degrees * 0.5).to_radians();
        let me = Owner::Player(id);
        let view = self.shot_view(t, history, id, input);
        let world = &self.collision.world;

        let mut hits = Vec::new();
        for h in &view.hitboxes {
            if !self.can_hit(me, h.target) {
                continue;
            }
            let pads = self.hit_pads(t, me, h.target);
            // A blast of air: no weak-point bonus.
            let Some((_, _, point)) = in_cone(h, pads, eye, aim, b.gust_range, half_cone) else {
                continue;
            };
            if world.line_blocked(eye, point, MASK_SHOTS) {
                continue;
            }
            let kb = aim * b.gust_knockback + Vec3::Y * b.gust_up;
            let info = DamageInfo::new(b.gust_damage, me, Attack::Gust)
                .at(point, aim)
                .knockback(kb);
            hits.push((h.target, info));
        }

        // Hostile projectiles: where they are now, or where the shooter saw them.
        let view_time = view.time;
        let in_zone = |pos: Vec3| {
            let to = pos - eye;
            let d = to.length();
            d <= b.gust_range
                && (d < 0.5 || to.normalize_or(aim).angle_between(aim) <= half_cone)
                && !world.line_blocked(eye, pos, MASK_SHOTS)
        };
        let deflect: Vec<NetId> = self
            .projectiles
            .values()
            .filter(|proj| proj.stuck.is_none())
            .filter(|proj| proj.pending.is_none_or(|pd| pd.victim == id))
            .filter(|proj| match proj.owner {
                Owner::Player(o) => o != id && self.rules.friendly_fire,
                Owner::Enemy(_) | Owner::World => true,
            })
            .filter(|proj| proj.trail.at(view_time).is_some_and(in_zone) || in_zone(proj.position))
            .map(|proj| proj.id)
            .collect();

        for (target, info) in hits {
            self.apply_damage(t, target, info);
        }
        for pid in deflect {
            let kind = self.projectiles.get(&pid).map(|p| p.kind);
            let Some(position) = self.redirect_projectile(
                t,
                id,
                pid,
                b.deflect_speed_multiplier,
                b.deflect_damage_multiplier,
                false,
            ) else {
                continue;
            };
            if let Some(proj) = self.projectiles.get_mut(&pid) {
                // Kills with it score (and show in the feed) as the Gust.
                proj.attack = Attack::Gust;
            }
            if let Some(kind) = kind {
                self.events.push(SimEvent::Deflected {
                    player: id,
                    id: pid,
                    kind,
                    position,
                });
            }
        }
        // Gust-jump.
        self.apply_impulse(id, -aim * b.gust_recoil);
        self.push_fired(id, Attack::Gust, eye);
    }

    /// Arclash Discharge: a nova centred on the shooter, radius / damage / outward knockback
    /// growing with `charge` (0..=1). Hits every hostile in the radius with line of sight from
    /// the centre; never the shooter. Emits [`SimEvent::Discharge`].
    pub(crate) fn discharge(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        charge: f32,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let a = &t.combat.arclash;
        let k = charge.clamp(0.0, 1.0);
        let lerp = |lo: f32, hi: f32| lo + (hi - lo) * k;
        let radius = lerp(a.discharge_min_radius, a.discharge_max_radius);
        let damage = lerp(a.discharge_min_damage, a.discharge_max_damage);
        let knockback = lerp(a.discharge_min_knockback, a.discharge_max_knockback);
        let (center, _, _) = p.movement.capsule(&t.movement);
        let me = Owner::Player(id);
        let view = self.shot_view(t, history, id, input);
        let world = &self.collision.world;
        let mut hits = Vec::new();
        for h in &view.hitboxes {
            if !self.can_hit(me, h.target) {
                continue;
            }
            let pads = self.hit_pads(t, me, h.target);
            let Some((d, _)) = h.distance_to_point_padded(center, pads) else {
                continue;
            };
            if d > radius {
                continue;
            }
            let aim = h.center().unwrap_or(center);
            let head = h.part_center(HitPart::Head);
            if world.line_blocked(center, aim, MASK_SHOTS)
                && head.is_none_or(|hp| world.line_blocked(center, hp, MASK_SHOTS))
            {
                continue;
            }
            let to = aim - center;
            let out = Vec3::new(to.x, 0.0, to.z).normalize_or(Vec3::X);
            let dir = to.normalize_or(out);
            let kb = (out + Vec3::Y * 0.35).normalize_or(Vec3::Y) * knockback;
            let info = DamageInfo::new(damage, me, Attack::Discharge)
                .at(aim, dir)
                .part(HitPart::Body)
                .knockback(kb);
            hits.push((h.target, info));
        }
        self.events.push(SimEvent::Discharge {
            player: id,
            position: center,
            radius,
            charge: k,
        });
        for (target, info) in hits {
            self.apply_damage(t, target, info);
        }
    }

    /// Kiln spray: `kiln.damage` and `kiln.burn_stacks` Burn stacks to every hostile whose
    /// (padded) shape is within `kiln.range` inside the `kiln.cone_degrees` cone along the aim
    /// (line of sight from the eye). Embers don't ricochet and have no weak-point bonus. Emits
    /// a `Shot` (no ray ends) for the remote spray.
    pub(crate) fn kiln_spray(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let k = &t.combat.kiln;
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let half_cone = (k.cone_degrees * 0.5).to_radians();
        let me = Owner::Player(id);
        let view = self.shot_view(t, history, id, input);
        let world = &self.collision.world;
        let mut hits = Vec::new();
        for h in &view.hitboxes {
            if !self.can_hit(me, h.target) {
                continue;
            }
            let pads = self.hit_pads(t, me, h.target);
            let Some((_, _, point)) = in_cone(h, pads, eye, aim, k.range, half_cone) else {
                continue;
            };
            if world.line_blocked(eye, point, MASK_SHOTS) {
                continue;
            }
            let info = DamageInfo::new(k.damage, me, Attack::Kiln)
                .at(point, aim)
                .part(HitPart::Body)
                .flags(DamageFlags::HITSCAN);
            hits.push((h.target, info));
        }
        for (target, info) in hits {
            self.apply_damage(t, target, info);
            self.apply_burn(t, me, Attack::Kiln, target, k.burn_stacks);
        }
        self.push_fired(id, Attack::Kiln, eye);
    }

    /// Tumbler Breach: explodes at the first surface or hostile within `breach_reach` along
    /// the aim (pulled back `breach_pullback`), or that far out in the air. Normal explosion
    /// rules, self damage included (the blast-jump).
    pub(crate) fn breach(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) {
        let Some(p) = self.players.get(&id) else {
            return;
        };
        let tb = &t.combat.tumbler;
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let me = Owner::Player(id);
        let world_t = self
            .collision
            .world
            .cast_ray(eye, aim, tb.breach_reach, MASK_SHOTS)
            .map_or(tb.breach_reach, |h| h.distance);
        let view = self.shot_view(t, history, id, input);
        let hit = self.first_target_on_ray(t, me, &view.hitboxes, eye, aim, world_t, None);
        let d = hit.map_or(world_t, |h| h.0);
        let center = eye + aim * (d - tb.breach_pullback).max(0.0);
        let direct: Option<HitTarget> = hit.map(|h| h.1);
        self.explode(
            t,
            center,
            Explosive {
                radius: tb.breach_radius,
                damage: tb.breach_damage,
                knockback: tb.breach_knockback,
            },
            me,
            Attack::Breach,
            DamageFlags::NONE,
            None,
            direct,
            me,
        );
    }
}
