//! Enemies, the Moulds (docs/roster.md, docs/enemies.md): things poured and set in the Foundry,
//! grey wax-flesh, slag, brass and chain, lit from inside where the heat collects. Gnawer
//! (knuckle-walking pack crawler), Censer (robed brazier-head with a chained censer), Harrier
//! (wax-winged carrion flyer), Gaoler (iron jailer with a cell-door shield, a chain flail and a
//! caged lantern on its back), Heron (stilt-legged lens-eyed sniper), Bellwether (stooped
//! bell-ringer with a belfry yoke), Snuffer (six-legged skitterer with a snuffer-cap head),
//! Smelter (bow-legged hulk with a slung crucible), Effigy (a wax training dummy on a post).
//! Each has a readable silhouette,
//! bright `Core` weak points, `Glow` telegraph parts that the pose functions drive from the
//! attack state, and pure time-based poses.
//!
//! Telegraph convention (all enemies): `Pose::glow` is ~0.4 when calm, ramps up through
//! `Windup`, flickers near the end of it (Gaoler bash: white-hot strobe in its parry window),
//! peaks on `Attack` and fades during `Recover`; `Stunned` / `Dying` go dark. Enemies with
//! several attacks pose (and light) by [`EnemyAnim::attack`] (the definition's attack index).

pub mod bellwether;
pub mod censer;
pub mod effigy;
pub mod gaoler;
pub mod gnawer;
pub mod harrier;
pub mod heron;
pub mod puddle;
pub mod slag;
pub mod smelter;
pub mod snuffer;

use std::sync::OnceLock;

use engine::glam::{Mat4, Quat, Vec3, Vec4};

use super::ModelId;
use super::rig::Pose;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnemyKind {
    Gnawer,
    Censer,
    Harrier,
    Gaoler,
    Heron,
    Bellwether,
    Snuffer,
    Smelter,
    Effigy,
}

impl EnemyKind {
    pub const ALL: [EnemyKind; 9] = [
        EnemyKind::Gnawer,
        EnemyKind::Censer,
        EnemyKind::Harrier,
        EnemyKind::Gaoler,
        EnemyKind::Heron,
        EnemyKind::Bellwether,
        EnemyKind::Snuffer,
        EnemyKind::Smelter,
        EnemyKind::Effigy,
    ];

    /// From an enemy definition name (`assets/enemies/<name>.ron` file stem).
    pub fn from_name(name: &str) -> Option<EnemyKind> {
        match name.to_ascii_lowercase().as_str() {
            "gnawer" => Some(EnemyKind::Gnawer),
            "censer" => Some(EnemyKind::Censer),
            "harrier" => Some(EnemyKind::Harrier),
            "gaoler" => Some(EnemyKind::Gaoler),
            "heron" => Some(EnemyKind::Heron),
            "bellwether" => Some(EnemyKind::Bellwether),
            "snuffer" => Some(EnemyKind::Snuffer),
            "smelter" => Some(EnemyKind::Smelter),
            "effigy" => Some(EnemyKind::Effigy),
            _ => None,
        }
    }

    pub fn model(self) -> ModelId {
        match self {
            EnemyKind::Gnawer => ModelId::Gnawer,
            EnemyKind::Censer => ModelId::Censer,
            EnemyKind::Harrier => ModelId::Harrier,
            EnemyKind::Gaoler => ModelId::Gaoler,
            EnemyKind::Heron => ModelId::Heron,
            EnemyKind::Bellwether => ModelId::Bellwether,
            EnemyKind::Snuffer => ModelId::Snuffer,
            EnemyKind::Smelter => ModelId::Smelter,
            EnemyKind::Effigy => ModelId::Effigy,
        }
    }

    /// Metres per full locomotion cycle.
    pub fn stride(self) -> f32 {
        match self {
            EnemyKind::Gnawer => 3.2,
            EnemyKind::Censer => 2.2,
            EnemyKind::Harrier => 6.0,
            EnemyKind::Gaoler => 3.4,
            EnemyKind::Heron => 4.2,
            EnemyKind::Bellwether => 3.0,
            EnemyKind::Snuffer => 1.8,
            EnemyKind::Smelter => 3.0,
            EnemyKind::Effigy => 1.0,
        }
    }

