//! Server-simulated projectiles and explosions.
//!
//! Every projectile is a swept sphere: each tick it moves along its (ballistic) trajectory,
//! sweeping against the level (`MASK_SHOTS`) and against the current hitboxes of everything
//! its owner may hurt (the owner itself and co-op teammates are never hit). What happens on
//! contact depends on the [`ProjectileKind`]: rivets and enemy orbs deal direct damage,
//! sawblades pierce (damage per contact interval) and bounce off walls, explosives explode,
//! stickies stick to the world or to whatever they hit.
//!
//! **Parryable hits are delayed**: when a parryable projectile (rocket, flak, mortar, enemy
//! orbs/bolts) touches a player it freezes in a *pending impact* for
//! [`SimWorld::parry_delay_ticks`] (`net.parry_tolerance_ms`, plus the victim's view lag when
//! `combat.parry_compensate_view_lag`), so a parry that the victim made on what they saw — and
//! that reaches the server a little later — still cancels it. See `parry.rs`.
//!
//! **Replication** (see `docs/combat.md`): projectiles are replicated by trajectory, not by
//! position: `position`/`velocity` are the state at `aux` = launch tick, and clients compute
//! `pos(t) = p0 + v0·Δ − ½·g·Δ²·ŷ` (Δ = (t − aux)/tick_rate; g = the kind's gravity, 0 when
//! `PARRIED`). The entity only changes when the trajectory changes (bounce, parry, stick), so
//! a sky full of rivets costs almost no bandwidth.

use engine::physics::{MASK_SHOTS, SweepHit};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::trail::Trail;
use super::{HitPart, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::protocol::{EntityState, NetId, kind, owner_code, projectile_flags};
use crate::tuning::{CombatTuning, Tuning};

/// Every projectile type. The replicated entity `kind` is [`ProjectileKind::net_kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProjectileKind {
    /// Riveter primary / Volley.
    Rivet,
    /// Riveter alt A: piercing, bouncing disc.
    Sawblade,
    /// Cinderbox primary.
    Rocket,
    /// Thresher alt B grenade (parryable by its owner too: punch it to launch it).
    Flak,
    /// Cinderbox alt A adhesive charge.
    Sticky,
    /// Cinderbox alt B lob.
    Mortar,
    /// Spitter's slow orb (enemy, parryable).
    SpitterOrb,
    /// Gnat's bolt (enemy, parryable).
    GnatBolt,
}

impl ProjectileKind {
    pub const ALL: [ProjectileKind; 8] = [
        ProjectileKind::Rivet,
        ProjectileKind::Sawblade,
        ProjectileKind::Rocket,
        ProjectileKind::Flak,
        ProjectileKind::Sticky,
        ProjectileKind::Mortar,
        ProjectileKind::SpitterOrb,
        ProjectileKind::GnatBolt,
    ];

    pub fn net_kind(self) -> u16 {
        match self {
            ProjectileKind::Rivet => kind::RIVET,
            ProjectileKind::Sawblade => kind::SAWBLADE,
            ProjectileKind::Rocket => kind::ROCKET,
            ProjectileKind::Flak => kind::FLAK,
            ProjectileKind::Sticky => kind::STICKY,
            ProjectileKind::Mortar => kind::MORTAR,
            ProjectileKind::SpitterOrb => kind::SPITTER_ORB,
            ProjectileKind::GnatBolt => kind::GNAT_BOLT,
        }
    }

