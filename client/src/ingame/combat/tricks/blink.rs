//! Blink Arrow (docs/rework.md §1): the arrow you step to, and the step.
//!
//! The arrow (`ModelId::BlinkArrow`, a cold mint flame in a glass bead) flies and sticks like
//! the others; while it can be stepped to (in flight, and for a moment after it sticks) its
//! flame pulses and the owner's HUD says so round the crosshair. The step: where the Shrike
//! was, a mint flash and a fading afterimage of it; a streak of light from there to the
//! arrow; where it arrives, a pop and a ring. The stepper's own camera snaps to the new place
//! at once (no correction smoothing smearing the view along the way: `NetClient::snap_view`).

use engine::gfx::{DrawItem, UiDrawList};
use engine::ui::draw::fade;
use engine::ui::{Rect, TextStyle, UiDrawExt};
use glam::{Mat4, Vec2, Vec3, Vec4, vec2};

use super::super::super::effects::{Effects, FxTex, Particle};
use super::super::Rng;
use super::super::fx::Beams;
use crate::models::tricks::BLINK_MINT;
use crate::models::{ModelLibrary, PlayerAnim, Tint};

/// How long the afterimage and the streak last (s).
const AFTERIMAGE: f32 = 0.4;

/// A step seen: where from, where to, the stepper's pose (for the afterimage).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    from: Vec3,
    to: Vec3,
    anim: Option<PlayerAnim>,
    team: Vec4,
    age: f32,
}

/// Steps in progress (their afterimages and streaks).
#[derive(Default)]
pub struct BlinkView {
    steps: Vec<Step>,
}

impl BlinkView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.steps.clear();
    }

    /// A Shrike stepped from `from` to `to` (feet): the flash where it left and where it
    /// arrives, a streak between them; `anim` (its pose when it left) leaves an afterimage
    /// (none for your own step: you never see yourself).
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        from: Vec3,
        to: Vec3,
        anim: Option<PlayerAnim>,
        team: Vec4,
        effects: &mut Effects,
        beams: &mut Beams,
        rng: &mut Rng,
    ) {
        if self.steps.len() >= 8 {
            self.steps.remove(0);
        }
        self.steps.push(Step {
            from,
            to,
            anim,
            team,
            age: 0.0,
        });
        let mint = mint(2.0, 0.9);
        let chest = Vec3::Y * 1.0;
        effects.glow(from + chest, mint, 1.6, 0.25);
        effects.glow(to + chest, mint, 1.2, 0.2);
        beams.ring(to + Vec3::Y * 0.1, Vec3::Y, 0.3, 1.6, 0.06, mint, 0.3);
        beams.light(to + chest, Vec3::new(1.0, 2.4, 1.9), 6.0, 0.25);
        // Motes left hanging along the path.
        let d = to - from;
        let count = ((d.length() / 0.8) as usize).clamp(4, 30);
        for _ in 0..count {
            let at = from + d * rng.unit() + chest + rng.dir() * 0.35;
            effects.spawn(Particle {
                position: at,
                velocity: rng.dir() * 0.5,
                drag: 2.0,
                max_life: rng.range(0.3, 0.6),
                size: Vec2::splat(rng.range(0.05, 0.1)),
                end_scale: 0.2,
                color: mint,
                texture: FxTex::Spark,
                additive: true,
                ..Default::default()
            });
        }
    }

    pub fn update(&mut self, dt: f32) {
        for s in &mut self.steps {
            s.age += dt.max(0.0);
        }
        self.steps.retain(|s| s.age < AFTERIMAGE);
    }

    /// This frame's streaks from where each step left to where it arrived, fading.
    pub fn streaks(&self, beams: &mut Beams) {
        for s in &self.steps {
            let k = 1.0 - s.age / AFTERIMAGE;
            let streak = mint(1.8, 0.7 * k);
            for h in [0.4f32, 1.0, 1.6] {
                beams.frame_beam(s.from + Vec3::Y * h, s.to + Vec3::Y * h, 0.06 * k, streak);
            }
        }
    }

    /// The afterimages: a mint ghost of the Shrike where it left, fading and drifting up.
    pub fn draw(&self, lib: Option<&ModelLibrary>, out: &mut Vec<DrawItem>) {
        for s in &self.steps {
            let k = 1.0 - s.age / AFTERIMAGE;
            if let (Some(lib), Some(anim)) = (lib, s.anim) {
                let start = out.len();
                lib.draw_player_scaled(
                    &anim,
                    s.from,
                    crate::models::shrike::REMOTE_SCALE,
                    &Tint::team(s.team),
                    out,
                );
                // Drawn as light: flat mint, fading and drifting up a little.
                let lift = Mat4::from_translation(Vec3::Y * 0.3 * (1.0 - k));
                for d in &mut out[start..] {
                    d.transform = lift * d.transform;
                    d.tint = mint(1.2, 0.45 * k);
                    d.unlit = true;
                }
            }
        }
    }
}

/// The Blink Arrow's mint at brightness `k`, alpha `a` (the palette leaves it: it is an own
/// effect, not a signal).
fn mint(k: f32, a: f32) -> Vec4 {
    (BLINK_MINT.truncate() * k).extend(a)
}

/// The owner's prompt while the arrow can be stepped to: "ALT  BLINK" under the crosshair with
/// a mint ring closing round it as the window runs out (`left` 1 → 0; in flight: 1).
pub fn draw_prompt(list: &mut UiDrawList, c: Rect, left: f32, time: f64) {
    let center = c.center().floor() + vec2(0.5, 0.5);
    let left = left.clamp(0.0, 1.0);
    let pulse = 0.75 + 0.25 * ((time * 8.0).sin() as f32).abs();
    let radius = 14.0;
    let n = 48;
    for i in 0..n {
        let k = i as f32 / n as f32;
        if k > left {
            break;
        }
        let a = k * std::f32::consts::TAU;
        let p = center + vec2(a.sin(), -a.cos()) * radius;
        list.rect(
            (p - Vec2::splat(1.0)).floor(),
            (p + Vec2::splat(1.0)).floor(),
            fade(BLINK_MINT, 0.85 * pulse),
        );
    }
    list.text(
        vec2(center.x, center.y + radius + 8.0),
        "BLINK",
        &TextStyle::new(fade(BLINK_MINT, pulse))
            .centered()
            .outline(Vec4::new(0.0, 0.0, 0.0, 0.85)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_leaves_an_afterimage_that_fades() {
        let mut v = BlinkView::new();
        let mut effects = Effects::headless();
        let mut beams = Beams::headless();
        let mut rng = Rng::default();
        v.step(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -12.0),
            Some(PlayerAnim::default()),
            Vec4::ONE,
            &mut effects,
            &mut beams,
            &mut rng,
        );
        assert!(effects.particle_count() > 4);
        v.update(0.2);
        assert_eq!(v.steps.len(), 1);
        v.update(0.25);
        assert!(v.steps.is_empty());
    }
}
