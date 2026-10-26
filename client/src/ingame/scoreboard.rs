//! Scoreboard (hold Tab; also on the results screen): every player with their colour, host
//! crown, ping and status, plus per-mode score columns.
//!
//! **Adding a column** (frags, deaths, kills, style...): columns come from [`columns_for`]
//! (or any `&[ScoreColumn]` you build); every [`ScoreRow::cells`] holds one text per column in
//! the same order (optionally coloured through [`ScoreRow::cell_colors`]). The game-mode / style
//! code fills them in `InGame::score_rows` from the server's scoreboard (`ingame::mode_hud`)
//! and sorts the rows by score. Wider tables grow automatically.

use engine::gfx::UiDrawList;
use engine::ui::draw::{self, fade};
use engine::ui::hud;
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec4, vec2};
use shared::lobby::GameMode;

use super::cues;

/// Cell text until a system provides the value.
pub const PLACEHOLDER: &str = "-";

/// A score column: header text and width in canvas units (right-aligned values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScoreColumn {
    pub title: &'static str,
    pub width: f32,
}

impl ScoreColumn {
    pub const fn new(title: &'static str, width: f32) -> Self {
        Self { title, width }
    }
}

/// The score columns of a mode, in display order.
pub fn columns_for(mode: GameMode) -> Vec<ScoreColumn> {
    match mode {
        GameMode::GunGame => vec![
            ScoreColumn::new("TIER", 84.0),
            ScoreColumn::new("KILLS", 40.0),
            ScoreColumn::new("DEATHS", 48.0),
            ScoreColumn::new("RANK", 70.0),
            ScoreColumn::new("STYLE", 44.0),
        ],
        GameMode::Deathmatch => vec![
            ScoreColumn::new("FRAGS", 40.0),
            ScoreColumn::new("KILLS", 40.0),
            ScoreColumn::new("DEATHS", 48.0),
            ScoreColumn::new("RANK", 70.0),
            ScoreColumn::new("STYLE", 44.0),
        ],
        GameMode::Campaign => vec![
            ScoreColumn::new("KILLS", 40.0),
            ScoreColumn::new("DEATHS", 48.0),
            ScoreColumn::new("RANK", 70.0),
            ScoreColumn::new("STYLE", 44.0),
        ],
        GameMode::Grind => vec![
            ScoreColumn::new("KILLS", 40.0),
            ScoreColumn::new("DEATHS", 48.0),
            ScoreColumn::new("REVIVES", 54.0),
            ScoreColumn::new("RANK", 70.0),
            ScoreColumn::new("STYLE", 44.0),
        ],
    }
}

/// One scoreboard line.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreRow {
    pub name: String,
    pub color: Vec4,
    /// Player slot: with shape cues on, its marker shape replaces the colour bar.
    pub slot: Option<usize>,
    /// Round-trip time in ms (`None`: unknown / bot / single-player).
    pub ping_ms: Option<f32>,
    pub is_local: bool,
    pub is_host: bool,
    /// Right-most column: "HOST", "BOT", "ALIVE", "DOWNED", "DEAD"...
    pub status: String,
    /// One text per score column (see the module docs).
    pub cells: Vec<String>,
    /// Optional colour per cell (same order; missing / `None` = the default colour).
    pub cell_colors: Vec<Option<Vec4>>,
}

impl ScoreRow {
    /// A row with [`PLACEHOLDER`] cells for `columns` columns.
    pub fn new(name: &str, color: Vec4, columns: usize) -> Self {
        Self {
            name: name.to_string(),
            color,
            slot: None,
            ping_ms: None,
            is_local: false,
            is_host: false,
            status: String::new(),
            cells: vec![PLACEHOLDER.to_string(); columns],
            cell_colors: Vec::new(),
        }
    }
}

/// What to draw.
pub struct ScoreTable<'a> {
    pub title: &'a str,
    pub columns: &'a [ScoreColumn],
    pub rows: &'a [ScoreRow],
    /// Optional line under the rows (match summary, "waiting for the host"...).
    pub footer: Option<&'a str>,
}