    pub fn from_net_kind(k: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.net_kind() == k)
    }

    /// Downward acceleration (m/s²) of an un-parried projectile of this kind.
    pub fn gravity(self, t: &CombatTuning) -> f32 {
        match self {
            ProjectileKind::Rivet | ProjectileKind::Sawblade | ProjectileKind::Rocket => 0.0,
            ProjectileKind::Flak => t.thresher.flak_gravity,
            ProjectileKind::Sticky => t.cinderbox.sticky_gravity,
            ProjectileKind::Mortar => t.cinderbox.mortar_gravity,
            ProjectileKind::SpitterOrb => t.enemy_projectiles.orb_gravity,
            ProjectileKind::GnatBolt => t.enemy_projectiles.bolt_gravity,
        }
    }

    /// Collision radius.
    pub fn radius(self, t: &CombatTuning) -> f32 {
        match self {
            ProjectileKind::Rivet => t.riveter.rivet_radius,
            ProjectileKind::Sawblade => t.riveter.saw_radius,
            ProjectileKind::Rocket => t.cinderbox.rocket_projectile_radius,
            ProjectileKind::Flak => t.thresher.flak_projectile_radius,
            ProjectileKind::Sticky => 0.15,
            ProjectileKind::Mortar => 0.25,
            ProjectileKind::SpitterOrb => t.enemy_projectiles.orb_radius,
            ProjectileKind::GnatBolt => t.enemy_projectiles.bolt_radius,
        }
    }

    /// Seconds before it expires (explosives explode then: the flak fuse).
    pub fn lifetime(self, t: &CombatTuning) -> f32 {
        match self {
            ProjectileKind::Rivet => t.riveter.rivet_lifetime,
            ProjectileKind::Sawblade => t.riveter.saw_lifetime,
            ProjectileKind::Rocket => t.cinderbox.rocket_lifetime,
            ProjectileKind::Flak => t.thresher.flak_fuse,
            ProjectileKind::Sticky => t.cinderbox.sticky_lifetime,
            ProjectileKind::Mortar => 10.0,
            ProjectileKind::SpitterOrb => t.enemy_projectiles.orb_lifetime,
            ProjectileKind::GnatBolt => t.enemy_projectiles.bolt_lifetime,
        }
    }

    /// Can be deflected by a punch.
    pub fn parryable(self) -> bool {
        matches!(
            self,
            ProjectileKind::Rocket
                | ProjectileKind::Flak
                | ProjectileKind::Mortar
                | ProjectileKind::SpitterOrb
                | ProjectileKind::GnatBolt
        )
    }

    /// The shooter (and co-op teammates) may punch their own (the flak launch combo).
    pub fn owner_can_parry(self) -> bool {
        self == ProjectileKind::Flak
    }

    pub fn default_attack(self) -> Attack {
        match self {
            ProjectileKind::Rivet => Attack::Rivet,
            ProjectileKind::Sawblade => Attack::Sawblade,
            ProjectileKind::Rocket => Attack::Rocket,
            ProjectileKind::Flak => Attack::Flak,
            ProjectileKind::Sticky => Attack::Sticky,
            ProjectileKind::Mortar => Attack::Mortar,
            ProjectileKind::SpitterOrb | ProjectileKind::GnatBolt => {
                Attack::Enemy { kind: 0, attack: 0 }
            }
        }
    }

    /// Explosion of explosive kinds.
    pub fn explosive(self, t: &CombatTuning) -> Option<Explosive> {
        let (radius, damage, knockback) = match self {
            ProjectileKind::Rocket => (
                t.cinderbox.rocket_radius,
                t.cinderbox.rocket_damage,
                t.cinderbox.rocket_knockback,
            ),
            ProjectileKind::Flak => (
                t.thresher.flak_radius,
                t.thresher.flak_damage,
                t.thresher.flak_knockback,
            ),
            ProjectileKind::Sticky => (
                t.cinderbox.sticky_radius,
                t.cinderbox.sticky_damage,
                t.cinderbox.sticky_knockback,
            ),
            ProjectileKind::Mortar => (
                t.cinderbox.mortar_radius,
                t.cinderbox.mortar_damage,
                t.cinderbox.mortar_knockback,
            ),
            _ => return None,
        };
        Some(Explosive {
            radius,
            damage,
            knockback,
        })
    }
}

/// An explosion's size and strength.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Explosive {
    pub radius: f32,
    /// Damage at the centre (falls off linearly to `explosion_edge_fraction` at the radius).
    pub damage: f32,
    /// Knockback speed at the centre.
    pub knockback: f32,
}

