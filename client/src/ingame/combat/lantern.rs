//! The Lantern (docs/rework.md §1, Rapid, area **client-b**): a brass lantern set down that
//! fires its carrier's primary at the nearest hostile for a few seconds.
//!
//! * **Tossed and set down** ([`draw`], [`set_down`]): `ModelId::Lantern` tumbling, then
//!   standing on its feet with a clank, its taper catching (`Sfx::LanternSet`); a warm light
//!   round it ([`light`]) and its lens in the carrier's primary colour.
//! * **Firing** ([`shot`]): the cage turns and tilts to its target ([`aim`]), the lens flares
//!   and the shot leaves it as the carrier's would — an Arclash's jagged zap, a Riveter's
//!   rivet streak, a Kiln's cone of embers — with the primary's own sound, quieter.
//! * **Its end** ([`burn_out`]): the taper gutters in its last moments, then the lantern goes
//!   dark in a puff of smoke with `Sfx::LanternOut`; destroyed, it bursts in glass and brass.

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::loadout::WeaponKind;
use shared::sim::Attack;

use super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::Rng;
use super::arsenal::kiln_spray;
use super::fx::Beams;
use crate::models::gear::{self, LANTERN_COLOR, LANTERN_MUZZLE};
use crate::models::{ModelId, ModelLibrary, Tint, WeaponLook as _};

/// Drawn this much larger than life (it reads at a distance).
pub const SCALE: f32 = 1.3;

/// The lens's colour: the carrier's primary colour.
pub fn lens_color(carrier: WeaponKind) -> Vec3 {
    carrier.primary_color().truncate()
}

/// The cage's yaw and pitch to point the lens at `target` from a lantern standing at `feet`
/// (yaw like a look: 0 = -Z, + = left).
pub fn aim(feet: Vec3, target: Vec3) -> (f32, f32) {
    let lens = feet + Vec3::Y * LANTERN_MUZZLE.y * SCALE;
    let d = target - lens;
    let flat = Vec2::new(d.x, d.z).length();
    if flat < 1e-3 {
        return (0.0, 0.0);
    }
    ((-d.x).atan2(-d.z), d.y.atan2(flat))
}

/// The lantern's root at `feet`: on the floor, or tumbling `spin` radians in flight.
pub fn root(feet: Vec3, spin: f32) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::splat(SCALE),
        Quat::from_rotation_x(spin) * Quat::from_rotation_z(spin * 0.4),
        feet,
    )
}

/// Where its lens is (world) for a lantern at `feet` aimed `yaw` / `pitch`.
pub fn muzzle(feet: Vec3, yaw: f32, pitch: f32) -> Vec3 {
    let pose = gear::lantern_pose(yaw, pitch, 0.0, Vec3::ONE, 1.0, 0.0);
    let head = pose.locals[gear::LANTERN_HEAD];
    let pivot = Vec3::new(0.0, 0.16, 0.0);
    feet + (head.transform_point3(LANTERN_MUZZLE - pivot) + pivot) * SCALE
}

/// Draws a lantern standing at `feet` (`spin` ≠ 0: tumbling in flight), aimed `yaw` /
/// `pitch`, `kick` 0..1 a shot just fired, `left` 0..1 of its time.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    lib: &ModelLibrary,
    feet: Vec3,
    spin: f32,
    yaw: f32,
    pitch: f32,
    kick: f32,
    carrier: WeaponKind,
    left: f32,
    time: f64,
    draws: &mut Vec<DrawItem>,
) {
    let pose = gear::lantern_pose(yaw, pitch, kick, lens_color(carrier), left, time);
    lib.get(ModelId::Lantern)
        .draw(&pose, root(feet, spin), &Tint::default(), draws);
}

/// The warm light a lit lantern casts (dimming as it gutters).
pub fn light(feet: Vec3, left: f32, time: f64) -> PointLight {
    let k = if left < 0.2 { left / 0.2 } else { 1.0 };
    let flicker = 0.9 + 0.1 * ((time * 9.0).sin() as f32);
    PointLight {
        position: feet + Vec3::Y * 0.5 * SCALE,
        color: LANTERN_COLOR.truncate() * 1.6 * flicker * k,
        radius: 5.0,
    }
}

