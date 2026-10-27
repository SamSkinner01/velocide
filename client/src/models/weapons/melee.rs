//! Melee weapons in the hand: how the Trimmer and the Tamper are held and swung, in first
//! person (view-space keyframes) and on the third-person Shrike (arm angles), and how a hurled
//! blade or head is drawn on its own.
//!
//! A swing is a [`Swing`]: which move ([`SwingKind`]), its progress through the animation
//! (0 = idle) and, for the Trimmer, which hand. The primary slashes strike early (the blow
//! lands on the tick the trigger is pulled, so the blade has to arrive within a few frames)
//! and spend the rest of the animation recovering: the Trimmer's alternate sides, right then
//! left; the Tamper's is one committed overhead slam that bounces off what it hits and is
//! hauled back up. The abilities have their own moves: the Lunge thrusts, the Whirl sweeps
//! everything round, the Hurl throws, the Upswing rises from the floor.

use engine::gfx::DrawItem;
use engine::glam::{EulerRot, Mat4, Quat, Vec3, Vec4};

use super::{VIEWMODEL_SCALE, smoothstep, tamper, trimmer, unit, wave};
use crate::models::ModelId;
use crate::models::library::ModelLibrary;
use crate::models::rig::{Pose, Tint};
use shared::loadout::WeaponKind;

/// The two melee weapons, as the models see them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeleeWeapon {
    /// Hooked wick-shears carried as two blades: fast slashes, alternating hands.
    Trimmer,
    /// The foundry ramming hammer: one heavy overhead slam.
    Tamper,
}

/// Which move a swing is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SwingKind {
    /// The primary: a slash (Trimmer) or the overhead slam (Tamper).
    #[default]
    Slash,
    /// Thrust forward with the body behind it.
    Lunge,
    /// A full turn, blades / head out.
    Whirl,
    /// A throw: the arm winds back and flings.
    Hurl,
    /// From the floor up past the head.
    Upswing,
}

/// A swing in progress.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Swing {
    pub kind: SwingKind,
    /// 0..1 through the animation (0 = not swinging).
    pub progress: f32,
    /// The Trimmer's left blade (slashes alternate hands; abilities lead with the right).
    pub left: bool,
}

impl Swing {
    /// Swinging right now.
    pub fn active(&self) -> bool {
        self.progress > 0.0 && self.progress < 1.0
    }

    /// Advances by `dt` seconds of a `weapon`'s swing; back to idle at the end.
    pub fn advance(&mut self, weapon: MeleeWeapon, dt: f32) {
        if self.progress <= 0.0 || !self.progress.is_finite() {
            self.progress = 0.0;
            return;
        }
        self.progress += dt.max(0.0) / duration(weapon, self.kind);
        if self.progress >= 1.0 {
            self.progress = 0.0;
        }
    }

    /// How bright the edge / band is: flares through the strike, fades on the recovery.
    pub fn flare(&self, weapon: MeleeWeapon) -> f32 {
        if !self.active() {
            return 0.0;
        }
        let hit = impact(weapon, self.kind);
        let p = self.progress;
        if p < hit {
            smoothstep(p / hit.max(1e-3))
        } else {
            1.0 - smoothstep((p - hit) / 0.35)
        }
    }
}

/// Pose of a melee weapon's model: the edges / band glow in `color` (the selected ability's,
/// `super::ability_color`), flaring with `kick` 0..1 (a swing's [`Swing::flare`]).
pub fn pose(weapon: MeleeWeapon, color: Vec3, kick: f32, time: f64) -> Pose {
    let mut glow = 0.9 * (0.85 + 0.15 * wave(time, 1.3)) + unit(kick) * 0.8;
    let mut pose = match weapon {
        MeleeWeapon::Trimmer => trimmer::pose(kick, time, &mut glow),
        MeleeWeapon::Tamper => tamper::pose(kick, time, &mut glow),
    };
    pose.glow = (color * glow).extend(1.0);
    pose
}

/// Seconds a move's animation lasts (a little under the weapon's interval / the ability's
/// committed time, so back-to-back swings flow on).
pub fn duration(weapon: MeleeWeapon, kind: SwingKind) -> f32 {
    match (weapon, kind) {
        (MeleeWeapon::Trimmer, SwingKind::Slash) => 0.3,
        (MeleeWeapon::Tamper, SwingKind::Slash) => 0.78,
        (_, SwingKind::Lunge) => 0.45,
        (_, SwingKind::Whirl) => 0.55,
        (_, SwingKind::Hurl) => 0.5,
        (MeleeWeapon::Trimmer, SwingKind::Upswing) => 0.45,
        (MeleeWeapon::Tamper, SwingKind::Upswing) => 0.6,
    }
}

