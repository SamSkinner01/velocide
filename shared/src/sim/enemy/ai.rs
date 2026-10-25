//! Enemy AI (server only): **idle → alert → chase → wind-up (telegraph) → strike → recover →
//! chase**, plus stunned and dying. Targets the nearest visible player with stickiness, and
//! switches to a player who hurts it badly. Ground enemies chase directly with capsule sweeps
//! against the level (collide-and-slide with step-up); navigation (A* on a nav grid) slots in
//! where `chase_velocity` is computed. Flying enemies steer directly at hover height.

use engine::physics::{CollisionWorld, MASK_MOVEMENT, MASK_SHOTS};
use glam::Vec3;

use super::{AiState, AttackKind, Enemy, Locomotion, ParryWindow};
use crate::movement::SKIN;
use crate::protocol::NetId;
use crate::sim::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use crate::sim::parry::ParryKey;
use crate::sim::weapons::{SpreadRng, spread_dir};
use crate::sim::{HitTarget, PlayerId, SimEvent, SimWorld};
use crate::tuning::Tuning;

/// What enemies know about a player this tick.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlayerView {
    pub id: PlayerId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub chest: Vec3,
    pub radius: f32,
    pub height: f32,
    pub targetable: bool,
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

fn move_towards(from: Vec3, to: Vec3, max_delta: f32) -> Vec3 {
    let d = to - from;
    let len = d.length();
    if len <= max_delta || len < 1e-6 {
        to
    } else {
        from + d * (max_delta / len)
    }
}

/// Yaw that looks along `dir` (see `input::look_dir`).
fn yaw_towards(dir: Vec3) -> Option<f32> {
    let h = horizontal(dir);
    (h.length_squared() > 1e-6).then(|| (-h.x).atan2(-h.z))
}

fn turn_towards(yaw: f32, target: f32, max_step: f32) -> f32 {
    let mut d = (target - yaw).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    yaw + d.clamp(-max_step, max_step)
}

impl SimWorld {
    pub(crate) fn player_views(&self, t: &Tuning) -> Vec<PlayerView> {
        self.players
            .values()
            .map(|p| {
                let (center, _, radius) = p.movement.capsule(&t.movement);
                PlayerView {
                    id: p.id,
                    position: p.movement.position,
                    velocity: p.movement.velocity,
                    chest: center + Vec3::Y * 0.3,
                    radius,
                    height: p.movement.height(&t.movement),
                    targetable: !p.combat.dead && !p.notarget,
                }
            })
            .collect()
    }

    /// Runs every enemy's AI and movement for one tick.
    pub(crate) fn step_enemies(&mut self, t: &Tuning) {
        if self.enemies.is_empty() {
            return;
        }
        let players = self.player_views(t);
        let bodies: Vec<(NetId, Vec3, f32, f32)> = self
            .enemies
            .values()
            .filter(|e| e.is_alive())
            .map(|e| (e.id, e.position, e.radius(), e.def.speed))
            .collect();
        let ids: Vec<NetId> = self.enemies.keys().copied().collect();
        for id in ids {
            let Some(mut e) = self.enemies.remove(&id) else {
                continue;
            };
            // Soft separation from other enemies (no stacking).
            let mut separation = Vec3::ZERO;
            if e.is_alive() {
                for (other, pos, r, _) in &bodies {
                    if *other == id {
                        continue;
                    }
                    let d = horizontal(e.position - *pos);
                    let min = e.radius() + r + 0.2;
                    let len = d.length();
                    if len < min && len > 1e-4 {
                        separation += d / len * ((min - len) / min) * e.def.speed;
                    }
                }
            }
            let keep = self.step_enemy(t, &mut e, &players, separation);
            if keep {
                self.enemies.insert(id, e);
            } else {
                self.events.push(SimEvent::EnemyRemoved { id });
            }
        }
        let tick = self.tick;
        for e in self.enemies.values_mut() {
            e.trail.record(tick, e.position);
        }
    }

