//! The style meter on the HUD: rank name in its colour with an animated fill, rank pips, the
//! STALE indicator and freshness of the held weapon, the scrolling style feed ("+ PARRY",
//! "+ AIRKILL x2", "- STALE"), the rank-up banner and the rank sounds.
//!
//! Data: the server-authoritative [`StyleView`] in the local player's `OwnState`
//! (`NetClient::own_style`) and the owner-only [`StyleEvent`]s (`GameEvent::Style`), see
//! `shared::sim::style` and docs/style_modes.md. Layout: right side of the screen, under the
//! top-right event feed.

use engine::audio::{Audio, Sfx};
use engine::gfx::UiDrawList;
use engine::ui::draw::{self, fade, mix};
use engine::ui::hud::{self, TextFeed};
use engine::ui::text::line_width;
use engine::ui::{Align, Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec4, vec2};
use shared::sim::style::{TOP_RANK, rank_name};
use shared::sim::{StyleBonus, StyleEvent, StyleKey, StyleView};
use shared::tuning::StyleTuning;

/// Width of the style block (canvas units).
const BLOCK_W: f32 = 150.0;
/// Top of the style block.
const BLOCK_TOP: f32 = 112.0;
/// Seconds the rank flash / shake lasts.
const RANK_FLASH: f64 = 0.45;
/// Seconds the rank-up banner stays.
const BANNER_TIME: f64 = 1.1;

pub struct StyleHud {
    /// The style feed under the meter.
    pub feed: TextFeed,
    /// Bonuses received this frame (grouped before they go into the feed).
    pending: Vec<(StyleBonus, u8)>,
    /// Displayed meter fill (eases towards the real one).
    shown: f32,
    shown_rank: u8,
    /// Last rank change: time and direction.
    rank_change: Option<(f64, bool)>,
    banner: Option<(u8, f64)>,
    /// Screenshots / tools: draw this instead of the network's view.
    pub override_view: Option<StyleView>,
}

impl Default for StyleHud {
    fn default() -> Self {
        let mut feed = TextFeed::new(8, 2.4);
        feed.newest_on_top = true;
        feed.fade = 0.5;
        Self {
            feed,
            pending: Vec::new(),
            shown: 0.0,
            shown_rank: 0,
            rank_change: None,
            banner: None,
            override_view: None,
        }
    }
}

/// Feed line for a bonus.
pub fn feed_text(bonus: StyleBonus) -> String {
    match bonus {
        StyleBonus::Stale => "- STALE".to_string(),
        b => format!("+ {}", b.label()),
    }
}

/// The bonuses worth a sound of their own (kills already have the kill confirm).
fn notable(bonus: StyleBonus) -> bool {
    !matches!(
        bonus,
        StyleBonus::Kill | StyleBonus::Frag | StyleBonus::Stale | StyleBonus::Headshot
    )
}

impl StyleHud {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything (level change, restart).
    pub fn reset(&mut self) {
        let keep = self.override_view;
        *self = Self::default();
        self.override_view = keep;
    }

    /// The rank currently shown.
    pub fn rank(&self) -> u8 {
        self.shown_rank
    }

    /// A style event for the local player.
    pub fn on_event(&mut self, e: &StyleEvent, audio: &mut Audio, now: f64) {
        match *e {
            StyleEvent::Bonus { bonus, count, .. } => self.pending.push((bonus, count)),
            StyleEvent::Rank { rank, up, .. } => {
                self.rank_change = Some((now, up));
                self.shown_rank = rank;
                self.shown = if up { 0.0 } else { self.shown.max(0.6) };
                if up {
                    self.banner = Some((rank, now));
                    audio.play_2d(Sfx::RankUp);
                } else {
                    audio.play_2d(Sfx::RankDown);
                }
            }
        }
    }

