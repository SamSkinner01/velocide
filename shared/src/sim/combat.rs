//! Combat core: the vocabulary every combat system shares ([`Owner`], [`Attack`],
//! [`DamageFlags`], [`DamageInfo`]), player vitals ([`PlayerCombat`]), the **single damage
//! pipeline** ([`SimWorld::apply_damage`]) and the default death/respawn rule.
//!
//! Everything that hurts anything goes through `apply_damage`: it applies the rules (brands,
//! dash i-frames, spawn protection, friendly fire, self damage, armour, weak points, hard
//! damage), knockback, ember bursts, aggro and kills, and emits [`SimEvent::Damage`] /
//! [`SimEvent::Killed`]. Those events are what the server turns into hit markers, damage
//! indicators and the kill feed, and what the style system scores.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::weapons::WeaponState;
use super::{HitPart, HitTarget, PlayerId, SimEvent, SimWorld};
use crate::loadout::{Ability, WeaponKind};
use crate::protocol::{NetId, kind};
use crate::tuning::{CombatTuning, Tuning};

/// Who caused something (damage, a projectile, an explosion).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Owner {
    /// The level (kill volumes) or nobody.
    #[default]
    World,
    Player(PlayerId),
    Enemy(NetId),
}

impl Owner {
    pub fn player(self) -> Option<PlayerId> {
        match self {
            Owner::Player(p) => Some(p),
            _ => None,
        }
    }

    pub fn enemy(self) -> Option<NetId> {
        match self {
            Owner::Enemy(id) => Some(id),
            _ => None,
        }
    }

    /// The owner as a hit target (players and enemies; `None` for the world).
    pub fn as_target(self) -> Option<HitTarget> {
        match self {
            Owner::World => None,
            Owner::Player(p) => Some(HitTarget::Player(p)),
            Owner::Enemy(id) => Some(HitTarget::Entity(id)),
        }
    }
}

/// What dealt damage: a weapon's primary fire or an ability, fist, slam, an enemy attack or the
/// world. Stable identifiers for the kill feed, style scoring ("variety") and sounds. An
/// ability is the same attack whichever weapon carries it ([`KillInfo::weapon`] says which).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Attack {
    /// Quill primary: precision hitscan quill.
    #[default]
    Quill,
    /// Quill alt A: marking shot (brands what it hits).
    Brand,
    /// Quill alt B: the main shot and each of the splinters it bursts into.
    Splinter,
    /// Bellows primary (one pellet).
    Blast,
    /// Bellows alt A: compressed-air cone (knockback, deflects projectiles). A projectile it
    /// turned around deals its damage as `Gust` too.
    Gust,
    /// Bellows alt B: both barrels at once (one pellet).
    BothBarrels,
    /// Arclash primary: one zap (and each chain jump).
    Arc,
    /// Arclash alt A: the ball of lightning's zaps and its pop.
    Wisp,
    /// Arclash alt B: the charged nova.
    Discharge,
    /// Tumbler primary: bouncing bomb.
    Tumble,
    /// Tumbler alt A: point-blank charge.
    Breach,
    /// Tumbler alt B: the canister's implosion.
    Gravewell,
    /// A thrown thorn hitting something in flight.
    Thorn,
    /// Punch (also the counter damage of a melee parry).
    Fist,
    /// Slam landing shockwave.
    Slam,
    /// An enemy attack: the enemy's replicated `kind` and the attack index in its definition.
    /// A deflected enemy projectile keeps this attack (with [`DamageFlags::PARRIED`]).
    Enemy { kind: u16, attack: u8 },
    /// Kill volumes / falling out of the level.
    World,
    /// Dev commands (`/kill`, `/killall`).
    Dev,

    // ---- expansion primaries (docs/arsenal.md) ----
    /// Lancet primary: a needle (hitscan).
    Lancet,
    /// Thresher primary (one pellet).
    Thresher,
    /// Riveter primary: one hot rivet.
    Riveter,
    /// Cinderbox primary: one cinder rocket of the salvo.
    Cinderbox,
    /// Longwick primary: a plain arrow (every draw).
    Longwick,
    /// Kiln primary: the ember spray (and the Burn it sets).
    Kiln,
    // ---- expansion abilities ----
    /// Charged piercing shot.
    Skewer,
    /// One shot of the three-shot burst.
    Flurry,
    /// The heavy slug.
    Slug,
    /// The flak shell's burst pellets.
    Flak,
    /// One shot of the volley.
    Volley,
    /// The flare and its burning patch.
    Flare,
    /// A sticky charge's blast.
    Limpet,
    /// The lobbed shell's blast.
    Mortar,
    /// One arrow of the fan.
    TripleNock,
    /// The pinning arrow.
    PinningArrow,
    /// The ember arrow's hit and flame burst.
    EmberArrow,
}

