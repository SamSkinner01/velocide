//! Wickline (docs/rework.md §1): a burning wick-thread whipped out to an anchor (the level, or
//! a hostile it yanks), drawn for the line's 0.35 s.
//!
//! The cord runs from the shooter's muzzle (wherever it is now: it is yanked along it) to the
//! anchor (a struck hostile's chest, following it as it is pulled in). It leaves with a ripple
//! that travels out and dies away, its core burning gold inside an orange glow, embers falling
//! off it; the wick burns down from the shooter's end, so the line vanishes into the anchor. A
//! miss flies out its full length and droops as it burns. Where it bites: a burn flash and
//! sparks.

use glam::{Vec2, Vec3, Vec4};
use shared::protocol::NetId;

use super::super::super::effects::{Effects, FxTex, Particle};
use super::super::Rng;
use super::super::fx::Beams;

/// How long a line shows (s): the contract's 0.35 s.
pub const LIFE: f32 = 0.35;
/// Segments of the drawn cord.
const SEGMENTS: usize = 16;

/// What the line's far end is fastened to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// The level (the shooter zips to it).
    Level(Vec3),
    /// A hostile (pulled in, or pulling a light shooter to a heavy one): followed while it
    /// moves; the point is where it was struck, kept if it disappears.
    Body(NetId, Vec3),
    /// Nothing: the cord flew its full length and falls.
    Miss(Vec3),
}

impl Anchor {
    pub fn point(self) -> Vec3 {
        match self {
            Anchor::Level(p) | Anchor::Body(_, p) | Anchor::Miss(p) => p,
        }
    }
}

/// One line in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    /// Whose (the local player's own follows the viewmodel's muzzle).
    pub shooter: Option<NetId>,
    pub from: Vec3,
    pub anchor: Anchor,
    pub age: f32,
}

/// The lines alive.
#[derive(Default)]
pub struct WicklineView {
    lines: Vec<Line>,
}

