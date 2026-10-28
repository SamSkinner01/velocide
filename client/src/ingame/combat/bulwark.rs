//! The Bulwark (docs/rework.md §1, Scatter, area **client-b**): a slab of hardened wax held up
//! in front of you while ALT is held.
//!
//! * **The slab** (`ModelId::Bulwark`, [`slab_transform`], [`draw_slab`]): 1.2 m out along
//!   the aim, turning with its yaw, its pitch held within ±30°, its face away from the holder.
//!   Yours is drawn more see-through (you aim through it); others' fully. Its rim's wick burns
//!   lower as its time runs out; the heat inside glows brighter as it fills.
//! * **Absorbing** ([`absorb`]): a blow taken flares the rim and the cracks and chips wax off
//!   where it struck (`Sfx::BulwarkAbsorb`); every crack stage it reaches splits with
//!   `Sfx::BulwarkCrack` ([`crack_stage`]).
//! * **Raised / dropped / shattered** ([`raise`], [`lower`], [`shatter`]): heaved up with a hiss
//!   (`Sfx::BulwarkRaise`); lowered, it slumps away (`Sfx::BulwarkDrop`); broken, it bursts
//!   into glowing wax chunks (`Sfx::BulwarkShatter`).
//! * **Yours on the HUD** ([`hud`]): a small wax bar under the crosshair — what it can still
//!   take, and a thin line for its time — in the ability's honey, turning the palette's
//!   warning colour (and blinking) when it is about to break.

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, UiDrawList};
use engine::ui::draw::fade;
use engine::ui::{Rect, Theme, UiDrawExt};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4, vec2};

use super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::Rng;
use super::fx::Beams;
use crate::models::gear::{self, BULWARK_COLOR, BULWARK_SIZE};
use crate::models::{ModelId, ModelLibrary, Tint};
use shared::tuning::BulwarkTuning;

/// How far in front of the eye the slab stands (docs/rework.md: 1.2 m).
pub const OUT: f32 = 1.2;
/// Its pitch follows the aim within this much (docs/rework.md: ±30°).
pub const PITCH_LIMIT: f32 = 30.0 * std::f32::consts::PI / 180.0;
/// The slab's centre stands this far below the eye (the server's guard shape).
pub const SLAB_DROP: f32 = 0.4;
/// How see-through your own slab is drawn (its alpha on top of the wax's).
const OWN_ALPHA: f32 = 0.45;

/// Where the slab stands for a holder whose eye is at `eye`, looking `yaw` / `pitch`: its
/// centre 1.2 m out along the (clamped) aim, its face (-Z) turned away from the holder.
pub fn slab_transform(eye: Vec3, yaw: f32, pitch: f32) -> Mat4 {
    let pitch = if pitch.is_finite() {
        pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT)
    } else {
        0.0
    };
    let yaw = if yaw.is_finite() { yaw } else { 0.0 };
    let rot = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    Mat4::from_rotation_translation(rot, eye + rot * (Vec3::NEG_Z * OUT))
}

/// [`slab_transform`] with the tuning's slab as the server stands it: centred 0.4 m below the
/// eye plus `distance` along the facing, its pitch limit, and its size (the model scaled to
/// `width` × `height`).
pub fn slab_transform_for(eye: Vec3, yaw: f32, pitch: f32, t: &BulwarkTuning) -> Mat4 {
    let limit = t.pitch_limit_degrees.to_radians();
    let pitch = if pitch.is_finite() {
        pitch.clamp(-limit, limit)
    } else {
        0.0
    };
    let yaw = if yaw.is_finite() { yaw } else { 0.0 };
    let rot = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    let size = Vec3::new(t.width / BULWARK_SIZE.x, t.height / BULWARK_SIZE.y, 1.0)
        .clamp(Vec3::splat(0.2), Vec3::splat(4.0));
    let center = eye - Vec3::Y * SLAB_DROP + rot * (Vec3::NEG_Z * t.distance);
    Mat4::from_scale_rotation_translation(size, rot, center)
}

/// Where a hit at `hit` (a point on the holder, behind the slab) shows on the slab at `root`:
/// projected onto its face, kept within it.
pub fn block_point(root: Mat4, hit: Vec3) -> Vec3 {
    let local = root.inverse().transform_point3(hit);
    let half = BULWARK_SIZE * 0.5;
    let on = Vec3::new(
        local.x.clamp(-half.x * 0.9, half.x * 0.9),
        local.y.clamp(-half.y * 0.9, half.y * 0.9),
        -half.z - 0.02,
    );
    root.transform_point3(if local.is_finite() { on } else { Vec3::ZERO })
}