    /// Moves this frame's bonuses into the feed: repeats are grouped ("+ KILL x3"), the
    /// multikill shows its size, the newest line flashes in.
    fn flush(&mut self, t: &Theme, rank: u8, audio: Option<&mut Audio>, now: f64) {
        if self.pending.is_empty() {
            return;
        }
        let mut groups: Vec<(StyleBonus, u32)> = Vec::new();
        let mut sound = false;
        for &(bonus, count) in &self.pending {
            sound |= notable(bonus);
            let n = if bonus == StyleBonus::Multikill {
                count.max(2) as u32
            } else {
                1
            };
            match groups.iter_mut().find(|g| g.0 == bonus) {
                Some(g) if bonus == StyleBonus::Multikill => g.1 = g.1.max(n),
                Some(g) => g.1 += 1,
                None => groups.push((bonus, n)),
            }
        }
        self.pending.clear();
        let accent = t.rank_color(rank as usize);
        for (bonus, n) in groups {
            let text = feed_text(bonus);
            let color = match bonus {
                StyleBonus::Stale => t.ash,
                StyleBonus::Kill | StyleBonus::Frag => t.bone,
                StyleBonus::ReturnToSender
                | StyleBonus::Multikill
                | StyleBonus::CloseCall
                | StyleBonus::Parry => mix(accent, t.bone, 0.25),
                _ => mix(t.bone, accent, 0.45),
            };
            if bonus == StyleBonus::Multikill
                && let Some(e) = self
                    .feed
                    .entries
                    .iter_mut()
                    .rev()
                    .find(|e| e.text == text && now - e.time < 1.0)
            {
                // The same burst keeps counting up on one line.
                e.count = e.count.max(n);
                e.time = now;
                continue;
            }
            self.feed.push(text, color, now);
            if let Some(e) = self.feed.entries.back_mut()
                && n > 1
            {
                e.count = e.count.max(n);
            }
        }
        if sound && let Some(audio) = audio {
            audio.play_2d(Sfx::StyleBonus);
        }
    }

    /// Music intensity this rank asks for (the game uses the max of this and its own).
    pub fn music_floor(&self) -> f32 {
        const FLOORS: [f32; 8] = [0.2, 0.3, 0.45, 0.6, 1.0, 1.0, 1.0, 1.0];
        FLOORS[(self.shown_rank as usize).min(7)]
    }

