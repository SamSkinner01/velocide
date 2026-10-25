//! Scoreboard (hold Tab): every player with their colour and one column per stat.
//!
//! Columns are data ([`Column`]: title, width, a function from the row to the cell text and
//! colour); [`columns`] picks the set for the game mode. Adding a column = a field on
//! [`ScoreRow`] (filled in `InGame::draw_scoreboard`) + an entry in [`columns`]. Game-mode stats
//! come from the server's `ModeEvent::Scoreboard` (`ingame::mode_hud`).

use engine::gfx::UiDrawList;
use engine::ui::draw::{self, fade};
use engine::ui::hud;
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec4, vec2};
use shared::sim::ModeKind;
use shared::sim::style::rank_name;

use super::style_hud::rank_text_color;

/// One scoreboard line.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreRow {
    pub name: String,
    pub color: Vec4,
    /// Round-trip time in ms, when known (only our own for now).
    pub ping_ms: Option<f32>,
    pub is_local: bool,
    /// ALIVE / DOWNED / DEAD / BOT / HOST.
    pub status: &'static str,
    // Game-mode stats (`None` until the server's scoreboard arrives).
    pub kills: Option<u32>,
    pub deaths: Option<u32>,
    /// Deathmatch score.
    pub frags: Option<i32>,
    /// Current style rank.
    pub rank: Option<u8>,
    /// Style points earned.
    pub style: Option<u32>,
}

impl ScoreRow {
    /// A row with only a name (stats filled in by the caller).
    pub fn new(name: impl Into<String>, color: Vec4) -> Self {
        Self {
            name: name.into(),
            color,
            ping_ms: None,
            is_local: false,
            status: "",
            kills: None,
            deaths: None,
            frags: None,
            rank: None,
            style: None,
        }
    }
}

/// A scoreboard column: header, width (canvas units, right-aligned cells) and the cell.
pub struct Column {
    pub title: &'static str,
    pub width: f32,
    pub cell: fn(&ScoreRow, &Theme, f64) -> (String, Vec4),
}

fn num<T: std::fmt::Display>(v: Option<T>, t: &Theme) -> (String, Vec4) {
    match v {
        Some(v) => (v.to_string(), t.bone),
        None => ("-".into(), t.ash),
    }
}

const FRAGS: Column = Column {
    title: "FRAGS",
    width: 44.0,
    cell: |r, t, _| match r.frags {
        Some(f) if f < 0 => (f.to_string(), t.blood_bright),
        f => num(f, t),
    },
};
const KILLS: Column = Column {
    title: "KILLS",
    width: 40.0,
    cell: |r, t, _| num(r.kills, t),
};
const DEATHS: Column = Column {
    title: "DEATHS",
    width: 46.0,
    cell: |r, t, _| {
        let (s, _) = num(r.deaths, t);
        (s, t.bone_dim)
    },
};
const RANK: Column = Column {
    title: "STYLE RANK",
    width: 74.0,
    cell: |r, t, time| match r.rank {
        Some(rank) => (rank_name(rank).to_string(), rank_text_color(t, rank, time)),
        None => ("-".into(), t.ash),
    },
};
const STYLE: Column = Column {
    title: "STYLE",
    width: 50.0,
    cell: |r, t, _| {
        let (s, _) = num(r.style, t);
        (s, t.bone_dim)
    },
};
const PING: Column = Column {
    title: "PING",
    width: 36.0,
    cell: |r, t, _| match r.ping_ms {
        Some(p) => (format!("{}", p.round() as i32), hud::ping_color(t, p)),
        None => ("-".into(), t.ash),
    },
};
const STATUS: Column = Column {
    title: "STATUS",
    width: 52.0,
    cell: |r, t, _| {
        let c = match r.status {
            "DOWNED" => t.blood_bright,
            "DEAD" => t.ash,
            _ => t.bone_dim,
        };
        (r.status.to_string(), c)
    },
};

/// The columns for a game mode (`None`: no mode info yet).
pub fn columns(mode: Option<ModeKind>) -> Vec<Column> {
    match mode {
        Some(ModeKind::Deathmatch) => vec![FRAGS, KILLS, DEATHS, RANK, STYLE, PING, STATUS],
        Some(ModeKind::Campaign) => vec![KILLS, DEATHS, RANK, STYLE, PING, STATUS],
        None => vec![PING, STATUS],
    }
}

pub fn draw(
    list: &mut UiDrawList,
    t: &Theme,
    canvas: Rect,
    title: &str,
    columns: &[Column],
    rows: &[ScoreRow],
    time: f64,
) {
    let name_w = 150.0;
    let cols_w: f32 = columns.iter().map(|c| c.width + 8.0).sum();
    let w = (24.0 + name_w + cols_w).max(340.0).min(canvas.width() - 20.0);
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
    // Column right edges, from the right side of the panel.
    let mut rights = Vec::with_capacity(columns.len());
    let mut x = rect.max.x - 12.0;
    for c in columns.iter().rev() {
        rights.push(x);
        x -= c.width + 8.0;
    }
    rights.reverse();
    let name_limit = x - x_name - 8.0;
    list.text(vec2(x_name, y0), "PLAYER", &head);
    for (c, right) in columns.iter().zip(&rights) {
        list.text(vec2(*right, y0), c.title, &head.right());
    }
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
        let name = engine::ui::text::truncate(&r.name, name_limit - 8.0, 1.0);
        list.text(vec2(x_name + 8.0, y), &name, &TextStyle::new(t.bone));
        for (c, right) in columns.iter().zip(&rights) {
            let (text, color) = (c.cell)(r, t, time);
            list.text(vec2(*right, y), &text, &TextStyle::new(color).right());
        }
    }
}
