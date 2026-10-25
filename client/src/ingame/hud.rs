//! In-game HUD, drawn with `engine::ui::hud` into the frame's UI list.
//!
//! Layout (canvas 360 units tall): level timer top centre, status line below the mode /
//! encounter readouts under it; ping / FPS top right with the event feed below; health +
//! stamina bottom left; speedometer bottom centre; weapon + fire modes bottom right; crosshair
//! in the middle; name tags over players. With shape cues on (`palette::shape_cues`), health
//! changes pop up as `+N` / `-N` over the health number and name tags show marker shapes.
//!
//! Adding an element: add a field to [`HudInfo`] (filled in `InGame::draw_hud`) and draw it in
//! [`Hud::draw`]; for stateful elements (feeds, banners) add the state to [`Hud`].

use engine::gfx::{Camera, UiDrawList};
use engine::ui::draw::{fade, mix};
use engine::ui::hud::{self, CrosshairStyle, TextFeed};
use engine::ui::{Align, Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};

use super::cues;
use crate::controls::WeaponSelect;
use crate::palette::{MarkerShape, palette, shape_cues};
use crate::settings::Settings;

/// Everything the HUD shows this frame.
pub struct HudInfo<'a> {
    /// Label over the health bar: "BLOOD", or "DEAD" / "DOWNED".
    pub health_label: &'a str,
    pub health: f32,
    pub max_health: f32,
    pub hard_damage: f32,
    /// Dash charges available (fractional while regenerating) and the maximum.
    pub stamina: f32,
    pub stamina_max: u32,
    pub weapons: &'a WeaponSelect,
    /// Horizontal speed (m/s).
    pub speed: f32,
    /// Level timer (seconds), `None` before spawning.
    pub timer: Option<f64>,
    pub ping_ms: Option<f32>,
    pub fps: f32,
    /// Connection / game status line ("CONNECTION INTERRUPTED", "PAUSED"...).
    pub status: Option<(&'a str, Vec4)>,
    pub settings: &'a Settings,
    pub time: f64,
}

/// A name above a player.
pub struct NameTag<'a> {
    pub position: Vec3,
    pub name: &'a str,
    pub color: Vec4,
    /// Shape drawn under the name instead of the plain colour chevron (shape cues on).
    pub marker: Option<MarkerShape>,
    /// Behind a wall (drawn faded).
    pub occluded: bool,
}

struct Banner {
    text: String,
    color: Vec4,
    start: f64,
}

/// A `+12` / `-8` next to the health number (shape cues: the sign says heal or damage).
#[derive(Clone, Copy, Debug, PartialEq)]
struct HealthPopup {
    amount: f32,
    time: f64,
}

/// Seconds a health popup rises and fades.
const POPUP_LIFE: f64 = 1.1;

pub struct Hud {
    /// Top-right feed: joins / leaves now, kills later ([`Hud::event`]).
    pub feed: TextFeed,
    banner: Option<Banner>,
    last_health: Option<f32>,
    popups: Vec<HealthPopup>,
}

impl Default for Hud {
    fn default() -> Self {
        let mut feed = TextFeed::new(6, 4.0);
        feed.newest_on_top = true;
        Self {
            feed,
            banner: None,
            last_health: None,
            popups: Vec::new(),
        }
    }
}

impl Hud {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a line to the event / kill feed.
    pub fn event(&mut self, text: impl Into<String>, color: Vec4, time: f64) {
        self.feed.push(text, color, time);
    }

    /// Big centred announcement ("RESTART", rank-ups, wave start).
    pub fn banner(&mut self, text: impl Into<String>, color: Vec4, time: f64) {
        self.banner = Some(Banner {
            text: text.into(),
            color,
            start: time,
        });
    }

    /// Drops the current banner (the death view takes the middle of the screen).
    pub fn clear_banner(&mut self) {
        self.banner = None;
    }

    /// Turns health changes into popups (merging a burst of hits into one number). Respawns
    /// (from 0) and the first frame don't count.
    fn track_health(&mut self, health: f32, now: f64) {
        let last = self.last_health.replace(health);
        self.popups.retain(|p| now - p.time < POPUP_LIFE);
        let Some(last) = last else {
            return;
        };
        let delta = health - last;
        if delta.abs() < 0.5 || last <= 0.0 {
            return;
        }
        match self.popups.last_mut() {
            Some(p) if now - p.time < 0.25 && p.amount.signum() == delta.signum() => {
                p.amount += delta;
                p.time = now;
            }
            _ => self.popups.push(HealthPopup {
                amount: delta,
                time: now,
            }),
        }
        if self.popups.len() > 4 {
            self.popups.remove(0);
        }
    }