/// Progress at which the blow lands (the edge's brightest moment, the trail's end).
pub fn impact(weapon: MeleeWeapon, kind: SwingKind) -> f32 {
    match (weapon, kind) {
        (MeleeWeapon::Trimmer, SwingKind::Slash) => 0.3,
        (MeleeWeapon::Tamper, SwingKind::Slash) => 0.19,
        (_, SwingKind::Lunge) => 0.2,
        (_, SwingKind::Whirl) => 0.6,
        (_, SwingKind::Hurl) => 0.3,
        (_, SwingKind::Upswing) => 0.32,
    }
}

// ---------------------------------------------------------------------------------------------
// First person

/// A view-space hand placement: grip position and Euler angles (YXZ: yaw left, pitch up,
/// roll counter-clockwise), as in `viewmodel_transform`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    roll: f32,
}

const fn key(x: f32, y: f32, z: f32, yaw: f32, pitch: f32, roll: f32) -> Key {
    Key {
        pos: Vec3::new(x, y, z),
        yaw,
        pitch,
        roll,
    }
}

impl Key {
    fn lerp(self, o: Key, t: f32) -> Key {
        Key {
            pos: self.pos.lerp(o.pos, t),
            yaw: self.yaw + (o.yaw - self.yaw) * t,
            pitch: self.pitch + (o.pitch - self.pitch) * t,
            roll: self.roll + (o.roll - self.roll) * t,
        }
    }

    /// The left hand's version of a right-hand key.
    fn mirrored(self) -> Key {
        Key {
            pos: Vec3::new(-self.pos.x, self.pos.y, self.pos.z),
            yaw: -self.yaw,
            pitch: self.pitch,
            roll: -self.roll,
        }
    }

    fn transform(self, extra: Vec3) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(VIEWMODEL_SCALE),
            Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, self.roll),
            self.pos + extra,
        )
    }
}

/// How a segment between two keys eases.
#[derive(Clone, Copy)]
enum Ease {
    /// Accelerating into the blow.
    In,
    /// Decelerating out of it.
    Out,
    Smooth,
}

/// Samples a keyframed track at `p` (`(at, key, ease into it)`, sorted by `at`).
fn sample(track: &[(f32, Key, Ease)], p: f32) -> Key {
    let mut prev = track[0];
    for &next in &track[1..] {
        if p <= next.0 {
            let t = ((p - prev.0) / (next.0 - prev.0).max(1e-4)).clamp(0.0, 1.0);
            let e = match next.2 {
                Ease::In => t * t * t,
                Ease::Out => 1.0 - (1.0 - t).powi(3),
                Ease::Smooth => smoothstep(t),
            };
            return prev.1.lerp(next.1, e);
        }
        prev = next;
    }
    prev.1
}

/// The Trimmer's right blade at rest: raised in a guard on the lower right, the jaw forward.
const TRIM_REST: Key = key(0.15, -0.13, -0.32, 0.12, -0.15, 0.3);
/// The Tamper at rest: the haft up from the lower right, the head right of the crosshair.
const TAMP_REST: Key = key(0.2, -0.25, -0.24, -0.15, -0.5, 0.25);

