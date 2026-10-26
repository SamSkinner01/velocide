//! Positional audio math: distance attenuation and stereo panning relative to the listener.
//!
//! Computed on the game thread when a sound starts and refreshed every frame while it plays
//! (the emitter stays where it started; the listener may move). Kept free of kira so it can be
//! unit-tested.

use glam::{Quat, Vec3};

/// Sounds closer than this play non-spatialised (centred, full volume).
pub const NEAR_DISTANCE: f32 = 1.0;
/// Full volume up to this distance; inverse-distance roll-off beyond.
pub const REF_DISTANCE: f32 = 2.0;
/// Inaudible at and beyond this distance.
pub const MAX_DISTANCE: f32 = 60.0;
/// Roll-off factor of the inverse-distance curve (lower = carries further).
pub const ROLLOFF: f32 = 0.6;
/// Sounds are never panned harder than this: fully hard-panned sounds are uncomfortable on
/// headphones and vanish on one-speaker setups.
pub const MAX_PAN: f32 = 0.85;
/// Extra attenuation of sounds directly behind the listener (a cheap front/back cue).
pub const BEHIND_ATTENUATION: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    /// Linear gain in `0..=1`.
    pub gain: f32,
    /// Stereo pan in `-1..=1` (negative = left).
    pub pan: f32,
}

impl Spatial {
    pub const CENTER: Spatial = Spatial {
        gain: 1.0,
        pan: 0.0,
    };
}

/// Distance attenuation: 1 within [`REF_DISTANCE`], inverse-distance roll-off beyond, eased to
/// exactly 0 at [`MAX_DISTANCE`].
pub fn attenuation(distance: f32) -> f32 {
    if !distance.is_finite() || distance >= MAX_DISTANCE {
        return 0.0;
    }
    if distance <= REF_DISTANCE {
        return 1.0;
    }
    let inverse = REF_DISTANCE / (REF_DISTANCE + ROLLOFF * (distance - REF_DISTANCE));
    let edge = (distance - REF_DISTANCE) / (MAX_DISTANCE - REF_DISTANCE);
    inverse * (1.0 - edge * edge)
}

/// Gain and pan of a source at `source` heard by a listener at `listener_pos` with orientation
/// `listener_rot` (the camera: looks down -Z, +X is right, +Y is up).
pub fn spatialize(listener_pos: Vec3, listener_rot: Quat, source: Vec3) -> Spatial {
    let offset = source - listener_pos;
    let distance = offset.length();
    if !distance.is_finite() {
        return Spatial {
            gain: 0.0,
            pan: 0.0,
        };
    }
    if distance <= NEAR_DISTANCE {
        return Spatial::CENTER;
    }
    let local = listener_rot.inverse() * (offset / distance);
    // Fade spatialisation in over the first metre past NEAR_DISTANCE so nothing jumps.
    let strength = ((distance - NEAR_DISTANCE) / NEAR_DISTANCE).clamp(0.0, 1.0);
    let pan = (local.x * MAX_PAN * strength).clamp(-1.0, 1.0);
    let behind = local.z.max(0.0) * strength;
    Spatial {
        gain: attenuation(distance) * (1.0 - BEHIND_ATTENUATION * behind),
        pan,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_sounds_are_not_spatialised() {
        let s = spatialize(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.5, 0.0, 0.3));
        assert_eq!(s, Spatial::CENTER);
    }

    #[test]
    fn attenuation_is_monotonic_and_reaches_zero() {
        let mut last = 1.0;
        for i in 0..700 {
            let a = attenuation(i as f32 * 0.1);
            assert!(a <= last + 1e-6 && (0.0..=1.0).contains(&a));
            last = a;
        }
        assert_eq!(attenuation(MAX_DISTANCE), 0.0);
        assert_eq!(attenuation(f32::NAN), 0.0);
    }

    #[test]
    fn pans_follow_listener_orientation() {
        // Identity: looking down -Z, +X is to the right.
        let right = spatialize(Vec3::ZERO, Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0));
        assert!(right.pan > 0.8);
        let left = spatialize(Vec3::ZERO, Quat::IDENTITY, Vec3::new(-10.0, 0.0, 0.0));
        assert!(left.pan < -0.8);
        let ahead = spatialize(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.0, 0.0, -10.0));
        assert!(ahead.pan.abs() < 1e-5);
        // Turned 90° left (positive yaw): the +X source is now behind us; -Z is on our right.
        let yaw_left = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let s = spatialize(Vec3::ZERO, yaw_left, Vec3::new(0.0, 0.0, -10.0));
        assert!(s.pan > 0.8, "pan {}", s.pan);
        let behind = spatialize(Vec3::ZERO, yaw_left, Vec3::new(10.0, 0.0, 0.0));
        assert!(behind.gain < ahead.gain);
    }

    #[test]
    fn spatialisation_is_continuous_at_the_near_threshold() {
        let a = spatialize(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(NEAR_DISTANCE - 1e-3, 0.0, 0.0),
        );
        let b = spatialize(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(NEAR_DISTANCE + 1e-3, 0.0, 0.0),
        );
        assert!((a.pan - b.pan).abs() < 0.01 && (a.gain - b.gain).abs() < 0.01);
    }
}
