//! Drinking the Kindle Flask in first person (docs/rework.md §3, area **client-b**): the
//! left hand brings the flask up from below the screen into the lower middle of the view,
//! tips it back (its neck to the mouth, its bottom rising) while the kindling inside flares
//! and drains, then drops the empty flask away to the lower left.
//! [`ViewmodelAnim::drink`](super::ViewmodelAnim::drink) is the progress 0..1 over
//! [`DRINK_TIME`]; the gun dips out of the way meanwhile ([`lowering`]).
//!
//! The hand is the knacks' left fist (`fist` model: knuckles at the origin pointing -Z), its
//! top turned towards the flask it grips from below left; the flask (`ModelId::KindleFlask`,
//! neck +Y) is placed in view space beside it ([`flask`]), so tipping it never swings the
//! fist across the view.

use engine::glam::{EulerRot, Mat4, Quat, Vec3};

use super::{VIEWMODEL_SCALE, smoothstep, unit, wave};

/// Seconds the drink takes on screen (the haste and the heal start at once: the drink is
/// presentation only).
pub const DRINK_TIME: f32 = 0.75;

/// Raised by this much of the drink.
const RAISED: f32 = 0.28;
/// Tipped back between these (then lowered again).
const TIP: (f32, f32) = (0.24, 0.66);
/// Dropped away from here.
const AWAY: f32 = 0.7;
/// The flask's drawn scale in view space (a little under the throw's: the fist's x 0.42).
const FLASK_SCALE: f32 = VIEWMODEL_SCALE * 0.42;

/// How far the hand is raised (0 below the screen .. 1 up).
fn raise(p: f32) -> f32 {
    smoothstep(p / RAISED)
}

/// How far the flask is tipped back (0 upright .. 1 drained).
fn tip(p: f32) -> f32 {
    smoothstep((p - TIP.0) / (TIP.1 - TIP.0) * 1.6) * (1.0 - smoothstep((p - TIP.1) / 0.1))
}

fn away(p: f32) -> f32 {
    smoothstep((p - AWAY) / (1.0 - AWAY))
}

/// Drinking at progress `p` (0 and 1 = not drinking)?
pub fn drinking(p: f32) -> bool {
    p > 0.0 && p < 1.0 && p.is_finite()
}

/// Where the hand is (view space) at `p`: below the screen, up under the flask, a little
/// higher while it tips, off the lower left.
fn hand_position(p: f32) -> Vec3 {
    let hidden = Vec3::new(-0.17, -0.34, -0.15);
    let up = Vec3::new(-0.062, -0.138, -0.225);
    let gone = Vec3::new(-0.27, -0.38, -0.2);
    hidden.lerp(up, raise(p)).lerp(gone, away(p)) + Vec3::new(0.012, 0.018, 0.012) * tip(p)
}

/// View-space transform of the left fist holding the flask (`None` when not drinking).
pub fn hand(p: f32) -> Option<Mat4> {
    if !drinking(p) {
        return None;
    }
    let (raise, tip, away) = (raise(p), tip(p), away(p));
    // Knuckles forward and up, the fist's top rolled towards the flask at its upper right.
    let rot = Quat::from_euler(
        EulerRot::YXZ,
        -0.3 + 0.15 * raise - 0.3 * away,
        0.35 + 0.35 * tip - 0.3 * away,
        -0.55 * raise - 0.2 * tip + 0.5 * away,
    );
    Some(Mat4::from_scale_rotation_translation(
        Vec3::splat(VIEWMODEL_SCALE),
        rot,
        hand_position(p),
    ))
}