impl WicklineView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// A line leaves `from` (a muzzle) for `anchor`: the bite's flash where it lands.
    pub fn shoot(
        &mut self,
        shooter: Option<NetId>,
        from: Vec3,
        anchor: Anchor,
        effects: &mut Effects,
        beams: &mut Beams,
    ) {
        if self.lines.len() >= 16 {
            self.lines.remove(0);
        }
        self.lines.push(Line {
            shooter,
            from,
            anchor,
            age: 0.0,
        });
        let at = anchor.point();
        if !matches!(anchor, Anchor::Miss(_)) {
            effects.glow(at, Vec4::new(2.0, 1.2, 0.5, 0.9), 0.7, 0.15);
            effects.sparks(at, (from - at).normalize_or(Vec3::Y), 10);
            beams.light(at, Vec3::new(2.2, 1.2, 0.4), 4.0, 0.2);
        }
    }

    /// Ages the lines; `ends(shooter, anchor)` gives each one's current ends (the muzzle it
    /// leaves now, the anchor's current point), embers drop off the burning cord.
    pub fn update(
        &mut self,
        dt: f32,
        ends: &dyn Fn(&Line) -> (Vec3, Vec3),
        effects: &mut Effects,
        rng: &mut Rng,
    ) {
        for l in &mut self.lines {
            l.age += dt.max(0.0);
            let (from, to) = ends(l);
            l.from = from;
            if let Anchor::Body(id, _) = l.anchor {
                l.anchor = Anchor::Body(id, to);
            }
            let burnt = ((l.age / LIFE - 0.35) / 0.65).clamp(0.0, 1.0);
            if rng.unit() < dt * 40.0 {
                let t = burnt + (1.0 - burnt) * rng.unit();
                let p = cord_point(l, t);
                effects.spawn(Particle {
                    position: p,
                    velocity: rng.dir() * 0.6 + Vec3::new(0.0, -0.5, 0.0),
                    gravity: 6.0,
                    max_life: rng.range(0.25, 0.5),
                    size: Vec2::splat(0.05),
                    end_scale: 0.3,
                    color: Vec4::new(1.0, 0.65, 0.25, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
        }
        self.lines.retain(|l| l.age < LIFE);
    }

    /// The cords this frame.
    pub fn draw(&self, beams: &mut Beams) {
        for l in &self.lines {
            draw_cord(l, beams);
        }
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }
}

/// Point `t` 0..1 along a line's cord as drawn now: a ripple running out from the shooter and
/// dying away; a miss sags as it burns.
pub fn cord_point(l: &Line, t: f32) -> Vec3 {
    let to = l.anchor.point();
    let d = to - l.from;
    let len = d.length().max(1e-3);
    let dir = d / len;
    let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = side.cross(dir);
    let k = (l.age / LIFE).clamp(0.0, 1.0);
    let envelope = (std::f32::consts::PI * t).sin();
    let ripple = (t * 9.0 - l.age * 38.0).sin() * 0.12 * (1.0 - k).powi(2) * envelope;
    let sag = match l.anchor {
        Anchor::Miss(_) => 0.08 * len * k * envelope,
        _ => 0.02 * len.min(10.0) * envelope * (1.0 - k),
    };
    l.from + d * t + side * ripple + up * (ripple * 0.5) - Vec3::Y * sag
}

/// Draws one cord: an orange glow round a gold core, burning away from the shooter's end over
/// the second half of its life.
fn draw_cord(l: &Line, beams: &mut Beams) {
    let k = (l.age / LIFE).clamp(0.0, 1.0);
    let burnt = ((k - 0.35) / 0.65).clamp(0.0, 1.0) * 0.9;
    let glow = Vec4::new(2.0, 0.85, 0.25, 0.85 * (1.0 - k * 0.4));
    let core = Vec4::new(2.4, 1.9, 1.0, 1.0);
    let mut prev = cord_point(l, burnt);
    for i in 1..=SEGMENTS {
        let t = burnt + (1.0 - burnt) * i as f32 / SEGMENTS as f32;
        let p = cord_point(l, t);
        beams.frame_beam(prev, p, 0.055, glow);
        beams.frame_beam(prev, p, 0.018, core);
        prev = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_runs_from_the_muzzle_to_the_anchor_and_burns_out() {
        let line = Line {
            shooter: None,
            from: Vec3::ZERO,
            anchor: Anchor::Level(Vec3::new(0.0, 0.0, -20.0)),
            age: 0.0,
        };
        assert!(cord_point(&line, 0.0).distance(Vec3::ZERO) < 1e-4);
        assert!(cord_point(&line, 1.0).distance(Vec3::new(0.0, 0.0, -20.0)) < 1e-3);
        let mut v = WicklineView::new();
        v.lines.push(line);
        let mut effects = Effects::headless();
        let mut rng = Rng::default();
        let ends = |l: &Line| (l.from, l.anchor.point());
        v.update(0.2, &ends, &mut effects, &mut rng);
        assert_eq!(v.lines().len(), 1);
        v.update(0.2, &ends, &mut effects, &mut rng);
        assert!(v.lines().is_empty(), "gone after {LIFE} s");
    }

    #[test]
    fn a_body_anchor_follows_what_it_holds() {
        let mut v = WicklineView::new();
        v.lines.push(Line {
            shooter: None,
            from: Vec3::ZERO,
            anchor: Anchor::Body(NetId(7), Vec3::new(0.0, 1.0, -10.0)),
            age: 0.0,
        });
        let mut effects = Effects::headless();
        let mut rng = Rng::default();
        let pulled = Vec3::new(0.0, 1.0, -6.0);
        v.update(0.05, &|_| (Vec3::ZERO, pulled), &mut effects, &mut rng);
        assert_eq!(v.lines()[0].anchor.point(), pulled);
    }
}
