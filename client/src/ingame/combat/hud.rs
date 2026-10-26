//! Combat HUD layer, drawn on top of `Hud::draw`: hit markers (body / headshot / armour /
//! kill), floating damage numbers, damage direction indicators, a low-health vignette, the
//! death screen ("RESPAWNING IN N"), thorn charges, per-weapon cooldowns, the Arclash
//! Discharge gauge next to the crosshair and a BRANDED warning while you carry a brand. Health and hard damage themselves go through
//! `HudInfo` into the existing health block.

use engine::gfx::{Camera, UiDrawList};
use engine::ui::draw::{fade, mix};
use engine::ui::hud;
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};
use shared::input::look_dir;
use shared::sim::{Weapon, WeaponState};
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
    pub time: f64,
}

#[derive(Default)]
pub struct CombatHud {
    markers: Vec<Marker>,
    numbers: Vec<Number>,
    indicators: Vec<Indicator>,
    died_at: Option<f64>,
}

const MARKER_TIME: f64 = 0.25;
const KILL_MARKER_TIME: f64 = 0.45;
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
        self.markers
            .retain(|m| m.kind == MarkerKind::Kill && kind != MarkerKind::Kill);
        self.markers.push(Marker { kind, time });
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

        // Low health vignette (pulsing).
        let low = (info.health / info.max_health.max(1.0)).clamp(0.0, 1.0);
        if !info.dead && low < 0.4 {
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
            let (color, r0, len, thick) = match m.kind {
                MarkerKind::Body => (t.bone, 5.0, 5.0, 1.0),
                MarkerKind::Head => (t.crimson_bright, 5.0, 7.0, 2.0),
                MarkerKind::Armour => (t.ash, 5.0, 4.0, 1.0),
                MarkerKind::Kill => (t.crimson_bright, 6.0 + 3.0 * age, 9.0, 2.0),
            };
            marker(
                list,
                center,
                r0,
                len,
                thick,
                fade(color, alpha),
                fade(t.ink, 0.8 * alpha),
            );
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

        // Cooldowns + Discharge gauge under / beside the crosshair.
        let w = info.weapons;
        let wi = (w.weapon as usize).min(3);
        let weapon = Weapon::from_slot(w.weapon).unwrap_or(Weapon::Quill);
        let rate = info.tick_rate as f32;
        let prim_total = (weapon.primary_interval(info.tuning) * rate).max(1.0);
        let alt_total = (weapon.alt_cooldown(w.mode, info.tuning) * rate).max(1.0);
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
        if w.charge > 0 {
            let f = w.charge_fraction(info.tuning, info.tick_rate);
            let h = 26.0;
            let bar = Rect::new(
                vec2(center.x + 14.0, center.y - h * 0.5),
                vec2(center.x + 17.0, center.y + h * 0.5),
            );
            list.fill(bar.expand(1.0), fade(t.ink, 0.7));
            let full = f >= 1.0 && (now * 10.0).fract() < 0.5;
            let color = if full {
                Vec4::ONE
            } else {
                mix(
                    Vec4::new(0.35, 0.65, 1.0, 1.0),
                    Vec4::new(0.8, 0.95, 1.0, 1.0),
                    f,
                )
            };
            list.fill(
                Rect::new(vec2(bar.min.x, bar.max.y - h * f), bar.max),
                color,
            );
            // Nova radius at this charge, beside the gauge.
            let a = &info.tuning.arclash;
            let radius =
                a.discharge_min_radius + (a.discharge_max_radius - a.discharge_min_radius) * f;
            list.text(
                vec2(bar.max.x + 4.0, bar.min.y - 2.0),
                &format!("{radius:.0}M"),
                &TextStyle::new(fade(color, 0.9)).outline(outline),
            );
        }

        // Thorns above the weapon block (bottom right).
        let charges = info.tuning.thorn.charges;
        let avail = w.thorns(info.tuning);
        let recharge =
            (w.thorn_recharge as f32 / (info.tuning.thorn.recharge_time * rate).max(1.0)).min(1.0);
        let pip = vec2(7.0, 4.0);
        let pw = hud::pips_width(pip, 3.0, charges);
        let px = c.max.x - 12.0 - pw;
        let py = c.max.y - 66.0;
        hud::pips(
            list,
            t,
            vec2(px, py),
            pip,
            3.0,
            avail as f32 + if avail < charges { recharge } else { 0.0 },
            charges,
        );
        list.text(
            vec2(px - 4.0, py - 2.0),
            "THORN",
            &TextStyle::new(t.bone_dim).right().outline(outline),
        );
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
        if info.dead {
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
            if info.respawn_in > 0.0 {
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
        } else {
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
}