impl Attack {
    /// The primary fire of `weapon`.
    pub fn primary(weapon: WeaponKind) -> Attack {
        match weapon {
            WeaponKind::Quill => Attack::Quill,
            WeaponKind::Bellows => Attack::Blast,
            WeaponKind::Arclash => Attack::Arc,
            WeaponKind::Tumbler => Attack::Tumble,
            WeaponKind::Lancet => Attack::Lancet,
            WeaponKind::Thresher => Attack::Thresher,
            WeaponKind::Riveter => Attack::Riveter,
            WeaponKind::Cinderbox => Attack::Cinderbox,
            WeaponKind::Longwick => Attack::Longwick,
            WeaponKind::Kiln => Attack::Kiln,
        }
    }

    /// The attack of `ability` (on any weapon).
    pub fn ability(ability: Ability) -> Attack {
        match ability {
            Ability::Brand => Attack::Brand,
            Ability::Splinter => Attack::Splinter,
            Ability::Gust => Attack::Gust,
            Ability::BothBarrels => Attack::BothBarrels,
            Ability::Wisp => Attack::Wisp,
            Ability::Discharge => Attack::Discharge,
            Ability::Breach => Attack::Breach,
            Ability::Gravewell => Attack::Gravewell,
            Ability::Skewer => Attack::Skewer,
            Ability::Flurry => Attack::Flurry,
            Ability::Slug => Attack::Slug,
            Ability::Flak => Attack::Flak,
            Ability::Volley => Attack::Volley,
            Ability::Flare => Attack::Flare,
            Ability::Limpet => Attack::Limpet,
            Ability::Mortar => Attack::Mortar,
            Ability::TripleNock => Attack::TripleNock,
            Ability::PinningArrow => Attack::PinningArrow,
            Ability::EmberArrow => Attack::EmberArrow,
        }
    }

    /// The weapon whose primary fire this is.
    pub fn primary_of(self) -> Option<WeaponKind> {
        WeaponKind::ALL
            .into_iter()
            .find(|w| Attack::primary(*w) == self)
    }

    /// The ability this attack is.
    pub fn as_ability(self) -> Option<Ability> {
        Ability::ALL
            .into_iter()
            .find(|a| Attack::ability(*a) == self)
    }

    /// The weapon this attack belongs to: the weapon of a primary fire, or the only weapon
    /// that can carry an ability (the bow's arrows). `None` for abilities several weapons can
    /// carry (the kill's [`KillInfo::weapon`] knows which one fired), thorn, fist, slam,
    /// enemies and the world.
    pub fn weapon(self) -> Option<WeaponKind> {
        if let Some(w) = self.primary_of() {
            return Some(w);
        }
        let ability = self.as_ability()?;
        let mut carriers = WeaponKind::ALL.into_iter().filter(|w| ability.fits(*w));
        match (carriers.next(), carriers.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    }

    /// Display name (kill feed).
    pub fn name(self) -> &'static str {
        match self {
            Attack::Quill => "Quill",
            Attack::Brand => "Brand",
            Attack::Splinter => "Splinter",
            Attack::Blast => "Bellows",
            Attack::Gust => "Gust",
            Attack::BothBarrels => "Both Barrels",
            Attack::Arc => "Arclash",
            Attack::Wisp => "Wisp",
            Attack::Discharge => "Discharge",
            Attack::Tumble => "Tumbler",
            Attack::Breach => "Breach",
            Attack::Gravewell => "Gravewell",
            Attack::Thorn => "Thorn",
            Attack::Fist => "Fist",
            Attack::Slam => "Slam",
            Attack::Enemy { .. } => "Enemy",
            Attack::World => "World",
            Attack::Dev => "Dev",
            Attack::Lancet => "Lancet",
            Attack::Thresher => "Thresher",
            Attack::Riveter => "Riveter",
            Attack::Cinderbox => "Cinderbox",
            Attack::Longwick => "Longwick",
            Attack::Kiln => "Kiln",
            Attack::Skewer => "Skewer",
            Attack::Flurry => "Flurry",
            Attack::Slug => "Slug",
            Attack::Flak => "Flak",
            Attack::Volley => "Volley",
            Attack::Flare => "Flare",
            Attack::Limpet => "Limpet",
            Attack::Mortar => "Mortar",
            Attack::TripleNock => "Triple Nock",
            Attack::PinningArrow => "Pinning Arrow",
            Attack::EmberArrow => "Ember Arrow",
        }
    }
}

