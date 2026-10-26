//! HUD drawing helpers. Plain functions that draw straight into a [`UiDrawList`]; they keep no
//! state except [`TextFeed`], which the game owns.

use std::collections::VecDeque;

use glam::{Vec2, Vec4};

use super::draw::{self, Rect, fade, fill, mix};
use super::font::{CAP_HEIGHT, GLYPH_ADVANCE, GLYPH_SPACING};
use super::text::{self, Align, TextStyle, draw_text, line_width};
use super::theme::Theme;
use crate::gfx::UiDrawList;

/// Colours of a [`segmented_bar`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarStyle {
    pub fill: Vec4,
    pub empty: Vec4,
    /// Capacity temporarily lost to hard damage (drawn hatched at the top end).
    pub hard: Vec4,
    /// Backing plate behind the segments.
    pub frame: Vec4,
    /// Gap between segments.
    pub gap: f32,
}

impl BarStyle {
    pub fn health(theme: &Theme) -> Self {
        Self {
            fill: theme.health,
            empty: mix(theme.ink, theme.ash, 0.35),
            hard: theme.hard_damage,
            frame: fade(theme.ink, 0.85),
            gap: 1.0,
        }
    }
}

/// A bar split into `segments` equal cells. `[0, value]` is filled, `[max - hard, max]` is
/// hard damage (capacity you can't heal into right now), the rest is empty.
pub fn segmented_bar(
    list: &mut UiDrawList,
    rect: Rect,
    value: f32,
    max: f32,
    hard: f32,
    segments: u32,
    style: &BarStyle,
) {
    let segments = segments.max(1);
    let max = max.max(1e-3);
    let value = value.clamp(0.0, max);
    let hard_start = (max - hard.clamp(0.0, max)).max(value);
    fill(list, rect.expand(1.0), style.frame);
    let seg_w = (rect.width() - style.gap * (segments - 1) as f32) / segments as f32;
    let per = max / segments as f32;
    for i in 0..segments {
        let x0 = (rect.min.x + i as f32 * (seg_w + style.gap)).round();
        let x1 = (rect.min.x + i as f32 * (seg_w + style.gap) + seg_w).round();
        let lo = i as f32 * per;
        let x_at = |v: f32| (x0 + ((v - lo) / per).clamp(0.0, 1.0) * (x1 - x0)).round();
        let seg = |a: f32, b: f32| Rect::new(Vec2::new(a, rect.min.y), Vec2::new(b, rect.max.y));
        let (xv, xh) = (x_at(value), x_at(hard_start));
        fill(list, seg(x0, xv), style.fill);
        fill(list, seg(xv, xh), style.empty);
        if xh < x1 {
            fill(list, seg(xh, x1), fade(style.hard, 0.55));
            draw::hatch(list, seg(xh, x1), 2.0, style.hard);
        }
        // Light top edge on filled cells for a bit of depth.
        if xv > x0 {
            draw::hline(list, x0, xv, rect.min.y, fade(Vec4::ONE, 0.35));
        }
    }
}

/// A row of `max_charges` pips (stamina / dash charges). `charges` may be fractional: the
/// next pip fills from the left while it regenerates.
pub fn pips(
    list: &mut UiDrawList,
    theme: &Theme,
    pos: Vec2,
    pip_size: Vec2,
    gap: f32,
    charges: f32,
    max_charges: u32,
) {
    let h = pip_size.y.round().max(1.0);
    let w = pip_size.x.round().max(1.0);
    for i in 0..max_charges {
        let x = (pos.x + i as f32 * (w + gap + h)).round();
        let f = (charges - i as f32).clamp(0.0, 1.0);
        // Slanted (parallelogram) pips: backing, then fill.
        draw::parallelogram(
            list,
            x - 1.0,
            pos.y - 1.0,
            w + 2.0,
            h + 2.0,
            fade(theme.ink, 0.85),
        );
        draw::parallelogram(list, x, pos.y, w, h, theme.stamina_empty);
        if f >= 1.0 {
            draw::parallelogram(list, x, pos.y, w, h, theme.stamina);
        } else if f > 0.0 {
            // Recharging: dimmer fill with a bright leading edge.
            let fw = (w * f).round().max(1.0);
            draw::parallelogram(
                list,
                x,
                pos.y,
                fw,
                h,
                mix(theme.stamina_empty, theme.stamina, 0.6),
            );
            draw::parallelogram(list, x + fw - 1.0, pos.y, 1.0, h, theme.bone);
        }
    }
}