/// What a stuck projectile is attached to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stuck {
    /// `None` = the level.
    pub to: Option<HitTarget>,
    /// Offset from the carrier's position (feet).
    pub offset: Vec3,
}

/// A parryable projectile touched a player; its effect is applied at `apply_tick` unless a
/// parry deflects it first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingImpact {
    pub victim: PlayerId,
    /// Tick it touched the victim.
    pub hit_tick: u32,
    pub apply_tick: u32,
    pub part: HitPart,
    /// Direction it was travelling when it hit.
    pub incoming: Vec3,
}

/// A live projectile.
#[derive(Clone, Debug)]
pub struct Projectile {
    pub id: NetId,
    pub kind: ProjectileKind,
    pub owner: Owner,
    /// Who fired it originally (differs from `owner` after a parry).
    pub original_owner: Owner,
    pub attack: Attack,
    pub position: Vec3,
    pub velocity: Vec3,
    pub radius: f32,
    pub gravity: f32,
    /// Direct-hit damage (rivets, saw per contact interval, enemy orbs...).
    pub damage: f32,
    pub explosion: Option<Explosive>,
    pub spawn_tick: u32,
    pub expire_tick: u32,
    /// Speed it was launched with (deflections scale this).
    pub speed: f32,
    /// Replicated trajectory: state at `launch_tick`.
    pub launch_tick: u32,
    pub launch_position: Vec3,
    pub launch_velocity: Vec3,
    pub bounces_left: u32,
    /// Deflected by a punch (now flies straight, ×damage).
    pub parried: bool,
    pub stuck: Option<Stuck>,
    pub pending: Option<PendingImpact>,
    /// Positions at the end of each tick (parry rewinding).
    pub trail: Trail,
    /// Sawblade: tick each target may be damaged again.
    pub contacts: Vec<(HitTarget, u32)>,
}

impl Projectile {
    /// Starts a new trajectory segment from the current state (bounce, parry, stick).
    pub fn relaunch(&mut self, tick: u32) {
        self.launch_tick = tick;
        self.launch_position = self.position;
        self.launch_velocity = self.velocity;
    }

    pub fn is_frozen(&self) -> bool {
        self.stuck.is_some() || self.pending.is_some()
    }

    /// Replicated state (see the module docs).
    pub fn entity_state(&self) -> EntityState {
        let mut flags = 0;
        if self.parried {
            flags |= projectile_flags::PARRIED;
        }
        if self.stuck.is_some() {
            flags |= projectile_flags::STUCK;
        }
        if self.pending.is_some() {
            flags |= projectile_flags::PENDING;
        }
        if matches!(self.original_owner, Owner::Enemy(_)) {
            flags |= projectile_flags::ENEMY;
        }
        EntityState {
            id: self.id,
            kind: self.kind.net_kind(),
            owner: owner_code(self.owner),
            position: self.launch_position,
            velocity: self.launch_velocity,
            flags,
            state: self.bounces_left.min(u16::MAX as u32) as u16,
            aux: self.launch_tick,
            ..Default::default()
        }
    }
}

impl SimWorld {
    /// Launch tick for a trajectory starting now at the current position. Before the
    /// projectile step of this tick, the step will still move it by one tick, so it counts as
    /// launched one tick ago (exactly what the replicated trajectory formula gives for this
    /// tick); after it, it starts now.
    pub(crate) fn launch_tick_now(&self) -> u32 {
        if self.projectiles_stepped {
            self.tick
        } else {
            self.tick.saturating_sub(1)
        }
    }

