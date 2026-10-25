//! Hitscan: lag-compensated rays, piercing beams and prism ricochets.
//!
//! A shot from an input is tested against the hitboxes **as the shooter saw them**: the
//! server's [`HitboxHistory`] rewinds every player and enemy hitbox to the input's
//! `view_tick + view_frac/256` (clamped to `net.lag_comp_max_ms` and to the shooter's
//! RTT-based bound, [`SimWorld::max_rewind_ticks`]), and live prisms are rewound
//! to the same time through their own position trails. Level geometry (`MASK_SHOTS`) is
//! static, so a wall between the shooter and the (rewound) target always blocks. The shooter
//! never hits itself; targets that may not be hurt (co-op teammates, dead players, dying
//! enemies) are transparent.
//!
//! **Ricochet**: a shot with `ricochet` enabled that passes within `prism.ricochet_radius` of a
//! live prism before anything else stops it shatters the prism and re-fires from it, ×1.5
//! damage per bounce, at the next live prism within `prism.chain_range` (chain), else at the
//! nearest hostile within `prism.target_range` that is in line of sight (weak point / head if
//! visible, else the body). Bounce legs are traced against current (not rewound) hitboxes
//! since they auto-aim at current positions.

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner};
use super::{HitPart, HitTarget, Hitbox, HitboxHistory, PlayerId, ShotEvent, SimEvent, SimWorld};
use crate::input::PlayerInput;
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// One ray to trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotSpec {
    pub origin: Vec3,
    pub dir: Vec3,
    pub range: f32,
    pub damage: f32,
    pub attack: Attack,
    /// Extra targets the ray passes through after the first.
    pub pierce: u32,
    /// Velocity change applied to each target along the ray.
    pub knockback: f32,
    /// Can ricochet off prisms.
    pub ricochet: bool,
}

impl ShotSpec {
    pub fn new(origin: Vec3, dir: Vec3, range: f32, damage: f32, attack: Attack) -> Self {
        Self {
            origin,
            dir: dir.normalize_or(Vec3::NEG_Z),
            range,
            damage,
            attack,
            pierce: 0,
            knockback: 0.0,
            ricochet: false,
        }
    }

    pub fn pierce(mut self, extra_targets: u32) -> Self {
        self.pierce = extra_targets;
        self
    }

    pub fn knockback(mut self, knockback: f32) -> Self {
        self.knockback = knockback;
        self
    }

    pub fn ricochet(mut self, on: bool) -> Self {
        self.ricochet = on;
        self
    }
}

/// The world as a shooter saw it: lag-compensated hitboxes and the (clamped) view time.
pub struct ShotView {
    /// View time in ticks, clamped to `[now - lag_comp_max_ms, now]`.
    pub time: f64,
    pub hitboxes: Vec<Hitbox>,
}

/// Result of tracing one shot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    /// End of the first leg (world hit, last pierced target, range end, or the first prism).
    pub end: Vec3,
    /// Targets damaged along the whole path (all legs).
    pub hits: u32,
    /// Prism bounce path: every prism hit, then the final end point. Empty without ricochet.
    pub ricochet: Vec<Vec3>,
}

impl SimWorld {
    /// Clamped lag-compensation view time of player `id`'s input: at most
    /// [`SimWorld::max_rewind_ticks`] in the past (the client chooses its view time; the
    /// server bounds it by `net.lag_comp_max_ms` and the player's measured RTT).
    pub fn view_time(&self, t: &Tuning, id: PlayerId, input: &PlayerInput) -> f64 {
        let now = self.tick as f64;
        let max = self.max_rewind_ticks(t, id);
        let v = input.view_time_ticks();
        if v.is_finite() {
            v.clamp(now - max, now)
        } else {
            now
        }
    }

