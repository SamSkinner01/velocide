//! Stoke (docs/rework.md §1, Rapid, area **client-b**): the weapon stoked for a few seconds
//! (faster, burning shots), then spent — overheated, no primary, venting.
//!
//! * **Stoked** ([`heat_tint`], [`stoked_effects`]): the held weapon's glow ramps from its
//!   ability colour towards white-hot over the stoke and its metal reddens; embers spit from
//!   the muzzle (a hot shimmer of them on every shot). `Sfx::StokeIgnite` when it catches.
//! * **Spent** ([`vent_effects`]): the glow drops to a dull red, steam jets from the muzzle and
//!   rolls off the weapon, `Sfx::StokeVent`; the metal ticks as it cools.
//! * **The HUD** ([`hud`]): a heat gauge right of the crosshair: STOKED and its time burning
//!   down in the ability's forge red, then OVERHEAT in the palette's warning colour, blinking,
//!   striped (the shape cue) as it refills to ready.
//!
//! The same tint and effects serve others' weapons (third person) when their state says so.

use engine::gfx::UiDrawList;
use engine::ui::draw::fade;
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};

use super::super::effects::{Effects, FxTex, Particle};
use super::Rng;
use crate::models::gear::STOKE_COLOR;

/// The weapon's state for the visuals.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Heat {
    /// Stoked: 0..1 of its time gone (0 = just stoked, 1 = about to be spent); `None`: not.
    pub stoked: Option<f32>,
    /// Spent (overheated): 0..1 of the vent left (1 = just spent); `None`: not.
    pub venting: Option<f32>,
}

impl Heat {
    pub fn active(&self) -> bool {
        self.stoked.is_some() || self.venting.is_some()
    }
}