/// Properties of a damage instance (bit set). Carried in [`DamageInfo`], hit confirms and
/// kill events; the style system keys bonuses off them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DamageFlags(pub u32);

impl DamageFlags {
    pub const NONE: DamageFlags = DamageFlags(0);
    /// Instant ray (Quill shots and splinters, Bellows pellets, Arclash zaps).
    pub const HITSCAN: DamageFlags = DamageFlags(1 << 0);
    /// Direct projectile impact (thorn, enemy projectiles...).
    pub const PROJECTILE: DamageFlags = DamageFlags(1 << 1);
    /// Explosion splash (bomb, breach, gravewell implosion, wisp pop).
    pub const EXPLOSIVE: DamageFlags = DamageFlags(1 << 2);
    /// Punch / melee swipe.
    pub const MELEE: DamageFlags = DamageFlags(1 << 3);
    /// Dealt by a deflected (parried) projectile, or the counter-hit of a melee parry.
    pub const PARRIED: DamageFlags = DamageFlags(1 << 4);
    /// The shot ricocheted off at least one thorn (see [`DamageInfo::ricochets`]).
    pub const RICOCHET: DamageFlags = DamageFlags(1 << 5);
    /// Slam shockwave.
    pub const SLAM: DamageFlags = DamageFlags(1 << 6);
    /// Hit a head (player head or an enemy weak point of kind `Head`).
    pub const HEADSHOT: DamageFlags = DamageFlags(1 << 7);
    /// Hit any enemy weak point (head or other).
    pub const WEAK_POINT: DamageFlags = DamageFlags(1 << 8);
    /// The victim is the source (own explosion).
    pub const SELF: DamageFlags = DamageFlags(1 << 9);
    /// An Arclash zap that jumped from another target (chain).
    pub const CHAINED: DamageFlags = DamageFlags(1 << 10);
    /// A deflected projectile hit the one who originally fired it (RETURN TO SENDER).
    pub const RETURNED: DamageFlags = DamageFlags(1 << 11);
    /// Kill volume / out of the level.
    pub const ENVIRONMENT: DamageFlags = DamageFlags(1 << 12);
    /// Armour reduced this hit.
    pub const ARMOURED: DamageFlags = DamageFlags(1 << 13);
    /// The victim was standing in the explosion's centre / hit directly by the explosive.
    pub const DIRECT: DamageFlags = DamageFlags(1 << 14);
    /// The victim was branded: the damage was multiplied by `quill.brand_multiplier`.
    pub const BRANDED: DamageFlags = DamageFlags(1 << 15);
    /// Burn damage over time (status effect, `status.rs`).
    pub const BURN: DamageFlags = DamageFlags(1 << 16);
    /// A Skewer shot that passed through at least one target before this one.
    pub const PIERCED: DamageFlags = DamageFlags(1 << 17);

    pub fn contains(self, other: DamageFlags) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }

    pub fn insert(&mut self, other: DamageFlags) {
        self.0 |= other.0;
    }
}

