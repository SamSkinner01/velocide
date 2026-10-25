//! Enemies: Husk, Spitter, Gnat, Bastion. Each has a readable silhouette, a `Glow` telegraph
//! part that the pose functions drive from the attack state, and pure time-based poses.
//!
//! Telegraph convention (all enemies): `Pose::glow` is ~0.4 when calm, ramps up through
//! `Windup`, flickers near the end of it (Bastion: white-hot strobe in its parry window),
//! peaks on `Attack` and fades during `Recover`; `Stunned` / `Dying` go dark.

pub mod bastion;
pub mod gnat;
pub mod husk;
pub mod spitter;

use engine::glam::{Mat4, Quat, Vec3, Vec4};

use super::ModelId;
use super::rig::Pose;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnemyKind {
    Husk,
    Spitter,
    Gnat,
    Bastion,
}

impl EnemyKind {
    pub const ALL: [EnemyKind; 4] = [
        EnemyKind::Husk,
        EnemyKind::Spitter,
        EnemyKind::Gnat,
        EnemyKind::Bastion,
    ];

    /// From an enemy definition name (`assets/enemies/<name>.ron` file stem).
    pub fn from_name(name: &str) -> Option<EnemyKind> {
        match name.to_ascii_lowercase().as_str() {
            "husk" => Some(EnemyKind::Husk),
            "spitter" => Some(EnemyKind::Spitter),
            "gnat" => Some(EnemyKind::Gnat),
            "bastion" => Some(EnemyKind::Bastion),
            _ => None,
        }
    }

    pub fn model(self) -> ModelId {
        match self {
            EnemyKind::Husk => ModelId::Husk,
            EnemyKind::Spitter => ModelId::Spitter,
            EnemyKind::Gnat => ModelId::Gnat,
            EnemyKind::Bastion => ModelId::Bastion,
        }
    }

    /// Metres per full locomotion cycle.
    pub fn stride(self) -> f32 {
        match self {
            EnemyKind::Husk => 2.4,
            EnemyKind::Spitter => 2.0,
            EnemyKind::Gnat => 4.0,
            EnemyKind::Bastion => 2.6,
        }
    }

    /// Advances a locomotion phase (cycles, wraps at 1) by `dt` at ground speed `speed`.
    pub fn advance_cycle(self, cycle: f32, speed: f32, dt: f32) -> f32 {
        let rate = (speed.abs() / self.stride()).min(2.5);
        (cycle + rate * dt).rem_euclid(1.0)
    }

    /// Approximate height of the model (feet to top), for name plates / debug.
    pub fn height(self) -> f32 {
        match self {
            EnemyKind::Husk => 1.9,
            EnemyKind::Spitter => 1.6,
            EnemyKind::Gnat => 0.5,
            EnemyKind::Bastion => 3.0,
        }
    }
}

/// Animation state; progress values run 0..1 through the phase (the client divides elapsed
/// time by the phase duration from the enemy definition).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum EnemyState {
    #[default]
    Idle,
    Walk,
    /// Attack telegraph (arms raise / sac swells / emitters charge / overhead lift).
    Windup(f32),
    /// The strike itself (swipe / spit / bolt burst / slam and impact hold).
    Attack(f32),
    /// Back to neutral after the strike.
    Recover(f32),
    /// Parried / staggered (Bastion after a parry).
    Stunned,
    /// Death collapse (progress over about one second).
    Dying(f32),
}

/// Everything an enemy pose depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyAnim {
    pub kind: EnemyKind,
    pub state: EnemyState,
    /// Ground speed in m/s (walk cycle amplitude).
    pub speed: f32,
    /// Locomotion phase in cycles ([`EnemyKind::advance_cycle`]).
    pub cycle: f32,
    /// Lateral velocity relative to facing, -1..1 (+ = moving right): Gnat banking, lean.
    pub strafe: f32,
    /// Pitch towards the target in radians (+ = up): heads / emitters aim.
    pub aim_pitch: f32,
    /// Seconds (any monotonically increasing clock).
    pub time: f64,
    /// Per-entity variation (twitch timing, hover phase).
    pub seed: u32,
}

