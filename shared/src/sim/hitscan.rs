//! Hitscan: lag-compensated rays, thorn ricochets, piercing beams and Arclash chains.
//!
//! A shot from an input is tested against the hitboxes **as the shooter saw them**: the
//! server's [`HitboxHistory`] rewinds every player and enemy hitbox to the input's
//! `view_tick + view_frac/256` (clamped to `net.lag_comp_max_ms` and to the shooter's
//! RTT-based bound, [`SimWorld::max_rewind_ticks`]), and live thorns are rewound to the same
//! time through their own position trails. Level geometry (`MASK_SHOTS`) is static, so a wall
//! between the shooter and the (rewound) target always blocks. The shooter never hits itself;
//! targets that may not be hurt (co-op teammates, dead players, dying enemies) are
//! transparent.
//!
//! **Hit assist**: player-owned rays test the plain shapes first and, on a miss, the shapes
//! inflated by [`SimWorld::hit_pads`] (`combat.hit_assist`); enemy attacks use plain shapes.
//!
//! **Ricochet**: a Quill shot (`ricochet` enabled) that passes within `thorn.ricochet_radius`
//! of a live thorn before anything else stops it shatters the thorn and re-fires from it, ×1.5
//! damage per bounce, at another live thorn within `thorn.chain_range` (chain), else at a
//! branded hostile within `thorn.target_range`, else at the nearest hostile in range — always
//! in line of sight, weak point / head if visible, else the body. Bounce legs are traced
//! against current (not rewound) hitboxes since they auto-aim at current positions.
//!
//! **Pierce** (the Lamplighter's full focus): the ray passes through targets, nearest first,
//! losing damage for each one.
//!
//! **Arc chain**: an Arclash zap that hits jumps to the nearest other hostile within
//! `arclash.chain_range` of it (line of sight from the target, branded targets first),
//! `chain_jumps` times at most, ×`chain_falloff` damage per jump.

use engine::physics::MASK_SHOTS;
use glam::Vec3;

use super::combat::{Attack, DamageFlags, DamageInfo, Owner};
use super::{
    HitPads, HitPart, HitTarget, Hitbox, HitboxHistory, PlayerId, ShotEvent, SimEvent, SimWorld,
};
use crate::input::PlayerInput;
use crate::protocol::NetId;
use crate::tuning::{CombatTuning, Tuning};

/// One ray to trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotSpec {
    pub origin: Vec3,
    pub dir: Vec3,
    pub range: f32,
    pub damage: f32,
    pub attack: Attack,
    /// Velocity change applied to the target along the ray.
    pub knockback: f32,
    /// Can ricochet off thorns.
    pub ricochet: bool,
    /// A target this ray passes through (it starts inside it).
    pub exclude: Option<HitTarget>,
    /// Harder hits up close (the scatterguns' pellets); not on ricochet legs.
    pub point_blank: Option<PointBlank>,
}

/// A ray that hits within `range` m of where it left deals × `multiplier`, easing back to × 1
/// at `end` m (a scattergun's point-blank blast).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointBlank {
    pub range: f32,
    pub end: f32,
    pub multiplier: f32,
}

impl PointBlank {
    /// The damage multiplier for a hit `distance` m along the ray.
    pub fn at(&self, distance: f32) -> f32 {
        let k = if distance <= self.range {
            1.0
        } else if self.end > self.range {
            ((self.end - distance) / (self.end - self.range)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        1.0 + (self.multiplier - 1.0) * k
    }
}

impl ShotSpec {
    pub fn new(origin: Vec3, dir: Vec3, range: f32, damage: f32, attack: Attack) -> Self {
        Self {
            origin,
            dir: dir.normalize_or(Vec3::NEG_Z),
            range,
            damage,
            attack,
            knockback: 0.0,
            ricochet: false,
            exclude: None,
            point_blank: None,
        }
    }

    pub fn knockback(mut self, knockback: f32) -> Self {
        self.knockback = knockback;
        self
    }

    pub fn ricochet(mut self, on: bool) -> Self {
        self.ricochet = on;
        self
    }

    pub fn exclude(mut self, target: Option<HitTarget>) -> Self {
        self.exclude = target;
        self
    }

    pub fn point_blank(mut self, point_blank: PointBlank) -> Self {
        self.point_blank = Some(point_blank);
        self
    }
}

/// The world as a shooter saw it: lag-compensated hitboxes and the (clamped) view time.
pub struct ShotView {
    /// View time in ticks, clamped to `[now - lag_comp_max_ms, now]`.
    pub time: f64,
    pub hitboxes: Vec<Hitbox>,
}

/// Where the last leg of a shot ended on something (Brand and Arc react to it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    pub point: Vec3,
    /// Direction of the leg.
    pub dir: Vec3,
    /// Surface normal when it hit the level.
    pub normal: Option<Vec3>,
    /// The target it hit (and where).
    pub target: Option<(HitTarget, HitPart)>,
}

/// Result of tracing one shot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    /// End of the first leg (world hit, target hit, range end, or the first thorn).
    pub end: Vec3,
    /// Targets damaged along the whole path (all legs).
    pub hits: u32,
    /// Thorn bounce path: every thorn hit, then the final end point. Empty without ricochet.
    pub ricochet: Vec<Vec3>,
    /// What the last leg hit (`None`: it ran out of range).
    pub impact: Option<Impact>,
}