impl std::ops::BitOr for DamageFlags {
    type Output = DamageFlags;
    fn bitor(self, rhs: DamageFlags) -> DamageFlags {
        DamageFlags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for DamageFlags {
    fn bitor_assign(&mut self, rhs: DamageFlags) {
        self.0 |= rhs.0;
    }
}

/// One damage instance, as handed to [`SimWorld::apply_damage`]. `amount` is the *base*
/// damage: weak-point/head multipliers (from `part`), armour, self-damage scaling and the
/// other rules are applied by the pipeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DamageInfo {
    pub amount: f32,
    pub source: Owner,
    pub attack: Attack,
    /// Where it hit (world space).
    pub point: Vec3,
    /// Direction the damage travels (shot direction, away from an explosion). Unit or zero.
    pub dir: Vec3,
    pub part: HitPart,
    /// Velocity change applied to the target (players: `MoveState::velocity`; enemies:
    /// scaled by their `knockback_scale`).
    pub knockback: Vec3,
    pub flags: DamageFlags,
    /// Thorn bounces before this hit (each multiplied the damage).
    pub ricochets: u8,
    /// The projectile that dealt it, if any.
    pub projectile: Option<NetId>,
}

impl DamageInfo {
    pub fn new(amount: f32, source: Owner, attack: Attack) -> Self {
        Self {
            amount,
            source,
            attack,
            ..Default::default()
        }
    }

    pub fn at(mut self, point: Vec3, dir: Vec3) -> Self {
        self.point = point;
        self.dir = dir.normalize_or_zero();
        self
    }

    pub fn part(mut self, part: HitPart) -> Self {
        self.part = part;
        self
    }

    pub fn knockback(mut self, knockback: Vec3) -> Self {
        self.knockback = knockback;
        self
    }

    pub fn flags(mut self, flags: DamageFlags) -> Self {
        self.flags |= flags;
        self
    }
}

/// Why a damage instance did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Blocked {
    /// Target missing or already dead.
    NoTarget,
    /// Dash invulnerability frames.
    Invulnerable,
    SpawnProtection,
    /// Co-op teammate (knockback from explosions still applies).
    FriendlyFire,
    /// Enemies don't hurt enemies.
    SameTeam,
    /// `/god`.
    God,
    /// Non-explosive damage to oneself.
    SelfDamage,
}

/// Outcome of one damage instance ([`SimEvent::Damage`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageEvent {
    pub target: HitTarget,
    pub info: DamageInfo,
    /// Damage actually removed from health (after every rule and multiplier).
    pub dealt: f32,
    /// Hard damage added (players).
    pub hard: f32,
    pub health_after: f32,
    pub killed: bool,
    pub blocked: Option<Blocked>,
    pub tick: u32,
}

/// A kill ([`SimEvent::Killed`], forwarded to every client as `GameEvent::Killed`). Rich on
/// purpose: the style system derives HEADSHOT, AIRKILL, AIRSHOT, MULTIKILL (from `tick`),
/// RETURN TO SENDER, RICOCHET, EXPLODED, SLAM KILL, FISTED, CLOSE CALL and FRAG from it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillInfo {
    pub victim: HitTarget,
    /// Replicated kind of the victim (`kind::PLAYER` or the enemy's kind).
    pub victim_kind: u16,
    pub killer: Owner,
    pub attack: Attack,
    /// Flags of the killing blow (HEADSHOT, EXPLOSIVE, PARRIED, RICOCHET, MELEE, SLAM...).
    pub flags: DamageFlags,
    pub part: HitPart,
    pub position: Vec3,
    /// Thorn bounces of the killing shot.
    pub ricochets: u8,
    /// The killer (a player) was in the air.
    pub killer_airborne: bool,
    /// The victim was in the air.
    pub victim_airborne: bool,
    /// The killing projectile had been deflected by the killer (RETURN TO SENDER).
    pub parried_projectile: bool,
    /// Killer's health and effective max health at the time (players; 0 otherwise) — for
    /// CLOSE CALL.
    pub killer_health: f32,
    pub killer_effective_max: f32,
    /// Damage beyond what was needed (how violently it breaks apart).
    pub overkill: f32,
    /// Metres from the killer (player or enemy) to the victim when it died; 0 when the killer
    /// has no body (world, dev, gone). Kill feed ("34 m") and the LONGSHOT style bonus.
    #[serde(default)]
    pub distance: f32,
    /// The weapon that fired the killing blow ([`SimWorld::attack_weapon`]): the weapon of a
    /// primary, or the killer's weapon carrying the ability (`None`: fist, thorn, slam,
    /// enemies, the world).
    #[serde(default)]
    pub weapon: Option<WeaponKind>,
    pub tick: u32,
}