/// Draws the slab at `root` (`absorbed` 0..1 of what it can take, `flash` 0..1 a blow just
/// taken, `left` 0..1 of its time; `own`: yours, more see-through).
#[allow(clippy::too_many_arguments)]
pub fn draw_slab(
    lib: &ModelLibrary,
    root: Mat4,
    absorbed: f32,
    flash: f32,
    left: f32,
    own: bool,
    time: f64,
    draws: &mut Vec<DrawItem>,
) {
    let pose = gear::bulwark_pose(absorbed, flash, left, time);
    let tint = Tint {
        body: Vec4::new(1.0, 1.0, 1.0, if own { OWN_ALPHA } else { 1.0 }),
        ..Tint::default()
    };
    lib.get(ModelId::Bulwark).draw(&pose, root, &tint, draws);
}

/// A point on the slab's face (`u`, `v` in -1..1 across and up) for effects.
pub fn face_point(root: Mat4, u: f32, v: f32) -> Vec3 {
    root.transform_point3(Vec3::new(
        u * BULWARK_SIZE.x * 0.5,
        v * BULWARK_SIZE.y * 0.5,
        -BULWARK_SIZE.z * 0.5 - 0.02,
    ))
}

/// Which crack stage `absorbed` (0..1) has reached (0 = none .. 3), as the model shows them.
pub fn crack_stage(absorbed: f32) -> usize {
    [0.25f32, 0.55, 0.8]
        .iter()
        .filter(|&&from| absorbed >= from)
        .count()
}

fn wax(k: f32, a: f32) -> Vec4 {
    (BULWARK_COLOR.truncate() * k).extend(a)
}

/// Heaved up at `root`: a puff of hot wax at its foot, the sound.
pub fn raise(effects: &mut Effects, audio: &mut Audio, root: Mat4, own: bool) {
    let foot = root.transform_point3(Vec3::new(0.0, -BULWARK_SIZE.y * 0.5, 0.0));
    effects.smoke(foot, 4);
    effects.glow(root.w_axis.truncate(), wax(1.0, 0.5), 1.8, 0.2);
    if own {
        audio.play_2d(Sfx::BulwarkRaise);
    } else {
        audio.play(Sfx::BulwarkRaise, PlayParams::at(foot));
    }
}

/// A blow taken at `at` on the slab (`normal`: its face): wax chips and sparks off the face,
/// the sound. `cracked`: it just reached a new crack stage (the split's sound too).
pub fn absorb(
    effects: &mut Effects,
    audio: &mut Audio,
    rng: &mut Rng,
    at: Vec3,
    normal: Vec3,
    cracked: bool,
) {
    let n = normal.normalize_or(Vec3::Y);
    effects.wax_spray(at, n, 0.6);
    effects.sparks(at, n, 6);
    effects.glow(at, wax(1.4, 0.8), 0.5, 0.12);
    for _ in 0..5 {
        effects.spawn(Particle {
            position: at,
            velocity: (n + rng.dir() * 0.6) * rng.range(2.0, 4.0),
            gravity: 12.0,
            max_life: rng.range(0.4, 0.7),
            size: Vec2::splat(rng.range(0.05, 0.09)),
            color: wax(1.0, 1.0),
            texture: FxTex::Lump,
            rotation: rng.angle(),
            spin: rng.range(-8.0, 8.0),
            collide: Some(OnHit::Bounce(0.3)),
            ..Default::default()
        });
    }
    audio.play(Sfx::BulwarkAbsorb, PlayParams::at(at));
    if cracked {
        audio.play(Sfx::BulwarkCrack, PlayParams::at(at));
    }
}