    /// Spawns a projectile with its kind's defaults from tuning (damage, radius, lifetime,
    /// explosion). Returns its id; callers may adjust fields afterwards. `None` (nothing
    /// spawned) while [`super::MAX_PROJECTILES`] are in flight.
    pub fn spawn_projectile(
        &mut self,
        t: &Tuning,
        kind: ProjectileKind,
        owner: Owner,
        position: Vec3,
        velocity: Vec3,
    ) -> Option<NetId> {
        if self.projectiles.len() >= super::MAX_PROJECTILES {
            log::debug!("projectile limit reached: {kind:?} not spawned");
            return None;
        }
        let id = self.alloc_net_id()?;
        let c = &t.combat;
        let now = self.tick;
        let damage = match kind {
            ProjectileKind::Rivet => c.riveter.rivet_damage,
            ProjectileKind::Sawblade => c.riveter.saw_damage,
            _ => 0.0,
        };
        let lifetime = secs_to_ticks(kind.lifetime(c), self.tick_rate, 1) as u32;
        let launch_tick = self.launch_tick_now();
        let p = Projectile {
            id,
            kind,
            owner,
            original_owner: owner,
            attack: kind.default_attack(),
            position,
            velocity,
            radius: kind.radius(c),
            gravity: kind.gravity(c),
            damage,
            explosion: kind.explosive(c),
            spawn_tick: now,
            expire_tick: now + lifetime,
            speed: velocity.length(),
            launch_tick,
            launch_position: position,
            launch_velocity: velocity,
            bounces_left: if kind == ProjectileKind::Sawblade {
                c.riveter.saw_bounces
            } else {
                0
            },
            parried: false,
            stuck: None,
            pending: None,
            trail: Trail::new(),
            contacts: Vec::new(),
        };
        self.projectiles.insert(id, p);
        Some(id)
    }

    /// Steps every projectile one tick (after player inputs, before enemies act).
    pub(crate) fn step_projectiles(&mut self, t: &Tuning) {
        let boxes = self.current_hitboxes(t);
        let ids: Vec<NetId> = self.projectiles.keys().copied().collect();
        for id in ids {
            let Some(mut p) = self.projectiles.remove(&id) else {
                continue;
            };
            if self.step_projectile(t, &mut p, &boxes) {
                self.projectiles.insert(id, p);
            }
        }
        let tick = self.tick;
        for p in self.projectiles.values_mut() {
            p.trail.record(tick, p.position);
        }
        // Live sticky counts (authoritative for the owners' weapon state).
        for pl in self.players.values_mut() {
            pl.combat.weapons.stickies = 0;
        }
        for p in self.projectiles.values() {
            if p.kind == ProjectileKind::Sticky
                && let Owner::Player(owner) = p.owner
                && let Some(pl) = self.players.get_mut(&owner)
            {
                pl.combat.weapons.stickies = pl.combat.weapons.stickies.saturating_add(1);
            }
        }
    }

    /// One tick of one projectile. Returns false when it is gone.
    fn step_projectile(&mut self, t: &Tuning, p: &mut Projectile, boxes: &[Hitbox]) -> bool {
        let now = self.tick;
        if let Some(pending) = p.pending {
            if now >= pending.apply_tick {
                self.resolve_pending(t, p, pending);
                return false;
            }
            return true;
        }
        if let Some(stuck) = p.stuck {
            if let Some(target) = stuck.to {
                match self.target_origin(target) {
                    Some(origin) => p.position = origin + stuck.offset,
                    None => {
                        p.stuck = Some(Stuck {
                            to: None,
                            offset: Vec3::ZERO,
                        })
                    }
                }
                p.relaunch(now);
            }
            if now >= p.expire_tick {
                self.explode_projectile(t, p, None);
                return false;
            }
            return true;
        }
        if now >= p.expire_tick {
            if p.explosion.is_some() {
                self.explode_projectile(t, p, None);
            }
            return false;
        }

        let dt = self.dt();
        let motion = p.velocity * dt - Vec3::Y * (0.5 * p.gravity * dt * dt);
        let new_velocity = p.velocity - Vec3::Y * (p.gravity * dt);
        let world_hit = self
            .collision
            .world
            .cast_sphere(p.position, p.radius, motion, MASK_SHOTS);
        let world_f = world_hit.map_or(1.0, |h| h.fraction);
        let dir = motion.normalize_or(p.velocity.normalize_or(Vec3::NEG_Z));

        if p.kind == ProjectileKind::Sawblade {
            self.saw_contacts(t, p, motion * world_f, boxes);
        } else {
            let first = boxes
                .iter()
                .filter(|h| self.can_hit(p.owner, h.target))
                .filter_map(|h| {
                    h.sweep_sphere(p.position, motion, p.radius)
                        .map(|(f, part)| (f, h.target, part))
                })
                .filter(|(f, _, _)| *f <= world_f)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((f, target, part)) = first {
                p.position += motion * f;
                p.velocity = new_velocity;
                return self.projectile_hit_target(t, p, target, part, dir);
            }
        }

        if let Some(hit) = world_hit {
            p.position += motion * hit.fraction + hit.normal * 0.02;
            p.velocity = new_velocity;
            return self.projectile_hit_world(t, p, hit);
        }
        p.position += motion;
        p.velocity = new_velocity;
        true
    }