    /// `+N` (heal colour) / `-N` (damage colour) floating up from the health number.
    fn draw_popups(&self, list: &mut UiDrawList, t: &Theme, anchor: Vec2, now: f64) {
        let p = palette();
        for (i, popup) in self.popups.iter().rev().enumerate() {
            let age = ((now - popup.time) / POPUP_LIFE).clamp(0.0, 1.0) as f32;
            let alpha = (1.0 - age * age).clamp(0.0, 1.0);
            let (text, color) = if popup.amount > 0.0 {
                (format!("+{}", popup.amount.round() as i32), p.heal)
            } else {
                (format!("-{}", (-popup.amount).round() as i32), p.damage)
            };
            let y = (anchor.y - 12.0 * age - i as f32 * 10.0).round();
            let style = TextStyle::new(fade(color, alpha)).outline(fade(t.ink, 0.9 * alpha));
            list.text(vec2(anchor.x, y), &text, &style);
        }
    }

    pub fn draw(&mut self, list: &mut UiDrawList, t: &Theme, info: &HudInfo) {
        let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
        let now = info.time;
        let outline = fade(t.ink, 0.9);
        let s = info.settings;

        // Top centre: level timer + status.
        let mut y = 8.0;
        if s.show_timer
            && let Some(timer) = info.timer
        {
            let style = TextStyle::new(t.bone)
                .scale(2.0)
                .centered()
                .outline(outline);
            list.text(vec2(c.center().x, y), &hud::format_timer(timer), &style);
            y += 20.0;
        }
        if let Some((status, color)) = info.status {
            let blink = (now * 2.0).fract() < 0.7;
            let style = TextStyle::new(fade(color, if blink { 1.0 } else { 0.6 }))
                .centered()
                .outline(outline);
            // Under the timer the slots are taken: deathmatch TIME LEFT (30), the encounter
            // readout (40) and the extraction countdown (46..60) sit there.
            let status_y = if y > 8.0 { 64.0 } else { y + 2.0 };
            list.text(vec2(c.center().x, status_y), status, &style);
        }

        // Top right: ping, FPS, then the event feed.
        let mut ry = 8.0;
        if s.show_ping
            && let Some(ping) = info.ping_ms
        {
            hud::readout(
                list,
                t,
                vec2(c.max.x - 10.0, ry),
                Align::Right,
                "PING",
                &format!("{}", ping.round() as i32),
                hud::ping_color(t, ping),
            );
            ry += 11.0;
        }
        if s.show_fps {
            hud::readout(
                list,
                t,
                vec2(c.max.x - 10.0, ry),
                Align::Right,
                "FPS",
                &format!("{}", info.fps.round() as i32),
                t.bone_dim,
            );
            ry += 11.0;
        }
        self.feed.update(now);
        self.feed.draw(
            list,
            t,
            vec2(c.max.x - 10.0, ry + 8.0),
            Align::Right,
            1.0,
            now,
        );

        // Banner.
        if let Some(b) = &self.banner
            && !hud::banner(
                list,
                t,
                vec2(c.center().x, c.min.y + 96.0),
                &b.text,
                now - b.start,
                1.6,
                b.color,
            )
        {
            self.banner = None;
        }

        // Crosshair.
        let mut cross = CrosshairStyle::new(t);
        match s.crosshair {
            0 => {}
            1 => cross.dot = false,
            2 => cross.size = 0.0,
            _ => cross.color.w = 0.0,
        }
        if cross.color.w > 0.0 {
            if cross.size == 0.0 {
                let d = Rect::from_center_size(c.center().floor() + vec2(0.5, 0.5), vec2(2.0, 2.0));
                list.fill(d.expand(1.0), fade(t.ink, 0.8));
                list.fill(d, cross.color);
            } else {
                hud::crosshair(list, c.center(), &cross);
            }
        }

        // Bottom left: health + stamina.
        let health_rect = Rect::new(vec2(14.0, c.max.y - 48.0), vec2(200.0, c.max.y - 22.0));
        hud::health(
            list,
            t,
            health_rect,
            info.health_label,
            info.health,
            info.max_health,
            info.hard_damage,
            now,
        );
        self.track_health(info.health, now);
        if shape_cues() {
            self.draw_popups(list, t, vec2(14.0, c.max.y - 60.0), now);
        }
        let pip = vec2(14.0, 5.0);
        hud::pips(
            list,
            t,
            vec2(14.0, c.max.y - 15.0),
            pip,
            4.0,
            info.stamina,
            info.stamina_max,
        );
        let label_x = 14.0 + hud::pips_width(pip, 4.0, info.stamina_max) + 6.0;
        let dash_color = if info.stamina < 1.0 {
            t.blood_bright
        } else {
            t.bone_dim
        };
        list.text(
            vec2(label_x, c.max.y - 16.0),
            "DASH",
            &TextStyle::new(dash_color).outline(outline),
        );

        // Bottom centre: speedometer.
        if s.show_speed {
            let speed = info.speed;
            let hot = ((speed - 16.0) / 32.0).clamp(0.0, 1.0);
            let color = mix(t.bone, t.blood_bright, hot);
            let num = format!("{}", speed.round() as i32);
            let style = TextStyle::new(color).scale(2.0).centered().outline(outline);
            let base_y = c.max.y - 30.0;
            list.text(vec2(c.center().x, base_y), &num, &style);
            list.text(
                vec2(c.center().x, base_y + 17.0),
                "M/S",
                &TextStyle::new(t.bone_dim).centered().outline(outline),
            );
            // Speed bar: fills up to dash speed.
            let w = 60.0;
            let bar = Rect::new(
                vec2(c.center().x - w * 0.5, base_y - 6.0),
                vec2(c.center().x + w * 0.5, base_y - 4.0),
            );
            list.fill(bar.expand(1.0), fade(t.ink, 0.7));
            let f = (speed / 48.0).clamp(0.0, 1.0);
            list.fill(
                Rect::new(bar.min, vec2(bar.min.x + w * f, bar.max.y)),
                color,
            );
        }

        // Bottom right: weapon + fire modes.
        let w = info.weapons;
        let wi = w.info();
        hud::weapon(
            list,
            t,
            vec2(c.max.x - 12.0, c.max.y - 12.0),
            w.current as u32 + 1,
            wi.name,
            &wi.modes,
            w.mode() as usize,
        );
    }
}

