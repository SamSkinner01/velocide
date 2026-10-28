//! Combat messages: reliable reports ([`DamageReport`], [`HitReport`]) and cosmetic
//! snapshot-batched effects ([`FxEvent`]). Who receives what and what clients show for each is
//! specified in `docs/combat.md` ("client presentation contract").

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::loadout::{Ability, Knack};
use crate::sim::{Attack, Blocked, DamageFlags, HitTarget, Owner, PlayerId};

use super::NetId;

/// A position quantised to 1/64 m (cheap varints for effects).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QPos(pub [i32; 3]);

impl QPos {
    pub const SCALE: f32 = 64.0;

    pub fn to_vec3(self) -> Vec3 {
        Vec3::new(self.0[0] as f32, self.0[1] as f32, self.0[2] as f32) / Self::SCALE
    }
}

impl From<Vec3> for QPos {
    fn from(v: Vec3) -> Self {
        // `as` saturates and maps NaN to 0.
        QPos([
            (v.x * Self::SCALE).round() as i32,
            (v.y * Self::SCALE).round() as i32,
            (v.z * Self::SCALE).round() as i32,
        ])
    }
}

/// The recipient took damage (sent to the victim only). Drives the damage flash, direction
/// indicator, hurt sound and the hard-damage HUD segment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageReport {
    /// Damage removed from health (0 when blocked).
    pub amount: f32,
    /// Hard damage added.
    pub hard: f32,
    /// Health after the hit.
    pub health: f32,
    pub source: Owner,
    pub attack: Attack,
    /// Where it hit and the direction it travelled (point the indicator at `-direction`).
    pub point: Vec3,
    pub direction: Vec3,
    pub flags: DamageFlags,
    /// Why it did nothing (i-frames: show a "dodge" flash; friendly fire...).
    pub blocked: Option<Blocked>,
    pub killed: bool,
}

/// The recipient's attacks hit something this tick (sent to the attacker only; all hits of one
/// tick on one target are merged: ten pellets = one report).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitReport {
    pub target: HitTarget,
    /// Replicated kind of the target (player or enemy kind).
    pub target_kind: u16,
    pub attack: Attack,
    /// Total damage dealt.
    pub damage: f32,
    /// Number of damage instances merged.
    pub hits: u16,
    /// Union of the hits' flags (HEADSHOT / WEAK_POINT → headshot marker; ARMOURED → dull
    /// "clank" marker; RICOCHET...).
    pub flags: DamageFlags,
    /// Position of the last hit (damage number).
    pub point: Vec3,
    pub killed: bool,
}

/// A cosmetic event batched into snapshots (unreliable). `seq` increases by one per event
/// server-wide; important events (explosions, ricochets, shockwaves) are repeated in the next
/// snapshot too, so clients must ignore a `seq` they already handled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FxEvent {
    pub seq: u32,
    /// Server tick it happened at (play it when the render clock reaches it).
    pub tick: u32,
    pub fx: Fx,
}