    /// Advances a locomotion phase (cycles, wraps at 1) by `dt` at ground speed `speed`.
    pub fn advance_cycle(self, cycle: f32, speed: f32, dt: f32) -> f32 {
        let rate = (speed.abs() / self.stride()).min(2.5);
        (cycle + rate * dt).rem_euclid(1.0)
    }

    /// Approximate height of the model as drawn (feet to top, [`EnemyKind::scale`] included),
    /// for name plates / debug.
    pub fn height(self) -> f32 {
        let built = match self {
            EnemyKind::Gnawer => 2.0,
            EnemyKind::Censer => 2.6,
            EnemyKind::Harrier => 1.3,
            EnemyKind::Gaoler => 3.7,
            EnemyKind::Heron => 3.4,
            EnemyKind::Bellwether => 3.1,
            EnemyKind::Snuffer => 1.4,
            EnemyKind::Smelter => 3.4,
            EnemyKind::Effigy => 2.2,
        };
        built * self.scale()
    }

    /// How much bigger than built the model is drawn: the definition's `model_scale` (the
    /// built-in `assets/enemies/*.ron`, like everything else the client reads about enemies).
    /// The Moulds are drawn larger than their movement capsules so they read and are easier to
    /// hit; their hit shapes are sized to the scaled model (docs/enemies.md).
    /// [`crate::models::ModelLibrary::draw_enemy`] applies it about the model origin.
    pub fn scale(self) -> f32 {
        static SCALES: OnceLock<[f32; 9]> = OnceLock::new();
        let scales = SCALES.get_or_init(|| {
            let defs = shared::sim::EnemyRegistry::builtin();
            EnemyKind::ALL.map(|k| {
                let name = format!("{k:?}").to_ascii_lowercase();
                defs.get(&name).map_or(1.0, |d| d.draw_scale())
            })
        });
        let i = EnemyKind::ALL.iter().position(|k| *k == self).unwrap_or(0);
        scales[i]
    }

    /// Height of the model's origin above the replicated feet position (the Harrier's model
    /// origin is its hover centre, half its capsule height up).
    pub fn root_lift(self) -> f32 {
        match self {
            EnemyKind::Harrier => harrier::HOVER_LIFT,
            _ => 0.0,
        }
    }

