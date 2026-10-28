//! Combat HUD layer, drawn on top of `Hud::draw`: hit markers (body / headshot / armour /
//! kill; bigger for a scattergun blast that landed most of its pellets), floating damage
//! numbers, damage direction indicators, a low-health vignette, the
//! death screen ("RESPAWNING IN N"), the knack (Q) slot, per-weapon cooldowns, the Arclash
//! Discharge gauge next to the crosshair and a BRANDED warning while you carry a brand. Health and hard damage themselves go through
//! `HudInfo` into the existing health block.

use engine::gfx::{Camera, UiDrawList};
use engine::ui::draw::{fade, mix};
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};
use shared::input::look_dir;
use shared::loadout::WeaponKind;
use shared::sim::{Ability, WeaponState};
use shared::tuning::CombatTuning;

use super::super::hud::project;

/// How a hit marker looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Body,
    Head,
    Armour,
    Kill,
}

#[derive(Clone, Copy, Debug)]
struct Marker {
    kind: MarkerKind,
    /// Size: 1 for a hit, more for a blast that landed most of its pellets.
    scale: f32,
    time: f64,
}

#[derive(Clone, Debug)]
struct Number {
    position: Vec3,
    text: String,
    color: Vec4,
    big: bool,
    time: f64,
}

#[derive(Clone, Copy, Debug)]
struct Indicator {
    /// World direction from the player towards where the damage came from.
    from: Vec3,
    strength: f32,
    time: f64,
}

/// What the combat HUD needs this frame.
pub struct CombatHudInfo<'a> {
    pub camera: &'a Camera,
    pub yaw: f32,
    pub health: f32,
    pub max_health: f32,
    pub dead: bool,
    pub respawn_in: f32,
    pub protected: bool,
    pub weapons: &'a WeaponState,
    pub tuning: &'a CombatTuning,
    pub tick_rate: u32,
    /// "KILLED BY ..." line for the death screen.
    pub killed_by: Option<&'a str>,
    /// Someone branded you (you take extra damage).
    pub branded: bool,
    /// You are burning (Burn status): ember-orange flickering screen edges.
    pub burning: bool,
    /// You are pinned (slowed): steel-blue bars at the screen edges and PINNED.
    pub pinned: bool,
    /// The Q slot (`knacks::draw_slot`); `None`: nothing to throw (the PvP Thorn).
    pub knack: Option<super::knacks::KnackSlot>,
    /// The punch's parry window still open, 1 → 0 (a closing ring round the crosshair in the
    /// parry colour, `docs/feel.md`).
    pub parry_window: f32,
    /// Out of the fight and watching a teammate (`ingame::spectate`): no death screen or
    /// low-health vignette over their view (the spectating HUD carries the countdown).
    pub spectating: bool,
    pub time: f64,
}

#[derive(Default)]
pub struct CombatHud {
    markers: Vec<Marker>,
    numbers: Vec<Number>,
    indicators: Vec<Indicator>,
    died_at: Option<f64>,
    /// When the bow reached full draw (the full-draw ring bursts from then).
    full_draw_at: Option<f64>,
}

const MARKER_TIME: f64 = 0.3;
const KILL_MARKER_TIME: f64 = 0.55;
/// Hit markers pop in bigger and settle over this long.
const MARKER_POP: f64 = 0.07;
const NUMBER_TIME: f64 = 0.8;
const INDICATOR_TIME: f64 = 1.3;

