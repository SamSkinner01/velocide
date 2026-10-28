//! Tally (docs/rework.md §1): hold ALT and sweep the crosshair over hostiles to tally them,
//! release to shoot each one.
//!
//! The owner's HUD shows the tally from their own state (the tallied ids the server keeps):
//! while the button is held, the assist circle round the crosshair (the generous cone a target
//! has to touch), the count against the limit and the time left closing round it; on every
//! tallied target a chalk bracket and its tally strokes (one, two, three, four: the order they
//! will be shot in). A new mark scratches in with a tick. The release's shots are tracers from
//! the muzzle to each one, a few hundredths of a second apart, each with its crack.

use engine::gfx::{Camera, UiDrawList};
use engine::ui::Rect;
use engine::ui::draw::fade;
use glam::{Vec2, Vec3, Vec4, vec2};

use super::super::super::hud::project;

/// Chalk.
pub const CHALK: Vec4 = Vec4::new(0.95, 0.94, 0.88, 1.0);
const INK: Vec4 = Vec4::new(0.0, 0.0, 0.0, 0.8);
/// How long a new mark takes to scratch in (s).
const SCRATCH: f64 = 0.12;

/// The tally while it is held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tallying {
    /// Targets tallied so far and the most there may be.
    pub count: usize,
    pub max: usize,
    /// The assist cone's half angle (degrees).
    pub assist_degrees: f32,
    /// Of the hold's time, how much is left (1 → 0).
    pub left: f32,
}

/// One tallied target: where to mark (its chest, world), its height, its place in the order
/// and when it was tallied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub chest: Vec3,
    pub height: f32,
    pub order: usize,
    pub since: f64,
}

/// A rect of pixels with a 1-pixel ink outline (`min` / `max` floored to whole pixels).
fn stroke(list: &mut UiDrawList, min: Vec2, max: Vec2, color: Vec4) {
    let (min, max) = (min.floor(), max.floor().max(min.floor() + Vec2::ONE));
    list.rect(min - Vec2::ONE, max + Vec2::ONE, fade(INK, color.w * 0.8));
    list.rect(min, max, color);
}

/// The held tally round the crosshair: the assist circle (dashed chalk), the time left as an
/// arc closing round it, the count under it.
pub fn draw_tallying(list: &mut UiDrawList, c: Rect, camera: &Camera, t: &Tallying, time: f64) {
    let center = c.center().floor() + vec2(0.5, 0.5);
    let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-3);
    let radius = (t.assist_degrees.to_radians().tan() / tan * c.size().y * 0.5).clamp(6.0, 60.0);
    let n = ((radius * std::f32::consts::TAU / 3.0) as usize).clamp(12, 64);
    for i in 0..n {
        if i % 2 == 1 {
            continue;
        }
        let a = i as f32 / n as f32 * std::f32::consts::TAU + time as f32 * 0.8;
        let p = center + vec2(a.cos(), a.sin()) * radius;
        stroke(
            list,
            p - Vec2::splat(0.5),
            p + Vec2::splat(0.5),
            fade(CHALK, 0.8),
        );
    }
    // The time left: an arc just outside, from the top, shrinking clockwise.
    let left = t.left.clamp(0.0, 1.0);
    let outer = radius + 5.0;
    let m = ((outer * std::f32::consts::TAU / 2.0) as usize).clamp(16, 96);
    for i in 0..m {
        let k = i as f32 / m as f32;
        if k > left {
            break;
        }
        let a = k * std::f32::consts::TAU;
        let p = center + vec2(a.sin(), -a.cos()) * outer;
        list.rect(
            (p - Vec2::splat(0.5)).floor(),
            (p + Vec2::splat(0.5)).floor() + Vec2::ONE,
            fade(CHALK, 0.55),
        );
    }
    // The count: tally strokes, one per target, the empty ones dim.
    let w = 3.0;
    let x0 = center.x - (t.max as f32 * w * 1.6) * 0.5;
    let y = center.y + outer + 6.0;
    for i in 0..t.max {
        let x = x0 + i as f32 * w * 1.6;
        let color = if i < t.count {
            CHALK
        } else {
            fade(CHALK, 0.25)
        };
        stroke(list, vec2(x, y), vec2(x + 1.0, y + 7.0), color);
    }
}