    /// One enemy, one tick. Returns false when it should be removed.
    fn step_enemy(
        &mut self,
        t: &Tuning,
        e: &mut Enemy,
        players: &[PlayerView],
        separation: Vec3,
    ) -> bool {
        let rate = self.tick_rate;
        let now = self.tick;
        let dt = self.dt();
        let gravity = t.movement.gravity;
        let def = e.def.clone();
        e.state_ticks += 1;
        for cd in &mut e.cooldowns {
            *cd = cd.saturating_sub(1);
        }
        let window = 2 * rate;
        e.aggro.retain(|a| now.saturating_sub(a.2) < window);

        if e.state == AiState::Dying {
            move_enemy(&self.collision.world, e, Vec3::ZERO, dt, gravity);
            return e.state_ticks < e.state_len;
        }

        // Target selection (periodic, or immediately when the target became invalid).
        let target_valid = e
            .target
            .is_some_and(|id| players.iter().any(|p| p.id == id && p.targetable));
        if e.retarget_in == 0 || !target_valid {
            let (target, visible) = self.select_target(e, players);
            if target != e.target && target.is_some() {
                e.last_seen_tick = now;
            }
            e.target = target;
            if visible && let Some(tp) = target.and_then(|id| players.iter().find(|p| p.id == id)) {
                e.last_seen_tick = now;
                e.last_seen_pos = tp.position;
            }
            e.retarget_in = secs_to_ticks(def.ai.retarget_interval, rate, 1) as u32;
        } else {
            e.retarget_in -= 1;
        }
        let lose = secs_to_ticks(def.ai.lose_target_time, rate, 1) as u32;
        if e.target.is_some() && now.saturating_sub(e.last_seen_tick) > lose {
            e.target = None;
        }
        let target = e
            .target
            .and_then(|id| players.iter().find(|p| p.id == id))
            .copied();

        let mut desired = Vec3::ZERO;
        let mut face: Option<Vec3> = None;
        match e.state {
            AiState::Idle => {
                if target.is_some() {
                    e.set_state(
                        AiState::Alert,
                        secs_to_ticks(def.ai.reaction_time, rate, 1) as u32,
                    );
                    self.events.push(SimEvent::EnemyAlerted {
                        id: e.id,
                        target: e.target,
                    });
                }
            }
            AiState::Alert => {
                face = target.map(|p| p.position);
                if e.state_ticks >= e.state_len {
                    e.set_state(AiState::Chase, 0);
                }
            }
            AiState::Chase => match target {
                None => {
                    if e.target.is_none() {
                        e.set_state(AiState::Idle, 0);
                    }
                }
                Some(tp) => {
                    face = Some(tp.position);
                    if let Some(i) = self.choose_attack(e, &tp) {
                        self.begin_windup(e, i);
                    } else {
                        desired = self.chase_velocity(e, &tp);
                    }
                }
            },
            AiState::Windup => {
                let Some(a) = def.attacks.get(e.attack) else {
                    e.set_state(AiState::Chase, 0);
                    return true;
                };
                if a.track_target {
                    face = target.map(|p| p.position);
                }
                if let Some(tp) = &target {
                    desired = horizontal(tp.position - e.position).normalize_or_zero()
                        * (def.speed * a.windup_move_scale);
                }
                if e.state_ticks >= e.state_len {
                    self.strike(t, e, players, target.as_ref());
                }
            }
            AiState::Strike => {
                self.continue_burst(t, e, target.as_ref());
                if e.state_ticks >= e.state_len {
                    let (cooldown, recover) = def
                        .attacks
                        .get(e.attack)
                        .map_or((0.5, 0.5), |a| (a.cooldown, a.recover));
                    if let Some(cd) = e.cooldowns.get_mut(e.attack) {
                        *cd = secs_to_ticks(cooldown + recover, rate, 0) as u32;
                    }
                    e.set_state(AiState::Recover, secs_to_ticks(recover, rate, 1) as u32);
                }
            }
            AiState::Recover | AiState::Stunned => {
                if e.state_ticks >= e.state_len {
                    e.set_state(AiState::Chase, 0);
                }
            }
            AiState::Dying => {}
        }

        if let Some(point) = face
            && let Some(yaw) = yaw_towards(point - e.position)
        {
            e.yaw = turn_towards(e.yaw, yaw, def.turn_rate.to_radians() * dt);
        }
        let control = matches!(e.state, AiState::Chase | AiState::Windup | AiState::Strike);
        let desired = if control {
            desired + separation
        } else {
            separation
        };
        move_enemy(&self.collision.world, e, desired, dt, gravity);
        true
    }

