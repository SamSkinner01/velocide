//! Server-simulated projectiles and explosions.
//!
//! Every projectile is a swept sphere: each tick it moves along its (ballistic) trajectory,
//! sweeping against the level (`MASK_SHOTS`) and against the current hitboxes of everything
//! its owner may hurt (the owner itself and co-op teammates are never hit; player-owned
//! projectiles get hit assist, see [`SimWorld::hit_pads`]). What happens on contact depends on
//! the [`ProjectileKind`]: needles deal direct damage, ember bombs explode, Tumbler bombs
//! bounce off the level and explode on their last contact, on a hostile or when the fuse runs
//! out, the Gravewell sticks to the level, pulls hostiles in and implodes, the Wisp drifts
//! through targets zapping the nearest one and pops on the level.
//!
//! **Parryable hits are delayed**: when a parryable projectile (enemy ember bombs / needles)
//! touches a player it freezes in a *pending impact* for
//! [`SimWorld::parry_delay_ticks`] (`net.parry_tolerance_ms`, plus the victim's view lag when
//! `combat.parry_compensate_view_lag`), so a parry that the victim made on what they saw — and
//! that reaches the server a little later — still cancels it. See `parry.rs`.
//!
//! **Replication** (see `docs/combat.md`): projectiles are replicated by trajectory, not by
//! position: `position`/`velocity` are the state at `aux` = launch tick, and clients compute
//! `pos(t) = p0 + v0·Δ − ½·g·Δ²·ŷ` (Δ = (t − aux)/tick_rate; g = the kind's gravity, 0 when
//! `PARRIED`). The entity only changes when the trajectory changes (bounce, parry, stick), so
//! a sky full of projectiles costs almost no bandwidth.

use engine::physics::{MASK_SHOTS, SweepHit};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::combat::{Attack, DamageFlags, DamageInfo, Owner, secs_to_ticks};
use super::trail::Trail;
use super::{HitPads, HitPart, HitTarget, Hitbox, PlayerId, SimEvent, SimWorld};
use crate::protocol::{EntityState, NetId, kind, owner_code, projectile_flags, thorn_flags};
use crate::tuning::{CombatTuning, Tuning};

/// Every projectile type. The replicated entity `kind` is [`ProjectileKind::net_kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProjectileKind {
    /// Tumbler primary: bouncing bomb (`bounces_left` = world bounces before it explodes).
    Bomb,
    /// Tumbler alt B: sticks to the level, pulls hostiles in, implodes.
    Gravewell,
    /// Arclash alt A: slow ball of lightning that zaps the nearest hostile.
    Wisp,
    /// The Censer's lobbed ember bomb (enemy, ballistic, explodes on impact, parryable).
    EmberBomb,
    /// The Heron's needle (enemy, fast and straight, parryable).
    Needle,
    // ---- expansion arsenal (docs/arsenal.md) ----
    /// Longwick arrow (also Triple Nock's): ballistic, direct damage by the draw.
    Arrow,
    /// Riveter hot rivet: fast, light gravity, direct damage.
    Rivet,
    /// Cinderbox rocket (one of a salvo): straight, explodes on contact.
    CinderRocket,
    /// Flak shell: straight, airbursts into a ring of pellets.
    FlakShell,
    /// Limpet sticky charge.
    Limpet,
    /// Mortar shell: lobbed high, wide blast on contact.
    MortarShell,
    /// Flare: lobbed, leaves a burning patch where it lands.
    Flare,
    /// Longwick pinning arrow.
    PinArrow,
    /// Longwick ember arrow: bursts into flame on impact.
    EmberArrow,
    /// Longwick branding arrow (Brand on the bow).
    BrandArrow,
    // ---- refit (docs/arsenal.md): melee and knacks ----
    /// Hurl (melee ability): a spinning blade / hammer-head, direct damage.
    Hurl,
    /// Kindle Flask (knack): lobbed, leaves a healing pool where it lands.
    KindleFlask,
    /// Snare (knack): thrown to the floor, lies there as a trap (`sim::knacks`).
    Snare,
    /// Soot Pot (knack): thrown, bursts into a soot cloud.
    SootPot,
}

