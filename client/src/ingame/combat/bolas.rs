//! The Bolas (docs/rework.md §1, Scatter, area **client-b**): two wax-weighted balls on a
//! wick cord.
//!
//! * **In flight** ([`draw_flight`], [`flight_trail`]): the `ModelId::Bolas` spinning flat
//!   about the flight's up axis (the cord stretched between the balls), embers shaken off the
//!   cord; `Sfx::BolasThrow` as it leaves.
//! * **Bound** ([`bind`], [`draw_bound`]): the cord whips round the first hostile it touches —
//!   a burst of embers, `Sfx::BolasBind` — and stays wound round its legs, the balls hanging
//!   at its knees and swinging, for as long as it is held.
//! * **The link** ([`link_snap`], [`link_cord`], [`share_pulse`]): a second hostile close by
//!   is caught too — the cord snaps across to it (`Sfx::BolasLink`) and hangs between the two
//!   as a sagging, smouldering line in the ability's colour while the link lasts, a pulse
//!   running along it (and `Sfx::BolasShare`) whenever damage is carried across.

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::DrawItem;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::super::effects::{Effects, FxTex, Particle};
use super::Rng;
use super::fx::Beams;
use crate::models::gear::BOLAS_COLOR;
use crate::models::gear::BOLAS_REACH;
use crate::models::{ModelId, ModelLibrary, Tint};

/// Drawn this much larger than life in flight (it reads at a distance).
const FLIGHT_SCALE: f32 = 1.5;
/// Spins this fast in flight (rad/s).
const SPIN: f32 = 14.0;

/// The cord's colour (the ability's), pushed hot for unlit drawing.
pub fn cord_color(k: f32, a: f32) -> Vec4 {
    (BOLAS_COLOR.truncate() * k).extend(a)
}

/// Draws the bolas flying at `position` along `velocity`, `age` seconds after the throw.
pub fn draw_flight(
    lib: &ModelLibrary,
    position: Vec3,
    velocity: Vec3,
    age: f32,
    draws: &mut Vec<DrawItem>,
) {
    let dir = velocity.normalize_or(Vec3::NEG_Z);
    // Level with the flight, spinning about its up (the balls sweep a disc round the cord).
    let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::NEG_Z);
    let face = Quat::from_rotation_arc(Vec3::NEG_Z, flat);
    let spin = Quat::from_rotation_y(age * SPIN);
    let root =
        Mat4::from_scale_rotation_translation(Vec3::splat(FLIGHT_SCALE), face * spin, position);
    let mut pose = lib.get(ModelId::Bolas).rest_pose();
    let g = 1.3 + 0.3 * (age * 9.0).sin();
    pose.core = Vec4::new(g, g * 0.9, g * 0.8, 1.0);
    lib.get(ModelId::Bolas)
        .draw(&pose, root, &Tint::default(), draws);
}