/// The right hand's track of a move (the left blade mirrors it).
fn track(weapon: MeleeWeapon, kind: SwingKind) -> Vec<(f32, Key, Ease)> {
    use Ease::*;
    match (weapon, kind) {
        // A cut right to left: the wrist turns the edge to lead, the blade whips across the
        // view and finishes low on the left, then comes back to the guard.
        (MeleeWeapon::Trimmer, SwingKind::Slash) => vec![
            (0.0, TRIM_REST, Smooth),
            (0.07, key(0.2, -0.08, -0.32, -0.6, 0.15, -0.6), Out),
            (0.3, key(-0.06, -0.17, -0.34, 0.95, -0.4, -0.9), In),
            (0.45, key(-0.09, -0.2, -0.33, 1.0, -0.5, -0.8), Out),
            (1.0, TRIM_REST, Smooth),
        ],
        // Raised back over the right shoulder, then slammed down the middle; it bounces off
        // and is hauled back up.
        (MeleeWeapon::Tamper, SwingKind::Slash) => vec![
            (0.0, TAMP_REST, Smooth),
            (0.09, key(0.2, -0.12, -0.24, 0.0, 0.7, 0.15), Out),
            (0.19, key(0.06, -0.22, -0.36, 0.08, -1.3, 0.08), In),
            (0.27, key(0.06, -0.19, -0.36, 0.08, -1.15, 0.08), Out),
            (0.42, key(0.07, -0.23, -0.36, 0.08, -1.25, 0.1), Smooth),
            (1.0, TAMP_REST, Smooth),
        ],
        // Point first, level, held through the dash.
        (MeleeWeapon::Trimmer, SwingKind::Lunge) => vec![
            (0.0, TRIM_REST, Smooth),
            (0.08, key(0.17, -0.15, -0.26, 0.1, -0.1, 0.0), Out),
            (0.2, key(0.08, -0.11, -0.38, 0.04, -0.3, 0.0), In),
            (0.7, key(0.08, -0.115, -0.39, 0.04, -0.32, 0.0), Smooth),
            (1.0, TRIM_REST, Smooth),
        ],
        (MeleeWeapon::Tamper, SwingKind::Lunge) => vec![
            (0.0, TAMP_REST, Smooth),
            (0.08, key(0.18, -0.24, -0.26, 0.1, -0.9, 0.2), Out),
            (0.2, key(0.08, -0.2, -0.42, 0.05, -1.35, 0.05), In),
            (0.7, key(0.08, -0.205, -0.43, 0.05, -1.37, 0.05), Smooth),
            (1.0, TAMP_REST, Smooth),
        ],
        // A full turn: the blade, laid flat, sweeps right across the view at chest height (as
        // if the room turned behind it).
        (MeleeWeapon::Trimmer, SwingKind::Whirl) => vec![
            (0.0, TRIM_REST, Smooth),
            (0.12, key(0.26, -0.1, -0.3, -0.8, -0.1, -1.4), Out),
            (0.6, key(-0.26, -0.12, -0.3, 0.8, -0.1, -1.4), Smooth),
            (0.75, key(-0.2, -0.22, -0.3, 0.6, -0.3, -1.0), Out),
            (1.0, TRIM_REST, Smooth),
        ],
        (MeleeWeapon::Tamper, SwingKind::Whirl) => vec![
            (0.0, TAMP_REST, Smooth),
            (0.12, key(0.3, -0.15, -0.25, -0.9, -1.3, 1.2), Out),
            (0.6, key(-0.3, -0.15, -0.25, 0.9, -1.3, 1.2), Smooth),
            (0.75, key(-0.2, -0.28, -0.27, 0.7, -1.1, 1.0), Out),
            (1.0, TAMP_REST, Smooth),
        ],
        // Wound back past the ear, then flung forward; the hand comes back up empty
        // (`hurl_hidden`) before the weapon is up again.
        (MeleeWeapon::Trimmer, SwingKind::Hurl) => vec![
            (0.0, TRIM_REST, Smooth),
            (0.15, key(0.22, -0.02, -0.16, -0.2, 0.9, 0.4), Out),
            (0.3, key(0.04, -0.1, -0.42, 0.15, -0.5, -0.2), In),
            (0.45, key(0.04, -0.24, -0.38, 0.2, -0.9, -0.2), Out),
            (1.0, TRIM_REST, Smooth),
        ],
        (MeleeWeapon::Tamper, SwingKind::Hurl) => vec![
            (0.0, TAMP_REST, Smooth),
            (0.15, key(0.22, -0.08, -0.18, 0.0, 0.9, 0.3), Out),
            (0.3, key(0.06, -0.2, -0.42, 0.1, -1.2, 0.1), In),
            (0.45, key(0.06, -0.26, -0.38, 0.12, -1.35, 0.1), Out),
            (1.0, TAMP_REST, Smooth),
        ],
        // From low in front, up past the face.
        (MeleeWeapon::Trimmer, SwingKind::Upswing) => vec![
            (0.0, TRIM_REST, Smooth),
            (0.12, key(0.08, -0.26, -0.36, 0.1, -1.2, 0.1), Out),
            (0.32, key(0.06, -0.02, -0.34, 0.05, 0.85, 0.0), In),
            (0.48, key(0.07, 0.0, -0.3, 0.05, 1.0, 0.05), Out),
            (1.0, TRIM_REST, Smooth),
        ],
        (MeleeWeapon::Tamper, SwingKind::Upswing) => vec![
            (0.0, TAMP_REST, Smooth),
            (0.14, key(0.1, -0.3, -0.36, 0.1, -1.7, 0.1), Out),
            (0.32, key(0.08, -0.06, -0.3, 0.05, 0.4, 0.05), In),
            (0.5, key(0.1, -0.05, -0.28, 0.05, 0.55, 0.1), Out),
            (1.0, TAMP_REST, Smooth),
        ],
    }
}

/// The idle placement of the right hand.
fn rest(weapon: MeleeWeapon) -> Key {
    match weapon {
        MeleeWeapon::Trimmer => TRIM_REST,
        MeleeWeapon::Tamper => TAMP_REST,
    }
}

/// View-space grip transform of the right hand's weapon (the Trimmer's right blade, the
/// Tamper) for `swing`, before bob, sway and switch offsets (`extra`, view-space metres).
pub fn view_right(weapon: MeleeWeapon, swing: &Swing, extra: Vec3) -> Mat4 {
    let key = if swing.active() && !(weapon == MeleeWeapon::Trimmer && swing.left_slash()) {
        sample(&track(weapon, swing.kind), unit(swing.progress))
    } else {
        rest(weapon)
    };
    key.transform(extra)
}

