//! Waxwork (docs/rework.md §1): the wax double of a Shrike, left standing where it cast it.
//!
//! The double is that player's own Shrike frozen in the pose it had when it was poured (the
//! client captures the caster's animation on the frame the double appears, so it is "posed as
//! you were and facing your look"). The caster and their teammates see it as wax: every paint
//! turned to pale tallow, a soft glisten, the wick and eyes dulled. In PvP the caster's
//! opponents see it as the player (their look and colours, no wax), the trick being that they
//! can't tell. Over its last half second (or the moment it is destroyed) it slumps, sagging
//! and spreading at the feet, then bursts in hot wax: a spray, lumps, a ring out to the
//! burst's radius, splats on the floor and a flash of heat.

use engine::gfx::DrawItem;
use glam::{Mat4, Vec3, Vec4};

use super::super::super::effects::Effects;
use super::super::Rng;
use super::super::fx::Beams;
use crate::models::{ModelLibrary, PlayerAnim, Tint};

/// The tallow the double is poured from.
pub const WAX: Vec4 = Vec4::new(0.96, 0.9, 0.74, 1.0);
/// How much brighter than the tallow itself the figure is drawn: the greyscale copies carry
/// the parts' dark leather and baked shading, and wax reads pale.
const WAX_GAIN: f32 = 1.55;
/// How long the slump takes before the burst (s).
pub const SLUMP_TIME: f32 = 0.45;

/// The wax look of a double: the whole figure in pale tallow (`Tint::flat`: every part's
/// shading kept, its colours gone, the wick and the eyes poured over too), with a slow
/// glisten running over it.
pub fn wax_tint(team: Vec4, time: f64) -> Tint {
    let glisten = 0.06 * ((time * 2.3).sin() as f32);
    let k = WAX_GAIN + glisten;
    Tint {
        flat: Some(Vec4::new(WAX.x * k, WAX.y * k, WAX.z * k, 1.0)),
        ..Tint::team(team)
    }
}

/// A frozen pose: the caster's animation with its clock stopped (no breathing, no flicker),
/// standing still where it was poured.
pub fn frozen(mut anim: PlayerAnim) -> PlayerAnim {
    anim.velocity = Vec3::ZERO;
    anim.firing = 0.0;
    anim.punching = 0.0;
    anim.dashing = false;
    anim.time = 0.0;
    anim.swing = Default::default();
    anim
}

/// The slump at `k` 0..1 (0 standing, 1 about to burst) as a transform about the feet: it sags
/// (shorter), spreads (wider at the base) and leans a little as it goes.
pub fn slump_transform(feet: Vec3, k: f32) -> Mat4 {
    let k = if k.is_nan() { 0.0 } else { k.clamp(0.0, 1.0) };
    let e = k * k * (3.0 - 2.0 * k);
    Mat4::from_translation(feet)
        * Mat4::from_rotation_x(0.12 * e)
        * Mat4::from_scale(Vec3::new(1.0 + 0.35 * e, 1.0 - 0.55 * e, 1.0 + 0.35 * e))
        * Mat4::from_translation(-feet)
}

/// Draws the double at `feet`: `anim` (frozen), `waxy` for the caster and their side (the
/// other side sees the player, `tint` as is), slumping by `slump` 0..1.
#[allow(clippy::too_many_arguments)]
pub fn draw_double(
    lib: &ModelLibrary,
    anim: &PlayerAnim,
    feet: Vec3,
    tint: &Tint,
    waxy: bool,
    slump: f32,
    time: f64,
    out: &mut Vec<DrawItem>,
) {
    let tint = if waxy {
        wax_tint(tint.team, time)
    } else {
        *tint
    };
    let start = out.len();
    lib.draw_player_scaled(anim, feet, crate::models::shrike::REMOTE_SCALE, &tint, out);
    if slump > 0.0 {
        let m = slump_transform(feet, slump);
        for d in &mut out[start..] {
            d.transform = m * d.transform;
        }
    }
    // Hot wax beading on it as it slumps (glistening, then glowing at the end).
    if waxy && slump > 0.0 {
        for d in &mut out[start..] {
            if !d.unlit {
                let heat = 0.25 * slump * slump;
                d.tint = Vec4::new(
                    d.tint.x * (1.0 + heat),
                    d.tint.y * (1.0 + heat * 0.6),
                    d.tint.z,
                    d.tint.w,
                );
            }
        }
    }
}

/// The burst at `feet` (a double of radius `radius`'s burst): hot wax sprayed out and up,
/// lumps, a ring rushing out to the radius along the floor, a flash and a light.
pub fn burst(effects: &mut Effects, beams: &mut Beams, rng: &mut Rng, feet: Vec3, radius: f32) {
    let chest = feet + Vec3::Y * 0.9;
    for _ in 0..3 {
        let dir = (rng.dir() * Vec3::new(1.0, 0.4, 1.0) + Vec3::Y * 0.6).normalize_or(Vec3::Y);
        effects.wax_spray(chest, dir, 2.2);
    }
    effects.lumps(chest, 8);
    effects.glow(
        chest,
        Vec4::new(1.6, 1.1, 0.55, 0.9),
        1.6 + radius * 0.4,
        0.25,
    );
    effects.smoke(chest, 5);
    let hot = Vec4::new(2.0, 1.3, 0.6, 0.9);
    beams.ring(feet + Vec3::Y * 0.15, Vec3::Y, 0.4, radius, 0.12, hot, 0.35);
    beams.ring(
        feet + Vec3::Y * 0.4,
        Vec3::Y,
        0.2,
        radius * 0.7,
        0.07,
        Vec4::new(1.4, 1.3, 1.1, 0.7),
        0.25,
    );
    beams.light(chest, Vec3::new(2.4, 1.5, 0.6), radius * 2.0, 0.35);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slump_sags_and_spreads_about_the_feet() {
        let feet = Vec3::new(3.0, 1.0, -2.0);
        assert!(slump_transform(feet, 0.0).abs_diff_eq(Mat4::IDENTITY, 1e-6));
        let m = slump_transform(feet, 1.0);
        // The feet stay put, the head comes down, the shoulders spread.
        assert!(m.transform_point3(feet).distance(feet) < 1e-5);
        let head = m.transform_point3(feet + Vec3::Y * 1.8);
        assert!(head.y < feet.y + 1.0 && head.y > feet.y + 0.6, "{head}");
        let side = m.transform_point3(feet + Vec3::X * 0.3);
        assert!((side.x - feet.x) > 0.35);
        assert!(slump_transform(feet, f32::NAN).is_finite());
    }

    #[test]
    fn wax_pours_the_whole_figure_in_tallow() {
        let t = wax_tint(Vec4::new(0.2, 0.4, 1.0, 1.0), 1.0);
        let flat = t.flat.expect("drawn flat");
        let hue = flat.truncate() / flat.x;
        assert!(hue.abs_diff_eq(WAX.truncate() / WAX.x, 1e-4), "{hue}");
        assert!(flat.x > 1.0, "drawn bright: wax reads pale");
        assert!(Tint::default().flat.is_none());
    }
}
