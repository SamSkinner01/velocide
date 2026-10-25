//! Text measuring, wrapping and drawing with the built-in bitmap font.
//!
//! All sizes are canvas units. At `scale` 1 a glyph is [`GLYPH_W`] x [`GLYPH_H`] units; use
//! whole-number scales for crisp pixels. Widths are *ink* widths: the empty spacing column
//! after the last character is not counted, so centring and right alignment are exact.

use glam::{Vec2, Vec4};

use super::draw::Rect;
use super::font::{self, GLYPH_ADVANCE, GLYPH_H, GLYPH_SPACING, GLYPH_W};
use crate::gfx::{UiDrawList, UiQuad, UiTexture};

/// Horizontal alignment relative to the x coordinate passed to the draw functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// `pos.x` is the left edge.
    #[default]
    Left,
    /// `pos.x` is the centre of every line.
    Center,
    /// `pos.x` is the right edge.
    Right,
}

/// How to draw a piece of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// Size multiplier (1 = 6 x 9 units per glyph). Whole numbers keep pixels crisp.
    pub scale: f32,
    pub color: Vec4,
    pub align: Align,
    /// Drop shadow colour, drawn `shadow_offset` units right and down.
    pub shadow: Option<Vec4>,
    /// Shadow offset in canvas units (default 1 unit; use `scale` for a chunky one-font-pixel
    /// extrusion on big titles).
    pub shadow_offset: f32,
    /// Outline colour, one font pixel thick in all 8 directions (HUD readability).
    pub outline: Option<Vec4>,
    /// Extra space between lines, in font pixels (multiplied by `scale`).
    pub line_gap: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self::new(Vec4::ONE)
    }
}

impl TextStyle {
    pub const fn new(color: Vec4) -> Self {
        Self {
            scale: 1.0,
            color,
            align: Align::Left,
            shadow: None,
            shadow_offset: 1.0,
            outline: None,
            line_gap: 2.0,
        }
    }

    pub const fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub const fn color(mut self, color: Vec4) -> Self {
        self.color = color;
        self
    }

    pub const fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub const fn centered(self) -> Self {
        self.align(Align::Center)
    }

    pub const fn right(self) -> Self {
        self.align(Align::Right)
    }

    pub const fn shadow(mut self, color: Vec4) -> Self {
        self.shadow = Some(color);
        self
    }

    /// Shadow with a custom offset (canvas units).
    pub const fn shadow_by(mut self, color: Vec4, offset: f32) -> Self {
        self.shadow = Some(color);
        self.shadow_offset = offset;
        self
    }

    pub const fn outline(mut self, color: Vec4) -> Self {
        self.outline = Some(color);
        self
    }

    pub const fn line_gap(mut self, gap: f32) -> Self {
        self.line_gap = gap;
        self
    }

    /// Distance between the tops of consecutive lines.
    pub fn line_height(&self) -> f32 {
        (GLYPH_H + self.line_gap) * self.scale
    }
}

/// Ink width of a single line (no newlines expected; `\n` is treated like any character).
pub fn line_width(line: &str, scale: f32) -> f32 {
    let n = line.chars().filter(|&c| c != '\r').count();
    if n == 0 {
        0.0
    } else {
        (n as f32 * GLYPH_ADVANCE - GLYPH_SPACING) * scale
    }
}

/// Width of the widest line of `text`.
pub fn text_width(text: &str, scale: f32) -> f32 {
    text.split('\n')
        .map(|l| line_width(l, scale))
        .fold(0.0, f32::max)
}

/// Size of a (multi-line) text block drawn with `style`.
pub fn text_size(text: &str, style: &TextStyle) -> Vec2 {
    let lines = text.split('\n').count() as f32;
    Vec2::new(
        text_width(text, style.scale),
        lines * GLYPH_H * style.scale + (lines - 1.0) * style.line_gap * style.scale,
    )
}

/// How many characters fit on one line of `max_width` units (at least 1).
pub fn max_chars(max_width: f32, scale: f32) -> usize {
    let per = GLYPH_ADVANCE * scale.max(0.01);
    (((max_width + GLYPH_SPACING * scale) / per).floor() as usize).max(1)
}

/// Greedy word wrap. Splits on `\n`, breaks lines at spaces and hard-breaks words that are
/// longer than a line. Returns slices of `text` (trailing spaces at break points dropped).
pub fn wrap_text(text: &str, max_width: f32, scale: f32) -> Vec<&str> {
    let max = max_chars(max_width, scale);
    let mut out = Vec::new();
    for para in text.split('\n') {
        wrap_paragraph(para, max, &mut out);
    }
    out
}

