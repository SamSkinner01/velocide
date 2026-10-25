//! Fist, parry and parry-delayed damage.
//!
//! **Punch** (`PUNCH` pressed, off cooldown): a lag-compensated melee hit on the nearest
//! target in a `punch_range` / `punch_cone_degrees` cone, and it opens the **parry window**
//! (`parry_window` seconds, `WeaponState::parry` ticks).
//!
//! **Projectile parry.** While the window is open, every tick (the press tick included) each
//! parryable projectile is tested in the *puncher's* frame of reference:
//!
//! 1. its position **rewound to the puncher's view time** (its trail at
//!    `now − view lag`, clamped to `net.lag_comp_max_ms` and the puncher's RTT-based bound,
//!    [`SimWorld::max_rewind_ticks`]) — what the puncher saw, and
//! 2. its current position (for projectiles that are already pending on the puncher: the
//!    point it hit, backed off along its incoming direction, so the puncher must face it).
//!
//! If either is within `parry_range` of the eye inside `parry_cone_degrees` of the aim (or
//! within `parry_close_range` at any angle) the projectile is **deflected**: it now belongs to
//! the puncher, flies along the aim — auto-aimed at the nearest hostile within
//! `parry_auto_aim_degrees` — without gravity, `parry_speed_multiplier` faster (flak:
//! `flak_parry_speed_multiplier`) and with `parry_damage_multiplier` damage, but never beyond
//! `parry_max_speed` / `parry_max_damage` however often it is punched back and forth.
//!
//! **Network tolerance.** A parryable projectile that touches a player does not hurt
//! immediately: it freezes in a *pending impact* for [`SimWorld::parry_delay_ticks`] =
//! `net.parry_tolerance_ms` (+ the victim's current view lag, clamped like the rewind,
//! when `combat.parry_compensate_view_lag`). A valid parry arriving inside that delay cancels
//! the damage; after it the hit is applied and the parry is too late.
//!
//! **Melee parry hook** (telegraphed enemy attacks, e.g. the Bastion slam): an enemy attack
//! with `parry_window` publishes the tick range of its parryable phase. A punch whose view time
//! falls inside that range, with the enemy (rewound) within `melee_parry_range`, stuns it for
//! the attack's `parry_stun` and deals its `parry_damage`; if the strike already landed, its
//! damage is still pending (delayed like projectiles) and gets cancelled.
//!
//! **Reward**: heal to the effective max after clearing `parry_hard_damage_clear` of the hard
//! damage; [`SimEvent::Parried`] (client: hitstop, white flash, `Sfx::Parry`).

use engine::physics::MASK_SHOTS;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::projectiles::ProjectileKind;
use super::{HitTarget, HitboxHistory, PlayerId, SimEvent, SimWorld};
use crate::input::{PlayerInput, look_dir};
use crate::protocol::NetId;
use crate::tuning::Tuning;

/// What was parried.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParryTarget {
    /// A projectile was deflected (it keeps its id and now belongs to the parrier).
    Projectile { id: NetId, kind: ProjectileKind },
    /// A telegraphed enemy melee attack was countered (the enemy is stunned).
    Melee { enemy: NetId },
}

/// A successful parry ([`SimEvent::Parried`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParryEvent {
    pub player: PlayerId,
    pub target: ParryTarget,
    /// Where it happened (the projectile / enemy).
    pub position: Vec3,
    /// Health restored by the reward.
    pub healed: f32,
    /// Ticks between the attack touching the player and the parry (0 = parried before it
    /// hit; > 0 = saved by the network tolerance window).
    pub late_ticks: u32,
    pub tick: u32,
}

/// Identifies a parryable attack whose damage may be pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParryKey {
    /// An enemy melee attack: (enemy, attack sequence number).
    Melee(NetId, u32),
}

/// Damage waiting for the parry tolerance window to close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingHit {
    pub key: ParryKey,
    pub victim: PlayerId,
    pub info: DamageInfo,
    pub apply_tick: u32,
    pub created_tick: u32,
}

/// `value × multiplier`, but a parry never raises it above `cap` (a value already above the
/// cap is kept, not lowered).
fn parry_boost(value: f32, multiplier: f32, cap: f32) -> f32 {
    (value * multiplier).min(cap.max(value))
}