/// Width taken by [`pips`] (for placing a label after them).
pub fn pips_width(pip_size: Vec2, gap: f32, max_charges: u32) -> f32 {
    let (w, h) = (pip_size.x.round().max(1.0), pip_size.y.round().max(1.0));
    max_charges as f32 * (w + gap + h) - gap
}

/// Thin progress meter with a label above it (style meter, charge meters).
#[allow(clippy::too_many_arguments)]
pub fn meter(
    list: &mut UiDrawList,
    theme: &Theme,
    rect: Rect,
    label: &str,
    label_scale: f32,
    fraction: f32,
    color: Vec4,
    align: Align,
) {
    let bar_h = 4.0;
    let bar = Rect::new(Vec2::new(rect.min.x, rect.max.y - bar_h), rect.max);
    let x = match align {
        Align::Left => rect.min.x,
        Align::Center => rect.center().x,
        Align::Right => rect.max.x,
    };
    let style = TextStyle::new(color)
        .scale(label_scale)
        .align(align)
        .outline(fade(theme.ink, 0.9));
    draw_text(list, Vec2::new(x, rect.min.y), label, &style);
    fill(list, bar.expand(1.0), fade(theme.ink, 0.85));
    fill(list, bar, fade(color, 0.18));
    let f = fraction.clamp(0.0, 1.0);
    let fx = (bar.min.x + bar.width() * f).round();
    fill(list, Rect::new(bar.min, Vec2::new(fx, bar.max.y)), color);
    if f > 0.0 {
        fill(
            list,
            Rect::new(
                Vec2::new(fx - 1.0, bar.min.y - 1.0),
                Vec2::new(fx + 1.0, bar.max.y + 1.0),
            ),
            theme.bone,
        );
    }
}

/// The player's health block: label, big number and segmented bar with hard damage.
/// Blinks red below 30%.
#[allow(clippy::too_many_arguments)]
pub fn health(
    list: &mut UiDrawList,
    theme: &Theme,
    rect: Rect,
    label: &str,
    health: f32,
    max: f32,
    hard: f32,
    time: f64,
) {
    let low = health < max * 0.3;
    let flash = low && (time * 4.0).fract() < 0.5;
    let num_color = if flash {
        theme.blood_bright
    } else {
        theme.bone
    };
    let num = format!("{}", health.max(0.0).ceil() as i32);
    let num_style = TextStyle::new(num_color)
        .scale(3.0)
        .outline(fade(theme.ink, 0.9));
    let num_y = rect.max.y - CAP_HEIGHT * 3.0;
    let num_rect = draw_text(list, Vec2::new(rect.min.x, num_y), &num, &num_style);
    let bar_x = (rect.min.x + line_width("000", 3.0) + 8.0).max(num_rect.max.x + 8.0);
    let bar = Rect::new(Vec2::new(bar_x, rect.max.y - 10.0), rect.max);
    let label_style = TextStyle::new(theme.bone_dim).outline(fade(theme.ink, 0.9));
    draw_text(
        list,
        Vec2::new(bar_x, bar.min.y - 12.0),
        label,
        &label_style,
    );
    // No "-100" under a dead player's empty bar.
    let hard_text = if hard >= 1.0 && health > 0.0 {
        format!("-{}", hard.round() as i32)
    } else {
        String::new()
    };
    let hard_style = TextStyle::new(theme.blood_bright)
        .right()
        .outline(fade(theme.ink, 0.9));
    draw_text(
        list,
        Vec2::new(bar.max.x, bar.min.y - 12.0),
        &hard_text,
        &hard_style,
    );
    let mut style = BarStyle::health(theme);
    if low {
        style.fill = if flash {
            theme.health_low
        } else {
            mix(theme.health_low, theme.bone, 0.3)
        };
    }
    let segments = (max / 10.0).round().max(1.0) as u32;
    segmented_bar(list, bar, health, max, hard, segments, &style);
}