/// View-space grip transform of the Trimmer's left blade.
pub fn view_left(swing: &Swing, extra: Vec3) -> Mat4 {
    let key = match swing.kind {
        // Its own slashes mirror the right blade's.
        SwingKind::Slash if swing.active() && swing.left => sample(
            &track(MeleeWeapon::Trimmer, SwingKind::Slash),
            unit(swing.progress),
        ),
        // The whirl takes both blades round, the left half a turn behind.
        SwingKind::Whirl if swing.active() => sample(
            &track(MeleeWeapon::Trimmer, SwingKind::Whirl),
            unit(swing.progress - 0.12),
        ),
        // Otherwise it stays up in the guard (a little pulled in while the right one works).
        _ if swing.active() => TRIM_REST.lerp(key(0.12, -0.2, -0.28, 0.1, 0.0, 0.2), 0.5),
        _ => TRIM_REST,
    };
    key.mirrored()
        .transform(Vec3::new(-extra.x, extra.y, extra.z))
}

impl Swing {
    /// A slash with the Trimmer's left blade.
    fn left_slash(&self) -> bool {
        self.left && self.kind == SwingKind::Slash
    }
}

/// While a Hurl's thrown blade / head is gone from the hand (from the release until it is
/// back, raised like a switch).
pub fn hurl_hidden(swing: &Swing) -> bool {
    swing.active() && swing.kind == SwingKind::Hurl && (0.3..0.62).contains(&swing.progress)
}

/// What the left hand is doing while a melee weapon is held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LeftHand {
    /// On the weapon: the Trimmer's left blade, the Tamper's haft.
    Weapon,
    /// Punching (the fist at this view transform, drawn by the caller): the Trimmer's left
    /// blade goes with it; it leaves the Tamper's haft.
    Punch(Mat4),
    /// Throwing a knack (drawn by the caller): the Trimmer's left blade is put away for it.
    Busy,
}

/// Draws the melee viewmodel: the weapon(s) with the right arm (the Trimmer's left blade in
/// the left fist), returning the view-space strike point and the direction of the blow.
#[allow(clippy::too_many_arguments)]
pub fn draw_view(
    lib: &ModelLibrary,
    weapon: MeleeWeapon,
    swing: &Swing,
    pose: &Pose,
    extra: Vec3,
    left_hand: LeftHand,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) -> (Vec3, Vec3) {
    let right = view_right(weapon, swing, extra);
    let mut pose = pose.clone();
    let model = model(weapon);
    let hidden = hurl_hidden(swing);
    match weapon {
        MeleeWeapon::Trimmer => {
            let left = match left_hand {
                // The blade rides in the punching fist (its grip is through the fist).
                LeftHand::Punch(fist) => fist * FIST_ON_GRIP.inverse(),
                _ => view_left(swing, extra),
            };
            // Each half in its own hand: the left one's local transform takes it from its
            // bind spot beside the right half to the left fist.
            let pivot = Vec3::from(trimmer::LEFT_GRIP);
            pose.set(
                trimmer::BLADE_L,
                Mat4::from_translation(-pivot) * right.inverse() * left,
            );
            if hidden {
                pose.hide(trimmer::BLADE_R);
                pose.hide(trimmer::EDGE_R);
                pose.hide(trimmer::WICK);
            }
            if left_hand == LeftHand::Busy {
                pose.hide(trimmer::BLADE_L);
                pose.hide(trimmer::EDGE_L);
            }
            lib.get(model).draw(&pose, right, tint, out);
            // The left fist closes round the left blade's grip.
            if left_hand == LeftHand::Weapon {
                draw_left_fist(
                    lib,
                    left * FIST_ON_GRIP,
                    left_shoulder(weapon) + extra,
                    tint,
                    out,
                );
            }
        }
        MeleeWeapon::Tamper => {
            if hidden {
                pose.hide(tamper::BAND);
                pose.hide(tamper::VENT);
            }
            lib.get(model).draw(&pose, right, tint, out);
            // Both hands on the haft, the left one higher up; for the overhead slam it slides
            // down to the right one (hidden behind it) and back once the head is down.
            if left_hand == LeftHand::Weapon && !hands_together(swing) {
                let fist = right * Mat4::from_translation(tamper::LEFT_HAND) * FIST_ON_GRIP;
                draw_left_fist(lib, fist, left_shoulder(weapon) + extra, tint, out);
            }
        }
    }
    // The right forearm runs back to the shoulder whatever the wrist is doing.
    let arm = aim_arm(right, ARM_DIR, right_shoulder(weapon) + extra);
    lib.get(ModelId::ArmRight).draw_static(arm, tint, out);
    let (point, dir) = strike(weapon, swing, right, extra);
    (point, dir)
}

/// The Tamper's hands are together at the end of the haft (raised for the slam and falling,
/// the whirl, the throw's wind-up, the upswing's rise).
fn hands_together(swing: &Swing) -> bool {
    let p = swing.progress;
    swing.active()
        && match swing.kind {
            SwingKind::Slash => (0.02..0.18).contains(&p),
            SwingKind::Whirl => true,
            SwingKind::Hurl => p < 0.3,
            SwingKind::Upswing => (0.2..0.6).contains(&p),
            SwingKind::Lunge => false,
        }
}