impl EnemyAnim {
    pub fn new(kind: EnemyKind) -> Self {
        Self {
            kind,
            state: EnemyState::Idle,
            speed: 0.0,
            cycle: 0.0,
            strafe: 0.0,
            aim_pitch: 0.0,
            time: 0.0,
            seed: 0,
        }
    }

    /// Root transform for an enemy at `position` (feet; Gnat: hover centre) facing `yaw`.
    pub fn root(position: Vec3, yaw: f32) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), position)
    }
}

/// Pose for any enemy.
pub fn pose(anim: &EnemyAnim) -> Pose {
    match anim.kind {
        EnemyKind::Husk => husk::pose(anim),
        EnemyKind::Spitter => spitter::pose(anim),
        EnemyKind::Gnat => gnat::pose(anim),
        EnemyKind::Bastion => bastion::pose(anim),
    }
}

// ---- shared animation helpers ------------------------------------------------------------

pub(crate) fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `sin(2 pi (time * hz + offset))` evaluated in f64 so large clocks stay precise.
pub(crate) fn wave(time: f64, hz: f64, offset: f32) -> f32 {
    ((time * hz + offset as f64).fract() as f32 * std::f32::consts::TAU).sin()
}

/// Square-ish strobe 0/1 at `hz`.
pub(crate) fn strobe(time: f64, hz: f64) -> f32 {
    if (time * hz).fract() < 0.5 { 1.0 } else { 0.0 }
}

/// Seed-derived phase offset in 0..1.
pub(crate) fn seed_phase(seed: u32, salt: u32) -> f32 {
    engine::assets::procedural::rand01(seed as i32, salt as i32, 0x51ed)
}

pub(crate) fn rx(a: f32) -> Quat {
    Quat::from_rotation_x(a)
}

pub(crate) fn ry(a: f32) -> Quat {
    Quat::from_rotation_y(a)
}

pub(crate) fn rz(a: f32) -> Quat {
    Quat::from_rotation_z(a)
}

/// Standard telegraph intensity for a state (`flicker_from` = wind-up progress where the
/// glow starts strobing).
pub(crate) fn telegraph(state: EnemyState, time: f64, flicker_from: f32) -> f32 {
    match state {
        EnemyState::Idle | EnemyState::Walk => 0.45 + 0.1 * wave(time, 0.7, 0.0),
        EnemyState::Windup(p) => {
            let p = p.clamp(0.0, 1.0);
            let ramp = 0.6 + 1.9 * p;
            if p >= flicker_from {
                ramp * (0.65 + 0.6 * strobe(time, 16.0))
            } else {
                ramp
            }
        }
        EnemyState::Attack(p) => 2.6 - 1.2 * p.clamp(0.0, 1.0),
        EnemyState::Recover(p) => 1.4 - 0.95 * p.clamp(0.0, 1.0),
        EnemyState::Stunned => 0.15 + 0.25 * strobe(time, 5.0),
        EnemyState::Dying(p) => 0.8 * (1.0 - p.clamp(0.0, 1.0)),
    }
}

pub(crate) fn gray(v: f32) -> Vec4 {
    Vec4::new(v, v, v, 1.0)
}

/// Legs of a walk cycle: (thigh, shin) swing angles for the left and right leg.
pub(crate) fn walk_legs(cycle: f32, amp: f32, knee: f32) -> [(f32, f32); 2] {
    let phase = cycle * std::f32::consts::TAU;
    let (s, c) = phase.sin_cos();
    [
        (amp * s, -amp * knee * (0.3 + c.max(0.0))),
        (-amp * s, -amp * knee * (0.3 + (-c).max(0.0))),
    ]
}