impl CombatHud {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.markers.clear();
        self.numbers.clear();
        self.indicators.clear();
        self.died_at = None;
    }

    pub fn hit_marker(&mut self, kind: MarkerKind, time: f64) {
        self.hit_marker_scaled(kind, 1.0, time);
    }

    /// A hit marker `scale` times the size (a blast that landed most of its pellets:
    /// [`blast_marker_scale`]).
    pub fn hit_marker_scaled(&mut self, kind: MarkerKind, scale: f32, time: f64) {
        self.markers
            .retain(|m| m.kind == MarkerKind::Kill && kind != MarkerKind::Kill);
        let scale = if scale.is_finite() {
            scale.clamp(1.0, 2.0)
        } else {
            1.0
        };
        self.markers.push(Marker { kind, scale, time });
    }

    pub fn damage_number(
        &mut self,
        position: Vec3,
        amount: f32,
        color: Vec4,
        big: bool,
        time: f64,
    ) {
        if amount < 0.5 {
            return;
        }
        // Merge with a fresh number at almost the same place (pellets).
        if let Some(n) = self.numbers.iter_mut().find(|n| {
            time - n.time < 0.12 && n.position.distance(position) < 1.5 && n.color == color
        }) {
            let total = n.text.parse::<f32>().unwrap_or(0.0) + amount;
            n.text = format!("{}", total.round() as i32);
            n.big |= big;
            return;
        }
        if self.numbers.len() > 24 {
            self.numbers.remove(0);
        }
        self.numbers.push(Number {
            position,
            text: format!("{}", amount.round() as i32),
            color,
            big,
            time,
        });
    }

    /// Damage arriving from world direction `from` (player → source).
    pub fn damage_from(&mut self, from: Vec3, strength: f32, time: f64) {
        let flat = Vec3::new(from.x, 0.0, from.z);
        if flat.length_squared() < 1e-4 {
            return;
        }
        if self.indicators.len() > 8 {
            self.indicators.remove(0);
        }
        self.indicators.push(Indicator {
            from: flat.normalize(),
            strength: strength.clamp(0.3, 1.0),
            time,
        });
    }

    pub fn draw(&mut self, list: &mut UiDrawList, t: &Theme, info: &CombatHudInfo) {
        let now = info.time;
        let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
        let center = c.center().floor() + vec2(0.5, 0.5);
        let outline = fade(t.ink, 0.9);

        // Low health vignette (pulsing; not over a teammate's view while you are down).
        let low = (info.health / info.max_health.max(1.0)).clamp(0.0, 1.0);
        if !info.dead && !info.spectating && low < 0.4 {
            let k = ((0.4 - low) / 0.25).clamp(0.0, 1.0);
            let pulse = 0.75 + 0.25 * ((now * 5.0).sin() as f32);
            vignette(list, c, t.crimson, 0.75 * (0.35 + 0.65 * k) * pulse);
        }

        // Damage direction indicators.
        self.indicators.retain(|i| now - i.time < INDICATOR_TIME);
        let fwd = look_dir(info.yaw, 0.0);
        let right = Vec3::new(-fwd.z, 0.0, fwd.x);
        for ind in &self.indicators {
            let age = ((now - ind.time) / INDICATOR_TIME) as f32;
            let alpha = (1.0 - age).powf(0.7) * ind.strength;
            let x = ind.from.dot(right);
            let y = ind.from.dot(fwd);
            let angle = x.atan2(y); // 0 = straight ahead, + = to the right
            arc_chevron(list, center, angle, 44.0, fade(t.crimson_bright, alpha));
        }

        // Hit markers.
        self.markers.retain(|m| {
            now - m.time
                < if m.kind == MarkerKind::Kill {
                    KILL_MARKER_TIME
                } else {
                    MARKER_TIME
                }
        });
        for m in &self.markers {
            let life = if m.kind == MarkerKind::Kill {
                KILL_MARKER_TIME
            } else {
                MARKER_TIME
            };
            let age = ((now - m.time) / life) as f32;
            let alpha = 1.0 - age * age;
            // Pop: drawn further out for a moment, then snapping in (docs/feel.md).
            let pop = 1.0 - ((now - m.time) / MARKER_POP).clamp(0.0, 1.0) as f32;
            let (color, r0, len, thick) = match m.kind {
                MarkerKind::Body => (t.bone, 5.0, 6.0, 2.0),
                MarkerKind::Head => (t.crimson_bright, 5.0, 9.0, 2.0),
                MarkerKind::Armour => (t.ash, 5.0, 4.0, 1.0),
                MarkerKind::Kill => (t.crimson_bright, 6.0 + 4.0 * age, 12.0, 3.0),
            };
            // A heavy blast: longer, thicker strokes further out (and a harder pop).
            let k = m.scale;
            marker(
                list,
                center,
                r0 * (0.5 + 0.5 * k) + 3.0 * k * pop,
                (len * k).round(),
                (thick + 2.0 * (k - 1.0)).round(),
                fade(color, alpha),
                fade(t.ink, 0.8 * alpha),
            );
            if m.kind == MarkerKind::Kill {
                ring(list, center, 10.0 + 22.0 * age, fade(color, 0.8 * alpha));
            }
        }

        // Damage numbers.
        self.numbers.retain(|n| now - n.time < NUMBER_TIME);
        for n in &self.numbers {
            let Some(p) = project(info.camera, c.size(), n.position) else {
                continue;
            };
            let age = ((now - n.time) / NUMBER_TIME) as f32;
            let alpha = if age < 0.6 {
                1.0
            } else {
                1.0 - (age - 0.6) / 0.4
            };
            let style = TextStyle::new(fade(n.color, alpha))
                .scale(if n.big { 2.0 } else { 1.0 })
                .centered()
                .outline(fade(t.ink, 0.85 * alpha));
            list.text(p - vec2(0.0, 6.0 + 18.0 * age), &n.text, &style);
        }

        // Cooldowns + charge / draw gauge under / beside the crosshair.
        let w = info.weapons;
        let wi = (w.slot as usize).min(w.primary_cooldown.len() - 1);
        let build = w.build();
        let rate = info.tick_rate as f32;
        let prim_total = (build.weapon.primary_interval(info.tuning) * rate).max(1.0);
        let alt_total = (build.ability(w.mode).cooldown(info.tuning) * rate).max(1.0);
        let bars = [
            (w.primary_cooldown[wi] as f32 / prim_total, t.bone_dim),
            (w.alt_cooldown[wi] as f32 / alt_total, t.crimson_bright),
        ];
        for (i, (frac, color)) in bars.iter().enumerate() {
            if *frac <= 0.0 || *frac > 1.5 {
                continue;
            }
            let y = center.y + 14.0 + i as f32 * 4.0;
            let width = 24.0;
            let bar = Rect::new(
                vec2(center.x - width * 0.5, y),
                vec2(center.x + width * 0.5, y + 2.0),
            );
            list.fill(bar.expand(1.0), fade(t.ink, 0.6));
            list.fill(
                Rect::new(
                    bar.min,
                    vec2(bar.min.x + width * (1.0 - frac.min(1.0)), bar.max.y),
                ),
                fade(*color, 0.85),
            );
        }
        // (The Lamplighter's focus has its own ring, and a held Tally / Riposte its own marks:
        // `tricks`.)
        let drawing = w.draw > 0 && w.current() == WeaponKind::Longwick;
        let charging = w.charge > 0 && !matches!(w.ability(), Ability::Tally | Ability::Riposte);
        if charging || drawing {
            let f = if drawing {
                w.draw_fraction(info.tuning, info.tick_rate)
            } else {
                w.charge_fraction(info.tuning, info.tick_rate)
            };
            let h = 26.0;
            let bar = Rect::new(
                vec2(center.x + 14.0, center.y - h * 0.5),
                vec2(center.x + 17.0, center.y + h * 0.5),
            );
            list.fill(bar.expand(1.0), fade(t.ink, 0.7));
            let full = f >= 1.0 && (now * 10.0).fract() < 0.5;
            // The bow's draw in wick gold, charges in arc blue.
            let (cold, hot) = if drawing {
                (
                    Vec4::new(1.0, 0.6, 0.25, 1.0),
                    Vec4::new(1.0, 0.92, 0.6, 1.0),
                )
            } else {
                (
                    Vec4::new(0.35, 0.65, 1.0, 1.0),
                    Vec4::new(0.8, 0.95, 1.0, 1.0),
                )
            };
            let color = if full { Vec4::ONE } else { mix(cold, hot, f) };
            list.fill(
                Rect::new(vec2(bar.min.x, bar.max.y - h * f), bar.max),
                color,
            );
            // Discharge: the nova radius at this charge, beside the gauge.
            let a = &info.tuning.arclash;
            if w.charge > 0 && w.ability() == Ability::Discharge {
                let radius =
                    a.discharge_min_radius + (a.discharge_max_radius - a.discharge_min_radius) * f;
                list.text(
                    vec2(bar.max.x + 4.0, bar.min.y - 2.0),
                    &format!("{radius:.0}M"),
                    &TextStyle::new(fade(color, 0.9)).outline(outline),
                );
            }
        }

        // The punch's parry window: a ring closing on the crosshair while it lasts.
        if info.parry_window > 0.0 && !info.dead {
            let pal = crate::palette::palette();
            let k = info.parry_window.clamp(0.0, 1.0);
            ring(
                list,
                center,
                7.0 + 12.0 * k,
                fade(pal.parry_window, 0.35 + 0.6 * k),
            );
        }

        // The bow reaching full draw (the Lamplighter its full focus): a gold ring bursts out
        // from the crosshair.
        let full = w.draw > 0 && w.draw_fraction(info.tuning, info.tick_rate) >= 1.0;
        match (full, self.full_draw_at) {
            (true, None) => self.full_draw_at = Some(now),
            (false, Some(_)) => self.full_draw_at = None,
            _ => {}
        }
        if let Some(at) = self.full_draw_at {
            let k = ((now - at) / 0.35) as f32;
            if k < 1.0 {
                let gold = Vec4::new(1.0, 0.85, 0.45, 1.0);
                ring(list, center, 8.0 + 30.0 * k, fade(gold, 1.0 - k));
                ring(
                    list,
                    center,
                    6.0 + 18.0 * k,
                    fade(Vec4::ONE, 0.8 * (1.0 - k)),
                );
            }
        }

        // The bow at full draw (and the ability it will loose).
        if drawing && w.draw_fraction(info.tuning, info.tick_rate) >= 1.0 {
            let text = match w.armed_ability() {
                Some(a) => format!("FULL - {}", a.name().to_uppercase()),
                None => "FULL".into(),
            };
            list.text(
                vec2(center.x + 21.0, center.y - 4.0),
                &text,
                &TextStyle::new(fade(t.bone, 0.9)).outline(outline),
            );
        }

        // Burn: flickering ember edges; Pinned: steel-blue bars and the word.
        if info.burning && !info.dead {
            let flicker =
                0.7 + 0.2 * ((now * 13.0).sin() as f32) + 0.1 * ((now * 31.0).sin() as f32);
            vignette(list, c, Vec4::new(1.0, 0.45, 0.08, 1.0), 0.55 * flicker);
            list.text(
                vec2(c.center().x, center.y + 40.0),
                "BURNING",
                &TextStyle::new(fade(Vec4::new(1.0, 0.6, 0.2, 1.0), flicker))
                    .centered()
                    .outline(outline),
            );
        }
        if info.pinned && !info.dead {
            let steel = Vec4::new(0.55, 0.75, 1.0, 1.0);
            for x in [c.min.x + 6.0, c.max.x - 10.0] {
                list.fill(
                    Rect::new(vec2(x, c.min.y + 40.0), vec2(x + 4.0, c.max.y - 40.0)),
                    fade(steel, 0.5),
                );
            }
            list.text(
                vec2(c.center().x, center.y + 50.0),
                "PINNED",
                &TextStyle::new(steel).centered().outline(outline),
            );
        }

        // The knack (Q) above the weapon block (bottom right).
        if let Some(slot) = &info.knack
            && !info.dead
        {
            super::knacks::draw_slot(list, t, c, slot, now);
        }
        if info.branded && !info.dead {
            let pulse = 0.65 + 0.35 * ((now * 6.0).sin() as f32);
            list.text(
                vec2(c.center().x, center.y + 30.0),
                "BRANDED",
                &TextStyle::new(fade(branded_text_color(), pulse))
                    .centered()
                    .outline(outline),
            );
        }
        if info.protected && !info.dead {
            list.text(
                vec2(c.center().x, c.max.y - 62.0),
                "PROTECTED",
                &TextStyle::new(fade(t.stamina, 0.8))
                    .centered()
                    .outline(outline),
            );
        }

        // Death screen.
        if info.dead && !info.spectating {
            let since = *self.died_at.get_or_insert(now);
            let k = ((now - since) / 0.6).min(1.0) as f32;
            list.fill(c, fade(Vec4::new(0.12, 0.0, 0.01, 1.0), 0.55 * k));
            vignette(list, c, t.crimson_dark, 0.7 * k);
            let title = TextStyle::new(fade(t.crimson_bright, k))
                .scale(4.0)
                .centered()
                .outline(outline);
            list.text(vec2(c.center().x, c.center().y - 44.0), "YOU DIED", &title);
            if let Some(by) = info.killed_by {
                let style = TextStyle::new(fade(t.bone, k)).centered().outline(outline);
                list.text(vec2(c.center().x, c.center().y - 6.0), by, &style);
            }
            // Longer waits are the game mode holding you out (grind: back at the next wave;
            // the mode HUD says so), not a countdown.
            if info.respawn_in > 0.0 && info.respawn_in < 600.0 {
                let style = TextStyle::new(fade(t.bone_dim, k))
                    .scale(2.0)
                    .centered()
                    .outline(outline);
                list.text(
                    vec2(c.center().x, c.center().y + 14.0),
                    &format!("RESPAWNING IN {:.1}", info.respawn_in),
                    &style,
                );
            }
        } else if !info.dead {
            self.died_at = None;
        }
    }
}

