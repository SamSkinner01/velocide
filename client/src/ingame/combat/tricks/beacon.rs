//! Beacon Arrow (docs/rework.md §1): the beacon it lights where it sticks, and the hostiles it
//! **reveals**, drawn as silhouettes through walls for the shooter and their side.
//!
//! The renderer has no "through walls" pass, so a silhouette is drawn in the viewmodel pass
//! (its own cleared depth, drawn after the world): the hidden model's world draws are carried
//! into view space and squeezed by the ratio of the two passes' fields of view, so they land
//! exactly where the world would show them, unlit, in the palette's `revealed` colour. Only
//! hostiles the camera can't see (the level is in the way of both their chest and their head)
//! get one; a revealed hostile in plain sight wears a small beacon mark over its head instead.

use engine::gfx::{Camera, DrawItem, PointLight};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use glam::{Mat4, Vec3, Vec4};

use super::super::super::effects::{Effects, FxTex};
use super::super::fx::Beams;
use crate::models::tricks::{BEACON_BULB, BEACON_GOLD};
use crate::palette::palette;

/// The deepest a silhouette is carried into the viewmodel pass (its far plane is 50 m).
const SILHOUETTE_DEPTH: f32 = 40.0;
/// How strongly a silhouette shows (alpha), pulsing a little.
const SILHOUETTE_ALPHA: f32 = 0.6;

/// Whether the level hides both `points` (a body's chest and head) from `eye`.
pub fn hidden(world: Option<&CollisionWorld>, eye: Vec3, points: &[Vec3]) -> bool {
    let Some(world) = world else {
        return false;
    };
    points.iter().all(|&p| {
        let d = p - eye;
        let len = d.length();
        len > 0.3
            && world
                .cast_ray(eye, d / len, len - 0.2, MASK_SHOTS)
                .is_some()
    })
}

/// The silhouette colour this frame (the palette's `revealed` role, pulsing).
pub fn silhouette_color(time: f64) -> Vec4 {
    let pulse = 0.85 + 0.15 * ((time * 4.0).sin() as f32);
    let c = palette().revealed.truncate() * (1.3 * pulse);
    c.extend(SILHOUETTE_ALPHA * pulse)
}

/// Carries world draws (`items`, one hidden body's) into the viewmodel pass as a flat,
/// unlit silhouette in `color`: view space, squeezed by tan(viewmodel fov / 2) / tan(camera
/// fov / 2) so it lands where the world would draw it, and pulled in uniformly (which keeps
/// it the same on screen) so it stays inside the pass's depth range.
pub fn silhouette(
    items: &[DrawItem],
    camera: &Camera,
    viewmodel_fov: f32,
    color: Vec4,
    out: &mut Vec<DrawItem>,
) {
    let tan = |deg: f32| (deg.clamp(1.0, 170.0).to_radians() * 0.5).tan();
    let k = tan(viewmodel_fov) / tan(camera.fov_y_degrees);
    let view = camera.view_matrix();
    let far = items
        .iter()
        .map(|d| -view.transform_point3(d.transform.w_axis.truncate()).z)
        .fold(0.0f32, f32::max);
    let s = if far > SILHOUETTE_DEPTH {
        SILHOUETTE_DEPTH / far
    } else {
        1.0
    };
    let squeeze = Mat4::from_scale(Vec3::new(k * s, k * s, s)) * view;
    out.extend(items.iter().map(|d| DrawItem {
        transform: squeeze * d.transform,
        tint: color,
        unlit: true,
        ..*d
    }));
}