/// Set down at `feet`: a clank, a puff of dust, the taper catching.
pub fn set_down(effects: &mut Effects, audio: &mut Audio, feet: Vec3) {
    effects.dust(feet, 6, 0.5);
    effects.glow(
        feet + Vec3::Y * 0.4 * SCALE,
        (LANTERN_COLOR.truncate() * 1.4).extend(0.8),
        0.8,
        0.25,
    );
    audio.play(Sfx::LanternSet, PlayParams::at(feet));
}

/// A shot of the carrier's primary from the lens at `from` to `to`: the zap, the rivet's
/// streak or the cone, the lens's flash, the primary's sound (quieter).
#[allow(clippy::too_many_arguments)]
pub fn shot(
    effects: &mut Effects,
    beams: &mut Beams,
    audio: &mut Audio,
    rng: &mut Rng,
    carrier: WeaponKind,
    from: Vec3,
    to: Vec3,
    cone: (f32, f32),
) {
    let c = lens_color(carrier);
    let dir = (to - from).normalize_or(Vec3::NEG_Z);
    effects.glow(from, (c * 1.6).extend(0.9), 0.45, 0.07);
    let style = super::attack_style(Attack::primary(carrier));
    match carrier {
        WeaponKind::Arclash => {
            beams.arc(from, to, 0.045, (c * 1.8).extend(0.95), 0.1, rng);
            effects.sparks(to, -dir, 4);
        }
        WeaponKind::Kiln => kiln_spray(effects, rng, from, dir, cone.0, cone.1),
        _ => {
            // A projectile's streak (the rivet itself is the replicated projectile): a short
            // hot line out of the lens.
            beams.beam(from, from + dir * 1.6, 0.05, (c * 1.6).extend(0.8), 0.06);
            effects.spawn(Particle {
                position: from,
                velocity: dir * 30.0,
                max_life: (from.distance(to) / 30.0).clamp(0.05, 0.6),
                size: Vec2::splat(0.12),
                end_scale: 1.0,
                color: (c * 1.8).extend(1.0),
                end_alpha: 1.0,
                texture: FxTex::Spark,
                additive: true,
                collide: Some(OnHit::Die),
                ..Default::default()
            });
        }
    }
    audio.play(style.sfx, PlayParams::at(from).with_volume(0.55));
}

/// Its time up at `feet` (`destroyed`: broken by a Mould instead): gone dark in smoke, or
/// burst in glass and brass.
pub fn burn_out(
    effects: &mut Effects,
    audio: &mut Audio,
    rng: &mut Rng,
    feet: Vec3,
    destroyed: bool,
) {
    let at = feet + Vec3::Y * 0.4 * SCALE;
    effects.smoke(at, if destroyed { 6 } else { 3 });
    if destroyed {
        for k in 0..12 {
            effects.spawn(Particle {
                position: at,
                velocity: (rng.dir() + Vec3::Y * 0.6) * rng.range(1.5, 4.0),
                gravity: 12.0,
                max_life: rng.range(0.4, 0.8),
                size: Vec2::splat(0.06),
                color: if k % 2 == 0 {
                    Vec4::new(0.95, 0.92, 0.8, 1.0)
                } else {
                    Vec4::new(0.8, 0.6, 0.27, 1.0)
                },
                texture: FxTex::Shard,
                rotation: rng.angle(),
                spin: rng.range(-10.0, 10.0),
                collide: Some(OnHit::Bounce(0.3)),
                ..Default::default()
            });
        }
        effects.glow(at, (LANTERN_COLOR.truncate() * 1.6).extend(0.9), 1.2, 0.2);
    }
    audio.play(Sfx::LanternOut, PlayParams::at(at));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_points_at_its_target() {
        let feet = Vec3::new(2.0, 0.0, 1.0);
        for target in [
            Vec3::new(2.0, 1.0, -8.0),
            Vec3::new(-6.0, 0.4, 1.0),
            Vec3::new(5.0, 3.0, 6.0),
        ] {
            let (yaw, pitch) = aim(feet, target);
            let lens = muzzle(feet, yaw, pitch);
            let pose = gear::lantern_pose(yaw, pitch, 0.0, Vec3::ONE, 1.0, 0.0);
            let pointing = pose.locals[gear::LANTERN_HEAD].transform_vector3(Vec3::NEG_Z);
            let want = (target - lens).normalize();
            assert!(pointing.dot(want) > 0.97, "{pointing} vs {want}");
        }
        assert_eq!(aim(feet, feet), (0.0, 0.0));
    }
}