    /// The attack whose wind-up and strike pose a leap along the path (the crouch and the
    /// flight): the Gnawer's pounce, the Snuffer's latching leap. Others crouch and fly with
    /// [`body_transform`] alone.
    pub fn leap_attack(self) -> Option<u8> {
        match self {
            EnemyKind::Gnawer => Some(gnawer::POUNCE),
            EnemyKind::Snuffer => Some(snuffer::LATCH),
            _ => None,
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
    /// Attack telegraph (jaw unhinges / censer whirls / wings fold / shield or chain draws
    /// back / the lens charges).
    Windup(f32),
    /// The strike itself (bite or pounce / lob or swing / dive / bash, sweep or rush /
    /// needle or kick).
    Attack(f32),
    /// Back to neutral after the strike.
    Recover(f32),
    /// Parried / swatted / interrupted / thrown off: helpless.
    Stunned,
    /// Clinging to a player (a latched Snuffer).
    Latched,
    /// Death: slumps, splits and melts (progress over the corpse time).
    Dying(f32),
    /// Crouching to leap along its path (progress through the crouch): a coil, without the
    /// telegraph glow (it isn't an attack). Translated by [`pose`]; see [`body_transform`].
    Crouch(f32),
    /// In the air on a leap or a pad ride (progress through the planned flight).
    Airborne(f32),
    /// Stranded: slumping into a wax puddle (progress; [`melt_depth`]).
    Melt(f32),
    /// Rising out of the puddle where it re-formed (progress; [`melt_depth`]).
    Reform(f32),
}

/// How far into its puddle a melting / re-forming Mould is drawn (0 standing … 1 only the
/// puddle): the server's curve (`shared::sim::enemy::melt_depth`), which its hit shapes follow.
pub fn melt_depth(state: EnemyState) -> f32 {
    use shared::sim::AiState;
    match state {
        EnemyState::Melt(p) => shared::sim::enemy::melt_depth(AiState::Melt, p),
        EnemyState::Reform(p) => shared::sim::enemy::melt_depth(AiState::Reform, p),
        _ => 0.0,
    }
}

/// Everything an enemy pose depends on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnemyAnim {
    pub kind: EnemyKind,
    pub state: EnemyState,
    /// Index of the current / last attack in the enemy's definition (replicated `aux` bits
    /// 8–15): which telegraph and strike to pose.
    pub attack: u8,
    /// Ground speed in m/s (walk cycle amplitude).
    pub speed: f32,
    /// Locomotion phase in cycles ([`EnemyKind::advance_cycle`]).
    pub cycle: f32,
    /// Lateral velocity relative to facing, -1..1 (+ = moving right): banking, lean.
    pub strafe: f32,
    /// Pitch of the aim / flight direction in radians (+ = up): heads aim, a diving Harrier
    /// points its body down its dive.
    pub aim_pitch: f32,
    /// The Heron's aim laser has locked (the lens burns brighter).
    pub aim_locked: bool,
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
            attack: 0,
            speed: 0.0,
            cycle: 0.0,
            strafe: 0.0,
            aim_pitch: 0.0,
            aim_locked: false,
            time: 0.0,
            seed: 0,
        }
    }

    /// Root transform for an enemy model at `position` (its origin: feet; Harrier: hover
    /// centre) facing `yaw`.
    pub fn root(position: Vec3, yaw: f32) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), position)
    }
}

/// Pose for any enemy. The getting-there states are drawn with poses every Mould already has:
/// a crouch with its leap attack's wind-up (the Gnawer's pounce coil, the Snuffer's bunched
/// legs) or standing, a flight with that attack's strike or a mid-stride, a melt / re-form with
/// its death slump (sunk as deep as [`melt_depth`]) — without the telegraph glow, which belongs
/// to attacks. [`body_transform`] adds the squash, stretch and sink.
pub fn pose(anim: &EnemyAnim) -> Pose {
    let mut a = *anim;
    a.state = match anim.state {
        EnemyState::Crouch(p) => match anim.kind.leap_attack() {
            Some(i) => {
                a.attack = i;
                EnemyState::Windup(p)
            }
            None => EnemyState::Idle,
        },
        EnemyState::Airborne(_) => {
            a.aim_pitch = 0.0;
            match anim.kind.leap_attack() {
                Some(i) => {
                    a.attack = i;
                    EnemyState::Attack(0.3)
                }
                None => {
                    a.speed = a.speed.max(4.0);
                    a.cycle = 0.3;
                    EnemyState::Walk
                }
            }
        }
        EnemyState::Melt(_) | EnemyState::Reform(_) => {
            EnemyState::Dying(0.85 * melt_depth(anim.state))
        }
        s => s,
    };
    let mut pose = match a.kind {
        EnemyKind::Gnawer => gnawer::pose(&a),
        EnemyKind::Censer => censer::pose(&a),
        EnemyKind::Harrier => harrier::pose(&a),
        EnemyKind::Gaoler => gaoler::pose(&a),
        EnemyKind::Heron => heron::pose(&a),
        EnemyKind::Bellwether => bellwether::pose(&a),
        EnemyKind::Snuffer => snuffer::pose(&a),
        EnemyKind::Smelter => smelter::pose(&a),
        EnemyKind::Effigy => effigy::pose(&a),
    };
    if a.state != anim.state {
        let glow = telegraph(anim.state, anim.time, 1.0);
        pose.glow = gray(glow);
        pose.core = gray(heat(anim.state, glow));
    }
    pose
}