fn wrap_paragraph<'a>(para: &'a str, max: usize, out: &mut Vec<&'a str>) {
    if para.chars().count() <= max {
        out.push(para);
        return;
    }
    let before = out.len();
    // (byte start of the current line, byte end of its last word, chars in the line)
    let mut line: Option<(usize, usize, usize)> = None;
    for (word_start, word) in words(para) {
        let word_len = word.chars().count();
        if let Some((start, end, len)) = line {
            let gap = para[end..word_start].chars().count();
            if len + gap + word_len <= max {
                line = Some((start, word_start + word.len(), len + gap + word_len));
                continue;
            }
            out.push(&para[start..end]);
        }
        // Start a new line with this word, hard-breaking it if it is too long.
        let mut rest = word;
        let mut rest_start = word_start;
        while rest.chars().count() > max {
            let split = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
            out.push(&rest[..split]);
            rest = &rest[split..];
            rest_start += split;
        }
        line = Some((rest_start, rest_start + rest.len(), rest.chars().count()));
    }
    if let Some((start, end, _)) = line {
        out.push(&para[start..end]);
    }
    if out.len() == before {
        out.push("");
    }
}

/// Maximal runs of non-space characters with their byte offsets.
fn words(s: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    s.split(' ').filter_map(move |w| {
        let start = offset;
        offset += w.len() + 1;
        (!w.is_empty()).then_some((start, w))
    })
}

/// Cut `text` so it fits in `max_width`, ending with `..` if anything was removed.
pub fn truncate(text: &str, max_width: f32, scale: f32) -> String {
    let max = max_chars(max_width, scale);
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(2);
    let mut s: String = text.chars().take(keep).collect();
    s.push_str(&".."[..max.min(2)]);
    s
}

/// Push one glyph quad.
pub fn draw_glyph(list: &mut UiDrawList, pos: Vec2, c: char, scale: f32, color: Vec4) {
    let (uv_min, uv_max) = font::glyph_uv(c);
    list.quads.push(UiQuad {
        min: pos,
        max: pos + Vec2::new(GLYPH_W, GLYPH_H) * scale,
        uv_min,
        uv_max,
        color,
        texture: UiTexture::Font,
    });
}

/// Draw `text` (may contain `\n`) with its first line's top at `pos.y`; `pos.x` is the left
/// edge, centre or right edge depending on `style.align`. Returns the bounding rectangle.
pub fn draw_text(list: &mut UiDrawList, pos: Vec2, text: &str, style: &TextStyle) -> Rect {
    draw_lines(list, pos, text.split('\n'), style)
}

/// Like [`draw_text`] but word-wrapped to `max_width`.
pub fn draw_text_wrapped(
    list: &mut UiDrawList,
    pos: Vec2,
    text: &str,
    max_width: f32,
    style: &TextStyle,
) -> Rect {
    draw_lines(
        list,
        pos,
        wrap_text(text, max_width, style.scale).into_iter(),
        style,
    )
}

/// Draw pre-split lines. Shadows and outlines of all glyphs are emitted before the glyphs
/// themselves so neighbouring characters never cover each other.
pub fn draw_lines<'a>(
    list: &mut UiDrawList,
    pos: Vec2,
    lines: impl Iterator<Item = &'a str>,
    style: &TextStyle,
) -> Rect {
    let s = style.scale;
    let mut glyphs: Vec<(Vec2, char)> = Vec::new();
    let mut bounds: Option<Rect> = None;
    let mut y = pos.y;
    for line in lines {
        let w = line_width(line, s);
        let x = match style.align {
            Align::Left => pos.x,
            Align::Center => pos.x - w * 0.5,
            Align::Right => pos.x - w,
        }
        .round();
        let line_rect = Rect::from_min_size(Vec2::new(x, y.round()), Vec2::new(w, GLYPH_H * s));
        bounds = Some(match bounds {
            Some(b) => Rect::new(b.min.min(line_rect.min), b.max.max(line_rect.max)),
            None => line_rect,
        });
        let mut gx = x;
        for c in line.chars() {
            match c {
                '\r' => continue,
                ' ' | '\t' => {}
                c => glyphs.push((Vec2::new(gx, y.round()), c)),
            }
            gx += GLYPH_ADVANCE * s;
        }
        y += style.line_height();
    }
    let px = s.max(1.0);
    if let Some(shadow) = style.shadow {
        for &(p, c) in &glyphs {
            draw_glyph(list, p + Vec2::splat(style.shadow_offset), c, s, shadow);
        }
    }
    if let Some(outline) = style.outline {
        const DIRS: [(f32, f32); 8] = [
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 0.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (0.0, 1.0),
            (1.0, 1.0),
        ];
        for &(p, c) in &glyphs {
            for (dx, dy) in DIRS {
                draw_glyph(list, p + Vec2::new(dx, dy) * px, c, s, outline);
            }
        }
    }
    for &(p, c) in &glyphs {
        draw_glyph(list, p, c, s, style.color);
    }
    bounds.unwrap_or(Rect::from_min_size(pos, Vec2::ZERO))
}