/// The bolas' wake in flight: embers shaken off the spinning cord.
pub fn flight_trail(effects: &mut Effects, rng: &mut Rng, position: Vec3, age: f32, dt: f32) {
    if rng.unit() < dt * 45.0 {
        let a = age * SPIN + rng.angle();
        let r = BOLAS_REACH * FLIGHT_SCALE * rng.range(0.2, 1.0);
        effects.spawn(Particle {
            position: position + Vec3::new(a.cos() * r, 0.0, a.sin() * r),
            velocity: Vec3::new(a.cos(), 0.3, a.sin()) * 1.2,
            gravity: -0.4,
            max_life: 0.3,
            size: Vec2::splat(0.06),
            color: cord_color(1.6, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// The bolas wrapping round a hostile at `feet` (`height` tall): the cord whipping round,
/// embers, the weights thudding together.
pub fn bind(effects: &mut Effects, beams: &mut Beams, audio: &mut Audio, feet: Vec3, height: f32) {
    let knees = feet + Vec3::Y * (height * 0.3).clamp(0.3, 0.9);
    for k in 0..3 {
        beams.ring(
            knees + Vec3::Y * (0.12 * k as f32),
            Vec3::Y,
            1.4 - 0.2 * k as f32,
            0.55,
            0.06,
            cord_color(1.8, 0.9),
            0.18 + 0.05 * k as f32,
        );
    }
    effects.glow(knees, cord_color(1.2, 0.8), 1.0, 0.2);
    for k in 0..12 {
        let a = k as f32 / 12.0 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), 0.15, a.sin());
        effects.spawn(Particle {
            position: knees + d * 0.5,
            velocity: d * 3.0,
            gravity: 3.0,
            drag: 1.5,
            max_life: 0.35,
            size: Vec2::splat(0.07),
            color: cord_color(1.6, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    audio.play(Sfx::BolasBind, PlayParams::at(knees));
}

/// Where the cord is wound on a body at `feet`, `height` tall and `radius` wide.
fn wrap_point(feet: Vec3, height: f32, radius: f32, a: f32, h: f32) -> Vec3 {
    let r = radius + 0.08;
    feet + Vec3::new(a.cos() * r, (height * h).clamp(0.2, 1.1), a.sin() * r)
}

/// A bound body this frame (`feet`, `height` tall, `radius` wide): the cord wound twice
/// round its legs, a spiral of short smouldering segments. `left` 0..1 of the bind's time (the
/// cord dims as it loosens).
pub fn wrap(beams: &mut Beams, feet: Vec3, height: f32, radius: f32, left: f32, time: f64) {
    let k = left.clamp(0.0, 1.0);
    let c = cord_color(1.4, 0.55 + 0.4 * k);
    let n = 16;
    let turn = (time * 0.4) as f32;
    let point = |i: usize| {
        let t = i as f32 / n as f32;
        wrap_point(
            feet,
            height,
            radius,
            turn + t * std::f32::consts::TAU * 2.0,
            0.22 + 0.16 * t,
        )
    };
    for i in 0..n {
        beams.frame_beam(point(i), point(i + 1), 0.06, c);
    }
}

/// The weights of a bound body's bolas: the slack end of the cord hanging off the wrap,
/// swinging a little (the model turned upright: one ball at the knot, the other dangling).
#[allow(clippy::too_many_arguments)]
pub fn draw_weights(
    lib: &ModelLibrary,
    feet: Vec3,
    height: f32,
    radius: f32,
    left: f32,
    time: f64,
    draws: &mut Vec<DrawItem>,
) {
    let k = left.clamp(0.0, 1.0);
    let turn = (time * 0.4) as f32;
    let swing = 0.3 * ((time * 3.1) as f32).sin();
    let knot = wrap_point(feet, height, radius, turn + 0.6, 0.24);
    let scale = 0.6;
    let hang = Quat::from_rotation_y(turn + 0.6)
        * Quat::from_rotation_x(swing * 0.5)
        * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2 + swing);
    let root = Mat4::from_scale_rotation_translation(
        Vec3::splat(scale),
        hang,
        knot + hang * (Vec3::X * BOLAS_REACH * scale),
    );
    let mut pose = lib.get(ModelId::Bolas).rest_pose();
    let g = 1.2 * k + 0.3;
    pose.core = Vec4::new(g, g * 0.9, g * 0.8, 1.0);
    lib.get(ModelId::Bolas)
        .draw(&pose, root, &Tint::default(), draws);
}

/// The cord between two linked bodies (chests `a` and `b`), this frame: sagging, smouldering,
/// a little slack swinging; `left` 0..1 of the link's time, `pulse` 0..1 a share just carried
/// along it (a bright knot running from `a` to `b`).
pub fn link_cord(beams: &mut Beams, a: Vec3, b: Vec3, left: f32, pulse: f32, time: f64) {
    if !(a.is_finite() && b.is_finite()) {
        return;
    }
    let k = left.clamp(0.0, 1.0);
    let span = a.distance(b);
    let sag = (0.15 + 0.08 * span) * (1.0 + 0.1 * ((time * 2.0).sin() as f32));
    let n = 14;
    let point = |i: usize| {
        let t = i as f32 / n as f32;
        a.lerp(b, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t)
    };
    let glow = 0.9 + 0.25 * ((time * 7.0).sin() as f32) * k;
    for i in 0..n {
        beams.frame_beam(
            point(i),
            point(i + 1),
            0.08,
            cord_color(1.7 * glow, 0.55 + 0.4 * k),
        );
    }
    if pulse > 0.0 {
        let p = pulse.clamp(0.0, 1.0);
        let at = point(((1.0 - p) * n as f32) as usize);
        beams.frame_beam(
            at - Vec3::Y * 0.12,
            at + Vec3::Y * 0.12,
            0.18,
            cord_color(3.0, p),
        );
    }
}

/// The cord snapping across from the first bound body to a second one (`from`, `to`: chests).
pub fn link_snap(
    effects: &mut Effects,
    beams: &mut Beams,
    audio: &mut Audio,
    from: Vec3,
    to: Vec3,
) {
    beams.beam(from, to, 0.1, cord_color(2.4, 1.0), 0.2);
    for at in [from, to] {
        effects.glow(at, cord_color(1.4, 0.8), 0.8, 0.2);
        effects.sparks(at, Vec3::Y, 6);
    }
    audio.play(Sfx::BolasLink, PlayParams::at(from.lerp(to, 0.5)));
}

/// Damage carried along a link (to `to`): the thrum, sparks where it arrives.
pub fn share_pulse(effects: &mut Effects, audio: &mut Audio, to: Vec3) {
    effects.glow(to, cord_color(1.6, 0.8), 0.6, 0.15);
    audio.play(Sfx::BolasShare, PlayParams::at(to));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cord_sags_between_its_ends_and_stays_finite() {
        let mut beams = Beams::headless();
        link_cord(&mut beams, Vec3::ZERO, Vec3::X * 5.0, 1.0, 0.5, 1.0);
        link_cord(&mut beams, Vec3::NAN, Vec3::X, 1.0, 0.0, 0.0);
        let p = wrap_point(Vec3::ZERO, 2.0, 0.5, 0.0, 0.3);
        assert!(p.is_finite() && (p.y - 0.6).abs() < 1e-5);
        assert!(cord_color(1.0, 1.0).is_finite());
    }
}