/// Darkened, coloured screen edges (concentric bands, strongest at the border).
/// "BRANDED" warning colour (`palette().brand` in the colour-blind palettes).
fn branded_text_color() -> Vec4 {
    let p = crate::palette::palette();
    p.effect(Vec4::new(1.0, 0.25, 0.4, 1.0), p.brand, 1.0)
}

fn vignette(list: &mut UiDrawList, c: Rect, color: Vec4, strength: f32) {
    if strength <= 0.01 {
        return;
    }
    let bands = 10;
    for i in 0..bands {
        let inset = i as f32 * 5.0;
        let alpha = strength * (1.0 - i as f32 / bands as f32).powf(1.6);
        let r = Rect::new(c.min + Vec2::splat(inset), c.max - Vec2::splat(inset));
        list.border(r, 5.0, fade(color, alpha));
    }
}

/// How big the hit marker for `hits` of a scattergun blast's `pellets` landing on one target
/// is: × 1 up to half of them, growing to × 1.7 for all of them.
pub fn blast_marker_scale(hits: u16, pellets: u32) -> f32 {
    let share = hits as f32 / pellets.max(1) as f32;
    1.0 + 0.7 * ((share - 0.5) / 0.5).clamp(0.0, 1.0)
}

/// A hit marker: four diagonal strokes around the crosshair.
fn marker(
    list: &mut UiDrawList,
    center: Vec2,
    r0: f32,
    len: f32,
    thick: f32,
    color: Vec4,
    shadow: Vec4,
) {
    for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        for pass in 0..2 {
            let (col, grow) = if pass == 0 {
                (shadow, 1.0)
            } else {
                (color, 0.0)
            };
            for i in 0..len as i32 {
                let d = r0 + i as f32;
                let p = center + vec2(sx * d, sy * d);
                let s = thick + grow * 2.0;
                list.fill(
                    Rect::from_center_size(p.floor() + vec2(0.5, 0.5), vec2(s, s)),
                    col,
                );
            }
        }
    }
}