const ROW_H: f32 = 13.0;
const NAME_MIN_W: f32 = 120.0;
const PING_W: f32 = 34.0;
const STATUS_W: f32 = 50.0;

/// Panel size for a table.
pub fn table_size(table: &ScoreTable, canvas: Rect) -> Vec2 {
    let cols: f32 = table.columns.iter().map(|c| c.width).sum();
    let w = (24.0 + 12.0 + NAME_MIN_W + cols + PING_W + STATUS_W)
        .max(340.0)
        .min(canvas.width() - 20.0);
    let footer = if table.footer.is_some() { 14.0 } else { 0.0 };
    vec2(
        w,
        40.0 + table.rows.len().max(1) as f32 * ROW_H + 10.0 + footer,
    )
}

/// Draws the table centred near the top of `canvas` (Tab scoreboard).
pub fn draw(list: &mut UiDrawList, t: &Theme, canvas: Rect, table: &ScoreTable) -> Rect {
    let size = table_size(table, canvas);
    let rect = Rect::from_center_size(
        vec2(canvas.center().x, canvas.min.y + 40.0 + size.y * 0.5),
        size,
    )
    .round();
    draw_in(list, t, rect, table);
    rect
}

/// Draws the table into `rect` (width from [`table_size`] or wider).
pub fn draw_in(list: &mut UiDrawList, t: &Theme, rect: Rect, table: &ScoreTable) {
    draw::fill(list, rect, t.panel);
    draw::border(list, rect, 1.0, t.panel_edge);
    draw::corner_brackets(list, rect.expand(2.0), 8.0, 1.0, t.bone_dim);
    let title_bar = Rect::new(rect.min, vec2(rect.max.x, rect.min.y + 15.0));
    draw::slanted(list, title_bar, true, t.crimson);
    list.text_in(
        title_bar.shrink2(vec2(8.0, 0.0)),
        table.title,
        &TextStyle::new(t.bone).shadow(t.crimson_dark),
    );
    let head = TextStyle::new(t.ash);
    let y0 = rect.min.y + 22.0;
    let x_name = rect.min.x + 12.0;
    let x_status = rect.max.x - 12.0;
    let x_ping = x_status - STATUS_W;
    // Score columns, right to left from the ping column.
    let mut col_right = Vec::with_capacity(table.columns.len());
    let mut x = x_ping - PING_W;
    for c in table.columns.iter().rev() {
        col_right.push(x);
        x -= c.width;
    }
    col_right.reverse();
    let name_right = x - 6.0;

    list.text(vec2(x_name + 8.0, y0), "PLAYER", &head);
    for (c, right) in table.columns.iter().zip(&col_right) {
        list.text(vec2(*right, y0), c.title, &head.right());
    }
    list.text(vec2(x_ping, y0), "PING", &head.right());
    list.text(vec2(x_status, y0), "STATUS", &head.right());
    draw::hline(
        list,
        rect.min.x + 8.0,
        rect.max.x - 8.0,
        y0 + 10.0,
        t.panel_edge,
    );
    for (i, r) in table.rows.iter().enumerate() {
        let y = y0 + 14.0 + i as f32 * ROW_H;
        if r.is_local {
            draw::fill(
                list,
                Rect::new(
                    vec2(rect.min.x + 6.0, y - 2.0),
                    vec2(rect.max.x - 6.0, y + 10.0),
                ),
                fade(t.crimson, 0.35),
            );
        }
        match r.slot {
            Some(slot) => {
                let ink = fade(t.ink, 0.9);
                cues::player_swatch(list, vec2(x_name, y), vec2(4.0, 7.0), slot, r.color, ink);
            }
            None => draw::fill(
                list,
                Rect::new(vec2(x_name, y), vec2(x_name + 4.0, y + 7.0)),
                r.color,
            ),
        }
        let mut name_x = x_name + 8.0;
        if r.is_host {
            crown(list, vec2(name_x, y), t.rank_color(1));
            name_x += 11.0;
        }
        let name = engine::ui::text::truncate(&r.name, name_right - name_x, 1.0);
        list.text(vec2(name_x, y), &name, &TextStyle::new(t.bone));
        for (i, (cell, right)) in r.cells.iter().zip(&col_right).enumerate() {
            let default = if cell == PLACEHOLDER { t.ash } else { t.bone };
            let color = r.cell_colors.get(i).copied().flatten().unwrap_or(default);
            list.text(vec2(*right, y), cell, &TextStyle::new(color).right());
        }
        let (ping, color) = match r.ping_ms {
            Some(p) => (format!("{}", p.round() as i32), hud::ping_color(t, p)),
            None => (PLACEHOLDER.to_string(), t.ash),
        };
        list.text(vec2(x_ping, y), &ping, &TextStyle::new(color).right());
        let status_color = match r.status.as_str() {
            "DOWNED" => t.crimson_bright,
            "DEAD" => t.ash,
            _ => t.bone_dim,
        };
        list.text(
            vec2(x_status, y),
            &r.status,
            &TextStyle::new(status_color).right(),
        );
    }
    if let Some(footer) = table.footer {
        let y = y0 + 14.0 + table.rows.len().max(1) as f32 * ROW_H + 2.0;
        draw::hline(
            list,
            rect.min.x + 8.0,
            rect.max.x - 8.0,
            y - 2.0,
            t.panel_edge,
        );
        let text = engine::ui::text::truncate(footer, rect.width() - 24.0, 1.0);
        list.text(vec2(x_name, y + 2.0), &text, &TextStyle::new(t.bone_dim));
    }
}