fn unit(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// The weapon's tint for `heat` (`Tint::glow`, `Tint::body`): stoked, its glow climbing
/// towards white-hot and its metal reddening; venting, a dull red glow on hot metal.
pub fn heat_tint(heat: &Heat, time: f64) -> (Vec4, Vec4) {
    let flicker = 0.92 + 0.08 * ((time * 23.0).sin() as f32);
    if let Some(p) = heat.stoked {
        let r = (0.35 + 0.65 * unit(p)) * flicker;
        (
            Vec4::new(1.0 + 2.6 * r, 1.0 + 1.3 * r, 1.0 + 0.4 * r, 1.0),
            Vec4::new(1.0 + 0.9 * r, 1.0 + 0.15 * r, 1.0 - 0.2 * r, 1.0),
        )
    } else if let Some(v) = heat.venting {
        let v = unit(v);
        (
            Vec4::new(0.55 + 0.3 * v, 0.28, 0.22, 1.0),
            Vec4::new(1.0 + 0.25 * v, 1.0 - 0.05 * v, 1.0 - 0.1 * v, 1.0),
        )
    } else {
        (Vec4::ONE, Vec4::ONE)
    }
}

fn forge(k: f32, a: f32) -> Vec4 {
    (STOKE_COLOR.truncate() * k).extend(a)
}

/// A stoked weapon's frame at its `muzzle` pointing `dir`: embers spitting out of it, more as
/// the stoke burns on.
pub fn stoked_effects(
    effects: &mut Effects,
    rng: &mut Rng,
    muzzle: Vec3,
    dir: Vec3,
    p: f32,
    dt: f32,
) {
    let rate = 24.0 + 40.0 * unit(p);
    if rng.unit() < dt * rate {
        effects.spawn(Particle {
            position: muzzle + rng.dir() * 0.03,
            velocity: dir * rng.range(0.5, 1.5) + Vec3::Y * rng.range(0.5, 1.5) + rng.dir() * 0.4,
            gravity: -0.8,
            drag: 1.2,
            max_life: rng.range(0.25, 0.45),
            size: Vec2::splat(rng.range(0.03, 0.06)),
            end_scale: 0.4,
            color: forge(1.8, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// A stoked shot: a hot shimmer of embers off the muzzle.
pub fn stoked_shot(effects: &mut Effects, rng: &mut Rng, muzzle: Vec3, dir: Vec3) {
    for _ in 0..4 {
        effects.spawn(Particle {
            position: muzzle,
            velocity: (dir + rng.dir() * 0.5) * rng.range(2.0, 4.0),
            gravity: 2.0,
            drag: 2.0,
            max_life: 0.25,
            size: Vec2::splat(0.04),
            color: forge(2.0, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// A spent weapon venting (`v` 0..1 of the vent left): steam jetting from the muzzle (hard at
/// first) and rolling up off `body` (a point on the weapon).
#[allow(clippy::too_many_arguments)]
pub fn vent_effects(
    effects: &mut Effects,
    rng: &mut Rng,
    muzzle: Vec3,
    body: Vec3,
    dir: Vec3,
    v: f32,
    small: bool,
    dt: f32,
) {
    let v = unit(v);
    let size = if small { 0.5 } else { 1.0 };
    let rate = 10.0 + 30.0 * v;
    if rng.unit() < dt * rate {
        effects.spawn(Particle {
            position: muzzle,
            velocity: (dir * (1.0 + 3.0 * v) + Vec3::Y * 0.8 + rng.dir() * 0.4) * size,
            gravity: -1.2,
            drag: 2.5,
            max_life: rng.range(0.5, 0.8),
            size: Vec2::splat(0.12 * size),
            end_scale: 3.0,
            color: Vec4::new(0.92, 0.9, 0.88, 0.32),
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            spin: rng.range(-1.5, 1.5),
            ..Default::default()
        });
    }
    if rng.unit() < dt * 8.0 {
        effects.spawn(Particle {
            position: body + rng.dir() * 0.05 * size,
            velocity: Vec3::Y * 0.6 * size + rng.dir() * 0.15,
            gravity: -0.8,
            drag: 1.5,
            max_life: rng.range(0.6, 1.0),
            size: Vec2::splat(0.08 * size),
            end_scale: 2.5,
            color: Vec4::new(0.85, 0.85, 0.85, 0.22),
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            ..Default::default()
        });
    }
}

/// The heat gauge right of the crosshair (`heat`: yours).
pub fn hud(list: &mut UiDrawList, t: &Theme, c: Rect, heat: &Heat, now: f64) {
    if !heat.active() {
        return;
    }
    let p = crate::palette::palette();
    let outline = fade(t.ink, 0.9);
    let center = c.center();
    let gauge = Rect::from_min_size(center + vec2(26.0, -18.0), vec2(5.0, 36.0));
    list.fill(gauge.expand(1.0), fade(t.ink, 0.7));
    let (fill, color, label) = if let Some(s) = heat.stoked {
        let flicker = 0.85 + 0.15 * ((now * 17.0).sin() as f32);
        (1.0 - unit(s), fade(STOKE_COLOR, flicker), "STOKED")
    } else {
        let v = unit(heat.venting.unwrap_or(0.0));
        let blink = 0.55 + 0.45 * ((now * 8.0).sin() as f32).abs();
        (1.0 - v, fade(p.warning, blink), "OVERHEAT")
    };
    let h = gauge.height() * unit(fill);
    let bar = Rect::new(vec2(gauge.min.x, gauge.max.y - h), gauge.max);
    list.fill(bar, color);
    if heat.venting.is_some() {
        // Stripes across the gauge (the overheat reads without its colour).
        let mut y = gauge.min.y + 1.0;
        while y < gauge.max.y {
            list.fill(
                Rect::new(vec2(gauge.min.x, y), vec2(gauge.max.x, y + 1.0)),
                fade(t.ink, 0.55),
            );
            y += 4.0;
        }
    }
    list.text(
        vec2(gauge.max.x + 3.0, gauge.max.y - 7.0),
        label,
        &TextStyle::new(color).outline(outline),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glow_climbs_while_stoked_and_dulls_while_venting() {
        let calm = heat_tint(&Heat::default(), 0.0);
        assert_eq!(calm, (Vec4::ONE, Vec4::ONE));
        let early = heat_tint(
            &Heat {
                stoked: Some(0.0),
                venting: None,
            },
            0.0,
        )
        .0;
        let late = heat_tint(
            &Heat {
                stoked: Some(1.0),
                venting: None,
            },
            0.0,
        )
        .0;
        assert!(late.x > early.x && early.x > 1.0);
        let vent = heat_tint(
            &Heat {
                stoked: None,
                venting: Some(1.0),
            },
            0.0,
        )
        .0;
        assert!(vent.x < 1.0 && vent.y < vent.x);
        let wild = heat_tint(
            &Heat {
                stoked: Some(f32::NAN),
                venting: None,
            },
            1e9,
        );
        assert!(wild.0.is_finite() && wild.1.is_finite());
    }
}