/// A player's combat state. Server-authoritative; sent unquantised to the owning client in
/// `OwnState` (HUD and weapon prediction). Health is stored as damage taken so `Default` is a
/// fresh, full-health player independent of tuning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerCombat {
    /// Health lost (0 = full). `health = max_health - damage`.
    pub damage: f32,
    /// Hard damage: lowers the effective max health (`max_health - hard`). Always `<= damage`.
    pub hard: f32,
    /// Ticks until hard damage starts recovering.
    pub hard_delay: u16,
    pub dead: bool,
    /// Ticks until respawn while dead (default death rule).
    pub respawn_in: u16,
    /// Ticks of spawn protection left.
    pub protection: u16,
    /// Dev `/god`: never takes damage.
    pub god: bool,
    /// Weapons, fist, thorns (predictable by the owning client).
    pub weapons: WeaponState,
    /// Tick of the last damage taken (0 = never).
    pub last_hurt_tick: u32,
    /// Who dealt it (death cam / "killed by" while dead).
    pub last_attacker: Owner,
    /// With what.
    pub last_attack: Attack,
}

impl PlayerCombat {
    pub fn health(&self, t: &CombatTuning) -> f32 {
        (t.max_health - self.damage).max(0.0)
    }

    pub fn effective_max(&self, t: &CombatTuning) -> f32 {
        (t.max_health - self.hard).max(0.0)
    }

    pub fn is_alive(&self) -> bool {
        !self.dead
    }

    /// Heals up to `amount`, never above the effective max. Returns the amount healed.
    pub fn heal(&mut self, amount: f32) -> f32 {
        if self.dead || amount <= 0.0 {
            return 0.0;
        }
        let before = self.damage;
        self.damage = (self.damage - amount).max(self.hard).max(0.0);
        before - self.damage
    }

    /// Health missing below the effective max.
    pub fn missing(&self) -> f32 {
        (self.damage - self.hard).max(0.0)
    }
}

/// Game-mode dependent combat rules. Defaults come from the level kind (arena = PvP).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CombatRules {
    /// Players can damage each other (PvP). Off in co-op (knockback still applies).
    pub friendly_fire: bool,
    /// Scale enemy health with the player count: `× (1 + 0.5 (players − 1))`.
    pub coop_enemy_scaling: bool,
}

impl CombatRules {
    pub fn pvp() -> Self {
        Self {
            friendly_fire: true,
            coop_enemy_scaling: false,
        }
    }

    pub fn coop() -> Self {
        Self {
            friendly_fire: false,
            coop_enemy_scaling: true,
        }
    }
}

/// Converts seconds to whole ticks (at least `min`).
pub fn secs_to_ticks(secs: f32, tick_rate: u32, min: u16) -> u16 {
    let ticks = (secs.max(0.0) * tick_rate as f32).round();
    (ticks.min(u16::MAX as f32) as u16).max(min)
}

impl SimWorld {
    /// The weapon `owner` dealt `attack` with: a primary's weapon; for an ability, the
    /// owner's carried weapon that has it (the one in hand first), else the only weapon that
    /// can carry it. `None` for fist, thorn, slam, enemies and the world.
    pub fn attack_weapon(&self, owner: Owner, attack: Attack) -> Option<WeaponKind> {
        if let Some(weapon) = attack.primary_of() {
            return Some(weapon);
        }
        let ability = attack.as_ability()?;
        let carried = owner
            .player()
            .and_then(|id| self.players.get(&id))
            .and_then(|p| {
                let w = &p.combat.weapons;
                if let Some(held) = w.held()
                    && held.abilities.contains(&ability)
                {
                    return Some(held.weapon);
                }
                w.loadout
                    .builds()
                    .find(|b| b.abilities.contains(&ability))
                    .map(|b| b.weapon)
            });
        carried.or_else(|| attack.weapon())
    }