/// Current weapon block anchored at its bottom-right corner: slot number, weapon name and the
/// fire modes with the active one highlighted.
pub fn weapon(
    list: &mut UiDrawList,
    theme: &Theme,
    bottom_right: Vec2,
    slot: u32,
    name: &str,
    modes: &[&str],
    active_mode: usize,
) {
    let outline = fade(theme.ink, 0.9);
    // Fire-mode chips along the bottom, right to left.
    let chip_h = 11.0;
    let mut x = bottom_right.x;
    let y = bottom_right.y - chip_h;
    for (i, mode) in modes.iter().enumerate().rev() {
        let w = line_width(mode, 1.0) + 8.0;
        let r = Rect::new(Vec2::new(x - w, y), Vec2::new(x, y + chip_h));
        if i == active_mode {
            fill(list, r.expand(1.0), fade(theme.ink, 0.85));
            fill(list, r, theme.blood);
            draw::hline(list, r.min.x, r.max.x, r.max.y - 1.0, theme.blood_bright);
            draw_text(
                list,
                Vec2::new(r.min.x + 4.0, r.min.y + 2.0),
                mode,
                &TextStyle::new(theme.bone),
            );
        } else {
            fill(list, r, fade(theme.ink, 0.85));
            draw_text(
                list,
                Vec2::new(r.min.x + 4.0, r.min.y + 2.0),
                mode,
                &TextStyle::new(theme.bone_dim),
            );
        }
        x -= w + 2.0;
    }
    let name_y = y - 4.0 - CAP_HEIGHT * 2.0;
    let style = TextStyle::new(theme.bone)
        .scale(2.0)
        .right()
        .outline(outline);
    let r = draw_text(list, Vec2::new(bottom_right.x, name_y), name, &style);
    // Slot number in a small box left of the name.
    let slot_text = slot.to_string();
    let bw = line_width(&slot_text, 1.0) + 6.0;
    let b = Rect::new(
        Vec2::new(r.min.x - bw - 5.0, name_y + 3.0),
        Vec2::new(r.min.x - 5.0, name_y + 14.0),
    );
    fill(list, b, theme.blood);
    draw_text(
        list,
        Vec2::new(b.min.x + 3.0, b.min.y + 2.0),
        &slot_text,
        &TextStyle::new(theme.bone),
    );
}

/// Style meter: rank name in its colour (top-ranked ones animate) above a draining meter.
/// `rank` indexes [`Theme::rank_colors`].
pub fn style_meter(
    list: &mut UiDrawList,
    theme: &Theme,
    rect: Rect,
    rank: usize,
    rank_name: &str,
    fraction: f32,
    time: f64,
) {
    let mut color = theme.rank_color(rank);
    let top = theme.rank_colors.len() - 1;
    if rank + 1 == top {
        // Second highest: slow throb.
        let k = (time * 6.0).sin() as f32 * 0.5 + 0.5;
        color = mix(color, theme.bone, k * 0.35);
    } else if rank >= top {
        // Highest: hard flicker between white-hot and red.
        color = if (time * 12.0).fract() < 0.5 {
            color
        } else {
            theme.blood_bright
        };
    }
    let scale = 2.0;
    let label_h = CAP_HEIGHT * scale + 5.0;
    let meter_rect = Rect::new(rect.min, Vec2::new(rect.max.x, rect.min.y + label_h + 4.0));
    meter(
        list,
        theme,
        meter_rect,
        rank_name,
        scale,
        fraction,
        color,
        Align::Right,
    );
    // Rank pips: one tick per rank, lit up to the current one.
    let n = theme.rank_colors.len();
    for i in 0..n {
        let x = rect.max.x - (n - i) as f32 * 4.0;
        let r = Rect::new(
            Vec2::new(x, meter_rect.max.y + 3.0),
            Vec2::new(x + 3.0, meter_rect.max.y + 5.0),
        );
        let c = if i <= rank {
            theme.rank_color(i)
        } else {
            fade(theme.ash, 0.5)
        };
        fill(list, r, c);
    }
}

