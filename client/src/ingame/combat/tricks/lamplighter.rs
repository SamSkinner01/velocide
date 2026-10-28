//! The Lamplighter (docs/rework.md §2): the scope (the zoom, the lens vignette, the focus ring
//! that is also the HUD's focus meter), the local focus feel (its sounds, the full-focus cue),
//! the beam and its hits, and the **tell**: other players see a focusing Lamplighter as a thin
//! glint line along its aim and a lens flare that grows with the focus (from the replicated
//! `DRAWING` + level, like the bow's draw), and hear its lens shimmer.
//!
//! The focus is the weapon state's draw (`WeaponState::draw_fraction`), so the viewmodel and
//! the third-person Shrike pose it through `HeldWeapon::draw` like the bow.

use std::collections::HashMap;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::UiDrawList;
use engine::physics::{CollisionWorld, MASK_SHOTS};
use engine::ui::Rect;
use engine::ui::draw::{fade, mix};
use glam::{Vec2, Vec3, Vec4, vec2};
use shared::protocol::NetId;

use super::super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::super::Rng;
use super::super::fx::Beams;
use crate::palette::palette;

/// The scope's view at full focus when the tuning has none: tan(fov/2) × 0.4
/// (`combat.lamplighter.zoom`).
pub const FULL_ZOOM: f32 = 0.4;
/// The wick's colours: the warm light of a focusing lens, and the white-hot one at full focus.
pub const WICK_GOLD: Vec4 = Vec4::new(1.0, 0.78, 0.4, 1.0);
pub const WHITE_HOT: Vec4 = Vec4::new(1.0, 0.96, 0.86, 1.0);
/// The scope's brass rim.
const BRASS: Vec4 = Vec4::new(0.85, 0.64, 0.3, 1.0);
/// The tell's glint line: how far it reaches when it meets nothing.
const TELL_RANGE: f32 = 90.0;
/// The tell's shimmer repeats this often while someone focuses.
const TELL_SOUND_EVERY: f64 = 0.55;