    /// Feet position of a player or enemy.
    pub(crate) fn target_origin(&self, target: HitTarget) -> Option<Vec3> {
        match target {
            HitTarget::Player(id) => self
                .players
                .get(&id)
                .filter(|p| !p.combat.dead)
                .map(|p| p.movement.position),
            HitTarget::Entity(id) => self
                .enemies
                .get(&id)
                .filter(|e| e.is_alive())
                .map(|e| e.position),
        }
    }

    fn projectile_hit_target(
        &mut self,
        t: &Tuning,
        p: &mut Projectile,
        target: HitTarget,
        part: HitPart,
        dir: Vec3,
    ) -> bool {
        let now = self.tick;
        if p.kind == ProjectileKind::Sticky {
            let offset = self
                .target_origin(target)
                .map_or(Vec3::ZERO, |o| p.position - o);
            p.stuck = Some(Stuck {
                to: Some(target),
                offset,
            });
            p.velocity = Vec3::ZERO;
            p.relaunch(now);
            return true;
        }
        if let HitTarget::Player(victim) = target
            && p.kind.parryable()
        {
            let delay = self.parry_delay_ticks(t, victim);
            p.pending = Some(PendingImpact {
                victim,
                hit_tick: now,
                apply_tick: now + delay,
                part,
                incoming: dir,
            });
            p.velocity = Vec3::ZERO;
            p.relaunch(now);
            return true;
        }
        if p.explosion.is_some() {
            self.explode_projectile(t, p, Some(target));
            return false;
        }
        self.projectile_direct_damage(t, p, target, part, dir);
        false
    }

    fn projectile_direct_damage(
        &mut self,
        t: &Tuning,
        p: &Projectile,
        target: HitTarget,
        part: HitPart,
        dir: Vec3,
    ) {
        let mut flags = DamageFlags::PROJECTILE;
        if p.parried {
            flags.insert(DamageFlags::PARRIED);
            if p.original_owner.as_target() == Some(target) {
                flags.insert(DamageFlags::RETURNED);
            }
        }
        let mut info = DamageInfo::new(p.damage, p.owner, p.attack)
            .at(p.position, dir)
            .part(part)
            .flags(flags);
        info.projectile = Some(p.id);
        self.apply_damage(t, target, info);
    }

    /// A pending (parry-delayed) impact that was not parried in time.
    fn resolve_pending(&mut self, t: &Tuning, p: &mut Projectile, pending: PendingImpact) {
        let target = HitTarget::Player(pending.victim);
        if p.explosion.is_some() {
            self.explode_projectile(t, p, Some(target));
        } else {
            self.projectile_direct_damage(t, p, target, pending.part, pending.incoming);
        }
    }