/// One line of a [`TextFeed`].
#[derive(Clone, Debug, PartialEq)]
pub struct FeedEntry {
    pub text: String,
    pub color: Vec4,
    /// Time the entry was (last) pushed.
    pub time: f64,
    /// Repeat count when merged (drawn as `x2`, `x3`...).
    pub count: u32,
}

/// Scrolling feed of short lines that slide in and fade out (style events, kill feed).
#[derive(Clone, Debug, PartialEq)]
pub struct TextFeed {
    pub entries: VecDeque<FeedEntry>,
    pub max_entries: usize,
    /// Seconds an entry stays fully visible...
    pub lifetime: f64,
    /// ...plus this many seconds of fade-out.
    pub fade: f64,
    /// Pushing the same text as the newest entry bumps its count instead of adding a line.
    pub merge_repeats: bool,
    /// Draw the newest entry at the anchor and older ones below it (otherwise above it).
    pub newest_on_top: bool,
}

impl TextFeed {
    pub fn new(max_entries: usize, lifetime: f64) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries,
            lifetime,
            fade: 0.6,
            merge_repeats: true,
            newest_on_top: true,
        }
    }

    pub fn push(&mut self, text: impl Into<String>, color: Vec4, time: f64) {
        let text = text.into();
        if self.merge_repeats
            && let Some(last) = self.entries.back_mut()
            && last.text == text
            && time - last.time < self.lifetime
        {
            last.count += 1;
            last.time = time;
            last.color = color;
            return;
        }
        self.entries.push_back(FeedEntry {
            text,
            color,
            time,
            count: 1,
        });
        while self.entries.len() > self.max_entries {
            self.entries.pop_front();
        }
    }

    /// Drop entries that have fully faded.
    pub fn update(&mut self, time: f64) {
        let ttl = self.lifetime + self.fade;
        self.entries.retain(|e| time - e.time < ttl);
    }

    /// Draw with the anchor `pos` (x per `align`, y = top of the newest line when
    /// `newest_on_top`, else bottom of the newest line). Every line stays inside the canvas:
    /// the slide-in never pushes text past the screen edge and lines too long for the space
    /// beside the anchor are cut short with "..".
    pub fn draw(
        &self,
        list: &mut UiDrawList,
        theme: &Theme,
        pos: Vec2,
        align: Align,
        scale: f32,
        time: f64,
    ) {
        let line_h = (CAP_HEIGHT + 5.0) * scale;
        // Room for the outline at the screen edges.
        let edge = scale.max(1.0) + 1.0;
        let canvas_w = list.canvas_size.x;
        let max_w = match align {
            Align::Right => pos.x - edge,
            Align::Left => canvas_w - edge - pos.x,
            Align::Center => canvas_w - 2.0 * edge,
        };
        // How far the slide-in may move a line outwards before it leaves the screen.
        let slide_room = match align {
            Align::Right => (canvas_w - edge - pos.x).max(0.0),
            _ => (pos.x - edge).max(0.0),
        };
        for (i, e) in self.entries.iter().rev().enumerate() {
            let age = time - e.time;
            let alpha = if age < self.lifetime {
                1.0
            } else {
                (1.0 - (age - self.lifetime) / self.fade.max(1e-3)).clamp(0.0, 1.0) as f32
            };
            if alpha <= 0.0 {
                continue;
            }
            // Slide in from the outer side for the first 0.12 s, flash white for 0.06 s.
            let slide = ((1.0 - age / 0.12).clamp(0.0, 1.0) as f32 * 14.0)
                .min(slide_room)
                .round();
            let dx = match align {
                Align::Right => slide,
                _ => -slide,
            };
            let y = if self.newest_on_top {
                pos.y + i as f32 * line_h
            } else {
                pos.y - (i + 1) as f32 * line_h
            };
            let color = if age < 0.06 { theme.bone } else { e.color };
            let suffix = if e.count > 1 {
                format!(" x{}", e.count)
            } else {
                String::new()
            };
            let text = fit_line(&e.text, &suffix, max_w, scale);
            let style = TextStyle::new(fade(color, alpha))
                .scale(scale)
                .align(align)
                .outline(fade(theme.ink, 0.85 * alpha));
            draw_text(list, Vec2::new(pos.x + dx, y), &text, &style);
        }
    }
}

