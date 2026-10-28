//! Riposte (docs/rework.md §1): hold to guard with the melee weapon, release to answer.
//!
//! The guard is a pose (the blade / haft raised crosswise in front, `models::weapons::melee`);
//! here is what it does to the screen and the world: while it is up, a steel arc across the
//! bottom of the view shows the guarded front and how long the guard can still be held, with a
//! pip per blow taken (each makes the counter harder); every block throws a fan of white-hot
//! sparks off the guard with a flash, a ring and a tiny hitstop; the counter-swing leaves a
//! wide bright crescent, wider for every block it answers.

use engine::gfx::UiDrawList;
use engine::ui::Rect;
use engine::ui::draw::fade;
use glam::{Vec2, Vec3, Vec4, vec2};

use super::super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::super::Rng;
use super::super::fx::Beams;

/// The guard's steel.
pub const STEEL: Vec4 = Vec4::new(0.9, 0.93, 1.0, 1.0);

/// A blow blocked at `at` coming along `dir` (towards the guard): sparks fanning back the way
/// it came, a white flash, a ring and a light.
pub fn block_sparks(effects: &mut Effects, beams: &mut Beams, rng: &mut Rng, at: Vec3, dir: Vec3) {
    let back = -dir.normalize_or(Vec3::NEG_Z);
    for _ in 0..18 {
        let v = (back + rng.dir() * 0.7).normalize_or(back) * rng.range(4.0, 11.0);
        effects.spawn(Particle {
            position: at,
            velocity: v,
            gravity: 12.0,
            drag: 1.5,
            max_life: rng.range(0.15, 0.4),
            size: Vec2::splat(rng.range(0.04, 0.08)),
            end_scale: 0.3,
            color: Vec4::new(1.0, 0.95, 0.8, 1.0),
            texture: FxTex::Spark,
            additive: true,
            collide: Some(OnHit::Bounce(0.3)),
            ..Default::default()
        });
    }
    effects.glow(at, Vec4::new(2.2, 2.2, 2.4, 0.9), 0.7, 0.08);
    beams.ring(
        at,
        back,
        0.05,
        0.6,
        0.025,
        Vec4::new(2.0, 2.1, 2.4, 0.9),
        0.15,
    );
    beams.light(at, Vec3::new(2.4, 2.4, 2.6), 5.0, 0.1);
}

/// The counter-swing's crescent: from the right of `eye` round to the left in front of it
/// (along `fwd`, `right`), wider for the `blocks` it answers.
pub fn counter_crescent(beams: &mut Beams, eye: Vec3, fwd: Vec3, right: Vec3, blocks: u32) {
    let reach = 2.4 + 0.3 * blocks.min(3) as f32;
    let width = 0.08 + 0.03 * blocks.min(3) as f32;
    let color = Vec4::new(2.0, 2.05, 2.3, 0.85);
    let n = 10;
    let at = |k: f32| {
        let a = (k - 0.5) * 2.2;
        eye + (fwd * a.cos() + right * a.sin()) * reach - Vec3::Y * (0.3 - 0.4 * k)
    };
    for i in 0..n {
        let (k0, k1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        // Thickest in the middle of the cut.
        let w = width * (1.0 - (2.0 * k0 - 1.0).abs() * 0.6);
        beams.beam(at(1.0 - k0), at(1.0 - k1), w, color, 0.18);
    }
}

/// The guard on the HUD: a steel arc low in the view spanning the guarded front, closing as
/// the hold runs out (`left` 1 → 0), and a pip per blow taken (`blocks`, of `max` that count).
pub fn draw_guard(list: &mut UiDrawList, c: Rect, left: f32, blocks: u32, max: u32, time: f64) {
    let center = c.center().floor() + vec2(0.5, 0.5);
    let left = left.clamp(0.0, 1.0);
    let radius = 46.0;
    // The front 120°, centred on straight ahead (drawn as an arc under the crosshair).
    let half = 60f32.to_radians() * (0.35 + 0.65 * left);
    let n = 40;
    let ink = Vec4::new(0.0, 0.0, 0.0, 0.7);
    let shimmer = 0.8 + 0.2 * ((time * 6.0).sin() as f32);
    for i in 0..=n {
        let a = -half + 2.0 * half * i as f32 / n as f32;
        let p = center + vec2(a.sin(), a.cos()) * radius;
        list.rect(
            (p - Vec2::splat(1.5)).floor(),
            (p + Vec2::splat(1.5)).floor(),
            ink,
        );
        list.rect(
            (p - Vec2::splat(1.0)).floor(),
            (p + Vec2::splat(1.0)).floor(),
            fade(STEEL, 0.75 * shimmer),
        );
    }
    // A pip per blow taken under the arc.
    for i in 0..max {
        let x = center.x - (max as f32 - 1.0) * 4.0 + i as f32 * 8.0;
        let y = center.y + radius + 6.0;
        let lit = i < blocks;
        list.rect(vec2(x - 3.0, y - 3.0), vec2(x + 3.0, y + 3.0), ink);
        list.rect(
            vec2(x - 2.0, y - 2.0),
            vec2(x + 2.0, y + 2.0),
            if lit { STEEL } else { fade(STEEL, 0.25) },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_throws_sparks_and_the_guard_counts_its_blows() {
        let mut effects = Effects::headless();
        let mut beams = Beams::headless();
        let mut rng = Rng::default();
        block_sparks(
            &mut effects,
            &mut beams,
            &mut rng,
            Vec3::new(0.0, 1.4, -0.6),
            Vec3::Z,
        );
        assert!(effects.particle_count() >= 18);
        let c = Rect::from_min_size(Vec2::ZERO, vec2(640.0, 360.0));
        let mut list = UiDrawList::new(c.size());
        draw_guard(&mut list, c, 0.5, 2, 3, 0.0);
        let lit = list.quads.iter().filter(|q| q.color == STEEL).count();
        assert_eq!(lit, 2, "a lit pip per block");
    }
}