/// What to show / play. See `docs/combat.md` for the full presentation contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fx {
    /// Another player fired: muzzle flash + weapon sound at the shooter; hitscan tracers from
    /// the shooter's muzzle to each end point (impact sparks/decals there). Projectile
    /// weapons have no `ends` (their projectiles are entities). Not sent to the shooter.
    Shot {
        shooter: PlayerId,
        attack: Attack,
        origin: QPos,
        ends: Vec<QPos>,
    },
    /// An attack relayed off thorns: shatter each thorn, draw the bright ricochet tracer
    /// through `points` (thorns, then the final end): a ray, or the ray a swing / cone / nova /
    /// blast flung the first thorn as. With `projectile` that projectile turned at the first
    /// thorn instead (no tracer: it flies the leg itself, heading for the last point). Sent to
    /// everyone (shooter included).
    Ricochet {
        shooter: u16,
        attack: Attack,
        points: Vec<QPos>,
        projectile: Option<NetId>,
    },
    /// Fireball, light, shake by distance, `Sfx::Explosion`. Sent to everyone. `owner` is the
    /// owner code (player id / `OWNER_ENEMY` / `OWNER_NONE`): the owner's client draws its own
    /// projectiles at the present time, so it plays its own explosions right away instead of
    /// at the render tick.
    Explosion {
        position: QPos,
        /// Radius in decimetres.
        radius_dm: u16,
        attack: Attack,
        owner: u16,
    },
    /// Another player punched (viewmodel/arm anim for remote players). Not sent to the puncher.
    Punch { player: PlayerId, hit: bool },
    /// A player drew in an ember burst (kindling): a stream of sparks from `from` into the
    /// player's chest lantern; heal flash if it is you.
    Heal {
        player: PlayerId,
        /// Tenths of HP.
        amount_tenths: u16,
        from: QPos,
    },
    /// Slam landing shockwave ring. Sent to everyone.
    SlamShockwave {
        player: PlayerId,
        position: QPos,
        /// 0..255.
        power: u8,
        radius_dm: u16,
    },
    /// Thorn throw (arm anim, `Sfx::ThornThrow`). Not sent to the thrower.
    ThornThrown { player: PlayerId },
    /// A thrown thorn planted itself in the level (`Sfx::ThornPlant`, chips).
    ThornPlanted { position: QPos },
    /// A thorn expired (small glass crumble, `Sfx::ThornShatter`).
    ThornExpired { id: NetId },
    /// A projectile burst into secondary rays at `origin` (the Flak ring, `attack` = `Flak`, or
    /// a Gust-deflected shell's `Gust`): a tracer to each end. Sent to everyone.
    Burst {
        shooter: u16,
        attack: Attack,
        origin: QPos,
        ends: Vec<QPos>,
    },
    /// An Arclash zap chained from target to target: jagged arcs through `points` (the first
    /// target hit, then each target it jumped to), `Sfx::ArcChain`. Sent to everyone.
    ArcChain { shooter: u16, points: Vec<QPos> },
    /// A Wisp zapped a target: jagged arc `from` → `to`, `Sfx::WispZap`. Sent to everyone.
    WispZap { from: QPos, to: QPos },
    /// A Brand shot marked a target (sigil flash, `Sfx::Branded` at `position`). Sent to
    /// everyone; the brand itself is the `BRANDED` entity flag.
    Branded { position: QPos },
    /// A Gust turned a hostile projectile around (`Sfx::Deflect`, sparks). Sent to everyone.
    Deflected { player: PlayerId, position: QPos },
    /// An Arclash Discharge nova around `position` (ring of `radius_dm`, `Sfx::DischargeRelease`).
    /// Not sent to the player who discharged (it predicts its own).
    Discharge {
        player: PlayerId,
        position: QPos,
        radius_dm: u16,
    },
    /// A bomb bounced off the level (`Sfx::BombBounce`, sparks).
    ProjectileBounce { kind: u16, position: QPos },
    /// An enemy projectile hit the level and vanished (sparks / splash, `Sfx::ProjectileImpact`).
    ProjectileImpact {
        kind: u16,
        position: QPos,
        normal: QPos,
    },
    /// Enemy telegraph started (`strike == false`: play the wind-up sound/flash) or the
    /// attack landed (`strike == true`: swipe/slam sound).
    EnemyAttack {
        enemy: NetId,
        attack: u8,
        strike: bool,
        parryable: bool,
    },
    /// Enemy fired a projectile (muzzle flash / spit).
    EnemyFired {
        enemy: NetId,
        attack: u8,
        muzzle: QPos,
    },
    /// Enemy slam shockwave ring.
    EnemyShockwave {
        enemy: NetId,
        position: QPos,
        radius_dm: u16,
    },
    /// An enemy noticed a player (alert bark).
    EnemyAlert { enemy: NetId },
    /// An enemy appeared (encounter wave or dev spawn): spawn flash + `Sfx::EnemySpawn` at
    /// `position` (its origin; `kind` = enemy kind).
    EnemySpawned {
        enemy: NetId,
        kind: u16,
        position: QPos,
    },
    // ---- refit (docs/arsenal.md, "Replication and wire ids"): melee and knacks ----
    /// Another player swung a melee weapon: a Trimmer / Tamper primary, a Whirl, a Riposte counter or
    /// the start of a Lunge (`attack`); `hits` = targets it struck. Remote swing animation,
    /// whoosh, hit sparks. Not sent to the swinger (it predicts its own).
    Swing {
        player: PlayerId,
        attack: Attack,
        hits: u8,
    },
    /// A Lunge reached its target (impact flash and sound at `position`). Sent to everyone.
    LungeStruck { player: PlayerId, position: QPos },
    /// Another player used a knack other than the Thorn (that one is `ThornThrown`): throw
    /// animation, the Surge streak, the start of Tallow Skin (the sheen itself is the
    /// `TALLOW` player flag), sound. Not sent to that player.
    Knack { player: PlayerId, knack: Knack },
    /// A Snare sprang on something at `position` (wax splash, snap). Sent to everyone.
    SnareSprung { position: QPos },
    // ---- rework (docs/rework.md, docs/arsenal.md "Replication and wire ids") ----
    /// A Wickline: the burning thread from `from` to `to` for `wickline.line_time`;
    /// `anchored` = it caught the level (a zip), else a target (a yank) or nothing. Sent to
    /// everyone (the caster can't predict where it lands).
    Wickline {
        player: PlayerId,
        from: QPos,
        to: QPos,
        anchored: bool,
    },
    /// A Tally's release: one ray from `origin` to each end (the marked targets). Sent to
    /// everyone.
    TallyFired {
        shooter: PlayerId,
        origin: QPos,
        ends: Vec<QPos>,
    },
    /// A Blink Arrow step: the player went from `from` to `to` (the owner's client snaps its
    /// camera on the reconciliation). Sent to everyone.
    Blink {
        player: PlayerId,
        from: QPos,
        to: QPos,
    },
    /// A guard (Bulwark, Riposte) caught a hit at `position`: block sparks, a thud; `broke` =
    /// the Bulwark shattered with it. Sent to everyone.
    GuardBlock {
        player: PlayerId,
        ability: Ability,
        position: QPos,
        broke: bool,
    },
    /// A Bolas bound its targets: the cord between `positions` (one or two). Sent to
    /// everyone.
    BolasBound { positions: Vec<QPos> },
    /// A Lantern fired the carrier's primary from `origin`: a tracer / zap / ember cone to each
    /// end (`attack` = `Lantern`; the lantern's `state` says which weapon it copies). Sent to
    /// everyone.
    LanternShot {
        lantern: NetId,
        origin: QPos,
        ends: Vec<QPos>,
    },
    /// A thrown Hurl came back to its thrower's hand (catch sound, part of the cooldown back).
    /// Sent to everyone.
    HurlCaught { player: PlayerId },
}

impl Fx {
    /// Worth repeating in the next snapshot (visible, gameplay-relevant, rare).
    pub fn important(&self) -> bool {
        matches!(
            self,
            Fx::Explosion { .. }
                | Fx::Ricochet { .. }
                | Fx::Burst { .. }
                | Fx::ArcChain { .. }
                | Fx::Branded { .. }
                | Fx::Deflected { .. }
                | Fx::Discharge { .. }
                | Fx::SlamShockwave { .. }
                | Fx::EnemyShockwave { .. }
                | Fx::EnemyAttack { .. }
                | Fx::EnemySpawned { .. }
                | Fx::LungeStruck { .. }
                | Fx::SnareSprung { .. }
                | Fx::Wickline { .. }
                | Fx::TallyFired { .. }
                | Fx::Blink { .. }
                | Fx::GuardBlock { .. }
                | Fx::BolasBound { .. }
        )
    }
}