/// `text` + `suffix`, with `text` cut short ("..") so the line is at most `max_width` wide
/// (the suffix, e.g. a repeat count, is always kept).
pub fn fit_line(text: &str, suffix: &str, max_width: f32, scale: f32) -> String {
    let full = format!("{text}{suffix}");
    if line_width(&full, scale) <= max_width {
        return full;
    }
    let advance = GLYPH_ADVANCE * scale.max(1e-3);
    let fit = ((max_width + GLYPH_SPACING * scale) / advance).floor().max(0.0) as usize;
    let keep = fit.saturating_sub(suffix.chars().count() + 2);
    let head: String = text.chars().take(keep).collect();
    format!("{}..{suffix}", head.trim_end())
}

/// Big centred announcement (rank-up, "ROUND START") with a pop-in scale pulse, a dark band
/// behind it and a fade-out. Returns false once `age >= duration` (nothing drawn).
pub fn banner(
    list: &mut UiDrawList,
    theme: &Theme,
    center: Vec2,
    text: &str,
    age: f64,
    duration: f64,
    color: Vec4,
) -> bool {
    if age >= duration || age < 0.0 {
        return false;
    }
    let base = 4.0;
    let pop = (1.0 - age / 0.22).clamp(0.0, 1.0) as f32;
    // Integer scales only, so the pulse stays pixel-crisp: 8 -> 4.
    let scale = (base + 4.0 * pop * pop).round();
    let alpha = (((duration - age) / 0.4).clamp(0.0, 1.0)) as f32;
    // A slanted band sized to the text that snaps open sideways during the first 0.1 s.
    let band_h = (CAP_HEIGHT * base + 12.0).round();
    let open = 0.3 + 0.7 * ((age / 0.1).clamp(0.0, 1.0)) as f32;
    let band_w = ((line_width(text, base) + 48.0) * open).round();
    let x = (center.x - (band_w + band_h) * 0.5).round();
    let y = (center.y - band_h * 0.5).round();
    draw::parallelogram(list, x, y, band_w, band_h, fade(theme.ink, 0.6 * alpha));
    let edge = fade(color, 0.85 * alpha);
    draw::hline(list, x + band_h - 1.0, x + band_h - 1.0 + band_w, y, edge);
    draw::hline(list, x, x + band_w, y + band_h - 1.0, edge);
    // Speed ticks trailing off both ends.
    for i in 0..3 {
        let len = 10.0 - i as f32 * 3.0;
        let ty = y + 6.0 + i as f32 * 4.0;
        let left = x + band_h - 8.0 - i as f32 * 5.0 - 6.0;
        draw::hline(list, left - len, left, ty, fade(edge, 0.7));
        let right = x + band_w + 6.0 + i as f32 * 5.0;
        draw::hline(
            list,
            right,
            right + len,
            y + band_h - 7.0 - i as f32 * 4.0,
            fade(edge, 0.7),
        );
    }
    let c = if age < 0.05 { theme.bone } else { color };
    let style = TextStyle::new(fade(c, alpha))
        .scale(scale)
        .centered()
        .outline(fade(theme.ink, alpha));
    let y = (center.y - CAP_HEIGHT * scale * 0.5).round();
    draw_text(list, Vec2::new(center.x, y), text, &style);
    true
}

/// Crosshair look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairStyle {
    pub color: Vec4,
    /// Dark outline around every arm (readability on bright backgrounds).
    pub outline: Option<Vec4>,
    /// Arm length.
    pub size: f32,
    /// Distance from the centre to the start of each arm.
    pub gap: f32,
    pub thickness: f32,
    /// Centre dot.
    pub dot: bool,
}