/// View-space transform of the flask while `p` is drinking (`None` once it has dropped out
/// of sight): upright above the hand, then tipped back about the view's X axis until its
/// neck points at the mouth and its bottom rises in front of the eye.
pub fn flask(p: f32, time: f64) -> Option<Mat4> {
    if !drinking(p) || p > 0.92 {
        return None;
    }
    let (tip, away) = (tip(p), away(p));
    let at = hand_position(p) + Vec3::new(0.034, 0.045 + 0.02 * tip, 0.004 + 0.026 * tip);
    let wobble = 0.08 * wave(time, 2.7) * (1.0 - tip);
    let rot = Quat::from_euler(
        EulerRot::YXZ,
        0.2 + wobble,
        1.5 * tip - 0.4 * away,
        -0.22 + 0.12 * tip + 0.6 * away,
    );
    Some(Mat4::from_scale_rotation_translation(
        Vec3::splat(FLASK_SCALE),
        rot,
        at,
    ))
}

/// The stopper is out (pulled as the flask comes up).
pub fn unstoppered(p: f32) -> bool {
    drinking(p) && p > RAISED * 0.6
}

/// The kindling's glow in the flask: bright as it is raised, flaring white-hot as it pours
/// out, dull once it is empty.
pub fn flask_glow(p: f32) -> f32 {
    let p = unit(p);
    let pour = tip(p);
    let drained = smoothstep((p - TIP.0) / (TIP.1 - TIP.0));
    (1.3 + 2.4 * pour) * (1.0 - 0.8 * drained) + 0.15
}

/// The third-person drink at `p`: how far the left hand is raised to the mask and how far
/// the flask (and the head) are tipped back (both 0..1).
pub fn third_person(p: f32) -> (f32, f32) {
    if drinking(p) {
        (raise(p) * (1.0 - away(p)), tip(p))
    } else {
        (0.0, 0.0)
    }
}

/// The flask in a third-person Shrike's left hand (hand space: in the palm, its neck up
/// along the raised forearm), or `None` once it is drunk.
pub fn third_person_flask(p: f32) -> Option<Mat4> {
    (drinking(p) && p < 0.88).then(|| {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(0.8),
            Quat::from_rotation_x(std::f32::consts::PI + 0.3),
            Vec3::new(0.0, -0.1, -0.03),
        )
    })
}

/// How far the gun dips out of the way of the drink (0..1, like a punch's extension).
pub fn lowering(p: f32) -> f32 {
    if drinking(p) {
        raise(p) * (1.0 - away(p))
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inside the 60° viewmodel frustum (16:9), with a margin.
    fn on_screen(v: Vec3) -> bool {
        let tan = 30f32.to_radians().tan();
        v.z < -0.05 && v.y.abs() < -v.z * tan && v.x.abs() < -v.z * tan * 16.0 / 9.0
    }

    #[test]
    fn the_flask_comes_up_tips_back_and_goes() {
        assert!(hand(0.0).is_none() && hand(1.0).is_none() && hand(f32::NAN).is_none());
        let at = |p: f32| {
            flask(p, 0.0)
                .map(|m| m.w_axis.truncate())
                .unwrap_or(Vec3::NAN)
        };
        assert!(
            !on_screen(at(0.01)),
            "starts below the screen: {}",
            at(0.01)
        );
        for p in [0.3, 0.45, 0.6] {
            assert!(on_screen(at(p)), "{p}: {}", at(p));
            assert!(at(p).x.abs() < 0.06, "near the middle: {}", at(p));
        }
        let hand_at = |p: f32| hand(p).map(|m| m.w_axis.truncate()).unwrap();
        assert!(hand_at(0.98).y < hand_at(0.45).y - 0.1, "dropped away");
        // Tipped back: the flask's neck (+Y) turns towards the camera (+Z).
        let neck = |p: f32| flask(p, 0.0).map(|m| m.transform_vector3(Vec3::Y).normalize());
        let (upright, tipped) = (neck(0.28).unwrap(), neck(0.5).unwrap());
        assert!(upright.y > 0.8 && tipped.z > 0.8, "{upright} -> {tipped}");
        assert!(flask(0.5, 0.0).is_some() && flask(0.95, 0.0).is_none());
        assert!(flask_glow(0.5) > flask_glow(0.1) && flask_glow(0.85) < flask_glow(0.1));
        assert!(lowering(0.4) > 0.5 && lowering(0.0) == 0.0);
    }
}
