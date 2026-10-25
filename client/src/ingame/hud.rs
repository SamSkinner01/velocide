//! In-game HUD, drawn with `engine::ui::hud` into the frame's UI list.
//!
//! Layout (canvas 360 units tall): level timer top centre, status line under it; ping / FPS
//! top right with the event feed below; health + stamina bottom left; speedometer bottom
//! centre; weapon + fire modes bottom right; crosshair in the middle; name tags over players.
//!
//! Adding an element: add a field to [`HudInfo`] (filled in `InGame::draw_hud`) and draw it in
//! [`Hud::draw`]; for stateful elements (feeds, banners) add the state to [`Hud`].

use engine::gfx::{Camera, UiDrawList};
use engine::ui::draw::{fade, mix};
use engine::ui::hud::{self, CrosshairStyle, TextFeed};
use engine::ui::{Align, Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};

use crate::controls::WeaponSelect;
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
    /// Behind a wall (drawn faded).
    pub occluded: bool,
}

struct Banner {
    text: String,
    color: Vec4,
    start: f64,
}

pub struct Hud {
    /// Top-right feed: joins / leaves now, kills later ([`Hud::event`]).
    pub feed: TextFeed,
    banner: Option<Banner>,
}

impl Default for Hud {
    fn default() -> Self {
        let mut feed = TextFeed::new(6, 4.0);
        feed.newest_on_top = true;
        Self { feed, banner: None }
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
            list.text(vec2(c.center().x, y + 2.0), status, &style);
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
        // Marker chevron in the player's colour under the name.
        let m = Rect::from_center_size(vec2(p.x, r.max.y + 4.0), vec2(5.0, 2.0));
        list.fill(m.expand(1.0), fade(t.ink, 0.8 * alpha));
        list.fill(m, fade(tag.color, alpha));
    }
}