impl CrosshairStyle {
    pub fn new(theme: &Theme) -> Self {
        Self {
            color: theme.bone,
            outline: Some(fade(theme.ink, 0.8)),
            size: 4.0,
            gap: 3.0,
            thickness: 1.0,
            dot: true,
        }
    }
}

/// Four-arm crosshair centred on `center` (snapped to whole units).
pub fn crosshair(list: &mut UiDrawList, center: Vec2, style: &CrosshairStyle) {
    let c = center.floor();
    let t = style.thickness;
    let half = (t * 0.5).floor();
    let (g, s) = (style.gap, style.size);
    let arms = [
        Rect::new(
            Vec2::new(c.x - g - s, c.y - half),
            Vec2::new(c.x - g, c.y - half + t),
        ),
        Rect::new(
            Vec2::new(c.x + g + t, c.y - half),
            Vec2::new(c.x + g + t + s, c.y - half + t),
        ),
        Rect::new(
            Vec2::new(c.x - half, c.y - g - s),
            Vec2::new(c.x - half + t, c.y - g),
        ),
        Rect::new(
            Vec2::new(c.x - half, c.y + g + t),
            Vec2::new(c.x - half + t, c.y + g + t + s),
        ),
    ];
    let dot = Rect::new(
        Vec2::new(c.x - half, c.y - half),
        Vec2::new(c.x - half + t, c.y - half + t),
    );
    if let Some(o) = style.outline {
        for r in arms {
            fill(list, r.expand(1.0), o);
        }
        if style.dot {
            fill(list, dot.expand(1.0), o);
        }
    }
    for r in arms {
        fill(list, r, style.color);
    }
    if style.dot {
        fill(list, dot, style.color);
    }
}

/// A chat message (or a system line when `name` is empty).
#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    pub name: String,
    pub name_color: Vec4,
    pub text: String,
    pub time: f64,
}

/// Line height used by chat text.
pub const CHAT_LINE_HEIGHT: f32 = 10.0;