    fn projectile_hit_world(&mut self, t: &Tuning, p: &mut Projectile, hit: SweepHit) -> bool {
        let now = self.tick;
        match p.kind {
            ProjectileKind::Sawblade if p.bounces_left > 0 => {
                let n = hit.normal;
                p.velocity -= n * (2.0 * p.velocity.dot(n));
                p.bounces_left -= 1;
                p.relaunch(now);
                self.events.push(SimEvent::ProjectileBounced {
                    id: p.id,
                    kind: p.kind,
                    position: p.position,
                });
                true
            }
            ProjectileKind::Sticky => {
                p.stuck = Some(Stuck {
                    to: None,
                    offset: Vec3::ZERO,
                });
                p.velocity = Vec3::ZERO;
                p.relaunch(now);
                true
            }
            _ if p.explosion.is_some() => {
                p.position += hit.normal * 0.1;
                self.explode_projectile(t, p, None);
                false
            }
            _ => {
                self.events.push(SimEvent::ProjectileImpact {
                    id: p.id,
                    kind: p.kind,
                    position: p.position,
                    normal: hit.normal,
                });
                false
            }
        }
    }

    /// Sawblade: damages everything it overlaps along this tick's motion, once per
    /// `saw_damage_interval` per target.
    fn saw_contacts(&mut self, t: &Tuning, p: &mut Projectile, motion: Vec3, boxes: &[Hitbox]) {
        let now = self.tick;
        let interval =
            secs_to_ticks(t.combat.riveter.saw_damage_interval, self.tick_rate, 1) as u32;
        p.contacts.retain(|(_, next)| *next > now);
        let touching: Vec<(HitTarget, HitPart)> = boxes
            .iter()
            .filter(|h| self.can_hit(p.owner, h.target))
            .filter_map(|h| {
                h.sweep_sphere(p.position, motion, p.radius)
                    .map(|(_, part)| (h.target, part))
            })
            .collect();
        let dir = motion.normalize_or(Vec3::NEG_Z);
        for (target, part) in touching {
            if p.contacts.iter().any(|(c, _)| *c == target) {
                continue;
            }
            p.contacts.push((target, now + interval));
            let mut info = DamageInfo::new(p.damage, p.owner, p.attack)
                .at(p.position, dir)
                .part(part)
                .flags(DamageFlags::PROJECTILE);
            if p.parried {
                info.flags.insert(DamageFlags::PARRIED);
            }
            info.projectile = Some(p.id);
            self.apply_damage(t, target, info);
        }
    }

    fn explode_projectile(&mut self, t: &Tuning, p: &Projectile, direct: Option<HitTarget>) {
        let Some(spec) = p.explosion else {
            return;
        };
        let mut flags = DamageFlags::NONE;
        if p.parried {
            flags.insert(DamageFlags::PARRIED);
        }
        self.explode(
            t,
            p.position,
            spec,
            p.owner,
            p.attack,
            flags,
            Some(p.id),
            direct,
            p.original_owner,
        );
    }

    /// An explosion: damage with linear falloff and line of sight from the centre, and
    /// knockback away from the centre (with an upward bias) to everything in the radius —
    /// including the owner (self-damage fraction, full knockback: rocket jumps) and co-op
    /// teammates (knockback only).
    #[allow(clippy::too_many_arguments)]
    pub fn explode(
        &mut self,
        t: &Tuning,
        center: Vec3,
        spec: Explosive,
        owner: Owner,
        attack: Attack,
        flags: DamageFlags,
        projectile: Option<NetId>,
        direct: Option<HitTarget>,
        original_owner: Owner,
    ) {
        self.events.push(SimEvent::Explosion {
            position: center,
            radius: spec.radius,
            attack,
            owner,
        });
        let c = &t.combat;
        let world = &self.collision.world;
        let mut hits = Vec::new();
        for h in self.current_hitboxes(t) {
            let alive = match h.target {
                HitTarget::Player(id) => self.players.get(&id).is_some_and(|p| !p.combat.dead),
                HitTarget::Entity(id) => self.enemies.get(&id).is_some_and(|e| e.is_alive()),
            };
            if !alive {
                continue;
            }
            let Some((d, _)) = h.distance_to_point(center) else {
                continue;
            };
            let d = d.max(0.0);
            let is_direct = direct == Some(h.target) || d <= 0.05;
            if d > spec.radius && !is_direct {
                continue;
            }
            let aim = h.center().unwrap_or(center);
            if !is_direct {
                let head = h.part_center(HitPart::Head);
                let blocked = world.line_blocked(center, aim, MASK_SHOTS)
                    && head.is_none_or(|hp| world.line_blocked(center, hp, MASK_SHOTS));
                if blocked {
                    continue;
                }
            }
            let frac = 1.0
                - (1.0 - c.explosion_edge_fraction) * (d / spec.radius.max(1e-3)).clamp(0.0, 1.0);
            let to = aim - center;
            let dir = if to.length() < 0.05 {
                Vec3::Y
            } else {
                to.normalize()
            };
            let kb = (dir + Vec3::Y * c.explosion_up_bias).normalize_or(Vec3::Y)
                * (spec.knockback * frac);
            let mut info = DamageInfo::new(spec.damage * frac, owner, attack)
                .at(aim, dir)
                .knockback(kb)
                .flags(flags | DamageFlags::EXPLOSIVE);
            if is_direct {
                info.flags.insert(DamageFlags::DIRECT);
            }
            if flags.contains(DamageFlags::PARRIED) && original_owner.as_target() == Some(h.target)
            {
                info.flags.insert(DamageFlags::RETURNED);
            }
            info.projectile = projectile;
            hits.push((h.target, info));
        }
        for (target, info) in hits {
            self.apply_damage(t, target, info);
        }
    }