/// A thin circle of `radius` around `center` (pixel dots, so it stays crisp).
fn ring(list: &mut UiDrawList, center: Vec2, radius: f32, color: Vec4) {
    if radius <= 0.0 || color.w <= 0.01 {
        return;
    }
    let n = ((radius * std::f32::consts::TAU / 2.0) as usize).clamp(12, 160);
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let p = center + vec2(a.cos(), a.sin()) * radius;
        list.fill(
            Rect::from_center_size(p.floor() + vec2(0.5, 0.5), vec2(1.5, 1.5)),
            color,
        );
    }
}

/// A chevron on a circle around the crosshair pointing outwards at `angle` (0 = up).
fn arc_chevron(list: &mut UiDrawList, center: Vec2, angle: f32, radius: f32, color: Vec4) {
    let dir = vec2(angle.sin(), -angle.cos());
    let side = vec2(-dir.y, dir.x);
    for i in -10..=10 {
        let s = i as f32;
        let back = (s.abs() * 0.5).round();
        for layer in 0..2 {
            let p = center + dir * (radius - back - layer as f32 * 2.0) + side * s;
            list.fill(
                Rect::from_center_size(p.floor() + vec2(0.5, 0.5), vec2(2.0, 2.0)),
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_numbers_and_indicators() {
        let mut h = CombatHud::new();
        h.hit_marker(MarkerKind::Body, 1.0);
        h.hit_marker(MarkerKind::Kill, 1.0);
        h.hit_marker(MarkerKind::Head, 1.01);
        assert_eq!(
            h.markers.len(),
            2,
            "a kill marker survives a newer hit marker"
        );
        // Pellets on one target merge into one number.
        for _ in 0..5 {
            h.damage_number(Vec3::ZERO, 9.0, Vec4::ONE, false, 1.0);
        }
        assert_eq!(h.numbers.len(), 1);
        assert_eq!(h.numbers[0].text, "45");
        h.damage_from(Vec3::new(1.0, 0.0, 0.0), 1.0, 1.0);
        h.damage_from(Vec3::Y, 1.0, 1.0);
        assert_eq!(
            h.indicators.len(),
            1,
            "no horizontal direction, no indicator"
        );
    }

    #[test]
    fn a_mostly_landed_blast_draws_a_bigger_marker() {
        assert_eq!(blast_marker_scale(3, 12), 1.0);
        assert_eq!(blast_marker_scale(6, 12), 1.0);
        assert!(blast_marker_scale(9, 12) > 1.3);
        assert!((blast_marker_scale(12, 12) - 1.7).abs() < 1e-6);
        assert!((blast_marker_scale(8, 8) - 1.7).abs() < 1e-6);
        let mut h = CombatHud::new();
        h.hit_marker_scaled(MarkerKind::Body, blast_marker_scale(12, 12), 1.0);
        h.hit_marker_scaled(MarkerKind::Body, f32::NAN, 1.1);
        assert_eq!(h.markers.len(), 1);
        assert_eq!(
            h.markers[0].scale, 1.0,
            "the newest marker, a NaN size made 1"
        );
    }
}