/// The squash, stretch and sink the getting-there states put on the whole body (about the
/// feet, in drawn metres: applied after the drawn scale), on top of its pose: a crouch
/// squashes it down and out, a leap stretches it at takeoff and tilts it along its flight
/// (`aim_pitch` = the flight's pitch), a melt sags it and sinks it down into its puddle (the
/// floor hides what is below), a re-form the reverse — the same sink as the server's hit
/// shapes (`Enemy::hitbox`, `shared::sim::enemy::{MELT_SINK, MELT_SAG}`).
pub fn body_transform(anim: &EnemyAnim) -> Mat4 {
    match anim.state {
        EnemyState::Crouch(p) => {
            let e = smoothstep(p / 0.8);
            Mat4::from_scale(Vec3::new(1.0 + 0.08 * e, 1.0 - 0.16 * e, 1.0 + 0.08 * e))
        }
        EnemyState::Airborne(p) => {
            let stretch = 1.0 + 0.1 * (1.0 - smoothstep(p * 2.5));
            let thin = 1.0 / stretch.sqrt();
            let pivot = Vec3::Y * (anim.kind.height() * 0.5);
            let tilt = if anim.kind == EnemyKind::Harrier {
                0.0
            } else {
                0.6 * anim.aim_pitch.clamp(-0.9, 0.9)
            };
            Mat4::from_translation(pivot)
                * Mat4::from_rotation_x(tilt)
                * Mat4::from_translation(-pivot)
                * Mat4::from_scale(Vec3::new(thin, stretch, thin))
        }
        EnemyState::Melt(_) | EnemyState::Reform(_) => {
            let d = melt_depth(anim.state);
            let sag = 1.0 - shared::sim::enemy::MELT_SAG * d;
            let spread = 1.0 + 0.25 * d;
            let sink = shared::sim::enemy::MELT_SINK * d * anim.kind.height();
            Mat4::from_translation(Vec3::NEG_Y * sink)
                * Mat4::from_scale(Vec3::new(spread, sag, spread))
        }
        _ => Mat4::IDENTITY,
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
        EnemyState::Idle | EnemyState::Walk | EnemyState::Crouch(_) | EnemyState::Airborne(_) => {
            0.45 + 0.1 * wave(time, 0.7, 0.0)
        }
        // The heat sinks into the floor with the body.
        EnemyState::Melt(_) | EnemyState::Reform(_) => {
            (0.45 + 0.1 * wave(time, 0.7, 0.0)) * (1.0 - melt_depth(state))
        }
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
        EnemyState::Latched => 2.0 + 0.5 * wave(time, 2.2, 0.0),
        EnemyState::Recover(p) => 1.4 - 0.95 * p.clamp(0.0, 1.0),
        EnemyState::Stunned => 0.15 + 0.25 * strobe(time, 5.0),
        EnemyState::Dying(p) => 0.8 * (1.0 - p.clamp(0.0, 1.0)),
    }
}

/// Weak-point heat: always bright enough to read through fog (≥ 1), flaring with the
/// telegraph; it gutters out as the corpse cools.
pub(crate) fn heat(state: EnemyState, glow: f32) -> f32 {
    match state {
        EnemyState::Dying(p) => 1.6 * (1.0 - smoothstep(p * 1.4)),
        EnemyState::Stunned => 0.7,
        // Still shootable while it sinks: the weak points glow on down into the puddle.
        EnemyState::Melt(_) | EnemyState::Reform(_) => 1.15 * (1.0 - 0.6 * melt_depth(state)),
        _ => glow.max(1.15),
    }
}

/// The death slump shared by the ground Moulds: the body sinks into a puddle and splits
/// (`e` = eased progress). Returns (pelvis drop, sideways split, melt squash 1 → 0.45).
pub(crate) fn melt(p: f32) -> (f32, f32, f32) {
    let e = smoothstep(p / 0.7);
    let m = smoothstep((p - 0.35) / 0.65);
    (e, m, 1.0 - 0.55 * m)
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