impl SimWorld {
    /// Ticks a parryable hit on `victim` is delayed by (see the module docs).
    pub fn parry_delay_ticks(&self, t: &Tuning, victim: PlayerId) -> u32 {
        let rate = self.tick_rate as f32;
        let tolerance = t.net.parry_tolerance_ms.max(0.0) / 1000.0 * rate;
        let lag = if t.combat.parry_compensate_view_lag {
            let max = self.max_rewind_ticks(t, victim) as f32;
            self.players
                .get(&victim)
                .map_or(0.0, |p| p.view_lag_ticks.clamp(0.0, max))
        } else {
            0.0
        };
        (tolerance + lag).ceil().max(0.0) as u32
    }

    /// `FireIntent::Punch`: melee hit + first parry check (the window stays open for the
    /// following ticks, see [`SimWorld::tick_parry_windows`]).
    pub(crate) fn punch(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) {
        let parried = self.try_parry(t, id);
        let hit = self.punch_hit(t, history, id, input);
        self.events.push(SimEvent::Punched {
            player: id,
            hit: hit || parried,
        });
    }

    /// The melee part of a punch: nearest hittable target (rewound to the view time) within
    /// range and cone, with line of sight.
    fn punch_hit(
        &mut self,
        t: &Tuning,
        history: Option<&dyn HitboxHistory>,
        id: PlayerId,
        input: &PlayerInput,
    ) -> bool {
        let Some(p) = self.players.get(&id) else {
            return false;
        };
        let c = &t.combat;
        let eye = p.movement.eye_position(&t.movement);
        let aim = input.look_dir();
        let half_cone = (c.punch_cone_degrees * 0.5).to_radians();
        let view = self.shot_view(t, history, id, input);
        let shooter = Owner::Player(id);
        let world = &self.collision.world;
        let best = view
            .hitboxes
            .iter()
            .filter(|h| self.can_hit(shooter, h.target))
            .filter_map(|h| {
                // Along the aim first (precise), else the closest shape in the cone.
                if let Some((d, part)) = h.ray_hit(eye, aim, c.punch_range) {
                    return Some((d, h.target, part, eye + aim * d));
                }
                let center = h.center()?;
                let (d, part) = h.distance_to_point(eye)?;
                let to = center - eye;
                (d <= c.punch_range && to.normalize_or_zero().angle_between(aim) <= half_cone)
                    .then_some((d.max(0.0), h.target, part, center))
            })
            .filter(|(_, _, _, point)| !world.line_blocked(eye, *point, MASK_SHOTS))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, target, part, point)) = best else {
            return false;
        };
        let flat = Vec3::new(aim.x, 0.0, aim.z).normalize_or(aim);
        let kb = (flat + Vec3::Y * 0.3).normalize_or(Vec3::Y) * c.punch_knockback;
        let info = DamageInfo::new(c.punch_damage, shooter, Attack::Fist)
            .at(point, aim)
            .part(part)
            .knockback(kb)
            .flags(DamageFlags::MELEE);
        self.apply_damage(t, target, info);
        true
    }

    /// Re-checks parries for every player whose parry window is open (called once per tick
    /// after projectiles moved, so projectiles arriving during the window are caught).
    pub(crate) fn tick_parry_windows(&mut self, t: &Tuning) {
        let open: Vec<PlayerId> = self
            .players
            .values()
            .filter(|p| !p.combat.dead && p.combat.weapons.parry > 0)
            .map(|p| p.id)
            .collect();
        for id in open {
            self.try_parry(t, id);
        }
    }

    /// Tests every parryable thing against player `id`'s current parry zone; deflects /
    /// counters all valid ones. Returns true if anything was parried.
    pub(crate) fn try_parry(&mut self, t: &Tuning, id: PlayerId) -> bool {
        let Some(p) = self.players.get(&id) else {
            return false;
        };
        if p.combat.dead {
            return false;
        }
        let c = &t.combat;
        let eye = p.movement.eye_position(&t.movement);
        let aim = look_dir(p.yaw, p.pitch);
        let now = self.tick;
        let max_lag = self.max_rewind_ticks(t, id);
        let view_time = now as f64 - (p.view_lag_ticks as f64).clamp(0.0, max_lag);
        let half_cone = (c.parry_cone_degrees * 0.5).to_radians();
        // In the cone within range, or touching-close anywhere in front (not from behind).
        let in_zone = |pos: Vec3| {
            let to = pos - eye;
            let d = to.length();
            let angle = to.normalize_or(aim).angle_between(aim);
            (d <= c.parry_close_range && angle <= std::f32::consts::FRAC_PI_2)
                || (d <= c.parry_range && angle <= half_cone)
        };

        let mut projectiles = Vec::new();
        for proj in self.projectiles.values() {
            if !proj.kind.parryable() || proj.stuck.is_some() {
                continue;
            }
            // Something you just deflected stays deflected.
            if proj.parried && proj.owner == Owner::Player(id) {
                continue;
            }
            // Own / teammates' projectiles only for kinds made to be punched (flak).
            let friendly = match proj.owner {
                Owner::Player(o) => o == id || !self.rules.friendly_fire,
                _ => false,
            };
            if friendly && !proj.kind.owner_can_parry() {
                continue;
            }
            let rewound = proj.trail.at(view_time);
            let current = match proj.pending {
                Some(pending) => {
                    if pending.victim != id {
                        // Someone else's incoming hit: only what we saw counts.
                        None
                    } else {
                        Some(proj.position - pending.incoming * 0.6)
                    }
                }
                None => Some(proj.position),
            };
            if rewound.is_some_and(in_zone) || current.is_some_and(in_zone) {
                let late = proj.pending.map_or(0, |pd| now.saturating_sub(pd.hit_tick));
                projectiles.push((proj.id, late));
            }
        }

        // Telegraphed melee attacks in their parryable phase.
        let mut melee = Vec::new();
        for e in self.enemies.values() {
            let Some(window) = e.parry_window else {
                continue;
            };
            let seen =
                view_time >= window.start as f64 - 1e-6 && view_time <= window.end as f64 + 1e-6;
            let current = now >= window.start && now <= window.end;
            if !(seen || current) {
                continue;
            }
            // Too late: the strike landed and its damage has already been applied.
            let key = ParryKey::Melee(e.id, window.seq);
            if now > window.end && !self.pending_hits.iter().any(|h| h.key == key) {
                continue;
            }
            let pos = e.trail.at(view_time).unwrap_or(e.position) + Vec3::Y * (e.height() * 0.5);
            let pos_now = e.position + Vec3::Y * (e.height() * 0.5);
            let range = c.melee_parry_range + e.radius();
            if pos.distance(eye) <= range || pos_now.distance(eye) <= range {
                melee.push((e.id, now.saturating_sub(window.end)));
            }
        }

        if projectiles.is_empty() && melee.is_empty() {
            return false;
        }
        let healed = self.parry_reward(t, id);
        let mut first = true;
        for (pid, late) in projectiles {
            if let Some(position) = self.deflect(t, id, pid) {
                let kind = self
                    .projectiles
                    .get(&pid)
                    .map_or(ProjectileKind::Rocket, |p| p.kind);
                self.events.push(SimEvent::Parried(ParryEvent {
                    player: id,
                    target: ParryTarget::Projectile { id: pid, kind },
                    position,
                    healed: if first { healed } else { 0.0 },
                    late_ticks: late,
                    tick: now,
                }));
                first = false;
            }
        }
        for (enemy, late) in melee {
            let position = self.counter_melee(t, id, enemy);
            self.events.push(SimEvent::Parried(ParryEvent {
                player: id,
                target: ParryTarget::Melee { enemy },
                position,
                healed: if first { healed } else { 0.0 },
                late_ticks: late,
                tick: now,
            }));
            first = false;
        }
        true
    }

    /// Deflects projectile `pid` for player `id`. Returns where it was parried.
    fn deflect(&mut self, t: &Tuning, id: PlayerId, pid: NetId) -> Option<Vec3> {
        let p = self.players.get(&id)?;
        let c = &t.combat;
        let eye = p.movement.eye_position(&t.movement);
        let aim = look_dir(p.yaw, p.pitch);
        let me = Owner::Player(id);
        let dir = self
            .auto_aim(
                t,
                eye,
                aim,
                me,
                c.parry_auto_aim_degrees,
                c.parry_auto_aim_range,
            )
            .map_or(aim, |target| (target - eye).normalize_or(aim));
        let now = self.tick;
        let launch = self.launch_tick_now();
        let lifetime_secs = self.projectiles.get(&pid)?.kind.lifetime(c).max(3.0);
        let lifetime = secs_to_ticks(lifetime_secs, self.tick_rate, 1) as u32;
        let proj = self.projectiles.get_mut(&pid)?;
        let where_ = proj.position;
        let mult = if proj.kind == ProjectileKind::Flak {
            c.thresher.flak_parry_speed_multiplier
        } else {
            c.parry_speed_multiplier
        };
        proj.owner = me;
        proj.parried = true;
        proj.pending = None;
        proj.gravity = 0.0;
        // Capped: two players punching one projectile back and forth must not grow it
        // geometrically (huge speeds made every sweep cover billions of grid cells).
        proj.damage = parry_boost(proj.damage, c.parry_damage_multiplier, c.parry_max_damage);
        if let Some(e) = &mut proj.explosion {
            e.damage = parry_boost(e.damage, c.parry_damage_multiplier, c.parry_max_damage);
        }
        proj.speed = parry_boost(proj.speed.max(1.0), mult, c.parry_max_speed);
        proj.position = eye + dir * 1.0;
        proj.velocity = dir * proj.speed;
        proj.expire_tick = now + lifetime;
        proj.contacts.clear();
        proj.trail.clear();
        proj.relaunch(launch);
        Some(where_)
    }

    /// Nearest hittable target (head / weak point if visible, else body) within `cone`
    /// degrees (full aperture) of `aim` and `range` metres, in line of sight.
    pub fn auto_aim(
        &self,
        t: &Tuning,
        eye: Vec3,
        aim: Vec3,
        shooter: Owner,
        cone_degrees: f32,
        range: f32,
    ) -> Option<Vec3> {
        let half = (cone_degrees * 0.5).to_radians();
        let world = &self.collision.world;
        let mut best: Option<(f32, Vec3)> = None;
        for h in self.current_hitboxes(t) {
            if !self.can_hit(shooter, h.target) {
                continue;
            }
            let Some(center) = h.center() else {
                continue;
            };
            let dist = center.distance(eye);
            if dist > range || best.is_some_and(|(d, _)| d <= dist) {
                continue;
            }
            let mut points: Vec<Vec3> = h
                .shapes
                .iter()
                .filter(|s| s.part != super::HitPart::Body)
                .map(|s| s.center())
                .collect();
            points.push(center);
            if let Some(p) = points.into_iter().find(|p| {
                (*p - eye).normalize_or_zero().angle_between(aim) <= half
                    && !world.line_blocked(eye, *p, MASK_SHOTS)
            }) {
                best = Some((dist, p));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Counter-hit of a parried telegraphed melee attack: cancel its pending damage, stun
    /// the enemy, deal the attack's parry damage.
    fn counter_melee(&mut self, t: &Tuning, id: PlayerId, enemy: NetId) -> Vec3 {
        let Some(e) = self.enemies.get_mut(&enemy) else {
            return Vec3::ZERO;
        };
        let position = e.position + Vec3::Y * (e.height() * 0.5);
        let (stun, damage, seq) = e.parry_stun();
        let key_seq = e.parry_window.map(|w| w.seq).unwrap_or(seq);
        e.stun(secs_to_ticks(stun, self.tick_rate, 1));
        self.pending_hits
            .retain(|h| h.key != ParryKey::Melee(enemy, key_seq));
        let dir = self.players.get(&id).map_or(Vec3::Z, |p| {
            (position - p.movement.eye_position(&t.movement)).normalize_or(Vec3::Z)
        });
        let info = DamageInfo::new(damage, Owner::Player(id), Attack::Fist)
            .at(position, dir)
            .flags(DamageFlags::MELEE | DamageFlags::PARRIED);
        self.apply_damage(t, HitTarget::Entity(enemy), info);
        position
    }

    /// Queues parry-delayed damage (telegraphed enemy attacks).
    pub fn queue_pending_hit(
        &mut self,
        t: &Tuning,
        key: ParryKey,
        victim: PlayerId,
        info: DamageInfo,
    ) {
        let delay = self.parry_delay_ticks(t, victim);
        self.pending_hits.push(PendingHit {
            key,
            victim,
            info,
            apply_tick: self.tick + delay,
            created_tick: self.tick,
        });
    }

    /// Applies pending hits whose tolerance window closed.
    pub(crate) fn apply_due_pending_hits(&mut self, t: &Tuning) {
        let now = self.tick;
        let (due, keep): (Vec<PendingHit>, Vec<PendingHit>) = self
            .pending_hits
            .drain(..)
            .partition(|h| h.apply_tick <= now);
        self.pending_hits = keep;
        for hit in due {
            self.apply_damage(t, HitTarget::Player(hit.victim), hit.info);
        }
    }

    /// Drops everything pending against a player (they died / left).
    pub(crate) fn cancel_pending_for_victim(&mut self, victim: PlayerId) {
        self.pending_hits.retain(|h| h.victim != victim);
        let ids: Vec<NetId> = self
            .projectiles
            .values()
            .filter(|p| p.pending.is_some_and(|pd| pd.victim == victim))
            .map(|p| p.id)
            .collect();
        for id in ids {
            self.projectiles.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sim::{BUILTIN_LEVEL, DEFAULT_TICK_RATE, TickInputs, builtin_level};

    #[test]
    fn parry_ping_pong_is_capped() {
        let t = Tuning::default();
        let level = Arc::new(builtin_level(BUILTIN_LEVEL).unwrap());
        let mut w = SimWorld::new(BUILTIN_LEVEL, level, DEFAULT_TICK_RATE);
        let (a, b) = (PlayerId(0), PlayerId(1));
        w.add_player(a, "A").movement.position = Vec3::new(0.0, 0.0, 10.0);
        w.add_player(b, "B").movement.position = Vec3::new(0.0, 0.0, -10.0);
        w.players.get_mut(&a).unwrap().yaw = 0.0; // facing -Z, at B
        w.players.get_mut(&b).unwrap().yaw = std::f32::consts::PI;
        let rocket = w
            .spawn_projectile(
                &t,
                ProjectileKind::Rocket,
                Owner::World,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 45.0),
            )
            .unwrap();
        let explosion = w.projectiles[&rocket].explosion.unwrap().damage;
        // Two players punching it back and forth: 1.5^n grew without bound, and the swept box
        // of every step soon covered billions of grid cells (seconds per tick).
        for i in 0..100 {
            let who = if i % 2 == 0 { a } else { b };
            assert!(w.deflect(&t, who, rocket).is_some());
            let p = &w.projectiles[&rocket];
            assert!(p.speed <= t.combat.parry_max_speed, "parry {i}: {}", p.speed);
            assert!(p.velocity.length() <= t.combat.parry_max_speed + 1e-3);
            let e = p.explosion.unwrap().damage;
            assert!(p.damage <= t.combat.parry_max_damage && e <= t.combat.parry_max_damage);
        }
        let p = &w.projectiles[&rocket];
        assert_eq!(p.speed, t.combat.parry_max_speed);
        assert_eq!(p.explosion.unwrap().damage, t.combat.parry_max_damage);
        // One parry is still a full boost.
        assert!(explosion * t.combat.parry_damage_multiplier < t.combat.parry_max_damage);
        // The world keeps ticking quickly with the capped projectile in flight.
        let start = std::time::Instant::now();
        for _ in 0..30 {
            w.step(&TickInputs::new(), &t, None);
        }
        assert!(start.elapsed().as_secs_f32() < 2.0);

        // A projectile already beyond the caps keeps its values (parrying never weakens it).
        let fast = w
            .spawn_projectile(&t, ProjectileKind::Rocket, Owner::World, Vec3::Y, Vec3::ZERO)
            .unwrap();
        {
            let p = w.projectiles.get_mut(&fast).unwrap();
            p.speed = 400.0;
            p.damage = 1000.0;
        }
        w.deflect(&t, a, fast).unwrap();
        let p = &w.projectiles[&fast];
        assert_eq!((p.speed, p.damage), (400.0, 1000.0));
    }
}