/// The slab at `root` breaking: glowing chunks of wax flung out of its face and falling, dust,
/// a flash and the sound.
pub fn shatter(
    effects: &mut Effects,
    beams: &mut Beams,
    audio: &mut Audio,
    rng: &mut Rng,
    root: Mat4,
) {
    let center = root.w_axis.truncate();
    let face = root
        .transform_vector3(Vec3::NEG_Z)
        .normalize_or(Vec3::NEG_Z);
    for k in 0..28 {
        let u = rng.range(-1.0, 1.0);
        let v = rng.range(-1.0, 1.0);
        let at = face_point(root, u, v);
        let out =
            (face * rng.range(0.5, 1.5) + (at - center) * 0.8 + rng.dir() * 0.4).normalize_or(face);
        let hot = k % 3 == 0;
        effects.spawn(Particle {
            position: at,
            velocity: out * rng.range(3.0, 7.0) + Vec3::Y * 2.0,
            gravity: 14.0,
            drag: 0.3,
            max_life: rng.range(0.7, 1.2),
            size: Vec2::splat(rng.range(0.1, 0.22)),
            end_scale: 0.6,
            color: if hot { wax(1.6, 1.0) } else { wax(0.95, 1.0) },
            end_alpha: 0.6,
            texture: FxTex::Lump,
            additive: hot,
            rotation: rng.angle(),
            spin: rng.range(-10.0, 10.0),
            collide: Some(OnHit::Bounce(0.25)),
            ..Default::default()
        });
    }
    effects.dust(center - Vec3::Y * BULWARK_SIZE.y * 0.5, 10, BULWARK_SIZE.x);
    effects.glow(center, wax(1.8, 0.9), 2.6, 0.25);
    beams.light(center, BULWARK_COLOR.truncate() * 2.2, 6.0, 0.3);
    audio.play(Sfx::BulwarkShatter, PlayParams::at(center));
}

/// Lowered (its time up or ALT released): the slab slumps away in a soft puff.
pub fn lower(effects: &mut Effects, audio: &mut Audio, root: Mat4) {
    let foot = root.transform_point3(Vec3::new(0.0, -BULWARK_SIZE.y * 0.5, 0.0));
    effects.dust(foot, 6, BULWARK_SIZE.x * 0.6);
    effects.wax_spray(foot + Vec3::Y * 0.3, Vec3::Y, 0.5);
    audio.play(Sfx::BulwarkDrop, PlayParams::at(foot));
}

/// Your slab on the HUD (under the crosshair): `hold` 0..1 what it can still take, `left`
/// 0..1 its time. Below a quarter it turns the palette's warning colour and blinks (the shape
/// cue: a notch cut in the bar).
pub fn hud(list: &mut UiDrawList, t: &Theme, c: Rect, hold: f32, left: f32, now: f64) {
    if !(hold.is_finite() && left.is_finite()) || left <= 0.0 {
        return;
    }
    let p = crate::palette::palette();
    let hold = hold.clamp(0.0, 1.0);
    let w = 46.0;
    let center = c.center();
    let bar = Rect::from_center_size(center + vec2(0.0, 20.0), vec2(w, 4.0));
    list.fill(bar.expand(1.0), fade(t.ink, 0.7));
    let low = hold < 0.25;
    let color = if low {
        let blink = 0.55 + 0.45 * ((now * 10.0).sin() as f32).abs();
        fade(p.warning, blink)
    } else {
        BULWARK_COLOR
    };
    list.fill(
        Rect::new(bar.min, vec2(bar.min.x + w * hold, bar.max.y)),
        color,
    );
    // The crack stages as ticks along it.
    for from in [0.25f32, 0.55, 0.8] {
        let x = bar.max.x - w * from;
        list.fill(
            Rect::new(vec2(x, bar.min.y), vec2(x + 1.0, bar.max.y)),
            fade(t.ink, 0.8),
        );
    }
    if low {
        // A notch over the bar's middle (reads without colour).
        list.fill(
            Rect::from_center_size(bar.center() - vec2(0.0, 3.0), vec2(3.0, 2.0)),
            color,
        );
    }
    let time = Rect::new(
        vec2(bar.min.x, bar.max.y + 2.0),
        vec2(bar.min.x + w * left.clamp(0.0, 1.0), bar.max.y + 3.0),
    );
    list.fill(time, fade(t.bone_dim, 0.8));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slab_stands_out_along_the_aim_within_its_pitch() {
        let eye = Vec3::new(1.0, 1.6, 2.0);
        let m = slab_transform(eye, 0.0, 0.0);
        let at = m.w_axis.truncate();
        assert!((at - (eye + Vec3::NEG_Z * OUT)).length() < 1e-5, "{at}");
        // Its face (-Z) points away from the holder.
        assert!(m.transform_vector3(Vec3::NEG_Z).z < -0.99);
        let up = slab_transform(eye, 0.0, 1.4);
        let dir = (up.w_axis.truncate() - eye).normalize();
        assert!((dir.y.asin() - PITCH_LIMIT).abs() < 1e-4, "pitch clamped");
        assert!(slab_transform(eye, f32::NAN, f32::NAN).is_finite());
        assert_eq!(crack_stage(0.0), 0);
        assert_eq!(crack_stage(0.6), 2);
        assert_eq!(crack_stage(1.0), 3);
    }
}