    /// Hitboxes as player `id` saw them when sending `input` (rewound through `history` to
    /// [`SimWorld::view_time`]; current hitboxes without a history or before it has frames).
    pub fn shot_view(
        &self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) -> ShotView {
        let time = self.view_time(t, id, input);
        let max_rewind_ms = (self.max_rewind_ticks(t, id) * 1000.0 / self.tick_rate as f64) as f32;
        let hitboxes = history
            .map(|h| h.rewind_hitboxes(input.view_time_ticks(), max_rewind_ms))
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| self.current_hitboxes(t));
        ShotView { time, hitboxes }
    }

    /// Current hitboxes of everything shootable.
    pub fn current_hitboxes(&self, t: &Tuning) -> Vec<Hitbox> {
        let mut out = Vec::new();
        self.hitboxes(t, &mut out);
        out
    }

    /// Can `shooter`'s attacks damage `target` right now? (Alive and hostile.) Non-hittable
    /// targets are transparent to shots and projectiles.
    pub fn can_hit(&self, shooter: Owner, target: HitTarget) -> bool {
        if shooter.as_target() == Some(target) {
            return false;
        }
        let alive = match target {
            HitTarget::Player(id) => self.players.get(&id).is_some_and(|p| !p.combat.dead),
            HitTarget::Entity(id) => self.enemies.get(&id).is_some_and(|e| e.is_alive()),
        };
        alive && self.is_hostile(shooter, target)
    }

    /// Fires a group of rays from one input (one shot, a fan, a pellet spread) against the
    /// lag-compensated view and emits one [`SimEvent::Shot`] (plus [`SimEvent::Ricochet`]
    /// per bounce path).
    pub(crate) fn fire_hitscan_group(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        attack: Attack,
        shots: &[ShotSpec],
    ) -> Vec<TraceResult> {
        let view = self.shot_view(t, history, id, input);
        let shooter = Owner::Player(id);
        let mut results = Vec::with_capacity(shots.len());
        for shot in shots {
            let r = self.trace_shot(t, shooter, &view.hitboxes, Some(view.time), shot, 0);
            if !r.ricochet.is_empty() {
                self.events.push(SimEvent::Ricochet {
                    shooter,
                    attack,
                    points: r.ricochet.clone(),
                });
            }
            results.push(r);
        }
        let origin = shots.first().map_or(Vec3::ZERO, |s| s.origin);
        self.events.push(SimEvent::Shot(ShotEvent {
            shooter: id,
            attack,
            origin,
            ends: results.iter().map(|r| r.end).collect(),
        }));
        results
    }

    /// Traces one shot (and its ricochet legs) against `hitboxes`; prisms are taken at
    /// `prism_time` (ticks; `None` = current positions). Applies the damage.
    pub fn trace_shot(
        &mut self,
        t: &Tuning,
        shooter: Owner,
        hitboxes: &[Hitbox],
        prism_time: Option<f64>,
        spec: &ShotSpec,
        ricochets: u8,
    ) -> TraceResult {
        let dir = spec.dir.normalize_or(Vec3::NEG_Z);
        let origin = spec.origin;
        let world_t = self
            .collision
            .world
            .cast_ray(origin, dir, spec.range, MASK_SHOTS)
            .map_or(spec.range, |h| h.distance);

        let mut hits: Vec<(f32, HitTarget, HitPart)> = hitboxes
            .iter()
            .filter(|h| self.can_hit(shooter, h.target))
            .filter_map(|h| {
                h.ray_hit(origin, dir, world_t)
                    .map(|(d, p)| (d, h.target, p))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Several shapes of one target can't both count.
        let mut seen = Vec::new();
        hits.retain(|h| {
            if seen.contains(&h.1) {
                false
            } else {
                seen.push(h.1);
                true
            }
        });
        let max_targets = spec.pierce as usize + 1;
        hits.truncate(max_targets);
        let block_t = if hits.len() == max_targets {
            hits.last().map_or(world_t, |h| h.0)
        } else {
            world_t
        };

        let prism = if spec.ricochet {
            self.prism_on_ray(t, origin, dir, block_t, prism_time)
        } else {
            None
        };
        if let Some((_, pt, _)) = prism {
            hits.retain(|h| h.0 < pt);
        }

        let mult = t.combat.prism.damage_multiplier.powi(ricochets as i32);
        let mut flags = Self::hitscan_flags(spec.attack);
        if ricochets > 0 {
            flags.insert(DamageFlags::RICOCHET);
        }
        let mut result = TraceResult {
            end: origin + dir * block_t,
            ..Default::default()
        };
        for (i, (d, target, part)) in hits.iter().enumerate() {
            let point = origin + dir * *d;
            let mut info = DamageInfo::new(spec.damage * mult, shooter, spec.attack)
                .at(point, dir)
                .part(*part)
                .knockback(dir * spec.knockback)
                .flags(flags);
            info.ricochets = ricochets;
            if i > 0 {
                info.flags.insert(DamageFlags::PIERCED);
            }
            self.apply_damage(t, *target, info);
            result.hits += 1;
        }

        let Some((prism_id, pt, prism_pos)) = prism else {
            return result;
        };
        // Ricochet: shatter the prism and re-fire from it.
        result.end = origin + dir * pt;
        self.shatter_prism(prism_id);
        result.ricochet.push(prism_pos);
        let next = self.ricochet_target(t, prism_pos, shooter);
        let (next_dir, range) = match next {
            Some(aim) => {
                let d = aim - prism_pos;
                (d.normalize_or(dir), d.length() + 2.0)
            }
            None => (dir, spec.range),
        };
        let leg = ShotSpec {
            origin: prism_pos,
            dir: next_dir,
            range,
            ..*spec
        };
        let current = self.current_hitboxes(t);
        let r = self.trace_shot(
            t,
            shooter,
            &current,
            None,
            &leg,
            ricochets.saturating_add(1),
        );
        result.hits += r.hits;
        if r.ricochet.is_empty() {
            result.ricochet.push(r.end);
        } else {
            result.ricochet.extend(r.ricochet);
        }
        result
    }

    /// Nearest live prism the ray passes within `ricochet_radius` of, before `max_t`:
    /// (id, distance along the ray, prism position).
    fn prism_on_ray(
        &self,
        t: &Tuning,
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
        time: Option<f64>,
    ) -> Option<(NetId, f32, Vec3)> {
        let r = t.combat.prism.ricochet_radius;
        self.prisms
            .values()
            .filter_map(|p| {
                let pos = time.and_then(|time| p.trail.at(time)).unwrap_or(p.position);
                let along = (pos - origin).dot(dir);
                if along <= 0.0 || along >= max_t {
                    return None;
                }
                let off = (pos - (origin + dir * along)).length();
                (off <= r).then_some((p.id, along, pos))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Where a ricochet from `from` goes: the nearest other live prism within chain range
    /// (line of sight), else the nearest hittable target within target range — its head /
    /// weak point if visible, else its body.
    pub fn ricochet_target(&self, t: &Tuning, from: Vec3, shooter: Owner) -> Option<Vec3> {
        let pt = &t.combat.prism;
        let world = &self.collision.world;
        let chain = self
            .prisms
            .values()
            .map(|p| (p.position, p.position.distance(from)))
            .filter(|(pos, d)| {
                *d > 0.05 && *d <= pt.chain_range && !world.line_blocked(from, *pos, MASK_SHOTS)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((pos, _)) = chain {
            return Some(pos);
        }
        let mut best: Option<(f32, Vec3)> = None;
        for h in self.current_hitboxes(t) {
            if !self.can_hit(shooter, h.target) {
                continue;
            }
            let Some(center) = h.center() else {
                continue;
            };
            let dist = center.distance(from);
            if dist > pt.target_range || best.is_some_and(|(d, _)| d <= dist) {
                continue;
            }
            // Weak points / head first, then the body.
            let mut points: Vec<Vec3> = h
                .shapes
                .iter()
                .filter(|s| s.part != HitPart::Body)
                .map(|s| s.center())
                .collect();
            points.push(center);
            if let Some(aim) = points
                .into_iter()
                .find(|p| !world.line_blocked(from, *p, MASK_SHOTS))
            {
                best = Some((dist, aim));
            }
        }
        best.map(|(_, aim)| aim)
    }
}