/// Projects a world point to canvas units; `None` if behind the camera or far off screen.
pub fn project(camera: &Camera, canvas: Vec2, world: Vec3) -> Option<Vec2> {
    let aspect = canvas.x / canvas.y.max(1.0);
    let proj = glam::camera::rh::proj::directx::perspective(
        camera.fov_y_degrees.to_radians(),
        aspect,
        camera.near,
        camera.far,
    );
    let clip = proj * camera.view_matrix() * world.extend(1.0);
    if clip.w <= 0.05 {
        return None;
    }
    let ndc = Vec2::new(clip.x, clip.y) / clip.w;
    if ndc.x.abs() > 1.2 || ndc.y.abs() > 1.2 {
        return None;
    }
    Some(Vec2::new(
        (ndc.x * 0.5 + 0.5) * canvas.x,
        (0.5 - ndc.y * 0.5) * canvas.y,
    ))
}

/// Name tags over players: coloured marker + name, fading with distance, dimmed when behind
/// a wall.
pub fn draw_name_tags(list: &mut UiDrawList, t: &Theme, camera: &Camera, tags: &[NameTag]) {
    let canvas = list.canvas_size;
    for tag in tags {
        let Some(p) = project(camera, canvas, tag.position) else {
            continue;
        };
        let dist = camera.position.distance(tag.position);
        let mut alpha = (1.0 - (dist - 40.0) / 60.0).clamp(0.0, 1.0);
        if tag.occluded {
            alpha *= 0.4;
        }
        if alpha <= 0.02 {
            continue;
        }
        let p = p.round();
        let style = TextStyle::new(fade(t.bone, alpha))
            .centered()
            .outline(fade(t.ink, 0.85 * alpha));
        let r = list.text(p - vec2(0.0, 10.0), &tag.name.to_uppercase(), &style);
        let ink = fade(t.ink, 0.8 * alpha);
        if let Some(shape) = tag.marker {
            // The player's marker shape in their colour (shape cues).
            let at = vec2(p.x - 2.0, r.max.y + 2.0);
            cues::draw_marker(list, at, shape, 1.0, fade(tag.color, alpha), ink);
        } else {
            // Marker chevron in the player's colour under the name.
            let m = Rect::from_center_size(vec2(p.x, r.max.y + 4.0), vec2(5.0, 2.0));
            list.fill(m.expand(1.0), ink);
            list.fill(m, fade(tag.color, alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_changes_become_signed_popups() {
        let mut hud = Hud::new();
        hud.track_health(100.0, 0.0);
        assert!(hud.popups.is_empty(), "first frame sets the baseline");
        hud.track_health(90.0, 0.1);
        hud.track_health(84.0, 0.2); // same burst: merged
        assert_eq!(hud.popups.len(), 1);
        assert_eq!(hud.popups[0].amount, -16.0);
        hud.track_health(96.0, 0.3); // heal: a new popup
        assert_eq!(hud.popups.len(), 2);
        assert_eq!(hud.popups[1].amount, 12.0);
        hud.track_health(96.2, 0.4); // tiny changes don't count
        assert_eq!(hud.popups.len(), 2);
        hud.track_health(0.0, 0.5);
        hud.track_health(100.0, 0.6); // respawn: no "+100"
        assert_eq!(hud.popups.len(), 3, "the death counts, the respawn doesn't");
        assert!((hud.popups[2].amount + 96.2).abs() < 1e-3);
        hud.track_health(100.0, 5.0);
        assert!(hud.popups.is_empty(), "old popups expire");
    }
}