/// Wrist to elbow of the right arm model and of the left fist model (their model space).
const ARM_DIR: Vec3 = Vec3::new(0.066, -0.13, 0.37);
const FIST_DIR: Vec3 = Vec3::new(0.0, -0.022, 0.35);
/// The left fist round a handle: the knuckles in front of it, the handle through the fist.
const FIST_ON_GRIP: Mat4 = Mat4::from_cols_array(&[
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -0.05, -0.035, 1.0,
]);

/// `hand` (a scaled view-space transform whose model has its forearm along `dir`) turned about
/// the hand so the forearm runs back to `shoulder`: the hand stays on the grip, the elbow goes
/// where an elbow would, however the weapon turns in the hand.
fn aim_arm(hand: Mat4, dir: Vec3, shoulder: Vec3) -> Mat4 {
    let (scale, rot, pos) = hand.to_scale_rotation_translation();
    let current = (rot * dir).normalize_or(Vec3::Z);
    let want = (shoulder - pos).normalize_or(current);
    Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_arc(current, want) * rot, pos)
}

/// Where the right forearm runs back to: straight on from the rest grip, so the arm at rest is
/// exactly as modelled.
fn right_shoulder(weapon: MeleeWeapon) -> Vec3 {
    let (_, rot, pos) = rest(weapon)
        .transform(Vec3::ZERO)
        .to_scale_rotation_translation();
    pos + rot * ARM_DIR.normalize() * 0.45
}

/// Where the left forearm runs back to.
fn left_shoulder(weapon: MeleeWeapon) -> Vec3 {
    match weapon {
        MeleeWeapon::Trimmer => {
            let (_, rot, pos) = (TRIM_REST.mirrored().transform(Vec3::ZERO) * FIST_ON_GRIP)
                .to_scale_rotation_translation();
            pos + rot * FIST_DIR.normalize() * 0.45
        }
        MeleeWeapon::Tamper => Vec3::new(-0.22, -0.46, 0.06),
    }
}

/// The left fist (knuckle embers lit softly) at `fist`, its forearm aimed at `shoulder`.
fn draw_left_fist(
    lib: &ModelLibrary,
    fist: Mat4,
    shoulder: Vec3,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) {
    let mut pose = Pose::rest(super::fist::JOINTS.len());
    pose.glow = Vec4::new(0.6, 0.51, 0.42, 1.0);
    lib.get(ModelId::Fist)
        .draw(&pose, aim_arm(fist, FIST_DIR, shoulder), tint, out);
}

/// The view-space strike point of the move and the direction it travels (for trails and
/// sparks).
fn strike(weapon: MeleeWeapon, swing: &Swing, right: Mat4, extra: Vec3) -> (Vec3, Vec3) {
    let muzzle = muzzle(weapon);
    let hand = if weapon == MeleeWeapon::Trimmer && swing.left_slash() {
        view_left(swing, extra)
    } else {
        right
    };
    let point = hand.transform_point3(muzzle);
    let dir = hand
        .transform_vector3(Vec3::NEG_Z)
        .normalize_or(Vec3::NEG_Z);
    (point, dir)
}

/// The model a melee weapon draws.
pub fn model(weapon: MeleeWeapon) -> ModelId {
    match weapon {
        MeleeWeapon::Trimmer => ModelId::Trimmer,
        MeleeWeapon::Tamper => ModelId::Tamper,
    }
}

/// Where a melee weapon bites, model space.
pub fn muzzle(weapon: MeleeWeapon) -> Vec3 {
    match weapon {
        MeleeWeapon::Trimmer => trimmer::MUZZLE,
        MeleeWeapon::Tamper => tamper::MUZZLE,
    }
}