    /// Is `attacker` allowed to hurt `victim`? (Same-team damage is blocked except for
    /// knockback.)
    pub fn is_hostile(&self, attacker: Owner, victim: HitTarget) -> bool {
        match (attacker, victim) {
            (Owner::World, _) => true,
            (Owner::Player(a), HitTarget::Player(v)) => a != v && self.rules.friendly_fire,
            (Owner::Player(_), HitTarget::Entity(id)) => self.enemies.contains_key(&id),
            (Owner::Enemy(_), HitTarget::Player(_)) => true,
            (Owner::Enemy(_), HitTarget::Entity(_)) => false,
        }
    }

    /// THE damage pipeline. Applies `info` to `target` with every rule, emits
    /// [`SimEvent::Damage`] (and [`SimEvent::Killed`]) and returns the outcome.
    ///
    /// A branded target (see [`SimWorld::brand_target`]) takes `quill.brand_multiplier` ×
    /// the damage from every source (flag [`DamageFlags::BRANDED`]); the world and dev
    /// commands are exempt.
    pub fn apply_damage(
        &mut self,
        t: &Tuning,
        target: HitTarget,
        mut info: DamageInfo,
    ) -> DamageEvent {
        if !matches!(info.attack, Attack::World | Attack::Dev) && self.is_branded(target) {
            info.amount *= t.combat.quill.brand_multiplier;
            info.flags.insert(DamageFlags::BRANDED);
        }
        let event = match target {
            HitTarget::Player(id) => self.damage_player(t, id, info),
            HitTarget::Entity(id) => self.damage_enemy(t, id, info),
        };
        if event.blocked != Some(Blocked::NoTarget) {
            self.events.push(SimEvent::Damage(event));
        }
        if event.killed {
            self.brands.remove(&target);
            self.on_killed(t, &event);
        }
        event
    }

    fn damage_player(&mut self, t: &Tuning, id: PlayerId, info: DamageInfo) -> DamageEvent {
        let tick = self.tick;
        let friendly_fire = self.rules.friendly_fire;
        let mut event = DamageEvent {
            target: HitTarget::Player(id),
            info,
            dealt: 0.0,
            hard: 0.0,
            health_after: 0.0,
            killed: false,
            blocked: None,
            tick,
        };
        let c = &t.combat;
        let Some(p) = self.players.get_mut(&id) else {
            event.blocked = Some(Blocked::NoTarget);
            return event;
        };
        if p.combat.dead {
            event.blocked = Some(Blocked::NoTarget);
            return event;
        }
        let explosive = info.flags.contains(DamageFlags::EXPLOSIVE);
        let mut amount = info.amount.max(0.0);
        let is_self = info.source == Owner::Player(id);
        if is_self {
            event.info.flags.insert(DamageFlags::SELF);
            if explosive {
                amount *= c.self_damage_fraction;
            } else {
                amount = 0.0;
                event.blocked = Some(Blocked::SelfDamage);
            }
        } else if let Owner::Player(_) = info.source
            && !friendly_fire
        {
            amount = 0.0;
            event.blocked = Some(Blocked::FriendlyFire);
        }
        if info.part == HitPart::Head && !explosive {
            amount *= c.player_head_multiplier;
            event.info.flags.insert(DamageFlags::HEADSHOT);
        }
        if event.blocked.is_none() {
            if p.combat.god {
                event.blocked = Some(Blocked::God);
            } else if p.combat.protection > 0 && info.attack != Attack::World {
                event.blocked = Some(Blocked::SpawnProtection);
            } else if p.movement.is_invulnerable() && info.attack != Attack::World {
                event.blocked = Some(Blocked::Invulnerable);
            }
        }
        // Knockback: always for explosions (blast jumps, teammates' blasts, through
        // i-frames); otherwise only when the hit counts.
        let kb_applies = event.blocked.is_none()
            || (explosive && event.blocked != Some(Blocked::God))
            || event.blocked == Some(Blocked::God) && is_self;
        if kb_applies && info.knockback.length_squared() > 0.0 {
            p.movement.velocity += info.knockback;
        }
        if event.blocked.is_some() {
            event.health_after = p.combat.health(c);
            return event;
        }
        let health_before = p.combat.health(c);
        let dealt = amount.min(health_before);
        let hard = (amount * c.hard_damage_fraction).max(0.0);
        p.combat.damage += amount;
        p.combat.hard = (p.combat.hard + hard)
            .min(p.combat.damage)
            .min(c.max_health);
        p.combat.hard_delay = secs_to_ticks(c.hard_damage_recover_delay, self.tick_rate, 0);
        p.combat.last_attacker = info.source;
        p.combat.last_attack = info.attack;
        p.combat.last_hurt_tick = tick;
        event.dealt = dealt;
        event.hard = hard;
        event.health_after = p.combat.health(c);
        if p.combat.damage >= c.max_health {
            event.killed = true;
        }
        // PvP: damaging another player spawns an ember burst too (players compete for it).
        if let Owner::Player(_) = info.source
            && !is_self
            && dealt > 0.0
        {
            self.spawn_embers_for_damage(t, &event);
        }
        event
    }

