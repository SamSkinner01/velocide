//! Combat messages: reliable reports ([`DamageReport`], [`HitReport`]) and cosmetic
//! snapshot-batched effects ([`FxEvent`]). Who receives what and what clients show for each is
//! specified in `docs/combat.md` ("client presentation contract").

use glam::Vec3;
use serde::{Deserialize, Serialize};

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
    /// A shot bounced off prisms: shatter each prism, draw the bright ricochet tracer through
    /// `points` (prisms, then the final end). Sent to everyone (shooter included).
    Ricochet {
        shooter: u16,
        attack: Attack,
        points: Vec<QPos>,
    },
    /// Fireball, light, shake by distance, `Sfx::Explosion`. Sent to everyone.
    Explosion {
        position: QPos,
        /// Radius in decimetres.
        radius_dm: u16,
        attack: Attack,
    },
    /// Another player punched (viewmodel/arm anim for remote players). Not sent to the puncher.
    Punch { player: PlayerId, hit: bool },
    /// A blood burst healed a player: red streak from `from` to the player; heal flash if it
    /// is you.
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
    /// Prism throw (arm anim, `Sfx::PrismThrow`). Not sent to the thrower.
    PrismThrown { player: PlayerId },
    /// A prism expired (small fizzle).
    PrismExpired { id: NetId },
    /// Stickies detonated by their owner (`Sfx::StickyDetonate` click).
    StickiesDetonated { player: PlayerId },
    /// A sawblade bounced (`Sfx::SawBounce`, sparks).
    ProjectileBounce { kind: u16, position: QPos },
    /// A rivet / orb hit the level and vanished (sparks / splash, `Sfx::ProjectileImpact`).
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
}

impl Fx {
    /// Worth repeating in the next snapshot (visible, gameplay-relevant, rare).
    pub fn important(&self) -> bool {
        matches!(
            self,
            Fx::Explosion { .. }
                | Fx::Ricochet { .. }
                | Fx::SlamShockwave { .. }
                | Fx::EnemyShockwave { .. }
                | Fx::EnemyAttack { .. }
        )
    }
}
