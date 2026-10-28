//! The Wisp's twist (docs/rework.md §1, Rapid, area **client-b**): the ball of lightning now
//! orbits its caller (1.6 m out at chest height) and zaps what comes near (`Fx::WispZap`, as
//! before), until it is flung along the aim and pops.
//!
//! * **Orbiting** ([`orbit_position`], [`draw_orbiting`], [`orbit_effects`]): `ModelId::Wisp`
//!   tumbling on its circle, a faint violet track of its last half turn, its crackle and hum;
//!   in first person it sweeps through the view once a turn.
//! * **Flung** ([`fling`]): a violet flash where it leaves the orbit and `Sfx::WispFling`;
//!   in flight it is the Wisp projectile as before, its pop the Wisp's explosion.

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::DrawItem;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::super::effects::{Effects, FxTex, Particle};
use super::Rng;
use super::fx::Beams;
use crate::models::{ModelId, ModelLibrary, Tint, WeaponKind, WeaponLook as _};

/// The orbit's radius and height above the caller's feet (docs/rework.md: 1.6 m, chest).
pub const ORBIT_RADIUS: f32 = 1.6;
pub const ORBIT_HEIGHT: f32 = 1.2;
/// Turns per second round the caller.
pub const ORBIT_RATE: f32 = 0.55;
/// Drawn this much larger than its model.
const SCALE: f32 = 1.25;

fn violet(k: f32, a: f32) -> Vec4 {
    (WeaponKind::Arclash.glow_color(0).truncate() * k).extend(a)
}

/// The orbit's angle `age` seconds after it started (`phase`: where it started).
pub fn orbit_angle(age: f32, phase: f32) -> f32 {
    phase + age * ORBIT_RATE * std::f32::consts::TAU
}

/// Where an orbiting wisp is round a caller standing at `feet`, at orbit angle `angle`.
pub fn orbit_position(feet: Vec3, angle: f32) -> Vec3 {
    feet + Vec3::new(
        angle.cos() * ORBIT_RADIUS,
        ORBIT_HEIGHT + 0.08 * (angle * 3.0).sin(),
        angle.sin() * ORBIT_RADIUS,
    )
}

/// Draws an orbiting wisp at `position` (tumbling like a flying one).
pub fn draw_orbiting(lib: &ModelLibrary, position: Vec3, time: f64, draws: &mut Vec<DrawItem>) {
    let rot =
        Quat::from_rotation_y((time * 7.0) as f32) * Quat::from_rotation_x((time * 4.3) as f32);
    let root = Mat4::from_scale_rotation_translation(Vec3::splat(SCALE), rot, position);
    lib.draw_model(ModelId::Wisp, root, &Tint::default(), draws);
}

/// An orbiting wisp's frame round `feet` at `angle`: a fading violet track over its last half
/// turn, its glow and crackle. `own`: round you (thinner, so it never hides your aim).
#[allow(clippy::too_many_arguments)]
pub fn orbit_effects(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    feet: Vec3,
    angle: f32,
    own: bool,
    dt: f32,
) {
    let pos = orbit_position(feet, angle);
    let n = 10;
    let width = if own { 0.025 } else { 0.05 };
    for i in 0..n {
        let a0 = angle - i as f32 / n as f32 * std::f32::consts::PI;
        let a1 = angle - (i + 1) as f32 / n as f32 * std::f32::consts::PI;
        let fade = 1.0 - i as f32 / n as f32;
        beams.frame_beam(
            orbit_position(feet, a0),
            orbit_position(feet, a1),
            width,
            violet(1.5, 0.6 * fade),
        );
    }
    effects.billboard(pos, 0.9, violet(1.3, 0.6), FxTex::Glow, 0.0);
    if rng.unit() < dt * 12.0 {
        let to = pos + rng.dir() * rng.range(0.3, 0.7);
        beams.arc(pos, to, 0.025, violet(1.8, 0.9), 0.07, rng);
    }
    if rng.unit() < dt * 20.0 {
        effects.spawn(Particle {
            position: pos + rng.dir() * 0.12,
            velocity: rng.dir() * 0.6,
            max_life: 0.25,
            size: Vec2::splat(0.08),
            color: violet(1.8, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// A flung wisp's frame at `position` (the staged one; a replicated one is the Wisp
/// projectile's): its glow and crackle.
pub fn flight_effects(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    position: Vec3,
    dt: f32,
) {
    effects.billboard(position, 1.0, violet(1.3, 0.7), FxTex::Glow, 0.0);
    if rng.unit() < dt * 16.0 {
        let to = position + rng.dir() * rng.range(0.4, 0.9);
        beams.arc(position, to, 0.03, violet(1.8, 0.9), 0.07, rng);
    }
}

/// The wisp flung off its orbit at `at` along `dir`: a flash, a spray of arcs, the sound.
pub fn fling(
    effects: &mut Effects,
    beams: &mut Beams,
    audio: &mut Audio,
    rng: &mut Rng,
    at: Vec3,
    dir: Vec3,
) {
    effects.glow(at, violet(1.6, 0.9), 1.4, 0.18);
    for _ in 0..4 {
        let to = at - dir.normalize_or(Vec3::NEG_Z) * rng.range(0.5, 1.2) + rng.dir() * 0.4;
        beams.arc(at, to, 0.03, violet(1.8, 0.9), 0.1, rng);
    }
    audio.play(Sfx::WispFling, PlayParams::at(at));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wisp_circles_its_caller_at_chest_height() {
        let feet = Vec3::new(3.0, 1.0, -2.0);
        for age in [0.0f32, 0.3, 1.1, 5.9] {
            let p = orbit_position(feet, orbit_angle(age, 0.4));
            let flat = Vec2::new(p.x - feet.x, p.z - feet.z).length();
            assert!((flat - ORBIT_RADIUS).abs() < 1e-4);
            assert!((p.y - feet.y - ORBIT_HEIGHT).abs() < 0.1);
        }
        // A full turn in 1 / ORBIT_RATE seconds.
        let a = orbit_angle(1.0 / ORBIT_RATE, 0.0);
        assert!((a - std::f32::consts::TAU).abs() < 1e-4);
    }
}