    /// (target, target currently visible).
    fn select_target(&self, e: &Enemy, players: &[PlayerView]) -> (Option<PlayerId>, bool) {
        let eye = e.eye();
        let world = &self.collision.world;
        let ai = &e.def.ai;
        let visible: Vec<(PlayerId, f32)> = players
            .iter()
            .filter(|p| p.targetable)
            .map(|p| (p.id, p.chest.distance(eye), p.chest))
            .filter(|(_, d, chest)| {
                *d <= ai.sight_range && !world.line_blocked(eye, *chest, MASK_SHOTS)
            })
            .map(|(id, d, _)| (id, d))
            .collect();
        let is_visible = |id: PlayerId| visible.iter().any(|v| v.0 == id);
        // Someone hurting us badly takes priority.
        let threshold = ai.aggro_switch_fraction * e.max_health;
        if let Some(a) = e
            .aggro
            .iter()
            .filter(|a| a.1 >= threshold && players.iter().any(|p| p.id == a.0 && p.targetable))
            .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            return (Some(a.0), is_visible(a.0));
        }
        let nearest = visible.iter().min_by(|a, b| a.1.total_cmp(&b.1)).copied();
        let current = e
            .target
            .filter(|id| players.iter().any(|p| p.id == *id && p.targetable));
        match (current, nearest) {
            (Some(cur), Some((best, best_d))) => match visible.iter().find(|v| v.0 == cur) {
                Some(&(_, cur_d)) if best_d >= ai.target_stickiness * cur_d => (Some(cur), true),
                _ => (Some(best), true),
            },
            (Some(cur), None) => (Some(cur), false),
            (None, Some((best, _))) => (Some(best), true),
            (None, None) => (None, false),
        }
    }

    /// First usable attack (definition order = priority) against `tp`.
    fn choose_attack(&self, e: &Enemy, tp: &PlayerView) -> Option<usize> {
        let world = &self.collision.world;
        e.def.attacks.iter().enumerate().find_map(|(i, a)| {
            if e.cooldowns.get(i).copied().unwrap_or(0) > 0 {
                return None;
            }
            let ok = match a.kind {
                AttackKind::Melee | AttackKind::Slam => {
                    let surface =
                        horizontal(tp.position - e.position).length() - e.radius() - tp.radius;
                    let vertical = tp.position.y < e.position.y + e.height()
                        && tp.position.y + tp.height > e.position.y - 0.3;
                    surface <= a.range && surface >= a.min_range - e.radius() && vertical
                }
                AttackKind::Projectile => {
                    let d = tp.chest.distance(e.eye());
                    d <= a.range && d >= a.min_range
                }
            };
            (ok && !world.line_blocked(e.eye(), tp.chest, MASK_SHOTS)).then_some(i)
        })
    }

    /// Movement while chasing: straight at the target (ground: horizontally), stopping at
    /// melee range; ranged enemies keep their preferred band and may strafe.
    fn chase_velocity(&self, e: &Enemy, tp: &PlayerView) -> Vec3 {
        let def = &e.def;
        let to = tp.position - e.position;
        let flat = horizontal(to);
        let dist = flat.length();
        let dir = flat.normalize_or_zero();
        let melee_range = def
            .attacks
            .iter()
            .filter(|a| a.kind != AttackKind::Projectile)
            .map(|a| a.range)
            .fold(f32::INFINITY, f32::min);
        let surface = dist - e.radius() - tp.radius;
        let mut v = match def.ai.preferred_range {
            Some((min, max)) => {
                let d = tp.chest.distance(e.eye());
                let mut v = if d > max {
                    dir * def.speed
                } else if d < min {
                    -dir * def.speed
                } else {
                    Vec3::ZERO
                };
                if def.ai.strafe {
                    let side = Vec3::new(-dir.z, 0.0, dir.x);
                    v += side * (def.speed * 0.6 * e.strafe_sign);
                }
                v
            }
            None if melee_range.is_finite() && surface <= melee_range * 0.6 => Vec3::ZERO,
            None => dir * def.speed,
        };
        if let Locomotion::Flying {
            min_height,
            max_height,
        } = def.locomotion
        {
            let ground = self
                .collision
                .world
                .cast_ray(e.position, Vec3::NEG_Y, 50.0, MASK_MOVEMENT)
                .map_or(50.0, |h| h.distance);
            let want = (min_height + max_height) * 0.5;
            let target_y = tp.position.y + want;
            let vy = if ground < min_height {
                def.speed * 0.5
            } else if ground > max_height {
                -def.speed * 0.5
            } else {
                ((target_y - e.position.y) * 2.0).clamp(-def.speed * 0.5, def.speed * 0.5)
            };
            v.y = vy;
        }
        v
    }

    fn begin_windup(&mut self, e: &mut Enemy, attack: usize) {
        let rate = self.tick_rate;
        let def = e.def.clone();
        let Some(a) = def.attacks.get(attack) else {
            return;
        };
        let len = secs_to_ticks(a.windup, rate, 1) as u32;
        let parry = a.parry_window;
        e.attack = attack;
        e.attack_seq = e.attack_seq.wrapping_add(1);
        e.shots_fired = 0;
        e.set_state(AiState::Windup, len);
        e.parry_window = parry.map(|w| {
            let wt = (secs_to_ticks(w, rate, 1) as u32).min(len);
            ParryWindow {
                start: self.tick + len - wt,
                end: self.tick + len,
                seq: e.attack_seq,
            }
        });
        self.events.push(SimEvent::EnemyAttack {
            id: e.id,
            attack: attack.min(255) as u8,
            phase: super::super::EnemyAttackPhase::Windup,
            parryable: parry.is_some(),
        });
    }

    /// The strike moment: melee arc / slam radius damage, or the first projectile.
    fn strike(
        &mut self,
        t: &Tuning,
        e: &mut Enemy,
        players: &[PlayerView],
        target: Option<&PlayerView>,
    ) {
        let rate = self.tick_rate;
        let def = e.def.clone();
        let idx = e.attack.min(def.attacks.len().saturating_sub(1));
        let Some(a) = def.attacks.get(idx) else {
            e.set_state(AiState::Chase, 0);
            return;
        };
        let attack = Attack::Enemy {
            kind: def.net_kind,
            attack: idx.min(255) as u8,
        };
        let source = Owner::Enemy(e.id);
        let parryable = a.parry_window.is_some();
        let world = &self.collision.world;
        let mut hits: Vec<(PlayerId, DamageInfo)> = Vec::new();
        match a.kind {
            AttackKind::Melee => {
                let half_arc = (a.arc * 0.5).to_radians();
                let fwd = e.forward();
                for p in players.iter().filter(|p| p.targetable) {
                    let to = horizontal(p.position - e.position);
                    let dist = to.length();
                    if dist - e.radius() - p.radius > a.range + 0.25 {
                        continue;
                    }
                    let vertical = p.position.y < e.position.y + e.height()
                        && p.position.y + p.height > e.position.y - 0.3;
                    if !vertical {
                        continue;
                    }
                    if dist > 0.3 && to.normalize_or_zero().angle_between(fwd) > half_arc {
                        continue;
                    }
                    if world.line_blocked(e.center(), p.chest, MASK_SHOTS) {
                        continue;
                    }
                    let dir = to.normalize_or(fwd);
                    let kb = (dir + Vec3::Y * 0.35).normalize_or(Vec3::Y) * a.knockback;
                    let info = DamageInfo::new(a.damage, source, attack)
                        .at(p.chest, dir)
                        .knockback(kb)
                        .flags(DamageFlags::MELEE);
                    hits.push((p.id, info));
                }
            }
            AttackKind::Slam => {
                let center = e.position + e.forward() * (e.radius() + 0.5);
                for p in players.iter().filter(|p| p.targetable) {
                    let to = p.position - center;
                    let d = horizontal(to).length();
                    if d > a.shockwave_radius || to.y.abs() > 2.0 {
                        continue;
                    }
                    let frac = 1.0 - 0.5 * (d / a.shockwave_radius.max(0.1));
                    let dir = horizontal(to).normalize_or(e.forward());
                    let kb = (dir + Vec3::Y * 0.8).normalize_or(Vec3::Y) * (a.knockback * frac);
                    let info = DamageInfo::new(a.damage * frac, source, attack)
                        .at(p.chest, dir)
                        .knockback(kb)
                        .flags(DamageFlags::SLAM);
                    hits.push((p.id, info));
                }
                self.events.push(SimEvent::EnemyShockwave {
                    id: e.id,
                    position: center,
                    radius: a.shockwave_radius,
                });
            }
            AttackKind::Projectile => {}
        }
        for (victim, info) in hits {
            if parryable {
                self.queue_pending_hit(t, ParryKey::Melee(e.id, e.attack_seq), victim, info);
            } else {
                self.apply_damage(t, HitTarget::Player(victim), info);
            }
        }
        self.events.push(SimEvent::EnemyAttack {
            id: e.id,
            attack: idx.min(255) as u8,
            phase: super::super::EnemyAttackPhase::Strike,
            parryable,
        });
        let mut len = secs_to_ticks(a.strike, rate, 1) as u32;
        if let Some(pa) = &a.projectile {
            let burst = secs_to_ticks(pa.interval, rate, 1) as u32 * pa.count.saturating_sub(1);
            len = len.max(burst + 1);
        }
        // (The parry window, if any, survives: late parries still cancel the pending hits.)
        e.set_state(AiState::Strike, len);
        if a.kind == AttackKind::Projectile {
            self.fire_enemy_projectile(t, e, target);
        }
    }

    /// Fires the remaining projectiles of a burst at their intervals.
    fn continue_burst(&mut self, t: &Tuning, e: &mut Enemy, target: Option<&PlayerView>) {
        let Some(a) = e.def.attacks.get(e.attack) else {
            return;
        };
        let Some(pa) = &a.projectile else {
            return;
        };
        if a.kind != AttackKind::Projectile || e.shots_fired >= pa.count {
            return;
        }
        let interval = secs_to_ticks(pa.interval, self.tick_rate, 1) as u32;
        if e.state_ticks >= e.shots_fired * interval {
            self.fire_enemy_projectile(t, e, target);
        }
    }

    fn fire_enemy_projectile(&mut self, t: &Tuning, e: &mut Enemy, target: Option<&PlayerView>) {
        let def = e.def.clone();
        let idx = e.attack.min(def.attacks.len().saturating_sub(1));
        let Some(a) = def.attacks.get(idx) else {
            return;
        };
        let Some(pa) = &a.projectile else {
            return;
        };
        let muzzle = e.local_to_world(pa.muzzle);
        let aim_point = match target {
            Some(tp) => {
                let time = tp.chest.distance(muzzle) / pa.speed.max(1.0);
                tp.chest + tp.velocity * (time * pa.lead)
            }
            None => muzzle + e.forward() * 10.0,
        };
        let mut dir = (aim_point - muzzle).normalize_or(e.forward());
        if pa.spread > 0.0 {
            let mut rng = SpreadRng::new(self.tick, PlayerId(0), e.id.0 ^ e.shots_fired);
            dir = spread_dir(dir, pa.spread, &mut rng);
        }
        let attack = Attack::Enemy {
            kind: def.net_kind,
            attack: idx.min(255) as u8,
        };
        self.spawn_enemy_projectile(t, pa.kind, e.id, attack, muzzle, dir * pa.speed, a.damage);
        e.shots_fired += 1;
        self.events.push(SimEvent::EnemyFired {
            id: e.id,
            attack: idx.min(255) as u8,
            position: muzzle,
        });
    }
}