/// Hit assist for an attack by `attacker` on `target` ([`SimWorld::hit_pads`]; clients
/// testing what their own attacks touch use it too): player-owned attacks get the
/// `combat.hit_assist` pads (enemies' or other players' sizes); everything else none.
pub fn assist_pads(t: &CombatTuning, attacker: Owner, target: HitTarget) -> HitPads {
    let a = &t.hit_assist;
    match (attacker, target) {
        (Owner::Player(_), HitTarget::Entity(_)) => HitPads {
            body: a.enemy_body_pad,
            weak: a.enemy_weak_pad,
        },
        (Owner::Player(_), HitTarget::Player(_)) => HitPads {
            body: a.player_body_pad,
            weak: a.player_head_pad,
        },
        _ => HitPads::NONE,
    }
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

    /// Hit assist for an attack by `attacker` on `target`: player-owned attacks get the
    /// `combat.hit_assist` pads (enemies' or other players' sizes); everything else none.
    pub fn hit_pads(&self, t: &Tuning, attacker: Owner, target: HitTarget) -> HitPads {
        assist_pads(&t.combat, attacker, target)
    }

    /// Fires a group of rays from one input (one shot, a pellet spread) against the
    /// lag-compensated view and emits one [`SimEvent::Shot`] (plus [`SimEvent::Ricochet`]
    /// per bounce path). Returns the view (for follow-up traces) and every ray's result.
    pub(crate) fn fire_hitscan_group(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        attack: Attack,
        shots: &[ShotSpec],
    ) -> (ShotView, Vec<TraceResult>) {
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
        (view, results)
    }

    /// A piercing shot (the Lamplighter): passes through up to `pierce` targets along the ray (before
    /// the level stops it), nearest first, the damage × `falloff` for each one passed
    /// ([`DamageFlags::PIERCED`] after the first). Lag-compensated like every shot; no thorn
    /// ricochet. Emits one [`SimEvent::Shot`] ending where it stopped.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fire_piercing(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
        spec: &ShotSpec,
        pierce: u32,
        falloff: f32,
    ) {
        let view = self.shot_view(t, history, id, input);
        let shooter = Owner::Player(id);
        let (origin, dir) = (spec.origin, spec.dir);
        let world_t = self
            .collision
            .world
            .cast_ray(origin, dir, spec.range, MASK_SHOTS)
            .map_or(spec.range, |h| h.distance);
        let mut hits: Vec<(f32, HitTarget, HitPart)> = view
            .hitboxes
            .iter()
            .filter(|h| self.can_hit(shooter, h.target))
            .filter_map(|h| {
                let pads = self.hit_pads(t, shooter, h.target);
                h.ray_hit_padded(origin, dir, world_t, pads)
                    .map(|(d, part)| (d, h.target, part))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let pierce = pierce.max(1) as usize;
        hits.truncate(pierce);
        let end_t = if hits.len() == pierce {
            hits.last().map_or(world_t, |h| h.0)
        } else {
            world_t
        };
        let mut damage = spec.damage;
        for (i, (d, target, part)) in hits.into_iter().enumerate() {
            let mut info = DamageInfo::new(damage, shooter, spec.attack)
                .at(origin + dir * d, dir)
                .part(part)
                .knockback(dir * spec.knockback)
                .flags(Self::hitscan_flags(spec.attack));
            if i > 0 {
                info.flags.insert(DamageFlags::PIERCED);
            }
            self.apply_damage(t, target, info);
            damage *= falloff;
        }
        self.events.push(SimEvent::Shot(ShotEvent {
            shooter: id,
            attack: spec.attack,
            origin,
            ends: vec![origin + dir * end_t],
        }));
    }

    /// Nearest target along a ray before `max_t` that `shooter` may hit (with hit assist):
    /// (distance, target, part).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn first_target_on_ray(
        &self,
        t: &Tuning,
        shooter: Owner,
        hitboxes: &[Hitbox],
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
        exclude: Option<HitTarget>,
    ) -> Option<(f32, HitTarget, HitPart)> {
        hitboxes
            .iter()
            .filter(|h| Some(h.target) != exclude && self.can_hit(shooter, h.target))
            .filter_map(|h| {
                h.ray_hit_padded(origin, dir, max_t, self.hit_pads(t, shooter, h.target))
                    .map(|(d, p)| (d, h.target, p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Traces one shot (and its ricochet legs) against `hitboxes`; thorns are taken at
    /// `thorn_time` (ticks; `None` = current positions). Applies the damage.
    pub fn trace_shot(
        &mut self,
        t: &Tuning,
        shooter: Owner,
        hitboxes: &[Hitbox],
        thorn_time: Option<f64>,
        spec: &ShotSpec,
        ricochets: u8,
    ) -> TraceResult {
        let dir = spec.dir.normalize_or(Vec3::NEG_Z);
        let origin = spec.origin;
        let world = self
            .collision
            .world
            .cast_ray(origin, dir, spec.range, MASK_SHOTS);
        let world_t = world.map_or(spec.range, |h| h.distance);
        let hit =
            self.first_target_on_ray(t, shooter, hitboxes, origin, dir, world_t, spec.exclude);
        let block_t = hit.map_or(world_t, |h| h.0);
        let thorn = if spec.ricochet {
            self.thorn_on_ray(t, origin, dir, block_t, thorn_time)
        } else {
            None
        };

        let mut result = TraceResult {
            end: origin + dir * block_t,
            ..Default::default()
        };
        let Some((thorn_id, thorn_t, thorn_pos)) = thorn else {
            if let Some((d, target, part)) = hit {
                let point = origin + dir * d;
                let mut mult = t.combat.thorn.damage_multiplier.powi(ricochets as i32);
                if let Some(pb) = spec.point_blank {
                    mult *= pb.at(d);
                }
                let mut flags = Self::hitscan_flags(spec.attack);
                if ricochets > 0 {
                    flags.insert(DamageFlags::RICOCHET);
                }
                let mut info = DamageInfo::new(spec.damage * mult, shooter, spec.attack)
                    .at(point, dir)
                    .part(part)
                    .knockback(dir * spec.knockback)
                    .flags(flags);
                info.ricochets = ricochets;
                self.apply_damage(t, target, info);
                result.hits = 1;
                result.impact = Some(Impact {
                    point,
                    dir,
                    normal: None,
                    target: Some((target, part)),
                });
            } else if let Some(w) = world {
                result.impact = Some(Impact {
                    point: w.point,
                    dir,
                    normal: Some(w.normal),
                    target: None,
                });
            }
            return result;
        };

        // Ricochet: shatter the thorn and re-fire from it.
        result.end = origin + dir * thorn_t;
        self.shatter_thorn(thorn_id);
        result.ricochet.push(thorn_pos);
        let next = self.ricochet_target(t, thorn_pos, shooter);
        let (next_dir, range) = match next {
            Some(aim) => {
                let d = aim - thorn_pos;
                (d.normalize_or(dir), d.length() + 2.0)
            }
            None => (dir, spec.range),
        };
        let leg = ShotSpec {
            origin: thorn_pos,
            dir: next_dir,
            range,
            exclude: None,
            point_blank: None,
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
        result.impact = r.impact;
        if r.ricochet.is_empty() {
            result.ricochet.push(r.end);
        } else {
            result.ricochet.extend(r.ricochet);
        }
        result
    }

    /// Nearest live thorn the ray passes within `ricochet_radius` of, before `max_t`:
    /// (id, distance along the ray, thorn position).
    fn thorn_on_ray(
        &self,
        t: &Tuning,
        origin: Vec3,
        dir: Vec3,
        max_t: f32,
        time: Option<f64>,
    ) -> Option<(NetId, f32, Vec3)> {
        let r = t.combat.thorn.ricochet_radius;
        self.thorns
            .values()
            .filter_map(|th| {
                let pos = time
                    .and_then(|time| th.trail.at(time))
                    .unwrap_or(th.position);
                let along = (pos - origin).dot(dir);
                if along <= 0.0 || along >= max_t {
                    return None;
                }
                let off = (pos - (origin + dir * along)).length();
                (off <= r).then_some((th.id, along, pos))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// Where to aim at `h` from `from`: its first weak point / head in line of sight, else its
    /// body centre if visible.
    fn visible_aim_point(&self, h: &Hitbox, from: Vec3) -> Option<Vec3> {
        let world = &self.collision.world;
        let mut points: Vec<Vec3> = h
            .shapes
            .iter()
            .filter(|s| s.part != HitPart::Body)
            .map(|s| s.center())
            .collect();
        points.extend(h.center());
        points
            .into_iter()
            .find(|p| !world.line_blocked(from, *p, MASK_SHOTS))
    }

    /// Where a ricochet from `from` goes: the nearest other live thorn within chain range
    /// (line of sight), else the nearest branded hittable target within target range, else
    /// the nearest hittable target in range — its head / weak point if visible, else its body.
    pub fn ricochet_target(&self, t: &Tuning, from: Vec3, shooter: Owner) -> Option<Vec3> {
        let tt = &t.combat.thorn;
        let world = &self.collision.world;
        let chain = self
            .thorns
            .values()
            .map(|th| (th.position, th.position.distance(from)))
            .filter(|(pos, d)| {
                *d > 0.05 && *d <= tt.chain_range && !world.line_blocked(from, *pos, MASK_SHOTS)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((pos, _)) = chain {
            return Some(pos);
        }
        // (branded, distance, aim point): branded targets first, then the nearest.
        let mut best: Option<(bool, f32, Vec3)> = None;
        for h in self.current_hitboxes(t) {
            if !self.can_hit(shooter, h.target) {
                continue;
            }
            let Some(center) = h.center() else {
                continue;
            };
            let dist = center.distance(from);
            if dist > tt.target_range {
                continue;
            }
            let branded = self.is_branded(h.target);
            let better = best.is_none_or(|(b, d, _)| (branded, -dist) > (b, -d));
            if !better {
                continue;
            }
            if let Some(aim) = self.visible_aim_point(&h, from) {
                best = Some((branded, dist, aim));
            }
        }
        best.map(|(_, _, aim)| aim)
    }

    /// Arclash chain from `first` (already hit at `point` for `damage`): jumps to the nearest
    /// other hostile within `chain_range` (of the padded shape, line of sight from the previous
    /// target; branded targets first), up to `chain_jumps` times, ×`chain_falloff` each.
    /// Emits [`SimEvent::ArcChain`] when it jumped at least once.
    pub(crate) fn arc_chain(
        &mut self,
        t: &Tuning,
        shooter: Owner,
        hitboxes: &[Hitbox],
        first: HitTarget,
        point: Vec3,
        damage: f32,
    ) {
        let a = &t.combat.arclash;
        let mut visited = vec![first];
        let mut from = hitboxes
            .iter()
            .find(|h| h.target == first)
            .and_then(Hitbox::center)
            .unwrap_or(point);
        let mut points = vec![point];
        let mut dmg = damage;
        for _ in 0..a.chain_jumps {
            let world = &self.collision.world;
            // (branded, distance, target, aim point)
            let mut best: Option<(bool, f32, HitTarget, Vec3)> = None;
            for h in hitboxes {
                if visited.contains(&h.target) || !self.can_hit(shooter, h.target) {
                    continue;
                }
                let Some(center) = h.center() else {
                    continue;
                };
                let pads = self.hit_pads(t, shooter, h.target);
                let Some((d, _)) = h.distance_to_point_padded(from, pads) else {
                    continue;
                };
                let d = d.max(0.0);
                if d > a.chain_range || world.line_blocked(from, center, MASK_SHOTS) {
                    continue;
                }
                let branded = self.is_branded(h.target);
                if best.is_none_or(|(b, bd, _, _)| (branded, -d) > (b, -bd)) {
                    best = Some((branded, d, h.target, center));
                }
            }
            let Some((_, _, target, center)) = best else {
                break;
            };
            dmg *= a.chain_falloff;
            let dir = (center - from).normalize_or(Vec3::NEG_Z);
            let info = DamageInfo::new(dmg, shooter, Attack::Arc)
                .at(center, dir)
                .flags(DamageFlags::HITSCAN | DamageFlags::CHAINED);
            self.apply_damage(t, target, info);
            visited.push(target);
            points.push(center);
            from = center;
        }
        if points.len() > 1 {
            self.events.push(SimEvent::ArcChain { shooter, points });
        }
    }
}