impl ProjectileKind {
    pub const ALL: [ProjectileKind; 19] = [
        ProjectileKind::Bomb,
        ProjectileKind::Gravewell,
        ProjectileKind::Wisp,
        ProjectileKind::EmberBomb,
        ProjectileKind::Needle,
        ProjectileKind::Arrow,
        ProjectileKind::Rivet,
        ProjectileKind::CinderRocket,
        ProjectileKind::FlakShell,
        ProjectileKind::Limpet,
        ProjectileKind::MortarShell,
        ProjectileKind::Flare,
        ProjectileKind::PinArrow,
        ProjectileKind::EmberArrow,
        ProjectileKind::BrandArrow,
        ProjectileKind::Hurl,
        ProjectileKind::KindleFlask,
        ProjectileKind::Snare,
        ProjectileKind::SootPot,
    ];

    pub fn net_kind(self) -> u16 {
        match self {
            ProjectileKind::Bomb => kind::BOMB,
            ProjectileKind::Gravewell => kind::GRAVEWELL,
            ProjectileKind::Wisp => kind::WISP,
            ProjectileKind::EmberBomb => kind::EMBER_BOMB,
            ProjectileKind::Needle => kind::NEEDLE,
            ProjectileKind::Arrow => kind::ARROW,
            ProjectileKind::Rivet => kind::RIVET,
            ProjectileKind::CinderRocket => kind::CINDER_ROCKET,
            ProjectileKind::FlakShell => kind::FLAK_SHELL,
            ProjectileKind::Limpet => kind::LIMPET,
            ProjectileKind::MortarShell => kind::MORTAR_SHELL,
            ProjectileKind::Flare => kind::FLARE,
            ProjectileKind::PinArrow => kind::PIN_ARROW,
            ProjectileKind::EmberArrow => kind::EMBER_ARROW,
            ProjectileKind::BrandArrow => kind::BRAND_ARROW,
            ProjectileKind::Hurl => kind::HURL,
            ProjectileKind::KindleFlask => kind::KINDLE_FLASK,
            ProjectileKind::Snare => kind::SNARE,
            ProjectileKind::SootPot => kind::SOOT_POT,
        }
    }

    /// A Longwick arrow of any sort.
    pub fn is_arrow(self) -> bool {
        matches!(
            self,
            ProjectileKind::Arrow
                | ProjectileKind::PinArrow
                | ProjectileKind::EmberArrow
                | ProjectileKind::BrandArrow
        )
    }