/// The marks on the tallied targets: a chalk bracket round each (sized by its distance) and,
/// above it, its tally strokes; a fresh one scratches in.
pub fn draw_marks(list: &mut UiDrawList, c: Rect, camera: &Camera, marks: &[Mark], time: f64) {
    let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-3);
    for m in marks {
        let Some(p) = project(camera, c.size(), m.chest) else {
            continue;
        };
        let dist = camera.position.distance(m.chest).max(0.5);
        let half = (m.height * 0.55 / dist / tan * c.size().y * 0.5).clamp(6.0, 90.0);
        let grow = (((time - m.since) / SCRATCH) as f32).clamp(0.0, 1.0);
        let half = half * (1.6 - 0.6 * grow);
        let alpha = 0.4 + 0.6 * grow;
        let color = fade(CHALK, alpha);
        let arm = (half * 0.35).max(3.0);
        let w = half * 0.7;
        // Four corners.
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let corner = p + vec2(sx * w, sy * half);
            let h_end = corner - vec2(sx * arm, 0.0);
            let v_end = corner - vec2(0.0, sy * arm);
            stroke(
                list,
                corner.min(h_end),
                corner.max(h_end) + vec2(0.0, 1.0),
                color,
            );
            stroke(
                list,
                corner.min(v_end),
                corner.max(v_end) + vec2(1.0, 0.0),
                color,
            );
        }
        // Its strokes above it: | || ||| ||||.
        let count = m.order + 1;
        let top = p.y - half - 11.0;
        let x0 = p.x - (count as f32 * 3.0) * 0.5;
        for i in 0..count {
            let x = x0 + i as f32 * 3.0;
            stroke(list, vec2(x, top), vec2(x + 1.0, top + 7.0), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            position: Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            fov_y_degrees: 90.0,
            near: 0.05,
            far: 600.0,
        }
    }

    #[test]
    fn marks_sit_on_their_targets_and_count_their_order() {
        let c = Rect::from_min_size(Vec2::ZERO, vec2(640.0, 360.0));
        let mut list = UiDrawList::new(c.size());
        let marks = [
            Mark {
                chest: Vec3::new(0.0, 0.0, -10.0),
                height: 1.8,
                order: 0,
                since: 0.0,
            },
            Mark {
                chest: Vec3::new(0.0, 0.0, 10.0),
                height: 1.8,
                order: 3,
                since: 0.0,
            },
        ];
        draw_marks(&mut list, c, &camera(), &marks, 1.0);
        // Only the one in front is drawn, centred on the crosshair.
        assert!(!list.quads.is_empty());
        let (lo, hi) = list.quads.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(lo, hi), q| (lo.min(q.min), hi.max(q.max)),
        );
        let mid = (lo + hi) * 0.5;
        assert!((mid.x - 320.0).abs() < 3.0, "{mid}");
        assert!(hi.y > 180.0 && lo.y < 180.0);
    }

    #[test]
    fn the_held_tally_counts_against_its_limit() {
        let c = Rect::from_min_size(Vec2::ZERO, vec2(640.0, 360.0));
        let mut list = UiDrawList::new(c.size());
        let t = Tallying {
            count: 2,
            max: 4,
            assist_degrees: 1.5,
            left: 0.5,
        };
        draw_tallying(&mut list, c, &camera(), &t, 0.0);
        let bright = list
            .quads
            .iter()
            .filter(|q| q.color == CHALK && q.max.y - q.min.y >= 6.0)
            .count();
        assert_eq!(bright, 2, "one bright stroke per target tallied");
    }
}
