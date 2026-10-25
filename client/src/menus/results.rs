//! End-of-match results screens with an animated tally reveal:
//!
//! * **Campaign, solo**: time vs par, kills (% of the level's enemies), style points and damage
//!   taken count up one after another, each stamped with its D/C/B/A/S grade, then the big
//!   overall rank slams in.
//! * **Campaign, co-op**: the team line (time, kills, team rank) and one row per player
//!   (kills, style, damage, deaths, revives, the four grades, overall), "LEFT BEHIND" for
//!   anyone who missed the extraction.
//! * **Deathmatch**: winner banner and the final standings (frags, deaths, best rank, style).
//!
//! Enter / click skips the reveal; then the lobby's buttons appear (CONTINUE TO LOBBY / LEAVE
//! for the host, RESTART / MAIN MENU in single player; drawn by `ingame::match_flow`).
//! Data: `shared::sim::MatchResults` (`ModeEvent::Ended`); grading rules in
//! docs/style_modes.md. Shown during the lobby's Results phase (`InGame::results_ui`).

use engine::audio::Sfx;
use engine::gfx::UiDrawList;
use engine::ui::draw::{self, fade, mix};
use engine::ui::hud::format_timer;
use engine::ui::text::line_width;
use engine::ui::{Rect, TextStyle, Theme, Ui, UiDrawExt, UiKey};
use glam::{Vec2, Vec4, vec2};
use shared::sim::style::rank_name;
use shared::lobby::GameMode;
use shared::sim::{EndReason, Grade, MatchResults, PlayerId, PlayerResult};

use crate::ingame::remote_players::player_color;
use crate::ingame::style_hud::rank_text_color;

/// One line of the solo tally: label, value at reveal progress 0..1, sub line, grade.
type TallyRow<'a> = (&'a str, Box<dyn Fn(f32) -> String + 'a>, String, Option<Grade>);

/// A reveal step: `(start, stamp)` seconds after opening.
#[derive(Clone, Copy, Debug)]
struct Step {
    start: f64,
    stamp: f64,
}

pub struct ResultsScreen {
    pub results: MatchResults,
    /// The local player (highlighted row, "VICTORY").
    pub local: Option<PlayerId>,
    opened: f64,
    /// Stamps already heard.
    stamps_played: usize,
}

const ROW_STEP: f64 = 0.55;
const FIRST: f64 = 0.55;

/// Colour of a grade letter.
pub fn grade_color(t: &Theme, g: Grade, time: f64) -> Vec4 {
    match g {
        Grade::S => {
            if (time * 7.0).fract() < 0.18 {
                t.blood_bright
            } else {
                t.rank_color(7)
            }
        }
        Grade::A => t.rank_color(4),
        Grade::B => t.rank_color(3),
        Grade::C => t.rank_color(1),
        Grade::D => t.ash,
    }
}

fn ease_out(p: f64) -> f32 {
    let p = p.clamp(0.0, 1.0) as f32;
    1.0 - (1.0 - p) * (1.0 - p) * (1.0 - p)
}