/// A lit beacon at `bulb` (the stuck arrow's lantern, world space; `up` the way its flare
/// rises): the bulb blazing in the beacon's gold, a warm light pulsing, a column of light
/// rising from it and motes drifting up; for the shooter's side (`ours`) a dashed ring on the
/// floor at the reveal radius, turning. `life` 0..1 of its time (it gutters over the last
/// tenth).
#[allow(clippy::too_many_arguments)]
pub fn lit_beacon(
    bulb: Vec3,
    floor: Option<Vec3>,
    radius: f32,
    ours: bool,
    life: f32,
    time: f64,
    effects: &mut Effects,
    beams: &mut Beams,
    lights: &mut Vec<PointLight>,
) {
    let pal = palette();
    let fade = if life > 0.9 {
        ((1.0 - life) / 0.1).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let flicker = 0.85 + 0.1 * ((time * 9.0).sin() as f32) + 0.05 * ((time * 23.0).sin() as f32);
    let gold = pal.effect(BEACON_GOLD, pal.revealed, 1.0);
    let k = fade * flicker;
    effects.billboard(
        bulb,
        0.9 * k,
        Vec4::new(gold.x * 1.6, gold.y * 1.5, gold.z * 1.2, 0.9 * k),
        FxTex::Glow,
        0.0,
    );
    effects.billboard(bulb, 0.3 * k, Vec4::new(2.2, 2.1, 1.8, k), FxTex::Glow, 0.0);
    lights.push(PointLight {
        position: bulb,
        color: pal.light(Vec3::new(2.4, 1.9, 0.9), pal.revealed, 2.2) * k,
        radius: 7.0,
    });
    // A column of light rising from the lantern.
    for (w, a, h) in [(0.16, 0.18, 3.5), (0.05, 0.5, 2.6)] {
        beams.frame_beam(
            bulb,
            bulb + Vec3::Y * h,
            w,
            Vec4::new(gold.x * 1.4, gold.y * 1.3, gold.z, a * k),
        );
    }
    // The reveal radius on the floor (for the side it reveals to).
    if ours && let Some(floor) = floor {
        let dashes = 40;
        let turn = (time * 0.4) as f32;
        for i in 0..dashes {
            if i % 2 == 1 {
                continue;
            }
            let a0 = turn + i as f32 / dashes as f32 * std::f32::consts::TAU;
            let a1 = turn + (i + 1) as f32 / dashes as f32 * std::f32::consts::TAU;
            let p = |a: f32| floor + Vec3::new(a.cos(), 0.06, a.sin()) * radius;
            beams.frame_beam(
                p(a0),
                p(a1),
                0.05,
                Vec4::new(gold.x, gold.y, gold.z, 0.5 * k),
            );
        }
    }
}

/// The beacon's bulb for a beacon arrow drawn with `transform` (its model's).
pub fn bulb_of(transform: Mat4) -> Vec3 {
    transform.transform_point3(BEACON_BULB)
}

/// The beacon catching where the arrow struck: a burst of gold, sparks, the light flaring.
pub fn ignite(effects: &mut Effects, beams: &mut Beams, at: Vec3) {
    let pal = palette();
    let gold = pal.effect(BEACON_GOLD, pal.revealed, 1.0);
    effects.glow(
        at,
        Vec4::new(gold.x * 2.0, gold.y * 1.8, gold.z * 1.4, 0.9),
        1.6,
        0.3,
    );
    effects.sparks(at, Vec3::Y, 14);
    beams.ring(
        at,
        Vec3::Y,
        0.2,
        2.5,
        0.06,
        Vec4::new(gold.x * 1.8, gold.y * 1.7, gold.z * 1.2, 0.9),
        0.4,
    );
    beams.light(at, Vec3::new(3.0, 2.4, 1.2), 9.0, 0.3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::gfx::{MeshId, TextureId};

    fn item(at: Vec3) -> DrawItem {
        DrawItem {
            mesh: MeshId(0),
            texture: TextureId(0),
            transform: Mat4::from_translation(at),
            tint: Vec4::ONE,
            unlit: false,
        }
    }

    /// Where `p` lands on screen (NDC) through a perspective of `fov` degrees.
    fn ndc(p: Vec3, fov: f32) -> (f32, f32) {
        let t = (fov.to_radians() * 0.5).tan();
        (p.x / (-p.z * t), p.y / (-p.z * t))
    }

    #[test]
    fn a_silhouette_lands_where_the_world_draws_it() {
        let camera = Camera {
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation: Camera::rotation_from_angles(0.4, -0.1, 0.0),
            fov_y_degrees: 40.0,
            near: 0.05,
            far: 600.0,
        };
        let world = camera.position + camera.rotation * Vec3::new(2.0, 1.0, -70.0);
        let mut out = Vec::new();
        silhouette(&[item(world)], &camera, 62.0, Vec4::ONE, &mut out);
        let d = out[0];
        assert!(d.unlit);
        let vm = d.transform.w_axis.truncate();
        let seen = camera.view_matrix().transform_point3(world);
        let (a, b) = (ndc(seen, 40.0), ndc(vm, 62.0));
        assert!(
            (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4,
            "{a:?} {b:?}"
        );
        // Pulled inside the viewmodel pass's far plane.
        assert!(-vm.z <= SILHOUETTE_DEPTH + 1e-3, "{vm}");
    }
}
