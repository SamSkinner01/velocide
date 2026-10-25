//! Scoreboard (hold Tab): every player with their colour, ping (when known) and status.
//! Combat / game modes add columns (frags, deaths, style) by extending [`ScoreRow`].

use engine::gfx::UiDrawList;
use engine::ui::draw::{self, fade};
use engine::ui::hud;
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec4, vec2};

/// One scoreboard line.
pub struct ScoreRow {
    pub name: String,
    pub color: Vec4,
    /// Round-trip time in ms, when known (only our own for now).
    pub ping_ms: Option<f32>,
    pub is_local: bool,
    pub status: &'static str,
}

pub fn draw(list: &mut UiDrawList, t: &Theme, canvas: Rect, title: &str, rows: &[ScoreRow]) {
    let w = 340.0f32.min(canvas.width() - 20.0);
    let row_h = 13.0;
    let h = 40.0 + rows.len() as f32 * row_h + 10.0;
    let rect = Rect::from_center_size(
        vec2(canvas.center().x, canvas.min.y + 40.0 + h * 0.5),
        vec2(w, h),
    );
    draw::fill(list, rect, t.panel);
    draw::border(list, rect, 1.0, t.panel_edge);
    draw::corner_brackets(list, rect.expand(2.0), 8.0, 1.0, t.bone_dim);
    let title_bar = Rect::new(rect.min, vec2(rect.max.x, rect.min.y + 15.0));
    draw::slanted(list, title_bar, true, t.blood);
    list.text_in(
        title_bar.shrink2(vec2(8.0, 0.0)),
        title,
        &TextStyle::new(t.bone).shadow(t.blood_dark),
    );
    let head = TextStyle::new(t.ash);
    let y0 = rect.min.y + 22.0;
    let x_name = rect.min.x + 12.0;
    let x_ping = rect.max.x - 90.0;
    let x_status = rect.max.x - 12.0;
    list.text(vec2(x_name, y0), "PLAYER", &head);
    list.text(vec2(x_ping, y0), "PING", &head.right());
    list.text(vec2(x_status, y0), "STATUS", &head.right());
    draw::hline(
        list,
        rect.min.x + 8.0,
        rect.max.x - 8.0,
        y0 + 10.0,
        t.panel_edge,
    );
    for (i, r) in rows.iter().enumerate() {
        let y = y0 + 14.0 + i as f32 * row_h;
        if r.is_local {
            draw::fill(
                list,
                Rect::new(
                    vec2(rect.min.x + 6.0, y - 2.0),
                    vec2(rect.max.x - 6.0, y + 10.0),
                ),
                fade(t.blood, 0.35),
            );
        }
        draw::fill(
            list,
            Rect::new(vec2(x_name, y), vec2(x_name + 4.0, y + 7.0)),
            r.color,
        );
        let name = engine::ui::text::truncate(&r.name, x_ping - x_name - 60.0, 1.0);
        list.text(vec2(x_name + 8.0, y), &name, &TextStyle::new(t.bone));
        let (ping, color) = match r.ping_ms {
            Some(p) => (format!("{}", p.round() as i32), hud::ping_color(t, p)),
            None => ("-".to_string(), t.ash),
        };
        list.text(vec2(x_ping, y), &ping, &TextStyle::new(color).right());
        list.text(
            vec2(x_status, y),
            r.status,
            &TextStyle::new(t.bone_dim).right(),
        );
    }
}