/// Wrapped lines of a chat message (`name: text`, or just `text` for system lines).
pub fn chat_wrap(name: &str, text: &str, width: f32) -> Vec<String> {
    let full = if name.is_empty() {
        text.to_string()
    } else {
        format!("{name}: {text}")
    };
    text::wrap_text(&full, width, 1.0)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Draw one chat message at `pos` wrapped to `width`: the `name:` prefix in `name_color`, the
/// message in `body_color`, with a dark outline. Returns the height used.
#[allow(clippy::too_many_arguments)]
pub fn draw_chat_line(
    list: &mut UiDrawList,
    theme: &Theme,
    pos: Vec2,
    width: f32,
    name: &str,
    name_color: Vec4,
    text: &str,
    body_color: Vec4,
    alpha: f32,
) -> f32 {
    let wrapped = chat_wrap(name, text, width);
    let outline = fade(theme.ink, 0.8 * alpha);
    let mut name_left = if name.is_empty() {
        0
    } else {
        name.chars().count() + 1
    };
    for (i, l) in wrapped.iter().enumerate() {
        let y = pos.y + i as f32 * CHAT_LINE_HEIGHT;
        let n = name_left.min(l.chars().count());
        let split = l.char_indices().nth(n).map_or(l.len(), |(b, _)| b);
        let (head, tail) = l.split_at(split);
        if !head.is_empty() {
            let st = TextStyle::new(fade(name_color, alpha)).outline(outline);
            draw_text(list, Vec2::new(pos.x, y), head, &st);
        }
        let st = TextStyle::new(fade(body_color, alpha)).outline(outline);
        draw_text(
            list,
            Vec2::new(pos.x + n as f32 * GLYPH_ADVANCE, y),
            tail,
            &st,
        );
        name_left -= n;
    }
    wrapped.len() as f32 * CHAT_LINE_HEIGHT
}

/// Chat log anchored to the bottom of `rect`, newest line at the bottom, wrapped to the
/// width. When `open` is false, lines fade out `fade_after` seconds after arriving and no
/// background is drawn; when open, all lines that fit are shown on a dark backing.
/// System lines (empty `name`) are drawn entirely in their `name_color`.
pub fn chat_box(
    list: &mut UiDrawList,
    theme: &Theme,
    rect: Rect,
    lines: &[ChatLine],
    time: f64,
    open: bool,
    fade_after: f64,
) {
    if open {
        fill(list, rect, fade(theme.ink, 0.6));
        draw::hline(list, rect.min.x, rect.max.x, rect.min.y, theme.blood);
    }
    let inner = rect.shrink2(Vec2::new(3.0, 2.0));
    let mut y = inner.max.y;
    for line in lines.iter().rev() {
        let alpha = if open {
            1.0
        } else {
            (1.0 - (time - line.time - fade_after)).clamp(0.0, 1.0) as f32
        };
        let block_h =
            chat_wrap(&line.name, &line.text, inner.width()).len() as f32 * CHAT_LINE_HEIGHT;
        if y - block_h < inner.min.y {
            break;
        }
        y -= block_h;
        if alpha > 0.0 {
            let body = if line.name.is_empty() {
                line.name_color
            } else {
                theme.bone
            };
            draw_chat_line(
                list,
                theme,
                Vec2::new(inner.min.x, y),
                inner.width(),
                &line.name,
                line.name_color,
                &line.text,
                body,
                alpha,
            );
        }
    }
}

/// Small "LABEL value" readout (ping, timer, score) with an outline for readability.
pub fn readout(
    list: &mut UiDrawList,
    theme: &Theme,
    pos: Vec2,
    align: Align,
    label: &str,
    value: &str,
    value_color: Vec4,
) -> Rect {
    let outline = fade(theme.ink, 0.85);
    let label_w = if label.is_empty() {
        0.0
    } else {
        line_width(label, 1.0) + 5.0
    };
    let value_w = line_width(value, 1.0);
    let total = label_w + value_w;
    let x0 = match align {
        Align::Left => pos.x,
        Align::Center => pos.x - total * 0.5,
        Align::Right => pos.x - total,
    }
    .round();
    draw_text(
        list,
        Vec2::new(x0, pos.y),
        label,
        &TextStyle::new(theme.bone_dim).outline(outline),
    );
    draw_text(
        list,
        Vec2::new(x0 + label_w, pos.y),
        value,
        &TextStyle::new(value_color).outline(outline),
    );
    Rect::from_min_size(Vec2::new(x0, pos.y), Vec2::new(total, CAP_HEIGHT))
}

/// Ping colour: bone when good, brass when noticeable, red when bad.
pub fn ping_color(theme: &Theme, ping_ms: f32) -> Vec4 {
    if ping_ms < 80.0 {
        theme.bone
    } else if ping_ms < 150.0 {
        theme.rank_color(1)
    } else {
        theme.blood_bright
    }
}

/// `MM:SS.cc` level timer text (hours roll into minutes).
pub fn format_timer(seconds: f64) -> String {
    let s = seconds.max(0.0);
    let total_cs = (s * 100.0).floor() as u64;
    let (m, rem) = (total_cs / 6000, total_cs % 6000);
    format!("{:02}:{:02}.{:02}", m, rem / 100, rem % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_format() {
        assert_eq!(format_timer(0.0), "00:00.00");
        assert_eq!(format_timer(83.456), "01:23.45");
        assert_eq!(format_timer(3600.0), "60:00.00");
    }

    #[test]
    fn feed_merges_and_expires() {
        let mut feed = TextFeed::new(3, 2.0);
        feed.push("+ PARRY", Vec4::ONE, 0.0);
        feed.push("+ PARRY", Vec4::ONE, 0.5);
        feed.push("+ AIRSHOT", Vec4::ONE, 0.6);
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].count, 2);
        for i in 0..5 {
            feed.push(format!("{i}"), Vec4::ONE, 1.0);
        }
        assert_eq!(feed.entries.len(), 3);
        feed.update(10.0);
        assert!(feed.entries.is_empty());
    }

    #[test]
    fn feed_lines_stay_on_screen() {
        let theme = Theme::default();
        let canvas = Vec2::new(640.0, 360.0);
        let mut feed = TextFeed::new(6, 4.0);
        feed.push("YOU [TUMBLER] HUSK", Vec4::ONE, 0.0);
        for k in 1..5 {
            // Merged repeats re-trigger the slide-in.
            feed.push("YOU [TUMBLER] HUSK", Vec4::ONE, k as f64 * 0.01);
        }
        feed.push("X".repeat(200), Vec4::ONE, 0.05);
        // Every moment of the slide-in, right-aligned 10 units from the edge (the kill feed).
        for step in 0..20 {
            let time = 0.05 + step as f64 * 0.01;
            let mut list = UiDrawList::new(canvas);
            feed.draw(
                &mut list,
                &theme,
                Vec2::new(630.0, 30.0),
                Align::Right,
                1.0,
                time,
            );
            assert!(!list.quads.is_empty());
            for q in &list.quads {
                assert!(
                    q.min.x >= 0.0 && q.max.x <= canvas.x,
                    "quad {:?}..{:?} off screen at t={time}",
                    q.min,
                    q.max
                );
            }
        }
        assert_eq!(fit_line("SHORT", " x2", 300.0, 1.0), "SHORT x2");
        let cut = fit_line(&"A".repeat(100), " x5", 120.0, 1.0);
        assert!(cut.ends_with(".. x5"), "{cut}");
        assert!(line_width(&cut, 1.0) <= 120.0, "{cut}");
    }

    #[test]
    fn health_hides_hard_damage_when_empty() {
        let theme = Theme::default();
        let rect = Rect::new(Vec2::new(14.0, 312.0), Vec2::new(200.0, 338.0));
        let red_glyphs = |health: f32| {
            let mut list = UiDrawList::new(Vec2::new(640.0, 360.0));
            // 30 hard damage; at t = 0.2 a low-health number isn't in its red blink phase.
            super::health(&mut list, &theme, rect, "BLOOD", health, 100.0, 30.0, 0.2);
            list.quads
                .iter()
                .filter(|q| q.color == theme.blood_bright && q.texture == crate::gfx::UiTexture::Font)
                .count()
        };
        assert!(red_glyphs(50.0) > 0, "the -30 hard damage counter shows");
        assert_eq!(red_glyphs(0.0), 0, "no '-100' under an empty bar");
    }

    #[test]
    fn bar_draws_hard_segment() {
        let theme = Theme::default();
        let mut list = UiDrawList::new(Vec2::new(640.0, 360.0));
        let rect = Rect::from_min_size(Vec2::new(10.0, 10.0), Vec2::new(99.0, 8.0));
        segmented_bar(
            &mut list,
            rect,
            50.0,
            100.0,
            30.0,
            10,
            &BarStyle::health(&theme),
        );
        let style = BarStyle::health(&theme);
        assert!(list.quads.iter().any(|q| q.color == style.hard));
        // Filled part ends at the middle segment boundary.
        let filled_max = list
            .quads
            .iter()
            .filter(|q| q.color == style.fill)
            .map(|q| q.max.x)
            .fold(0.0, f32::max);
        assert!((filled_max - 59.0).abs() <= 1.0, "{filled_max}");
    }

    #[test]
    fn banner_expires() {
        let theme = Theme::default();
        let mut list = UiDrawList::new(Vec2::new(640.0, 360.0));
        assert!(banner(
            &mut list,
            &theme,
            Vec2::new(320.0, 100.0),
            "MERCILESS",
            0.1,
            1.5,
            Vec4::ONE
        ));
        let n = list.quads.len();
        assert!(!banner(
            &mut list,
            &theme,
            Vec2::new(320.0, 100.0),
            "MERCILESS",
            2.0,
            1.5,
            Vec4::ONE
        ));
        assert_eq!(list.quads.len(), n);
    }
}