    /// Everything that happens when a damage event kills its target.
    fn on_killed(&mut self, t: &Tuning, event: &DamageEvent) {
        let info = event.info;
        let (killer_airborne, killer_health, killer_effective_max) = match info.source {
            Owner::Player(k) => self.players.get(&k).map_or((false, 0.0, 0.0), |p| {
                (
                    !p.movement.on_ground,
                    p.combat.health(&t.combat),
                    p.combat.effective_max(&t.combat),
                )
            }),
            _ => (false, 0.0, 0.0),
        };
        let parried_projectile = info.flags.contains(DamageFlags::PARRIED)
            && info.projectile.is_some()
            && info.source.player().is_some();
        let (victim_kind, victim_airborne, position) = match event.target {
            HitTarget::Player(id) => self
                .players
                .get(&id)
                .map_or((kind::PLAYER, false, info.point), |p| {
                    (kind::PLAYER, !p.movement.on_ground, p.movement.position)
                }),
            HitTarget::Entity(id) => self.enemies.get(&id).map_or((0, false, info.point), |e| {
                (e.net_kind(), !e.on_ground, e.position)
            }),
        };
        let killer_position = match info.source {
            Owner::Player(k) => self.players.get(&k).map(|p| p.movement.position),
            Owner::Enemy(e) => self.enemies.get(&e).map(|e| e.position),
            _ => None,
        };
        let distance = killer_position.map_or(0.0, |k| k.distance(position));
        let weapon = self.attack_weapon(info.source, info.attack);
        let kill = KillInfo {
            victim: event.target,
            victim_kind,
            killer: info.source,
            attack: info.attack,
            flags: event.info.flags,
            part: info.part,
            position,
            ricochets: info.ricochets,
            killer_airborne,
            victim_airborne,
            parried_projectile,
            killer_health,
            killer_effective_max,
            overkill: (info.amount - event.dealt).max(0.0),
            distance,
            weapon,
            tick: self.tick,
        };
        self.events.push(SimEvent::Killed(kill));
        match event.target {
            HitTarget::Player(id) => self.on_player_died(t, id, &kill),
            HitTarget::Entity(id) => self.on_enemy_died(t, id, &kill),
        }
    }