/// "12,345".
pub fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl ResultsScreen {
    pub fn new(results: MatchResults, local: Option<PlayerId>, now: f64) -> Self {
        Self {
            results,
            local,
            opened: now,
            stamps_played: 0,
        }
    }

    fn is_coop(&self) -> bool {
        self.results.rules.kind == GameMode::Campaign && self.results.players.len() > 1
    }

    /// The reveal steps of this layout (their stamps make the sounds).
    fn steps(&self) -> Vec<Step> {
        match self.results.rules.kind {
            GameMode::Campaign if !self.is_coop() => {
                let mut v: Vec<Step> = (0..4)
                    .map(|i| {
                        let start = FIRST + i as f64 * ROW_STEP;
                        Step {
                            start,
                            stamp: start + 0.42,
                        }
                    })
                    .collect();
                let last = v.last().map_or(FIRST, |s| s.stamp);
                v.push(Step {
                    start: last + 0.25,
                    stamp: last + 0.45,
                });
                v
            }
            GameMode::Campaign => {
                let mut v: Vec<Step> = (0..self.results.players.len())
                    .map(|i| {
                        let start = FIRST + 0.35 + i as f64 * 0.45;
                        Step {
                            start,
                            stamp: start + 0.35,
                        }
                    })
                    .collect();
                let last = v.last().map_or(FIRST, |s| s.stamp);
                v.push(Step {
                    start: last + 0.2,
                    stamp: last + 0.45,
                });
                v
            }
            GameMode::Deathmatch => {
                let mut v = vec![Step {
                    start: 0.35,
                    stamp: 0.6,
                }];
                v.extend((0..self.results.players.len()).map(|i| {
                    let start = 0.8 + i as f64 * 0.16;
                    Step {
                        start,
                        stamp: start + 0.12,
                    }
                }));
                v
            }
        }
    }

    /// Seconds until the reveal is complete.
    pub fn reveal_time(&self) -> f64 {
        self.steps().last().map_or(0.0, |s| s.stamp) + 0.3
    }

    pub fn finished(&self, now: f64) -> bool {
        now - self.opened >= self.reveal_time()
    }

    /// Jumps to the end of the reveal (finished from the next frame on, so the key that
    /// skipped doesn't also press a button).
    pub fn skip(&mut self, now: f64) {
        self.opened = now - self.reveal_time() + 1e-6;
        self.stamps_played = self.steps().len();
    }

    /// Draws the screen (not the buttons: see the module docs). Enter / Esc / click during
    /// the reveal skips it. Returns the sounds to play this frame (stamps, the final rank).
    pub fn draw(&mut self, ui: &mut Ui, now: f64) -> Vec<Sfx> {
        let mut sounds = Vec::new();
        let input = ui.input();
        let skip_pressed = input.mouse_pressed
            || input
                .keys
                .iter()
                .any(|k| matches!(k, UiKey::Enter | UiKey::Escape));
        if !self.finished(now) && skip_pressed {
            self.skip(now);
        }
        let age = now - self.opened;
        // Stamp sounds as the reveal passes them.
        let steps = self.steps();
        while self.stamps_played < steps.len() && age >= steps[self.stamps_played].stamp {
            let last = self.stamps_played + 1 == steps.len();
            sounds.push(if last {
                match self.results.rules.kind {
                    GameMode::Campaign => Sfx::RankUp,
                    GameMode::Deathmatch => Sfx::KillConfirm,
                }
            } else {
                Sfx::KillConfirm
            });
            self.stamps_played += 1;
        }

        let t = ui.theme();
        let c = ui.canvas();
        let list = ui.painter();
        backdrop(list, t, c, age);
        match self.results.rules.kind {
            GameMode::Campaign if self.is_coop() => self.draw_coop(list, t, c, age, now, &steps),
            GameMode::Campaign => self.draw_solo(list, t, c, age, now, &steps),
            GameMode::Deathmatch => self.draw_deathmatch(list, t, c, age, now, &steps),
        }
        if !self.finished(now) {
            let hint = TextStyle::new(t.ash).centered().shadow(t.shadow);
            list.text(vec2(c.center().x, c.max.y - 40.0), "ENTER  SKIP", &hint);
        }
        sounds
    }

    // ---- layouts ----------------------------------------------------------------------------

    fn draw_solo(&self, list: &mut UiDrawList, t: &Theme, c: Rect, age: f64, now: f64, steps: &[Step]) {
        let r = &self.results;
        let cx = c.center().x;
        title_block(list, t, vec2(cx, 22.0), "LEVEL COMPLETE", &r.level.to_uppercase(), age);
        let Some(me) = r.players.first() else {
            return;
        };
        let g = me.grades;
        let left = cx - 280.0;
        let right = cx + 30.0;
        let top = 96.0;
        let kill_pct = if r.total_enemies > 0 {
            me.stats.kills as f32 / r.total_enemies as f32 * 100.0
        } else {
            0.0
        };
        let rows: [TallyRow; 4] = [
            (
                "TIME",
                Box::new(|p| format_timer((r.duration_secs * p) as f64)),
                if r.par_secs > 0.0 {
                    format!("PAR {}", format_timer(r.par_secs as f64))
                } else {
                    "NO PAR TIME".into()
                },
                g.and_then(|g| g.time),
            ),
            (
                "KILLS",
                Box::new(|p| format!("{}", (me.stats.kills as f32 * p).round() as u32)),
                if r.total_enemies > 0 {
                    format!("OF {}  ({:.0}%)", r.total_enemies, kill_pct)
                } else {
                    "NO ENEMIES".into()
                },
                g.and_then(|g| g.kills),
            ),
            (
                "STYLE",
                Box::new(|p| thousands((me.style_points as f32 * p).round() as u32)),
                format!("BEST RANK {}", rank_name(me.best_rank)),
                g.map(|g| g.style),
            ),
            (
                "DAMAGE TAKEN",
                Box::new(|p| format!("{}", (me.stats.damage_taken * p).round() as u32)),
                format!(
                    "{} DEATH{}",
                    me.stats.deaths,
                    if me.stats.deaths == 1 { "" } else { "S" }
                ),
                g.map(|g| g.damage),
            ),
        ];
        for (i, (label, value, sub, grade)) in rows.iter().enumerate() {
            let s = steps[i];
            let y = top + i as f32 * 40.0;
            let a = ((age - s.start) / 0.15).clamp(0.0, 1.0) as f32;
            if a <= 0.0 {
                continue;
            }
            let slide = ((1.0 - a) * 30.0).round();
            let row = Rect::new(vec2(left - slide, y), vec2(right - slide, y + 34.0));
            draw::slanted(list, row, true, fade(t.ink, 0.55 * a));
            draw::fill(
                list,
                Rect::new(row.min, vec2(row.min.x + 2.0, row.max.y)),
                fade(t.blood, a),
            );
            list.text(
                vec2(row.min.x + 12.0, y + 5.0),
                label,
                &TextStyle::new(fade(t.bone_dim, a)),
            );
            list.text(
                vec2(row.min.x + 12.0, y + 19.0),
                sub,
                &TextStyle::new(fade(t.ash, a)),
            );
            let p = ease_out((age - s.start - 0.05) / 0.35);
            let counting = p < 1.0;
            let num_col = if counting { t.bone_dim } else { t.bone };
            list.text(
                vec2(row.max.x - 46.0, y + 9.0),
                &value(p),
                &TextStyle::new(fade(num_col, a))
                    .scale(2.0)
                    .right()
                    .outline(fade(t.ink, 0.9 * a)),
            );
            let gbox = Rect::new(vec2(row.max.x - 36.0, y + 3.0), vec2(row.max.x - 8.0, y + 31.0));
            match grade {
                Some(gr) => stamp(list, t, gbox, *gr, 3.0, age - s.stamp, now),
                None if age >= s.stamp => {
                    list.text_in(gbox, "-", &TextStyle::new(t.ash).scale(2.0).centered());
                }
                None => {}
            }
        }
        // The overall rank.
        let s = steps[4];
        let center = vec2(cx + 160.0, top + 72.0);
        let frame = Rect::from_center_size(center, vec2(132.0, 132.0));
        let a = ((age - s.start) / 0.2).clamp(0.0, 1.0) as f32;
        if a > 0.0 {
            draw::fill(list, frame, fade(t.ink, 0.6 * a));
            draw::border(list, frame, 1.0, fade(t.panel_edge, a));
            draw::corner_brackets(list, frame.expand(3.0), 14.0, 2.0, fade(t.bone_dim, a));
            list.text(
                vec2(center.x, frame.min.y - 14.0),
                "RANK",
                &TextStyle::new(fade(t.bone_dim, a)).centered(),
            );
            if let Some(g) = g {
                stamp(list, t, frame.shrink(10.0), g.overall, 12.0, age - s.stamp, now);
                if age >= s.stamp {
                    let k = (age - s.stamp - 0.3).clamp(0.0, 1.0) as f32;
                    list.text(
                        vec2(center.x, frame.max.y + 8.0),
                        grade_word(g.overall),
                        &TextStyle::new(fade(grade_color(t, g.overall, now), k)).centered(),
                    );
                }
            }
        }
    }

    fn draw_coop(&self, list: &mut UiDrawList, t: &Theme, c: Rect, age: f64, now: f64, steps: &[Step]) {
        let r = &self.results;
        let cx = c.center().x;
        title_block(list, t, vec2(cx, 22.0), "LEVEL COMPLETE", &r.level.to_uppercase(), age);
        let w = 600.0f32.min(c.width() - 20.0);
        let x0 = cx - w * 0.5;
        let x1 = cx + w * 0.5;
        // Team line.
        let a = ((age - 0.3) / 0.2).clamp(0.0, 1.0) as f32;
        let ty = 84.0;
        let team = Rect::new(vec2(x0, ty), vec2(x1, ty + 26.0));
        draw::slanted(list, team, true, fade(t.ink, 0.6 * a));
        let pct = if r.total_enemies > 0 {
            format!(
                "  ({:.0}%)",
                r.team_kills as f32 / r.total_enemies as f32 * 100.0
            )
        } else {
            String::new()
        };
        let par = if r.par_secs > 0.0 {
            format!("  PAR {}", format_timer(r.par_secs as f64))
        } else {
            String::new()
        };
        let line = format!(
            "TIME {}{}     KILLS {}/{}{}",
            format_timer(r.duration_secs as f64),
            par,
            r.team_kills,
            r.total_enemies,
            pct
        );
        list.text(
            vec2(x0 + 12.0, ty + 9.0),
            &line,
            &TextStyle::new(fade(t.bone, a)),
        );
        // Columns.
        let cols: [(&str, f32); 8] = [
            ("KILLS", 250.0),
            ("STYLE", 300.0),
            ("DMG", 340.0),
            ("DEATHS", 385.0),
            ("REVIVES", 432.0),
            ("T  K  S  D", 505.0),
            ("RANK", 570.0),
            ("", 0.0),
        ];
        let hy = ty + 38.0;
        let head = TextStyle::new(fade(t.ash, a)).right();
        list.text(vec2(x0 + 12.0, hy), "PLAYER", &TextStyle::new(fade(t.ash, a)));
        for (name, x) in cols.iter().filter(|c| !c.0.is_empty()) {
            list.text(vec2(x0 + x * w / 600.0, hy), name, &head);
        }
        draw::hline(list, x0 + 6.0, x1 - 6.0, hy + 10.0, fade(t.panel_edge, a));
        for (i, p) in r.players.iter().enumerate() {
            let s = steps[i];
            let y = hy + 16.0 + i as f32 * 26.0;
            let a = ((age - s.start) / 0.15).clamp(0.0, 1.0) as f32;
            if a <= 0.0 {
                continue;
            }
            let slide = ((1.0 - a) * 24.0).round();
            let row = Rect::new(vec2(x0 - slide, y - 3.0), vec2(x1 - slide, y + 19.0));
            let local = Some(p.player) == self.local;
            draw::slanted(
                list,
                row,
                true,
                if local {
                    fade(t.blood, 0.35 * a)
                } else {
                    fade(t.ink, 0.5 * a)
                },
            );
            let col = player_color(p.player);
            draw::fill(
                list,
                Rect::new(vec2(row.min.x + 6.0, y), vec2(row.min.x + 10.0, y + 13.0)),
                fade(col, a),
            );
            let xs = |x: f32| row.min.x + x * w / 600.0;
            let name = engine::ui::text::truncate(&p.name.to_uppercase(), 150.0, 1.0);
            list.text(vec2(row.min.x + 16.0, y + 1.0), &name, &TextStyle::new(fade(t.bone, a)));
            if !p.extracted {
                list.text(
                    vec2(row.min.x + 16.0, y + 10.0),
                    "LEFT BEHIND",
                    &TextStyle::new(fade(t.blood_bright, a * 0.8)),
                );
            }
            let num = TextStyle::new(fade(t.bone, a)).right();
            let p01 = ease_out((age - s.start) / 0.3);
            let cells = [
                (250.0, format!("{}", (p.stats.kills as f32 * p01).round() as u32)),
                (300.0, thousands((p.style_points as f32 * p01).round() as u32)),
                (340.0, format!("{}", (p.stats.damage_taken * p01).round() as u32)),
                (385.0, format!("{}", p.stats.deaths)),
                (432.0, format!("{}", p.stats.revives)),
            ];
            for (x, text) in cells {
                list.text(vec2(xs(x), y + 4.0), &text, &num);
            }
            if let Some(g) = p.grades {
                let small = [g.time, g.kills, Some(g.style), Some(g.damage)];
                for (k, gr) in small.iter().enumerate() {
                    let gx = xs(505.0) - 57.0 + k as f32 * 18.0;
                    let r = Rect::new(vec2(gx, y), vec2(gx + 12.0, y + 13.0));
                    match gr {
                        Some(gr) => stamp(list, t, r, *gr, 1.0, age - s.stamp + 0.05 * k as f64, now),
                        None if age >= s.stamp => {
                            list.text_in(r, "-", &TextStyle::new(t.ash).centered());
                        }
                        None => {}
                    }
                }
                let rr = Rect::new(vec2(xs(570.0) - 20.0, y - 2.0), vec2(xs(570.0), y + 16.0));
                stamp(list, t, rr, g.overall, 2.0, age - s.stamp - 0.15, now);
            }
        }
        // Team rank.
        let s = *steps.last().unwrap_or(&Step { start: 0.0, stamp: 0.0 });
        let a = ((age - s.start) / 0.2).clamp(0.0, 1.0) as f32;
        if a > 0.0
            && let Some(g) = r.team_grade
        {
            let by = hy + 16.0 + r.players.len() as f32 * 26.0 + 10.0;
            let label = TextStyle::new(fade(t.bone_dim, a)).right();
            list.text(vec2(x1 - 70.0, by + 16.0), "TEAM RANK", &label);
            let frame = Rect::new(vec2(x1 - 60.0, by), vec2(x1 - 6.0, by + 46.0));
            draw::fill(list, frame, fade(t.ink, 0.6 * a));
            draw::corner_brackets(list, frame.expand(2.0), 8.0, 1.0, fade(t.bone_dim, a));
            stamp(list, t, frame.shrink(4.0), g, 5.0, age - s.stamp, now);
        }
    }

    fn draw_deathmatch(&self, list: &mut UiDrawList, t: &Theme, c: Rect, age: f64, now: f64, steps: &[Step]) {
        let r = &self.results;
        let cx = c.center().x;
        let reason = match r.reason {
            EndReason::FragLimit => format!("FRAG LIMIT ({})", r.rules.frag_limit),
            EndReason::TimeLimit => format!("TIME LIMIT ({})", format_timer(r.rules.time_limit_secs as f64)),
            EndReason::Exit => "LEVEL EXIT".into(),
            EndReason::Host => "ENDED BY THE HOST".into(),
        };
        title_block(list, t, vec2(cx, 22.0), "MATCH OVER", &format!("{}  //  {}", r.level.to_uppercase(), reason), age);
        // Winner banner.
        let s = steps[0];
        if let Some(winner) = r.players.first() {
            let a = ((age - s.start) / 0.2).clamp(0.0, 1.0) as f32;
            let pop = (1.0 - ((age - s.stamp) / 0.2).clamp(0.0, 1.0)) as f32;
            let me = Some(winner.player) == self.local;
            let tied = r.players.get(1).is_some_and(|b| b.stats.frags == winner.stats.frags);
            let head = if tied && !me {
                "TOP OF THE TABLE"
            } else if me {
                "VICTORY"
            } else {
                "WINNER"
            };
            let y = 80.0;
            let band = Rect::new(vec2(cx - 200.0, y), vec2(cx + 200.0, y + 44.0));
            draw::slanted(list, band, true, fade(t.ink, 0.65 * a));
            let col = player_color(winner.player);
            draw::hline(list, band.min.x + 20.0, band.max.x, band.min.y, fade(col, a));
            draw::hline(list, band.min.x, band.max.x - 20.0, band.max.y - 1.0, fade(col, a));
            list.text(
                vec2(cx, y + 6.0),
                head,
                &TextStyle::new(fade(if me { t.rank_color(7) } else { t.bone_dim }, a)).centered(),
            );
            if age >= s.start {
                let scale = (3.0 + 2.0 * pop).round();
                let name = winner.name.to_uppercase();
                let y_name = y + 18.0 - (scale - 3.0) * 3.5;
                list.text(
                    vec2(cx, y_name.round()),
                    &name,
                    &TextStyle::new(fade(mix(col, t.bone, 0.2), a))
                        .scale(scale)
                        .centered()
                        .outline(fade(t.ink, a)),
                );
                // Chevrons either side.
                let half = line_width(&name, scale) * 0.5 + 14.0;
                for side in [-1.0f32, 1.0] {
                    for k in 0..3 {
                        let x = cx + side * (half + k as f32 * 7.0);
                        let r = Rect::from_center_size(vec2(x, y + 28.0), vec2(3.0, 9.0 - k as f32 * 2.0));
                        draw::fill(list, r, fade(col, a * (1.0 - k as f32 * 0.25)));
                    }
                }
            }
        }
        // Standings.
        let w = 520.0f32.min(c.width() - 20.0);
        let x0 = cx - w * 0.5;
        let hy = 140.0;
        let a = ((age - 0.7) / 0.2).clamp(0.0, 1.0) as f32;
        let head = TextStyle::new(fade(t.ash, a));
        let cols = [(330.0, "FRAGS"), (390.0, "DEATHS"), (470.0, "BEST RANK"), (515.0, "STYLE")];
        list.text(vec2(x0 + 8.0, hy), "#", &head);
        list.text(vec2(x0 + 30.0, hy), "PLAYER", &head);
        for (x, name) in cols {
            list.text(vec2(x0 + x * w / 520.0, hy), name, &head.right());
        }
        draw::hline(list, x0, x0 + w, hy + 10.0, fade(t.panel_edge, a));
        for (i, p) in r.players.iter().enumerate() {
            let s = steps[1 + i];
            let a = ((age - s.start) / 0.12).clamp(0.0, 1.0) as f32;
            if a <= 0.0 {
                continue;
            }
            let y = hy + 16.0 + i as f32 * 17.0;
            let slide = ((1.0 - a) * 20.0).round();
            self.standing_row(list, t, x0 - slide, w, y, i, p, a, now);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn standing_row(
        &self,
        list: &mut UiDrawList,
        t: &Theme,
        x0: f32,
        w: f32,
        y: f32,
        place: usize,
        p: &PlayerResult,
        a: f32,
        now: f64,
    ) {
        let local = Some(p.player) == self.local;
        let row = Rect::new(vec2(x0, y - 3.0), vec2(x0 + w, y + 12.0));
        let bg = if local {
            fade(t.blood, 0.4 * a)
        } else if place == 0 {
            fade(t.bone, 0.08 * a)
        } else {
            fade(t.ink, 0.45 * a)
        };
        draw::slanted(list, row, true, bg);
        let place_col = if place == 0 { t.rank_color(7) } else { t.bone_dim };
        list.text(vec2(x0 + 8.0, y), &format!("{}", place + 1), &TextStyle::new(fade(place_col, a)));
        draw::fill(
            list,
            Rect::new(vec2(x0 + 22.0, y), vec2(x0 + 26.0, y + 7.0)),
            fade(player_color(p.player), a),
        );
        let name = engine::ui::text::truncate(&p.name.to_uppercase(), 220.0, 1.0);
        list.text(vec2(x0 + 30.0, y), &name, &TextStyle::new(fade(t.bone, a)));
        let xs = |x: f32| x0 + x * w / 520.0;
        let num = |c: Vec4| TextStyle::new(fade(c, a)).right();
        let frag_col = if p.stats.frags < 0 { t.blood_bright } else { t.bone };
        list.text(vec2(xs(330.0), y), &format!("{}", p.stats.frags), &num(frag_col).scale(1.0));
        list.text(vec2(xs(390.0), y), &format!("{}", p.stats.deaths), &num(t.bone_dim));
        list.text(
            vec2(xs(470.0), y),
            rank_name(p.best_rank),
            &num(rank_text_color(t, p.best_rank, now)),
        );
        list.text(vec2(xs(515.0), y), &thousands(p.style_points), &num(t.bone_dim));
    }
}

fn grade_word(g: Grade) -> &'static str {
    match g {
        Grade::S => "SUPREME",
        Grade::A => "ADEPT",
        Grade::B => "BRUTAL",
        Grade::C => "CLUMSY",
        Grade::D => "DULL",
    }
}

/// Darkened game with a blood vignette and slow scanlines.
fn backdrop(list: &mut UiDrawList, t: &Theme, c: Rect, age: f64) {
    let a = (age / 0.25).clamp(0.0, 1.0) as f32;
    list.fill(c, fade(t.ink, 0.72 * a));
    // Vertical vignette bands.
    let steps = 10;
    for i in 0..steps {
        let f = i as f32 / steps as f32;
        let k = (1.0 - f).powf(2.0) * 0.35 * a;
        let wband = c.width() * 0.5 / steps as f32;
        list.fill(
            Rect::new(vec2(c.min.x + wband * i as f32, c.min.y), vec2(c.min.x + wband * (i + 1) as f32, c.max.y)),
            fade(t.blood_dark, k),
        );
        list.fill(
            Rect::new(vec2(c.max.x - wband * (i + 1) as f32, c.min.y), vec2(c.max.x - wband * i as f32, c.max.y)),
            fade(t.blood_dark, k),
        );
    }
    let mut y = c.min.y + ((age * 6.0) % 3.0) as f32;
    while y < c.max.y {
        draw::hline(list, c.min.x, c.max.x, y, fade(t.ink, 0.18 * a));
        y += 3.0;
    }
}

/// Big title with a blood extrusion, sliding in, and a subtitle under a slanted rule.
fn title_block(list: &mut UiDrawList, t: &Theme, top_center: Vec2, title: &str, subtitle: &str, age: f64) {
    let p = ease_out(age / 0.3);
    let scale = 3.0;
    let w = line_width(title, scale);
    let x = (top_center.x - w * 0.5 - (1.0 - p) * 60.0).round();
    let y = top_center.y;
    for k in (1..=5).rev() {
        let o = k as f32;
        list.text(
            vec2(x + o, y + o),
            title,
            &TextStyle::new(fade(mix(t.blood, t.blood_dark, k as f32 / 5.0), p)).scale(scale),
        );
    }
    list.text(
        vec2(x, y),
        title,
        &TextStyle::new(fade(t.bone, p)).scale(scale).outline(fade(t.ink, 0.9 * p)),
    );
    let ry = y + 30.0;
    draw::slanted(
        list,
        Rect::new(vec2(x - 10.0, ry), vec2(x + w * p + 30.0, ry + 3.0)),
        true,
        fade(t.blood, p),
    );
    // Speed streaks off the left end.
    for i in 0..3 {
        let len = 30.0 + i as f32 * 18.0;
        draw::hline(
            list,
            x - 24.0 - len - (1.0 - p) * 80.0,
            x - 24.0,
            y + 6.0 + i as f32 * 7.0,
            fade(t.blood_bright, 0.5 * p),
        );
    }
    list.text(
        vec2(top_center.x, ry + 8.0),
        subtitle,
        &TextStyle::new(fade(t.bone_dim, p)).centered().shadow(t.shadow),
    );
}

/// A grade letter stamped into `rect`: pops in big, flashes white, settles with a glow.
/// `since` = seconds since the stamp (negative: not yet).
fn stamp(list: &mut UiDrawList, t: &Theme, rect: Rect, g: Grade, scale: f32, since: f64, now: f64) {
    if since < 0.0 {
        return;
    }
    let pop = (1.0 - (since / 0.16).clamp(0.0, 1.0)) as f32;
    let s = (scale * (1.0 + 0.8 * pop)).round().max(1.0);
    let color = grade_color(t, g, now);
    if pop > 0.0 {
        draw::fill(list, rect.expand(2.0 * pop), fade(t.bone, 0.35 * pop));
    }
    if g == Grade::S {
        // Pulsing halo for the best grade.
        let k = ((now * 4.0).sin() as f32 * 0.5 + 0.5) * 0.25;
        draw::border(list, rect.expand(1.0), 1.0, fade(color, 0.3 + k));
    }
    let style = TextStyle::new(if since < 0.05 { t.bone } else { color })
        .scale(s)
        .centered()
        .outline(fade(t.ink, 0.95));
    list.text_in(rect, g.letter(), &style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::sim::{Grades, ModeRules, PlayerStats};

    fn results(kind: GameMode, players: usize) -> MatchResults {
        MatchResults {
            rules: match kind {
                GameMode::Campaign => ModeRules::campaign(),
                GameMode::Deathmatch => ModeRules::deathmatch(20, 600.0),
            },
            reason: EndReason::Exit,
            level: "Test".into(),
            duration_secs: 300.0,
            par_secs: 330.0,
            total_enemies: 86,
            team_kills: 80,
            players: (0..players)
                .map(|i| PlayerResult {
                    player: PlayerId(i as u8),
                    name: format!("P{i}"),
                    stats: PlayerStats::default(),
                    style_points: 1234,
                    best_rank: 3,
                    extracted: true,
                    grades: (kind == GameMode::Campaign).then_some(Grades {
                        time: Some(Grade::S),
                        kills: Some(Grade::A),
                        style: Grade::B,
                        damage: Grade::C,
                        overall: Grade::A,
                    }),
                })
                .collect(),
            team_grade: Some(Grade::A),
            winner: None,
        }
    }

    #[test]
    fn reveal_has_a_stamp_per_row_plus_the_rank() {
        let s = ResultsScreen::new(results(GameMode::Campaign, 1), None, 0.0);
        assert_eq!(s.steps().len(), 5);
        assert!(!s.finished(1.0));
        assert!(s.finished(10.0));
        let s = ResultsScreen::new(results(GameMode::Campaign, 3), None, 0.0);
        assert_eq!(s.steps().len(), 4);
        let mut s = ResultsScreen::new(results(GameMode::Deathmatch, 4), None, 0.0);
        assert_eq!(s.steps().len(), 5);
        s.skip(0.1);
        assert!(!s.finished(0.1), "not in the frame that skipped");
        assert!(s.finished(0.12));
    }

    #[test]
    fn numbers_get_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