fn smoothstep(t: f32) -> f32 {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// The view's tan(fov/2) scale at `focus` 0..1 (eased like the bow's zoom): 1 → `full` (the
/// tuning's `lamplighter.zoom`, 0.4) at full focus.
pub fn zoom(focus: f32, full: f32) -> f32 {
    1.0 - (1.0 - full.clamp(0.05, 1.0)) * smoothstep(focus)
}

/// How strongly the scope's overlay shows at a view zoom (`CameraRig::zoom_scale`; `full` the
/// full focus's): it follows the eased zoom, so it fades in and out with it rather than with
/// the raw focus.
pub fn scope_strength(zoom_scale: f32, full: f32) -> f32 {
    let depth = (1.0 - full.clamp(0.05, 1.0)).max(0.05);
    ((1.0 - zoom_scale) / depth).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------------------------
// The scope (HUD)

/// The lens vignette at `strength` 0..1 (from [`scope_strength`]): the screen darkens outside
/// a round aperture that closes in as the lens focuses, with a soft inner edge, a thin brass
/// rim and four stadia lines reaching in from it; inside, a faint warm wash of the wick's light.
pub fn draw_scope(list: &mut UiDrawList, c: Rect, strength: f32, time: f64) {
    let s = strength.clamp(0.0, 1.0);
    if s <= 0.01 {
        return;
    }
    let center = c.center();
    let radius = c.size().y * (0.78 - 0.3 * s);
    let ink = Vec4::new(0.03, 0.015, 0.01, 1.0);
    // The wick's warm light through the glass.
    list.rect(c.min, c.max, fade(WICK_GOLD, 0.05 * s));
    // Dark outside the aperture, then a soft edge inside it.
    outside(list, c, center, radius, fade(ink, 0.9 * s));
    for (i, (inset, alpha)) in [(4.0, 0.5), (9.0, 0.28), (16.0, 0.12)].iter().enumerate() {
        let inner = radius - inset;
        let outer = if i == 0 { radius } else { radius - inset + 5.0 };
        annulus(list, center, inner, outer, fade(ink, alpha * s));
    }
    // The rim, glinting a little as the light moves on it.
    let glint = 0.55 + 0.15 * ((time * 1.7).sin() as f32);
    dots(list, center, radius, 1.5, fade(BRASS, glint * s));
    // Stadia lines from the rim towards the crosshair, stopping short of it.
    let line = fade(ink, 0.55 * s);
    let reach = radius * 0.42;
    for (dir, across) in [
        (vec2(1.0, 0.0), vec2(0.0, 1.0)),
        (vec2(-1.0, 0.0), vec2(0.0, 1.0)),
        (vec2(0.0, 1.0), vec2(1.0, 0.0)),
        (vec2(0.0, -1.0), vec2(1.0, 0.0)),
    ] {
        let a = center + dir * radius;
        let b = center + dir * (radius - reach);
        let (lo, hi) = (a.min(b) - across * 0.5, a.max(b) + across * 0.5);
        list.rect(lo.floor(), hi.floor() + across, line);
        // A tick every eighth of the way in.
        for k in 1..4 {
            let p = center + dir * (radius - reach * k as f32 / 4.0);
            let half = across * (2.0 + (k % 2) as f32 * 2.0);
            list.rect((p - half).floor(), (p + half).floor() + dir.abs(), line);
        }
    }
}

/// The focus ring round the crosshair, which is the focus meter too: a dim dotted circle that
/// closes in on the crosshair as the lens focuses, the focused part of it (from the top,
/// clockwise) bright in the wick's gold, four notches closing in with it; at full focus the
/// whole ring burns white and pulses.
pub fn draw_focus_ring(list: &mut UiDrawList, center: Vec2, focus: f32, time: f64) {
    let f = focus.clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    let e = smoothstep(f);
    let radius = 26.0 - 15.0 * e;
    let full = f >= 0.999;
    let pulse = if full {
        0.75 + 0.25 * ((time * 12.0).sin() as f32).abs()
    } else {
        1.0
    };
    let ink = Vec4::new(0.0, 0.0, 0.0, 0.75);
    let lit = if full {
        WHITE_HOT
    } else {
        mix(WICK_GOLD, WHITE_HOT, e * 0.6)
    };
    let n = ((radius * std::f32::consts::TAU / 2.5) as usize).clamp(16, 96);
    for i in 0..n {
        let k = i as f32 / n as f32;
        let a = k * std::f32::consts::TAU;
        // From the top, clockwise on screen.
        let p = center + vec2(a.sin(), -a.cos()) * radius;
        let on = full || k <= f;
        let color = if on {
            fade(lit, pulse)
        } else {
            fade(WICK_GOLD, 0.3)
        };
        let size = if on { 2.0 } else { 1.0 };
        list.rect(
            (p - Vec2::splat(size * 0.5 + 0.5)).floor(),
            (p + Vec2::splat(size * 0.5 + 0.5)).floor(),
            ink,
        );
        list.rect(
            (p - Vec2::splat(size * 0.5)).floor(),
            (p + Vec2::splat(size * 0.5)).floor(),
            color,
        );
    }
    // Four notches just outside the ring, pointing in.
    for dir in [
        vec2(0.0, -1.0),
        vec2(1.0, 0.0),
        vec2(0.0, 1.0),
        vec2(-1.0, 0.0),
    ] {
        let a = center + dir * (radius + 3.0);
        let b = center + dir * (radius + 7.0);
        let across = vec2(dir.y, dir.x).abs();
        let (lo, hi) = (a.min(b) - across, a.max(b) + across);
        list.rect(
            lo.floor(),
            hi.floor(),
            fade(lit, 0.8 * pulse * (0.4 + 0.6 * e)),
        );
    }
}

/// Fills the canvas outside the circle (`center`, `radius`) in 2-pixel rows.
fn outside(list: &mut UiDrawList, c: Rect, center: Vec2, radius: f32, color: Vec4) {
    let mut y = c.min.y;
    while y < c.max.y {
        let dy = (y + 1.0 - center.y).abs();
        let row = |list: &mut UiDrawList, x0: f32, x1: f32| {
            if x1 > x0 {
                list.rect(vec2(x0, y), vec2(x1, y + 2.0), color);
            }
        };
        if dy >= radius {
            row(list, c.min.x, c.max.x);
        } else {
            let dx = (radius * radius - dy * dy).sqrt();
            row(list, c.min.x, (center.x - dx).floor());
            row(list, (center.x + dx).ceil(), c.max.x);
        }
        y += 2.0;
    }
}

/// Fills the ring between `inner` and `outer` (2-pixel rows).
fn annulus(list: &mut UiDrawList, center: Vec2, inner: f32, outer: f32, color: Vec4) {
    if outer <= inner || outer <= 0.0 {
        return;
    }
    let mut y = (center.y - outer).floor();
    while y < center.y + outer {
        let dy = (y + 1.0 - center.y).abs();
        if dy < outer {
            let xo = (outer * outer - dy * dy).sqrt();
            let xi = if dy < inner {
                (inner * inner - dy * dy).sqrt()
            } else {
                0.0
            };
            for (x0, x1) in [
                (center.x - xo, center.x - xi),
                (center.x + xi, center.x + xo),
            ] {
                if x1 - x0 >= 0.5 {
                    list.rect(vec2(x0.floor(), y), vec2(x1.ceil(), y + 2.0), color);
                }
            }
        }
        y += 2.0;
    }
}

/// A thin circle of pixel dots.
fn dots(list: &mut UiDrawList, center: Vec2, radius: f32, size: f32, color: Vec4) {
    let n = ((radius * std::f32::consts::TAU / 2.0) as usize).clamp(12, 720);
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let p = center + vec2(a.cos(), a.sin()) * radius;
        list.rect(
            (p - Vec2::splat(size * 0.5)).floor(),
            (p + Vec2::splat(size * 0.5)).floor() + Vec2::ONE,
            color,
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The beam

/// The Lamplighter's beam from `muzzle` through `ends` (a full focus pierces: the first end is
/// where it passed through a target, the last where it stopped) at `focus` 0..1: a white-hot
/// core inside a wide wick-gold glow, thicker and longer-lived the more it was focused, a wake
/// of embers along it, a light at the lens and a flash where it struck.
pub fn beam(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    muzzle: Vec3,
    ends: &[Vec3],
    focus: f32,
    local: bool,
) {
    let f = focus.clamp(0.0, 1.0);
    let pal = palette();
    let gold = pal.effect(Vec4::new(2.0, 1.45, 0.65, 0.9), pal.aim_laser, 1.8);
    let core = Vec4::new(2.6, 2.5, 2.2, 1.0);
    let life = 0.3 + 0.35 * f;
    let mut from = muzzle;
    for (i, end) in ends.iter().enumerate() {
        let end = *end;
        let d = end - from;
        let len = d.length();
        if len < 0.05 || !len.is_finite() {
            continue;
        }
        let dir = d / len;
        // The shooter's own starts right under the camera: keep its first metres thin.
        let start = if local && i == 0 {
            from + dir * len.min(1.5)
        } else {
            from
        };
        if local && i == 0 {
            beams.beam(from, start, 0.02 + 0.02 * f, core, life * 0.6);
        }
        beams.beam(start, end, 0.08 + 0.12 * f, gold, life);
        beams.beam(start, end, 0.025 + 0.03 * f, core, life * 0.8);
        // Embers left hanging along the line, drifting up as they cool.
        let count = ((len / 2.5) as usize).clamp(3, 24);
        for _ in 0..count {
            let at = from + d * rng.unit();
            effects.spawn(Particle {
                position: at,
                velocity: rng.dir() * 0.4 + Vec3::Y * rng.range(0.2, 0.8),
                drag: 1.5,
                max_life: rng.range(0.4, 0.9) * (0.6 + 0.4 * f),
                size: Vec2::splat(rng.range(0.04, 0.08)),
                end_scale: 0.3,
                color: Vec4::new(1.0, 0.7, 0.3, 1.0),
                texture: FxTex::Spark,
                additive: true,
                ..Default::default()
            });
        }
        // Where it struck (or passed through): a flash, sparks and a light.
        effects.glow(end, fade(WHITE_HOT, 0.9), 0.5 + 0.8 * f, 0.12);
        for _ in 0..(6 + (10.0 * f) as usize) {
            effects.spawn(Particle {
                position: end,
                velocity: (-dir + rng.dir() * 0.8) * rng.range(2.0, 7.0),
                gravity: 9.0,
                drag: 1.0,
                max_life: rng.range(0.15, 0.4),
                size: Vec2::splat(0.06),
                end_scale: 0.3,
                color: Vec4::new(1.0, 0.82, 0.45, 1.0),
                texture: FxTex::Spark,
                additive: true,
                collide: Some(OnHit::Die),
                ..Default::default()
            });
        }
        beams.light(end, Vec3::new(2.2, 1.6, 0.8) * (0.6 + 0.6 * f), 5.0, 0.15);
        from = end;
    }
    // The lens flares as the light leaves it.
    effects.glow(muzzle, fade(WHITE_HOT, 0.8), 0.25 + 0.35 * f, 0.08);
    beams.light(muzzle, Vec3::new(2.6, 2.0, 1.1) * (0.6 + 0.5 * f), 7.0, 0.1);
}

/// Where the shooter's own beam goes as their screen shows it (the server's is authoritative):
/// from `eye` along `dir`, stopping at the first of `bodies` (capsules: a, b, radius) or the
/// level (`wall`: how far the level is along it); a full focus (`pierce` targets) passes
/// through the first ones and stops at the last. Returns each end in order, and whether the
/// last one is a body.
pub fn beam_ends(
    eye: Vec3,
    dir: Vec3,
    wall: f32,
    bodies: &[(Vec3, Vec3, f32)],
    pierce: u32,
) -> (Vec<Vec3>, bool) {
    let mut hits: Vec<f32> = bodies
        .iter()
        .filter_map(|&(a, b, r)| {
            shared::sim::HitShape::capsule(shared::sim::HitPart::Body, a, b, r)
                .ray_hit(eye, dir, wall)
        })
        .collect();
    hits.sort_by(f32::total_cmp);
    hits.truncate(pierce.max(1) as usize);
    let body = !hits.is_empty();
    // A pierce that ran out of targets carries on to the wall.
    if hits.len() < pierce.max(1) as usize {
        hits.push(wall);
    }
    let last_is_body = body && hits.last() != Some(&wall);
    (
        hits.into_iter().map(|d| eye + dir * d).collect(),
        last_is_body,
    )
}

/// How a beam at `focus` sounds to its shooter (volume, pitch): a tap is a lighter, higher
/// crack, a full focus the whole thing.
pub fn shot_sound(focus: f32) -> (f32, f32) {
    let f = focus.clamp(0.0, 1.0);
    (0.55 + 0.45 * f, 1.18 - 0.18 * f)
}

// ---------------------------------------------------------------------------------------------
// The tell

/// A focusing Lamplighter as others see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focusing {
    pub player: NetId,
    /// The lens (world).
    pub muzzle: Vec3,
    /// Its aim.
    pub aim: Vec3,
    /// 0..1.
    pub focus: f32,
}

/// Per-player memory of the tell (the shimmer's timing, the last focus for the beam's weight).
#[derive(Clone, Copy, Debug, Default)]
struct Remote {
    focus: f32,
    sound_at: f64,
    seen: bool,
}

/// What the Lamplighter remembers between frames.
#[derive(Default)]
pub struct LamplighterView {
    time: f64,
    remote: HashMap<NetId, Remote>,
    /// The local focus: the focusing sound is playing / full focus was reached.
    focusing: bool,
    full: bool,
}

impl LamplighterView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The local player's focus this frame (`None`: not focusing a Lamplighter): the focusing
    /// sound while it lasts, the lens note at full focus. Returns whether full focus was
    /// reached this frame (the caller flashes).
    pub fn local_feel(&mut self, focus: Option<f32>, audio: &mut Audio) -> bool {
        let focusing = focus.is_some();
        if focusing && !self.focusing {
            audio.play_2d(Sfx::LamplighterFocus);
        } else if !focusing && self.focusing {
            audio.stop(Sfx::LamplighterFocus);
            self.full = false;
        }
        self.focusing = focusing;
        let full_now = focus.is_some_and(|f| f >= 1.0) && !self.full;
        if full_now {
            audio.play_2d(Sfx::LamplighterFull);
            self.full = true;
        }
        full_now
    }

    /// How focused `player`'s Lamplighter was when last seen (their shot's beam weight).
    pub fn remote_focus(&self, player: NetId) -> f32 {
        self.remote.get(&player).map_or(0.0, |r| r.focus)
    }

    /// Per frame: every focusing Lamplighter but your own draws its tell (seen from
    /// `camera`, looking along `view`) and shimmers now and then.
    #[allow(clippy::too_many_arguments)]
    pub fn update_tells(
        &mut self,
        focusing: &[Focusing],
        world: Option<&CollisionWorld>,
        camera: Vec3,
        view: Vec3,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        dt: f32,
    ) {
        self.time += dt.max(0.0) as f64;
        for r in self.remote.values_mut() {
            r.seen = false;
        }
        for f in focusing {
            let r = self.remote.entry(f.player).or_default();
            r.seen = true;
            r.focus = f.focus;
            if self.time - r.sound_at >= TELL_SOUND_EVERY {
                r.sound_at = self.time;
                audio.play(
                    Sfx::LamplighterTell,
                    PlayParams::at(f.muzzle).with_volume(0.4 + 0.6 * f.focus),
                );
            }
            tell(f, world, camera, view, self.time, effects, beams);
        }
        // Forget players who stopped focusing a while ago (their last focus is kept for the
        // shot that usually follows the release).
        let now = self.time;
        self.remote.retain(|_, r| r.seen || now - r.sound_at < 2.0);
    }
}

/// One Lamplighter's tell this frame: a thin glint line along its aim to what it would hit,
/// and a lens flare (a glow and a short horizontal streak) that grows with the focus and is
/// brightest looking down its barrel. Colour-vision palettes draw it in the aim laser's colour.
pub fn tell(
    f: &Focusing,
    world: Option<&CollisionWorld>,
    camera: Vec3,
    view: Vec3,
    time: f64,
    effects: &mut Effects,
    beams: &mut Beams,
) {
    let aim = f.aim.normalize_or(Vec3::NEG_Z);
    let e = smoothstep(f.focus);
    let full = f.focus >= 0.999;
    let strobe = if full {
        0.8 + 0.2 * ((time * 11.0).sin() as f32).abs()
    } else {
        1.0
    };
    let pal = palette();
    let gold = pal.effect(Vec4::new(1.6, 1.2, 0.6, 1.0), pal.aim_laser, 1.4);
    let reach = world
        .and_then(|w| w.cast_ray(f.muzzle, aim, TELL_RANGE, MASK_SHOTS))
        .map_or(TELL_RANGE, |h| h.distance);
    let end = f.muzzle + aim * reach;
    beams.frame_beam(
        f.muzzle,
        end,
        0.01 + 0.012 * e,
        fade(gold, (0.12 + 0.38 * e) * strobe),
    );
    // The glint: strongest when the lens looks at you.
    let to_eye = (camera - f.muzzle).normalize_or(Vec3::Y);
    let facing = aim.dot(to_eye).max(0.0).powi(6);
    let flare = (0.3 + 2.0 * e) * (0.4 + 0.6 * facing) * strobe;
    let distance = camera.distance(f.muzzle).max(1.0);
    // Far away it keeps a minimum size on screen, so a sniper can be spotted across the map.
    let size = flare.max(distance * 0.025 * (0.3 + e));
    effects.billboard(
        f.muzzle,
        size,
        fade(pal.effect(WICK_GOLD, pal.aim_laser, 1.0), 0.45 + 0.4 * e),
        FxTex::Glow,
        0.0,
    );
    effects.billboard(
        f.muzzle,
        size * 0.4,
        Vec4::new(2.0, 1.9, 1.7, 1.0),
        FxTex::Glow,
        0.0,
    );
    // The streak: across the screen, through the lens.
    let right = view.cross(Vec3::Y).normalize_or(Vec3::X);
    let half = size * (0.9 + 1.2 * facing);
    beams.frame_beam(
        f.muzzle - right * half,
        f.muzzle + right * half,
        size * 0.06,
        fade(gold, 0.6 * (0.3 + 0.7 * e) * strobe),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_zooms_to_four_tenths_eased_with_the_focus() {
        let full = FULL_ZOOM;
        assert_eq!(zoom(0.0, full), 1.0);
        assert!((zoom(1.0, full) - 0.4).abs() < 1e-6);
        assert!(zoom(0.25, full) > 0.9 && zoom(0.5, full) < 0.75 && zoom(0.5, full) > 0.65);
        assert_eq!(zoom(f32::NAN, full), 1.0);
        assert_eq!(scope_strength(1.0, full), 0.0);
        assert!((scope_strength(0.4, full) - 1.0).abs() < 1e-6);
        // A tuning with a shallower scope still fills the overlay at its full focus.
        assert!((scope_strength(zoom(1.0, 0.7), 0.7) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_scope_and_the_ring_draw_only_while_focusing() {
        let c = Rect::from_min_size(Vec2::ZERO, vec2(640.0, 360.0));
        let mut list = UiDrawList::new(c.size());
        draw_scope(&mut list, c, 0.0, 0.0);
        draw_focus_ring(&mut list, c.center(), 0.0, 0.0);
        assert!(list.quads.is_empty());
        draw_scope(&mut list, c, 1.0, 0.0);
        let scope = list.quads.len();
        // The corners are dark, the middle is clear.
        let covers = |p: Vec2| {
            list.quads
                .iter()
                .filter(|q| q.color.w > 0.5)
                .any(|q| p.x >= q.min.x && p.x < q.max.x && p.y >= q.min.y && p.y < q.max.y)
        };
        assert!(covers(vec2(3.0, 3.0)) && covers(vec2(636.0, 356.0)));
        assert!(!covers(c.center() + vec2(20.0, 10.0)));
        assert!(scope > 100 && scope < 4000, "{scope} quads");
        draw_focus_ring(&mut list, c.center(), 0.5, 0.0);
        assert!(list.quads.len() > scope);
    }

    #[test]
    fn the_local_feel_plays_the_focus_once_and_the_full_note_once() {
        let mut v = LamplighterView::new();
        let mut audio = Audio::disabled();
        assert!(!v.local_feel(Some(0.3), &mut audio));
        assert!(v.local_feel(Some(1.0), &mut audio));
        assert!(!v.local_feel(Some(1.0), &mut audio));
        assert!(!v.local_feel(None, &mut audio));
        assert!(
            v.local_feel(Some(1.0), &mut audio),
            "a new focus rings again"
        );
    }
}