    /// Detonates every sticky of `player` (Cinderbox alt A pressed again).
    pub fn detonate_stickies(&mut self, t: &Tuning, player: PlayerId) {
        let ids: Vec<NetId> = self
            .projectiles
            .values()
            .filter(|p| p.kind == ProjectileKind::Sticky && p.owner == Owner::Player(player))
            .map(|p| p.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.events.push(SimEvent::StickiesDetonated {
            player,
            count: ids.len() as u32,
        });
        for id in ids {
            if let Some(p) = self.projectiles.remove(&id) {
                self.explode_projectile(t, &p, None);
            }
        }
        if let Some(p) = self.players.get_mut(&player) {
            p.combat.weapons.stickies = 0;
        }
    }

    /// Spawns an enemy projectile (for enemy attacks): `damage` per direct hit. `None` when
    /// the projectile limit is reached.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_enemy_projectile(
        &mut self,
        t: &Tuning,
        kind: ProjectileKind,
        enemy: NetId,
        attack: Attack,
        position: Vec3,
        velocity: Vec3,
        damage: f32,
    ) -> Option<NetId> {
        let id = self.spawn_projectile(t, kind, Owner::Enemy(enemy), position, velocity)?;
        if let Some(p) = self.projectiles.get_mut(&id) {
            p.damage = damage;
            p.attack = attack;
        }
        Some(id)
    }
}

/// Client helper: where a trajectory-replicated entity (projectile or prism) is at
/// `time_ticks` (fractional server ticks, e.g. the render tick). Other kinds: `position`.
///
/// `position`/`velocity` are the state at launch tick `aux`; gravity is the kind's (prisms:
/// `prism.gravity`), 0 for `PARRIED` projectiles; `STUCK`/`PENDING` projectiles and `RESTING`
/// prisms are at `position`.
pub fn replicated_position(
    e: &EntityState,
    time_ticks: f64,
    tick_rate: u32,
    t: &CombatTuning,
) -> Vec3 {
    let gravity = if e.kind == kind::PRISM {
        if e.flags & crate::protocol::prism_flags::RESTING != 0 {
            return e.position;
        }
        t.prism.gravity
    } else if let Some(k) = ProjectileKind::from_net_kind(e.kind) {
        if e.flags & (projectile_flags::STUCK | projectile_flags::PENDING) != 0 {
            return e.position;
        }
        if e.flags & projectile_flags::PARRIED != 0 {
            0.0
        } else {
            k.gravity(t)
        }
    } else {
        return e.position;
    };
    let dt = ((time_ticks - e.aux as f64) / tick_rate.max(1) as f64).max(0.0) as f32;
    e.position + e.velocity * dt - Vec3::Y * (0.5 * gravity * dt * dt)
}