    pub fn from_net_kind(k: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.net_kind() == k)
    }

    /// Downward acceleration (m/s²) of an un-parried projectile of this kind.
    pub fn gravity(self, t: &CombatTuning) -> f32 {
        match self {
            ProjectileKind::Bomb => t.tumbler.bomb_gravity,
            ProjectileKind::Gravewell => t.tumbler.gravewell_gravity,
            ProjectileKind::Wisp => 0.0,
            ProjectileKind::EmberBomb => t.enemy_projectiles.ember_gravity,
            ProjectileKind::Needle => t.enemy_projectiles.needle_gravity,
            k if k.is_arrow() => t.longwick.arrow_gravity,
            ProjectileKind::Rivet => t.riveter.rivet_gravity,
            ProjectileKind::Limpet => t.limpet.gravity,
            ProjectileKind::MortarShell => t.mortar.gravity,
            ProjectileKind::Flare => t.flare.gravity,
            ProjectileKind::Hurl => t.hurl.gravity,
            ProjectileKind::KindleFlask => t.knacks.kindle_flask.gravity,
            ProjectileKind::Snare => t.knacks.snare.gravity,
            ProjectileKind::SootPot => t.knacks.soot_pot.gravity,
            _ => 0.0,
        }
    }

    /// Collision radius.
    pub fn radius(self, t: &CombatTuning) -> f32 {
        match self {
            ProjectileKind::Bomb => t.tumbler.bomb_projectile_radius,
            ProjectileKind::Gravewell => t.tumbler.gravewell_projectile_radius,
            ProjectileKind::Wisp => t.arclash.wisp_radius,
            ProjectileKind::EmberBomb => t.enemy_projectiles.ember_radius,
            ProjectileKind::Needle => t.enemy_projectiles.needle_radius,
            k if k.is_arrow() => t.longwick.arrow_radius,
            ProjectileKind::Rivet => t.riveter.rivet_radius,
            ProjectileKind::CinderRocket => t.cinderbox.rocket_radius,
            ProjectileKind::FlakShell => t.flak.shell_radius,
            ProjectileKind::Limpet => t.limpet.projectile_radius,
            ProjectileKind::MortarShell => t.mortar.projectile_radius,
            ProjectileKind::Hurl => t.hurl.projectile_radius,
            ProjectileKind::KindleFlask => t.knacks.kindle_flask.projectile_radius,
            ProjectileKind::Snare => t.knacks.snare.projectile_radius,
            ProjectileKind::SootPot => t.knacks.soot_pot.projectile_radius,
            _ => t.flare.projectile_radius,
        }
    }

    /// Seconds before it expires (explosives explode then: the bomb fuse, the wisp pop).
    pub fn lifetime(self, t: &CombatTuning) -> f32 {
        match self {
            ProjectileKind::Bomb => t.tumbler.bomb_fuse,
            ProjectileKind::Gravewell => t.tumbler.gravewell_flight_time,
            ProjectileKind::Wisp => t.arclash.wisp_lifetime,
            ProjectileKind::EmberBomb => t.enemy_projectiles.ember_lifetime,
            ProjectileKind::Needle => t.enemy_projectiles.needle_lifetime,
            k if k.is_arrow() => t.longwick.arrow_lifetime,
            ProjectileKind::Rivet => t.riveter.rivet_lifetime,
            ProjectileKind::CinderRocket => t.cinderbox.rocket_lifetime,
            // It bursts where its flight ends.
            ProjectileKind::FlakShell => t.flak.burst_distance / t.flak.shell_speed.max(1e-3),
            ProjectileKind::Limpet => t.limpet.fuse,
            ProjectileKind::MortarShell => t.mortar.flight_time,
            ProjectileKind::Hurl => t.hurl.flight_time,
            ProjectileKind::KindleFlask => t.knacks.kindle_flask.flight_time,
            // Thrown and lying on the floor: the trap's whole life.
            ProjectileKind::Snare => t.knacks.snare.lifetime,
            ProjectileKind::SootPot => t.knacks.soot_pot.flight_time,
            _ => t.flare.flight_time,
        }
    }

    /// Can be deflected by a punch (and touching a player is parry-delayed). Players'
    /// projectiles are not; a Gust deflects any hostile projectile.
    pub fn parryable(self) -> bool {
        matches!(self, ProjectileKind::EmberBomb | ProjectileKind::Needle)
    }

    /// The shooter (and co-op teammates) may punch their own. No current kind.
    pub fn owner_can_parry(self) -> bool {
        false
    }

    /// Collides with hit shapes in flight (the Gravewell and the Wisp pass through targets, a
    /// thrown Snare falls to the floor past them).
    pub fn hits_targets(self) -> bool {
        !matches!(
            self,
            ProjectileKind::Gravewell | ProjectileKind::Wisp | ProjectileKind::Snare
        )
    }

    pub fn default_attack(self) -> Attack {
        match self {
            ProjectileKind::Bomb => Attack::Tumble,
            ProjectileKind::Gravewell => Attack::Gravewell,
            ProjectileKind::Wisp => Attack::Wisp,
            ProjectileKind::EmberBomb | ProjectileKind::Needle => {
                Attack::Enemy { kind: 0, attack: 0 }
            }
            ProjectileKind::Arrow => Attack::Longwick,
            ProjectileKind::Rivet => Attack::Riveter,
            ProjectileKind::CinderRocket => Attack::Cinderbox,
            ProjectileKind::FlakShell => Attack::Flak,
            ProjectileKind::Limpet => Attack::Limpet,
            ProjectileKind::MortarShell => Attack::Mortar,
            ProjectileKind::Flare => Attack::Flare,
            ProjectileKind::PinArrow => Attack::PinningArrow,
            ProjectileKind::EmberArrow => Attack::EmberArrow,
            ProjectileKind::BrandArrow => Attack::Brand,
            ProjectileKind::Hurl => Attack::Hurl,
            ProjectileKind::Snare => Attack::Snare,
            // They deal no damage.
            ProjectileKind::KindleFlask | ProjectileKind::SootPot => Attack::World,
        }
    }

    /// Explosion of explosive kinds.
    pub fn explosive(self, t: &CombatTuning) -> Option<Explosive> {
        let (radius, damage, knockback) = match self {
            ProjectileKind::Bomb => (
                t.tumbler.bomb_radius,
                t.tumbler.bomb_damage,
                t.tumbler.bomb_knockback,
            ),
            ProjectileKind::Gravewell => (
                t.tumbler.gravewell_radius,
                t.tumbler.gravewell_damage,
                t.tumbler.gravewell_knockback,
            ),
            ProjectileKind::Wisp => (
                t.arclash.wisp_pop_radius,
                t.arclash.wisp_pop_damage,
                t.arclash.wisp_pop_knockback,
            ),
            // Damage comes from the enemy's attack (`spawn_enemy_projectile`).
            ProjectileKind::EmberBomb => (
                t.enemy_projectiles.ember_explosion_radius,
                0.0,
                t.enemy_projectiles.ember_knockback,
            ),
            ProjectileKind::CinderRocket => (
                t.cinderbox.explosion_radius,
                t.cinderbox.damage,
                t.cinderbox.knockback,
            ),
            ProjectileKind::Limpet => (
                t.limpet.explosion_radius,
                t.limpet.damage,
                t.limpet.knockback,
            ),
            ProjectileKind::MortarShell => (
                t.mortar.explosion_radius,
                t.mortar.damage,
                t.mortar.knockback,
            ),
            ProjectileKind::EmberArrow => (
                t.ember_arrow.explosion_radius,
                t.ember_arrow.damage,
                t.ember_arrow.knockback,
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
    /// Direct-hit damage (enemy projectiles; the Wisp: per zap).
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
    /// Deflected by a punch or a Gust (now flies straight, ×damage).
    pub parried: bool,
    pub stuck: Option<Stuck>,
    pub pending: Option<PendingImpact>,
    /// Positions at the end of each tick (parry rewinding).
    pub trail: Trail,
    /// Per-target re-hit timers (target, tick it may be damaged again) for kinds that touch
    /// targets repeatedly; cleared by a deflection.
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
            ProjectileKind::Wisp => c.arclash.wisp_zap_damage,
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
            bounces_left: if kind == ProjectileKind::Bomb {
                c.tumbler.bomb_contacts.saturating_sub(1)
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
            if p.kind == ProjectileKind::Gravewell {
                self.gravewell_pull(t, p);
            }
            return true;
        }
        if now >= p.expire_tick {
            if !self.payload_expired(t, p) && p.explosion.is_some() {
                self.explode_projectile(t, p, None);
            }
            return false;
        }
        if p.kind == ProjectileKind::Wisp {
            self.wisp_zap(t, p, boxes);
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

        if p.kind.hits_targets() {
            let first = boxes
                .iter()
                .filter(|h| self.can_hit(p.owner, h.target))
                .filter_map(|h| {
                    let pads = self.hit_pads(t, p.owner, h.target);
                    h.sweep_sphere_padded(p.position, motion, p.radius, pads)
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
        // The expansion's sticky charges, airbursts, flares and special arrows (payload.rs).
        if let Some(keep) = self.payload_hit_target(t, p, target, part, dir) {
            return keep;
        }
        if p.explosion.is_some() {
            self.explode_projectile(t, p, Some(target));
            return false;
        }
        self.projectile_direct_damage(t, p, target, part, dir);
        false
    }

    pub(crate) fn projectile_direct_damage(
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
        if let Some(keep) = self.payload_hit_world(t, p, hit) {
            return keep;
        }
        match p.kind {
            ProjectileKind::Bomb if p.bounces_left > 0 && !p.parried => {
                let n = hit.normal;
                p.velocity = (p.velocity - n * (2.0 * p.velocity.dot(n)))
                    * t.combat.tumbler.bomb_restitution;
                p.bounces_left -= 1;
                p.relaunch(now);
                self.events.push(SimEvent::ProjectileBounced {
                    id: p.id,
                    kind: p.kind,
                    position: p.position,
                });
                true
            }
            ProjectileKind::Gravewell if !p.parried => {
                // Sticks where it lands, pulls for `gravewell_pull_time`, then implodes.
                p.stuck = Some(Stuck {
                    to: None,
                    offset: Vec3::ZERO,
                });
                p.velocity = Vec3::ZERO;
                p.expire_tick = now
                    + secs_to_ticks(t.combat.tumbler.gravewell_pull_time, self.tick_rate, 1) as u32;
                p.relaunch(now);
                self.events.push(SimEvent::GravewellStuck {
                    id: p.id,
                    position: p.position,
                });
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

    /// Gravewell (stuck): pulls every hostile enemy within `gravewell_pull_radius` towards
    /// its centre at up to `gravewell_pull_speed` × the enemy's `knockback_scale` (hostile
    /// players in PvP at `gravewell_player_pull` of that), easing off within 2 m of the centre.
    /// Pulled enemies are kept staggered so their own steering doesn't fight it.
    fn gravewell_pull(&mut self, t: &Tuning, p: &Projectile) {
        let tb = &t.combat.tumbler;
        let center = p.position;
        let pull = |d: f32| tb.gravewell_pull_speed * (d / 2.0).clamp(0.0, 1.0);
        let targets: Vec<HitTarget> = self
            .enemies
            .keys()
            .map(|id| HitTarget::Entity(*id))
            .chain(self.players.keys().map(|id| HitTarget::Player(*id)))
            .filter(|target| self.can_hit(p.owner, *target))
            .collect();
        for target in targets {
            match target {
                HitTarget::Entity(id) => {
                    let Some(e) = self.enemies.get_mut(&id) else {
                        continue;
                    };
                    let to = center - e.center();
                    let d = to.length();
                    if d > tb.gravewell_pull_radius || d < 1e-3 {
                        continue;
                    }
                    let dir = to / d;
                    let speed = pull(d) * e.def.knockback_scale;
                    let along = e.velocity.dot(dir);
                    if along < speed {
                        e.velocity += dir * (speed - along);
                        if e.velocity.y > 0.5 {
                            e.on_ground = false;
                        }
                    }
                    e.stagger = e.stagger.max(2);
                }
                HitTarget::Player(id) => {
                    let Some(pl) = self.players.get_mut(&id) else {
                        continue;
                    };
                    let (body, _, _) = pl.movement.capsule(&t.movement);
                    let to = center - body;
                    let d = to.length();
                    if d > tb.gravewell_pull_radius || d < 1e-3 {
                        continue;
                    }
                    let dir = to / d;
                    let speed = pull(d) * tb.gravewell_player_pull;
                    let along = pl.movement.velocity.dot(dir);
                    if along < speed {
                        pl.movement.velocity += dir * (speed - along);
                    }
                }
            }
        }
    }

    /// Wisp: every `wisp_zap_interval` it zaps the nearest hostile within `wisp_zap_range`
    /// (hit assist, line of sight) for its damage. Emits [`SimEvent::WispZap`].
    fn wisp_zap(&mut self, t: &Tuning, p: &Projectile, boxes: &[Hitbox]) {
        let a = &t.combat.arclash;
        let interval = secs_to_ticks(a.wisp_zap_interval, self.tick_rate, 1) as u32;
        let age = self.tick.wrapping_sub(p.spawn_tick);
        if age == 0 || !age.is_multiple_of(interval) {
            return;
        }
        let world = &self.collision.world;
        let best = boxes
            .iter()
            .filter(|h| self.can_hit(p.owner, h.target))
            .filter_map(|h| {
                let pads = self.hit_pads(t, p.owner, h.target);
                let (d, _) = h.distance_to_point_padded(p.position, pads)?;
                let aim = h.center()?;
                (d <= a.wisp_zap_range && !world.line_blocked(p.position, aim, MASK_SHOTS))
                    .then_some((d, h.target, aim))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, target, aim)) = best else {
            return;
        };
        let dir = (aim - p.position).normalize_or(Vec3::NEG_Z);
        let mut info = DamageInfo::new(p.damage, p.owner, p.attack)
            .at(aim, dir)
            .flags(DamageFlags::HITSCAN);
        if p.parried {
            info.flags.insert(DamageFlags::PARRIED);
        }
        info.projectile = Some(p.id);
        self.events.push(SimEvent::WispZap {
            id: p.id,
            from: p.position,
            to: aim,
        });
        self.apply_damage(t, target, info);
    }

    /// Explodes `p` (if it is explosive); returns everything the blast reached.
    pub(crate) fn explode_projectile(
        &mut self,
        t: &Tuning,
        p: &Projectile,
        direct: Option<HitTarget>,
    ) -> Vec<HitTarget> {
        let Some(spec) = p.explosion else {
            return Vec::new();
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
        )
    }

    /// An explosion: damage with linear falloff and line of sight from the centre, and
    /// knockback away from the centre (with an upward bias) to everything in the radius —
    /// including the owner (self-damage fraction, full knockback: blast jumps) and co-op
    /// teammates (knockback only). Player-owned explosions measure the distance to the
    /// shapes inflated by the hit assist. Returns every target the blast reached.
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
    ) -> Vec<HitTarget> {
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
            // Hit assist for player-owned explosions (not on the owner itself).
            let pads = if owner.as_target() == Some(h.target) {
                HitPads::NONE
            } else {
                self.hit_pads(t, owner, h.target)
            };
            let Some((d, _)) = h.distance_to_point_padded(center, pads) else {
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
        let reached = hits.iter().map(|(target, _)| *target).collect();
        for (target, info) in hits {
            self.apply_damage(t, target, info);
        }
        reached
    }

    /// Spawns an enemy projectile (for enemy attacks): `damage` per direct hit, or at the
    /// centre of its explosion for explosive kinds (ember bombs). `None` when the projectile
    /// limit is reached.
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
            if let Some(e) = &mut p.explosion {
                e.damage = damage;
            }
        }
        Some(id)
    }
}

/// Client helper: where a trajectory-replicated entity (projectile or thorn) is at
/// `time_ticks` (fractional server ticks, e.g. the render tick). Other kinds: `position`.
///
/// `position`/`velocity` are the state at launch tick `aux`; gravity is the kind's (thorns:
/// `thorn.gravity`), 0 for `PARRIED` projectiles; `STUCK`/`PENDING` projectiles and `PLANTED`
/// thorns are at `position`.
pub fn replicated_position(
    e: &EntityState,
    time_ticks: f64,
    tick_rate: u32,
    t: &CombatTuning,
) -> Vec3 {
    let gravity = if e.kind == kind::THORN {
        if e.flags & thorn_flags::PLANTED != 0 {
            return e.position;
        }
        t.thorn.gravity
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