/// Convenience drawing methods on [`UiDrawList`].
pub trait UiDrawExt {
    /// See [`draw_text`].
    fn text(&mut self, pos: Vec2, text: &str, style: &TextStyle) -> Rect;
    /// See [`draw_text_wrapped`].
    fn text_wrapped(&mut self, pos: Vec2, text: &str, max_width: f32, style: &TextStyle) -> Rect;
    /// Text vertically centred in `rect`, aligned horizontally inside it per `style.align`.
    fn text_in(&mut self, rect: Rect, text: &str, style: &TextStyle) -> Rect;
    /// Solid rectangle.
    fn fill(&mut self, rect: Rect, color: Vec4);
    /// Rectangle outline.
    fn border(&mut self, rect: Rect, thickness: f32, color: Vec4);
}

impl UiDrawExt for UiDrawList {
    fn text(&mut self, pos: Vec2, text: &str, style: &TextStyle) -> Rect {
        draw_text(self, pos, text, style)
    }

    fn text_wrapped(&mut self, pos: Vec2, text: &str, max_width: f32, style: &TextStyle) -> Rect {
        draw_text_wrapped(self, pos, text, max_width, style)
    }

    fn text_in(&mut self, rect: Rect, text: &str, style: &TextStyle) -> Rect {
        let size = text_size(text, style);
        let x = match style.align {
            Align::Left => rect.min.x,
            Align::Center => rect.center().x,
            Align::Right => rect.max.x,
        };
        draw_text(
            self,
            Vec2::new(x, text_top_in(rect, text, style.scale, size.y)),
            text,
            style,
        )
    }

    fn fill(&mut self, rect: Rect, color: Vec4) {
        super::draw::fill(self, rect, color);
    }

    fn border(&mut self, rect: Rect, thickness: f32, color: Vec4) {
        super::draw::border(self, rect, thickness, color);
    }
}

/// Top y that vertically centres text in `rect`. Single lines are centred on their cap height
/// (descenders hang below), which looks right for buttons and rows.
pub fn text_top_in(rect: Rect, text: &str, scale: f32, block_height: f32) -> f32 {
    let h = if text.contains('\n') {
        block_height
    } else {
        font::CAP_HEIGHT * scale
    };
    (rect.min.y + (rect.height() - h) * 0.5).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(line_width("", 1.0), 0.0);
        assert_eq!(line_width("A", 1.0), 5.0);
        assert_eq!(line_width("ABC", 1.0), 17.0);
        assert_eq!(line_width("ABC", 2.0), 34.0);
        assert_eq!(text_width("AB\nABCD\nA", 1.0), 23.0);
        let size = text_size("AB\nCD", &TextStyle::default());
        assert_eq!(size, Vec2::new(11.0, 9.0 * 2.0 + 2.0));
        assert_eq!(max_chars(17.0, 1.0), 3);
        assert_eq!(max_chars(22.9, 1.0), 3);
        assert_eq!(max_chars(23.0, 1.0), 4);
    }

    #[test]
    fn wrapping() {
        // 3 chars = 17 units, so 23 units fits 4 chars.
        assert_eq!(wrap_text("aa bb cc", 23.0, 1.0), vec!["aa", "bb", "cc"]);
        // 29 units fits 5 chars.
        assert_eq!(wrap_text("aa bb cc", 29.0, 1.0), vec!["aa bb", "cc"]);
        assert_eq!(wrap_text("aa b cc", 29.0, 1.0), vec!["aa b", "cc"]);
        assert_eq!(
            wrap_text("abcdefghij", 23.0, 1.0),
            vec!["abcd", "efgh", "ij"]
        );
        assert_eq!(
            wrap_text("hi\n\nyo there", 23.0, 1.0),
            vec!["hi", "", "yo", "ther", "e"]
        );
        assert_eq!(wrap_text("short", 1000.0, 1.0), vec!["short"]);
        assert_eq!(wrap_text("one  two", 23.0, 1.0), vec!["one", "two"]);
        // Scale halves the characters per line.
        assert_eq!(wrap_text("aa bb", 23.0, 2.0), vec!["aa", "bb"]);
        // Multi-byte characters are counted as one glyph each.
        assert_eq!(
            wrap_text("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}", 23.0, 1.0).len(),
            2
        );
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("hello", 100.0, 1.0), "hello");
        assert_eq!(truncate("hello world", 29.0, 1.0), "hel..");
    }

    #[test]
    fn draw_alignment_and_effects() {
        let mut list = UiDrawList::new(Vec2::new(640.0, 360.0));
        let r = draw_text(
            &mut list,
            Vec2::new(100.0, 10.0),
            "AB C",
            &TextStyle::default().right(),
        );
        assert_eq!(r.max.x, 100.0);
        assert_eq!(list.quads.len(), 3, "spaces emit no quads");
        list.clear();
        let style = TextStyle::default()
            .centered()
            .shadow(Vec4::ZERO)
            .outline(Vec4::ZERO);
        let r = draw_text(&mut list, Vec2::new(100.0, 10.0), "AB", &style);
        assert_eq!(list.quads.len(), 2 * (1 + 8 + 1));
        assert!((r.center().x - 100.0).abs() <= 0.5);
        // Fill quads come last, in the requested colour.
        assert_eq!(list.quads.last().map(|q| q.color), Some(Vec4::ONE));
    }
}