    /// Draws the style block. `weapon` / `mode`: the held weapon slot and alt mode (for the
    /// STALE indicator and freshness pips).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        list: &mut UiDrawList,
        t: &Theme,
        view: Option<&StyleView>,
        tuning: &StyleTuning,
        weapon: u8,
        mode: u8,
        audio: Option<&mut Audio>,
        now: f64,
        dt: f32,
    ) {
        let view = self.override_view.or(view.copied()).unwrap_or_default();
        if view.rank != self.shown_rank {
            // Missed / reordered event: follow the authoritative rank.
            self.shown_rank = view.rank;
            self.shown = view.fraction();
        }
        let target = view.fraction();
        let k = (dt * 12.0).clamp(0.0, 1.0);
        self.shown += (target - self.shown) * k;
        self.flush(t, view.rank, audio, now);

        let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
        let rank = view.rank;
        let color = t.rank_color(rank as usize);
        let outline = fade(t.ink, 0.9);

        // Rank change: shake (down) / white flash (up) for a moment.
        let (flash, up) = match self.rank_change {
            Some((at, up)) if now - at < RANK_FLASH => (1.0 - ((now - at) / RANK_FLASH) as f32, up),
            _ => (0.0, true),
        };
        let shake = if !up && flash > 0.0 {
            ((now * 70.0).sin() as f32 * 3.0 * flash).round()
        } else {
            0.0
        };
        let right = c.max.x - 12.0 + shake;
        let rect = Rect::new(
            vec2(right - BLOCK_W, BLOCK_TOP),
            vec2(right, BLOCK_TOP + 24.0),
        );

        // Backing slab with a rank-coloured edge.
        let slab = Rect::new(
            rect.min - vec2(10.0, 4.0),
            vec2(rect.max.x + 6.0, rect.max.y + 18.0),
        );
        draw::slanted(list, slab, true, fade(t.ink, 0.45));
        draw::fill(
            list,
            Rect::new(vec2(slab.max.x - 2.0, slab.min.y), slab.max),
            fade(color, 0.8),
        );
        if flash > 0.0 {
            let band = Rect::new(
                vec2(rect.min.x, rect.min.y - 2.0),
                vec2(rect.max.x, rect.min.y + 17.0),
            );
            let fc = if up { t.bone } else { t.blood_bright };
            draw::slanted(list, band, true, fade(fc, 0.55 * flash));
        }
        if rank >= TOP_RANK {
            // VELOCIDE: speed streaks streaming off the rank name.
            let name_left = rect.max.x - line_width(rank_name(rank), 2.0) - 6.0;
            for i in 0..5 {
                let phase = ((now * 2.5 + i as f64 * 0.37).fract()) as f32;
                let y = (rect.min.y + 1.0 + i as f32 * 3.5).round();
                let x1 = name_left - phase * 46.0;
                let len = 8.0 + (i % 3) as f32 * 7.0;
                let col = if i % 2 == 0 { t.bone } else { t.blood_bright };
                draw::hline(list, x1 - len, x1, y, fade(col, 0.8 * (1.0 - phase)));
            }
        }
        hud::style_meter(
            list,
            t,
            rect,
            rank as usize,
            rank_name(rank),
            self.shown,
            now,
        );

        // STALE indicator + freshness of the held weapon (primary, alt A, alt B).
        let keys = StyleKey::weapon_keys(weapon);
        let active = [keys[0], keys[1 + (mode as usize).min(1)]];
        let stale = active.iter().find(|k| view.is_stale(**k, tuning));
        let pip_y = rect.max.y + 3.0;
        for (i, key) in keys.iter().enumerate() {
            let f = view.freshness(*key, tuning);
            let selected = i == 0 || i == 1 + (mode as usize).min(1);
            let w = 12.0;
            let x = rect.min.x + i as f32 * (w + 3.0);
            let r = Rect::new(vec2(x, pip_y), vec2(x + w, pip_y + 3.0));
            draw::fill(list, r.expand(1.0), fade(t.ink, 0.8));
            let fill = ((f - tuning.freshness_min) / (1.0 - tuning.freshness_min).max(1e-3))
                .clamp(0.0, 1.0);
            let col = if f < tuning.stale_threshold {
                t.blood_bright
            } else {
                mix(t.bone_dim, t.bone, fill)
            };
            draw::fill(
                list,
                Rect::new(r.min, vec2(r.min.x + (w * fill).round().max(1.0), r.max.y)),
                fade(col, if selected { 1.0 } else { 0.45 }),
            );
        }
        if let Some(key) = stale {
            // Its own line under the pips (the rank pips sit on the right of the pip row).
            let blink = (now * 3.0).fract() < 0.65;
            let col = if blink { t.blood_bright } else { t.ash };
            let label = format!("STALE {}", key.label());
            list.text(
                vec2(rect.max.x, pip_y + 6.0),
                &label,
                &TextStyle::new(col).right().outline(outline),
            );
        }

        // The feed, newest on top, under the block.
        self.feed.update(now);
        self.feed.draw(
            list,
            t,
            vec2(right, rect.max.y + 22.0),
            Align::Right,
            1.0,
            now,
        );

        // Rank-up banner (its own slot above the game banner).
        if let Some((r, at)) = self.banner
            && !hud::banner(
                list,
                t,
                vec2(c.center().x, c.min.y + 62.0),
                rank_name(r),
                now - at,
                BANNER_TIME,
                t.rank_color(r as usize),
            )
        {
            self.banner = None;
        }
    }
}

/// Colour for a rank number in tables (scoreboard, results).
pub fn rank_text_color(t: &Theme, rank: u8, time: f64) -> Vec4 {
    let c = t.rank_color(rank as usize);
    if rank >= TOP_RANK && (time * 8.0).fract() < 0.5 {
        t.blood_bright
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_are_grouped_and_multikills_count_up() {
        let t = Theme::default();
        let mut h = StyleHud::new();
        for b in [
            StyleBonus::Kill,
            StyleBonus::Exploded,
            StyleBonus::Kill,
            StyleBonus::Exploded,
        ] {
            h.pending.push((b, 0));
        }
        h.pending.push((StyleBonus::Multikill, 2));
        h.pending.push((StyleBonus::Kill, 0));
        h.pending.push((StyleBonus::Multikill, 3));
        h.flush(&t, 0, None, 1.0);
        let lines: Vec<_> = h
            .feed
            .entries
            .iter()
            .map(|e| (e.text.as_str(), e.count))
            .collect();
        assert_eq!(
            lines,
            [("+ KILL", 3), ("+ EXPLODED", 2), ("+ MULTIKILL", 3)]
        );
        // A later multikill of the same burst updates the line.
        h.pending.push((StyleBonus::Kill, 0));
        h.pending.push((StyleBonus::Multikill, 4));
        h.flush(&t, 0, None, 1.3);
        let multi = h
            .feed
            .entries
            .iter()
            .find(|e| e.text == "+ MULTIKILL")
            .unwrap();
        assert_eq!(multi.count, 4);
        assert_eq!(feed_text(StyleBonus::Stale), "- STALE");
    }
}