/// Kinematic enemy movement: accelerate towards `desired` (ground: horizontally, with
/// gravity), collide-and-slide against the level with a step-up, snap to the ground.
pub(crate) fn move_enemy(
    world: &CollisionWorld,
    e: &mut Enemy,
    desired: Vec3,
    dt: f32,
    gravity: f32,
) {
    let def = e.def.clone();
    let r = def.radius;
    let h = def.height.max(2.0 * r);
    let half = (h * 0.5 - r).max(0.0);
    let lift = Vec3::Y * (h * 0.5);
    let flying = matches!(def.locomotion, Locomotion::Flying { .. });
    let alive = e.is_alive();

    if flying && alive {
        e.velocity = move_towards(e.velocity, desired, def.acceleration * dt);
        e.on_ground = false;
    } else {
        if e.on_ground && e.velocity.y > 0.5 {
            e.on_ground = false;
        }
        if e.on_ground {
            let target = horizontal(desired);
            let accel = if alive {
                def.acceleration
            } else {
                def.acceleration * 0.5
            };
            e.velocity = move_towards(horizontal(e.velocity), target, accel * dt);
        } else {
            e.velocity.y -= gravity * dt;
        }
    }

    let mut center = e.position + lift;
    // Depenetrate (spawned in a wall, pushed by a door...).
    for _ in 0..2 {
        let contacts = world.capsule_contacts(center, half, r, 0.0, MASK_MOVEMENT);
        let mut moved = false;
        for c in contacts {
            if c.distance < 0.0 {
                center += c.normal * (-c.distance + 1e-3);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    let mut motion = e.velocity * dt;
    for _ in 0..4 {
        if motion.length_squared() < 1e-10 {
            break;
        }
        let Some(hit) = world.cast_capsule(center, half, r, motion, SKIN, MASK_MOVEMENT) else {
            center += motion;
            break;
        };
        center += motion * hit.fraction;
        let mut remaining = motion * (1.0 - hit.fraction);
        let n = hit.normal;
        // Step up small ledges while walking.
        if e.on_ground && !flying && n.y.abs() < 0.3 && def.step_height > 0.0 {
            let up = Vec3::Y * def.step_height;
            let flat = horizontal(remaining);
            if world
                .cast_capsule(center, half, r, up, SKIN, MASK_MOVEMENT)
                .is_none()
                && world
                    .cast_capsule(center + up, half, r, flat, SKIN, MASK_MOVEMENT)
                    .is_none()
            {
                center += up + flat;
                remaining = Vec3::ZERO;
                motion = remaining;
                continue;
            }
        }
        remaining -= n * remaining.dot(n);
        let vn = e.velocity.dot(n);
        if vn < 0.0 {
            e.velocity -= n * vn;
        }
        motion = remaining;
    }

    if !flying || !alive {
        if e.velocity.y <= 0.0 {
            let probe = if e.on_ground {
                def.step_height + 0.05
            } else {
                0.05
            } + SKIN;
            match world.cast_capsule(center, half, r, Vec3::NEG_Y * probe, SKIN, MASK_MOVEMENT) {
                Some(hit) if hit.normal.y > 0.7 => {
                    center += Vec3::NEG_Y * (probe * hit.fraction);
                    e.on_ground = true;
                    e.velocity.y = 0.0;
                }
                _ => e.on_ground = false,
            }
        } else {
            e.on_ground = false;
        }
    }
    if center.is_finite() {
        e.position = center - lift;
    } else {
        e.velocity = Vec3::ZERO;
    }
}