/// The path the strike point sweeps through a move (view space at `extra` = 0, in the
/// order it travels), sampled `n` times up to the impact: the slash trail.
pub fn strike_path(weapon: MeleeWeapon, swing: &Swing, n: usize) -> Vec<Vec3> {
    let hit = impact(weapon, swing.kind);
    let from = match swing.kind {
        SwingKind::Slash | SwingKind::Upswing => hit * 0.3,
        _ => 0.0,
    };
    (0..n.max(2))
        .map(|i| {
            let s = Swing {
                progress: (from + (hit - from) * i as f32 / (n.max(2) - 1) as f32).max(1e-3),
                ..*swing
            };
            let right = view_right(weapon, &s, Vec3::ZERO);
            strike(weapon, &s, right, Vec3::ZERO).0
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Third person

/// Arm angles of a third-person Shrike holding a melee weapon (`shrike::pose_full`):
/// rotations of the upper arms / forearms / hands about their joints, both sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arms {
    pub upper_r: Quat,
    pub fore_r: f32,
    pub hand_r: Quat,
    pub upper_l: Quat,
    pub fore_l: f32,
    pub hand_l: Quat,
}

/// Swing angles about X (+ = forward / up) and Y (+ = across to the left) of one arm.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ArmKey {
    yaw: f32,
    raise: f32,
    elbow: f32,
    /// The wrist: tips the weapon forward (+) / back (-) along the arm.
    wrist: f32,
    /// The wrist turning the edge (about the forearm).
    twist: f32,
}

const fn arm(yaw: f32, raise: f32, elbow: f32, wrist: f32, twist: f32) -> ArmKey {
    ArmKey {
        yaw,
        raise,
        elbow,
        wrist,
        twist,
    }
}

impl ArmKey {
    fn lerp(self, o: ArmKey, t: f32) -> ArmKey {
        let l = |a: f32, b: f32| a + (b - a) * t;
        ArmKey {
            yaw: l(self.yaw, o.yaw),
            raise: l(self.raise, o.raise),
            elbow: l(self.elbow, o.elbow),
            wrist: l(self.wrist, o.wrist),
            twist: l(self.twist, o.twist),
        }
    }
}

/// The right arm's track of a move. The weapon turns with the whole arm: at a key it points
/// its rest direction plus (raise - rest raise) + (elbow - rest elbow) + wrist about X.
fn arm_track(weapon: MeleeWeapon, kind: SwingKind) -> Vec<(f32, ArmKey, Ease)> {
    use Ease::*;
    let rest = arm_rest(weapon);
    match (weapon, kind) {
        // Out to the right with the edge turned to lead, then across to the left.
        (MeleeWeapon::Trimmer, SwingKind::Slash) => vec![
            (0.0, rest, Smooth),
            (0.07, arm(-1.0, 1.45, 0.35, 0.0, 1.2), Out),
            (0.3, arm(1.05, 1.2, 0.1, 0.1, 1.4), In),
            (0.45, arm(1.1, 1.15, 0.15, 0.1, 1.3), Out),
            (1.0, rest, Smooth),
        ],
        // Cocked behind the head (the haft pointing back), then down in front.
        (MeleeWeapon::Tamper, SwingKind::Slash) => vec![
            (0.0, rest, Smooth),
            (0.09, arm(0.1, 2.6, 0.5, 0.0, 0.0), Out),
            (0.19, arm(0.3, 0.75, 0.1, -1.3, 0.0), In),
            (0.42, arm(0.3, 0.72, 0.12, -1.25, 0.0), Smooth),
            (1.0, rest, Smooth),
        ],
        (_, SwingKind::Lunge) => vec![
            (0.0, rest, Smooth),
            (0.2, arm(0.15, 1.55, 0.0, 0.0, 0.0), In),
            (0.7, arm(0.15, 1.55, 0.0, 0.0, 0.0), Smooth),
            (1.0, rest, Smooth),
        ],
        (_, SwingKind::Whirl) => vec![
            (0.0, rest, Smooth),
            (0.12, arm(-1.3, 1.5, 0.1, 0.2, 1.5), Out),
            (0.6, arm(1.2, 1.5, 0.1, 0.2, 1.5), Smooth),
            (1.0, rest, Smooth),
        ],
        (_, SwingKind::Hurl) => vec![
            (0.0, rest, Smooth),
            (0.15, arm(-0.2, 2.7, 1.2, -0.6, 0.0), Out),
            (0.3, arm(0.2, 1.3, 0.0, 0.3, 0.0), In),
            (1.0, rest, Smooth),
        ],
        (_, SwingKind::Upswing) => vec![
            (0.0, rest, Smooth),
            (0.14, arm(0.2, 0.3, 0.1, -0.6, 0.0), Out),
            (0.32, arm(0.15, 2.8, 0.2, 0.0, 0.0), In),
            (1.0, rest, Smooth),
        ],
    }
}

/// The right arm at rest: the weapon held up in front (a gun's aim, so the third-person
/// Shrike points it along the look like any other weapon).
fn arm_rest(weapon: MeleeWeapon) -> ArmKey {
    match weapon {
        MeleeWeapon::Trimmer => arm(0.32, 0.85, 0.6, 0.0, 0.0),
        MeleeWeapon::Tamper => arm(0.32, 0.95, 0.45, 0.0, 0.0),
    }
}

fn sample_arm(track: &[(f32, ArmKey, Ease)], p: f32) -> ArmKey {
    let mut prev = track[0];
    for &next in &track[1..] {
        if p <= next.0 {
            let t = ((p - prev.0) / (next.0 - prev.0).max(1e-4)).clamp(0.0, 1.0);
            let e = match next.2 {
                Ease::In => t * t * t,
                Ease::Out => 1.0 - (1.0 - t).powi(3),
                Ease::Smooth => smoothstep(t),
            };
            return prev.1.lerp(next.1, e);
        }
        prev = next;
    }
    prev.1
}

/// The arms of a Shrike holding `weapon` mid-`swing`. `reach` is the total X swing the arm
/// chain needs to point the weapon along the look (90 degrees + pitch, minus the body's lean
/// and torso pitch: the gun pose's sum); `pitch` the look pitch.
pub fn arms(weapon: MeleeWeapon, swing: &Swing, reach: f32, pitch: f32) -> Arms {
    let rest = arm_rest(weapon);
    let pitch = pitch.clamp(-1.3, 1.3);
    // The hand angle that points the weapon along the look at rest.
    let aim = reach - (rest.raise + 0.45 * pitch) - rest.elbow;
    let active = swing.active();
    let key = |left: bool| -> ArmKey {
        let own = match weapon {
            MeleeWeapon::Trimmer => {
                active && (swing.left == left || swing.kind != SwingKind::Slash)
            }
            MeleeWeapon::Tamper => active,
        };
        if own {
            sample_arm(&arm_track(weapon, swing.kind), unit(swing.progress))
        } else {
            rest
        }
    };
    let r = key(false);
    let raise_r = r.raise + 0.45 * pitch;
    let (upper_l, fore_l, hand_l) = match weapon {
        // The left blade: a mirrored guard (or its own slash / the whirl).
        MeleeWeapon::Trimmer => {
            let l = key(true);
            (
                Quat::from_rotation_y(-l.yaw) * Quat::from_rotation_x(l.raise + 0.45 * pitch),
                l.elbow,
                Quat::from_rotation_y(l.twist) * Quat::from_rotation_x(aim + l.wrist),
            )
        }
        // Both hands on the haft: the left arm follows the right one.
        MeleeWeapon::Tamper => (
            Quat::from_rotation_y(-0.6 + r.yaw * 0.6) * Quat::from_rotation_x(raise_r * 0.95),
            r.elbow + 0.55,
            Quat::from_rotation_x(0.2),
        ),
    };
    Arms {
        upper_r: Quat::from_rotation_y(r.yaw) * Quat::from_rotation_x(raise_r),
        fore_r: r.elbow,
        hand_r: Quat::from_rotation_y(-r.twist) * Quat::from_rotation_x(aim + r.wrist),
        upper_l,
        fore_l,
        hand_l,
    }
}

/// The melee weapon a weapon is (`None`: a gun).
pub fn of(weapon: WeaponKind) -> Option<MeleeWeapon> {
    match weapon {
        WeaponKind::Trimmer => Some(MeleeWeapon::Trimmer),
        WeaponKind::Tamper => Some(MeleeWeapon::Tamper),
        _ => None,
    }
}

/// The melee weapon of a replicated / input weapon id.
pub fn of_id(id: u8) -> Option<MeleeWeapon> {
    WeaponKind::from_id(id).and_then(of)
}

/// The weapon a melee weapon is.
pub fn weapon_kind(m: MeleeWeapon) -> WeaponKind {
    match m {
        MeleeWeapon::Trimmer => WeaponKind::Trimmer,
        MeleeWeapon::Tamper => WeaponKind::Tamper,
    }
}

/// The Trimmer's left blade on a third-person Shrike: the local transform for
/// `trimmer::BLADE_L` that puts its grip in the left hand (`left_grip`: world transform of
/// the left hand's grip frame; `right_grip`: the weapon's root).
pub fn left_blade_local(right_grip: Mat4, left_grip: Mat4) -> Mat4 {
    Mat4::from_translation(-Vec3::from(trimmer::LEFT_GRIP)) * right_grip.inverse() * left_grip
}

// ---------------------------------------------------------------------------------------------
// Hurled

/// Draws a hurled blade (Trimmer) or hammer head (Tamper) at `position` flying along
/// `velocity`, spun end over end by `spin` radians. The head of a hurled Tamper is a hot
/// casting: its slag glows brighter.
#[allow(clippy::too_many_arguments)]
pub fn draw_hurled(
    lib: &ModelLibrary,
    weapon: MeleeWeapon,
    position: Vec3,
    velocity: Vec3,
    spin: f32,
    glow: Vec4,
    tint: &Tint,
    out: &mut Vec<DrawItem>,
) {
    let dir = velocity.normalize_or(Vec3::NEG_Z);
    let face = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    let rig = lib.get(model(weapon));
    let mut pose = Pose::rest(rig.parts.len());
    pose.glow = glow;
    pose.core = Vec4::new(1.6, 1.6, 1.6, 1.0);
    // Spin end over end about the flight's sideways axis, around the thrown piece's middle.
    let (center, parts): (Vec3, &[usize]) = match weapon {
        MeleeWeapon::Trimmer => (
            trimmer::BLADE_CENTER,
            &[trimmer::BLADE_R, trimmer::EDGE_R, trimmer::WICK],
        ),
        MeleeWeapon::Tamper => (
            tamper::head_center(),
            &[tamper::HEAD, tamper::BAND, tamper::VENT],
        ),
    };
    let scale = 1.25;
    let root = Mat4::from_translation(position)
        * Mat4::from_quat(face * Quat::from_rotation_x(-spin))
        * Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_translation(-center);
    let mut world = Vec::with_capacity(rig.parts.len());
    rig.world_transforms(&pose, root, &mut world);
    for &i in parts {
        if let Some(w) = world.get(i) {
            rig.draw_part(i, *w, &pose, tint, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEAPONS: [MeleeWeapon; 2] = [MeleeWeapon::Trimmer, MeleeWeapon::Tamper];
    const KINDS: [SwingKind; 5] = [
        SwingKind::Slash,
        SwingKind::Lunge,
        SwingKind::Whirl,
        SwingKind::Hurl,
        SwingKind::Upswing,
    ];

    #[test]
    fn swings_run_their_course_and_land_early() {
        for weapon in WEAPONS {
            for kind in KINDS {
                let mut s = Swing {
                    kind,
                    progress: 1e-3,
                    left: false,
                };
                assert!(s.active());
                let mut t = 0.0;
                while s.active() && t < 5.0 {
                    s.advance(weapon, 1.0 / 60.0);
                    t += 1.0 / 60.0;
                }
                let d = duration(weapon, kind);
                assert!((t - d).abs() < 0.05, "{weapon:?} {kind:?}: {t} vs {d}");
                assert!(!s.active() && s.progress == 0.0);
                // The blow lands within ~0.15 s of the click (the hit is on that tick).
                assert!(impact(weapon, kind) * d < 0.35, "{weapon:?} {kind:?}");
            }
            // The primary slash fits inside the weapon's starting interval.
            assert!(duration(weapon, SwingKind::Slash) <= 0.85);
        }
        // Garbage in: back to idle.
        let mut s = Swing {
            progress: f32::NAN,
            ..Swing::default()
        };
        s.advance(MeleeWeapon::Tamper, 0.1);
        assert_eq!(s.progress, 0.0);
    }

    #[test]
    fn every_pose_is_finite_and_the_rest_is_framed() {
        for weapon in WEAPONS {
            for kind in KINDS {
                for left in [false, true] {
                    for i in 0..=20 {
                        let s = Swing {
                            kind,
                            progress: i as f32 / 20.0,
                            left,
                        };
                        assert!(view_right(weapon, &s, Vec3::ZERO).is_finite());
                        assert!(view_left(&s, Vec3::ZERO).is_finite());
                        let a = arms(weapon, &s, 1.4, 0.3);
                        for q in [a.upper_r, a.hand_r, a.upper_l, a.hand_l] {
                            assert!(q.is_finite() && q.is_normalized());
                        }
                        assert!(pose(weapon, Vec3::ONE, s.flare(weapon), 3.0).is_finite());
                    }
                }
            }
            // At rest the grip sits lower right and the strike point is in front of the eye,
            // inside the 60 degree viewmodel frustum.
            let rest = view_right(weapon, &Swing::default(), Vec3::ZERO);
            let grip = rest.transform_point3(Vec3::ZERO);
            let bite = rest.transform_point3(muzzle(weapon));
            assert!(grip.x > 0.05 && grip.y < -0.05 && grip.z < -0.1, "{grip}");
            assert!(bite.z < grip.z, "{weapon:?}: {bite}");
            let tan = 30f32.to_radians().tan();
            assert!(bite.y / -bite.z > -tan && bite.x / -bite.z < tan * 16.0 / 9.0);
        }
    }

    #[test]
    fn the_trimmer_alternates_hands_and_the_slash_crosses_the_view() {
        let right = Swing {
            kind: SwingKind::Slash,
            progress: impact(MeleeWeapon::Trimmer, SwingKind::Slash),
            left: false,
        };
        let left = Swing {
            left: true,
            ..right
        };
        let rest = view_right(MeleeWeapon::Trimmer, &Swing::default(), Vec3::ZERO);
        // The right slash ends left of where it started; the left one mirrors it.
        let r = strike_path(MeleeWeapon::Trimmer, &right, 6);
        let l = strike_path(MeleeWeapon::Trimmer, &left, 6);
        assert!(r[0].x > r[5].x + 0.1, "{r:?}");
        assert!(l[0].x < l[5].x - 0.1, "{l:?}");
        // A left slash leaves the right blade in its guard.
        assert_eq!(view_right(MeleeWeapon::Trimmer, &left, Vec3::ZERO), rest);
        // The Tamper's slam ends lower than it starts.
        let slam = Swing {
            kind: SwingKind::Slash,
            progress: impact(MeleeWeapon::Tamper, SwingKind::Slash),
            left: false,
        };
        let p = strike_path(MeleeWeapon::Tamper, &slam, 6);
        assert!(p[0].y > p[5].y + 0.1, "{p:?}");
    }

    #[test]
    fn the_left_blade_lands_in_the_left_hand() {
        let right = Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::X);
        let left = Mat4::from_rotation_translation(Quat::from_rotation_x(-0.4), -Vec3::X);
        let local = left_blade_local(right, left);
        // world = root * bind (pivot) * local, mesh in joint space (grip at the origin).
        let world = right * Mat4::from_translation(Vec3::from(trimmer::LEFT_GRIP)) * local;
        assert!(world.abs_diff_eq(left, 1e-5));
        assert_eq!(of_id(10), Some(MeleeWeapon::Trimmer));
        assert_eq!(of_id(11), Some(MeleeWeapon::Tamper));
        assert_eq!(of_id(3), None);
    }
}