/// A 9 x 7 pixel crown (lobby host marker) with its top-left at `pos`.
pub fn crown(list: &mut UiDrawList, pos: Vec2, color: Vec4) {
    let p = pos.round();
    let px = |list: &mut UiDrawList, x: f32, y: f32, w: f32, h: f32| {
        draw::fill(list, Rect::from_min_size(p + vec2(x, y), vec2(w, h)), color);
    };
    // Three points, a band and a base.
    px(list, 0.0, 1.0, 1.0, 3.0);
    px(list, 4.0, 0.0, 1.0, 4.0);
    px(list, 8.0, 1.0, 1.0, 3.0);
    px(list, 1.0, 2.0, 1.0, 2.0);
    px(list, 7.0, 2.0, 1.0, 2.0);
    px(list, 3.0, 2.0, 3.0, 2.0);
    px(list, 0.0, 4.0, 9.0, 2.0);
    draw::fill(
        list,
        Rect::from_min_size(p + vec2(0.0, 6.0), vec2(9.0, 1.0)),
        fade(color, 0.6),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_line_up_with_cells() {
        for mode in GameMode::ALL {
            let cols = columns_for(mode);
            let row = ScoreRow::new("A", Vec4::ONE, cols.len());
            assert_eq!(row.cells.len(), cols.len());
            assert!(row.cells.iter().all(|c| c == PLACEHOLDER));
        }
    }

    #[test]
    fn tables_grow_with_columns_and_draw() {
        let canvas = Rect::from_min_size(Vec2::ZERO, vec2(640.0, 360.0));
        let few = [ScoreColumn::new("A", 30.0)];
        let many = [ScoreColumn::new("A", 80.0); 4];
        let rows = vec![ScoreRow::new("Alpha", Vec4::ONE, 4); 3];
        let small = table_size(
            &ScoreTable {
                title: "T",
                columns: &few,
                rows: &rows,
                footer: None,
            },
            canvas,
        );
        let table = ScoreTable {
            title: "T",
            columns: &many,
            rows: &rows,
            footer: Some("footer"),
        };
        let big = table_size(&table, canvas);
        assert!(big.x > small.x && big.y > small.y);
        let mut list = UiDrawList::new(canvas.size());
        let r = draw(&mut list, &Theme::default(), canvas, &table);
        assert!(canvas.contains_rect(r));
        assert!(!list.quads.is_empty());
    }
}