    /// Kills a player outright (kill volumes, `/kill`).
    pub fn kill_player(&mut self, t: &Tuning, id: PlayerId, attack: Attack) {
        let Some(p) = self.players.get_mut(&id) else {
            return;
        };
        if p.combat.dead {
            return;
        }
        let point = p.movement.position;
        let was_god = std::mem::replace(&mut p.combat.god, false);
        p.combat.protection = 0;
        let flags = if attack == Attack::World {
            DamageFlags::ENVIRONMENT
        } else {
            DamageFlags::NONE
        };
        let info = DamageInfo::new(t.combat.max_health * 10.0, Owner::World, attack)
            .at(point, Vec3::NEG_Y)
            .flags(flags);
        // Dash i-frames don't save you from a kill volume.
        let invuln = std::mem::replace(&mut p.movement.invuln_time, 0.0);
        self.apply_damage(t, HitTarget::Player(id), info);
        if let Some(p) = self.players.get_mut(&id) {
            p.combat.god = was_god;
            if !p.combat.dead {
                p.movement.invuln_time = invuln;
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Death & respawn rules. The game modes own them (`modes.rs`: solo checkpoint restart,
    // co-op downed / revive, deathmatch respawns at the farthest spawn); these two entry points
    // are what the damage pipeline and `tick_vitals` call.
    // -----------------------------------------------------------------------------------------

    /// A player just died: dead / downed and the respawn delay by mode
    /// ([`SimWorld::mode_player_died`]).
    pub fn on_player_died(&mut self, t: &Tuning, id: PlayerId, kill: &KillInfo) {
        self.mode_player_died(t, id, kill);
    }

    /// Puts a player back into the game where the mode says (checkpoint / farthest spawn) with
    /// full health, fresh weapons and spawn protection, and emits [`SimEvent::Respawned`]
    /// ([`SimWorld::mode_respawn_player`]).
    pub fn respawn_player(&mut self, t: &Tuning, id: PlayerId) {
        self.mode_respawn_player(t, id);
    }

    /// Per-tick vitals: hard damage recovery, spawn protection, respawn timers.
    pub(crate) fn tick_vitals(&mut self, t: &Tuning) {
        let dt = self.dt();
        let mut respawn = Vec::new();
        for p in self.players.values_mut() {
            let c = &mut p.combat;
            c.protection = c.protection.saturating_sub(1);
            if c.dead {
                c.respawn_in = c.respawn_in.saturating_sub(1);
                if c.respawn_in == 0 {
                    respawn.push(p.id);
                }
                continue;
            }
            if c.hard_delay > 0 {
                c.hard_delay -= 1;
            } else if c.hard > 0.0 {
                c.hard = (c.hard - t.combat.hard_damage_recover_rate * dt).max(0.0);
            }
        }
        for id in respawn {
            self.respawn_player(t, id);
        }
    }

    /// Parry reward: heal to the effective max after clearing part of the hard damage.
    pub(crate) fn parry_reward(&mut self, t: &Tuning, id: PlayerId) -> f32 {
        let Some(p) = self.players.get_mut(&id) else {
            return 0.0;
        };
        if p.combat.dead {
            return 0.0;
        }
        p.combat.hard *= (1.0 - t.combat.parry_hard_damage_clear).clamp(0.0, 1.0);
        let before = p.combat.damage;
        p.combat.damage = p.combat.hard;
        before - p.combat.damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healing_respects_hard_damage() {
        let mut c = PlayerCombat {
            damage: 60.0,
            hard: 20.0,
            ..Default::default()
        };
        let t = CombatTuning::default();
        assert_eq!(c.health(&t), 40.0);
        assert_eq!(c.effective_max(&t), 80.0);
        assert_eq!(c.heal(100.0), 40.0);
        assert_eq!(c.health(&t), 80.0);
        assert_eq!(c.heal(5.0), 0.0);
    }

    #[test]
    fn flags_compose() {
        let f = DamageFlags::HITSCAN | DamageFlags::HEADSHOT;
        assert!(f.contains(DamageFlags::HEADSHOT));
        assert!(!f.contains(DamageFlags::EXPLOSIVE));
        assert!(!f.contains(DamageFlags::NONE));
    }

    #[test]
    fn ticks_round() {
        assert_eq!(secs_to_ticks(0.3, 60, 1), 18);
        assert_eq!(secs_to_ticks(1.0 / 12.0, 60, 1), 5);
        assert_eq!(secs_to_ticks(0.0, 60, 1), 1);
        assert_eq!(secs_to_ticks(-5.0, 60, 0), 0);
    }
}
