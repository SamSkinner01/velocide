//! Built-in bitmap font: "SHRIKE 5x7", an original pixel font made for VELOCIDE.
//!
//! CONTRACT (used by the renderer): `build_atlas()` returns the font atlas texture and
//! `glyph_uv(c)` returns the (uv_min, uv_max) of a character inside it. Glyphs are
//! `GLYPH_W` x `GLYPH_H` canvas units at scale 1, advancing `GLYPH_ADVANCE` units.
//! Sample the atlas with nearest filtering.
//!
//! Design: 5 px wide ink in a 6 px cell (the right column is always empty spacing),
//! cap height 7 px, lowercase x-height 5 px, 2 px descenders, so a glyph cell is 6 x 9.
//! Round letters have their top-left and bottom-right corners notched, which gives the
//! whole font a slight forward lean ("speed") while staying square and readable.
//! Every printable ASCII character (32..=126) is covered; anything else renders as a
//! hatched box.
//!
//! Glyphs are defined below as "sheets": a string of characters plus 9 rows of art where
//! glyph `i` is the `i`-th space-separated 5-character chunk of every row (`#` = ink).

use std::sync::OnceLock;

use glam::Vec2;

use crate::gfx::TextureData;

/// Width of one glyph quad in canvas units at scale 1 (5 px ink + 1 px spacing).
pub const GLYPH_W: f32 = 6.0;
/// Height of one glyph quad in canvas units at scale 1 (7 px caps + 2 px descenders).
pub const GLYPH_H: f32 = 9.0;
/// Horizontal advance per character (the font is monospaced).
pub const GLYPH_ADVANCE: f32 = 6.0;
/// Height of capital letters and digits; rows below this are descenders.
pub const CAP_HEIGHT: f32 = 7.0;
/// Empty spacing column at the right of every glyph cell (excluded from measured widths).
pub const GLYPH_SPACING: f32 = 1.0;

/// Ink pixels per glyph row.
const INK_W: usize = 5;
/// Pixel size of a glyph cell (ink + spacing column) in the atlas.
const CELL_W: u32 = 6;
const CELL_H: u32 = 9;
/// Atlas slots are padded by one transparent pixel on every side so nearest sampling at
/// non-integer scales never bleeds a neighbour's pixels into a glyph.
const SLOT_W: u32 = CELL_W + 2;
const SLOT_H: u32 = CELL_H + 2;
const COLS: u32 = 16;
const ROWS: u32 = 6;
/// Atlas size in pixels.
pub const ATLAS_W: u32 = COLS * SLOT_W;
pub const ATLAS_H: u32 = ROWS * SLOT_H;

/// Number of glyph slots: ASCII 32..=126 plus the "missing glyph" box in slot 95.
const GLYPH_COUNT: usize = 96;
const MISSING_SLOT: usize = 95;

/// One glyph as 9 row bitmasks; bit 4 is the leftmost ink column, bit 0 the rightmost.
pub type GlyphRows = [u8; CELL_H as usize];

const SHEETS: &[(&str, [&str; 9])] = &[
    (
        "ABCDEFGHIJKLM",
        [
            ".#### ####. .#### ####. ##### ##### .#### #...# .###. ..### #...# #.... #...#",
            "#...# #...# #.... #...# #.... #.... #.... #...# ..#.. ....# #..#. #.... ##.##",
            "#...# #...# #.... #...# #.... #.... #.... #...# ..#.. ....# #.#.. #.... #.#.#",
            "##### ####. #.... #...# ####. ####. #..## ##### ..#.. ....# ##... #.... #.#.#",
            "#...# #...# #.... #...# #.... #.... #...# #...# ..#.. ....# #.#.. #.... #...#",
            "#...# #...# #.... #...# #.... #.... #...# #...# ..#.. #...# #..#. #.... #...#",
            "#...# ####. ####. ####. ##### #.... ####. #...# .###. ####. #...# ##### #...#",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
        ],
    ),
    (
        "NOPQRSTUVWXYZ",
        [
            "#...# .#### ####. .#### ####. .#### ##### #...# #...# #...# #...# #...# #####",
            "#...# #...# #...# #...# #...# #.... ..#.. #...# #...# #...# #...# #...# ....#",
            "##..# #...# #...# #...# #...# #.... ..#.. #...# #...# #...# .#.#. .#.#. ...#.",
            "#.#.# #...# ####. #...# ####. .###. ..#.. #...# #...# #.#.# ..#.. ..#.. ..#..",
            "#..## #...# #.... #.#.# #.#.. ....# ..#.. #...# #...# #.#.# .#.#. ..#.. .#...",
            "#...# #...# #.... #..#. #..#. ....# ..#.. #...# .#.#. #.#.# #...# ..#.. #....",
            "#...# ####. #.... ###.# #...# ####. ..#.. ####. ..#.. .#.#. #...# ..#.. #####",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
        ],
    ),
    (
        "abcdefghijklm",
        [
            "..... #.... ..... ....# ..... ..### ..... #.... ..#.. ...#. #.... .##.. .....",
            "..... #.... ..... ....# ..... .#... ..... #.... ..... ..... #.... ..#.. .....",
            "####. ####. .#### .#### .#### ####. .#### ####. .##.. ..##. #..#. ..#.. ####.",
            "....# #...# #.... #...# #...# .#... #...# #...# ..#.. ...#. #.#.. ..#.. #.#.#",
            ".#### #...# #.... #...# ##### .#... #...# #...# ..#.. ...#. ##... ..#.. #.#.#",
            "#...# #...# #.... #...# #.... .#... #...# #...# ..#.. ...#. #.#.. ..#.. #.#.#",
            ".#### ####. ####. .#### ####. .#... .#### #...# .###. ...#. #..#. .###. #.#.#",
            "..... ..... ..... ..... ..... ..... ....# ..... ..... ...#. ..... ..... .....",
            "..... ..... ..... ..... ..... ..... ####. ..... ..... ###.. ..... ..... .....",
        ],
    ),
    (
        "nopqrstuvwxyz",
        [
            "..... ..... ..... ..... ..... ..... .#... ..... ..... ..... ..... ..... .....",
            "..... ..... ..... ..... ..... ..... .#... ..... ..... ..... ..... ..... .....",
            "####. .#### ####. .#### #.### .#### ####. #...# #...# #...# #...# #...# #####",
            "#...# #...# #...# #...# ##... #.... .#... #...# #...# #...# .#.#. #...# ...#.",
            "#...# #...# #...# #...# #.... .###. .#... #...# #...# #.#.# ..#.. #...# ..#..",
            "#...# #...# #...# #...# #.... ....# .#... #...# .#.#. #.#.# .#.#. #...# .#...",
            "#...# ####. ####. .#### #.... ####. ..##. .#### ..#.. .#.#. #...# .#### #####",
            "..... ..... #.... ....# ..... ..... ..... ..... ..... ..... ..... ....# .....",
            "..... ..... #.... ....# ..... ..... ..... ..... ..... ..... ..... ####. .....",
        ],
    ),
    (
        "0123456789",
        [
            ".#### .##.. ####. ####. #...# ##### .#### ##### .#### .####",
            "#...# #.#.. ....# ....# #...# #.... #.... ....# #...# #...#",
            "#..## ..#.. ....# ....# #...# #.... #.... ....# #...# #...#",
            "#.#.# ..#.. .###. .###. ##### ####. ####. ...#. .###. .####",
            "##..# ..#.. #.... ....# ....# ....# #...# ..#.. #...# ....#",
            "#...# ..#.. #.... ....# ....# ....# #...# ..#.. #...# ....#",
            "####. ##### ##### ####. ....# ####. ####. ..#.. ####. ####.",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
        ],
    ),
    (
        " !\"#$%&'()*+,-./",
        [
            "..... ..#.. .#.#. .#.#. ..#.. ##..# .##.. ..#.. ...#. .#... ..... ..... ..... ..... ..... ....#",
            "..... ..#.. .#.#. .#.#. .#### ##..# #..#. ..#.. ..#.. ..#.. ..#.. ..#.. ..... ..... ..... ....#",
            "..... ..#.. ..... ##### #.#.. ...#. #.#.. ..... .#... ...#. #.#.# ..#.. ..... ..... ..... ...#.",
            "..... ..#.. ..... .#.#. .###. ..#.. .#... ..... .#... ...#. .###. ##### ..... .###. ..... ..#..",
            "..... ..#.. ..... ##### ..#.# .#... #.#.# ..... .#... ...#. #.#.# ..#.. ..... ..... ..... .#...",
            "..... ..... ..... .#.#. ####. #..## #..#. ..... ..#.. ..#.. ..#.. ..#.. ..... ..... ..... #....",
            "..... ..#.. ..... .#.#. ..#.. #..## .##.# ..... ...#. .#... ..... ..... ..#.. ..... ..#.. #....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .#... ..... ..... .....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
        ],
    ),
    (
        ":;<=>?@[\\]^_`{|}~",
        [
            "..... ..... ...#. ..... .#... ####. .###. .###. #.... .###. ..#.. ..... .#... ...## ..#.. ##... .....",
            "..... ..... ..#.. ..... ..#.. ....# #...# .#... #.... ...#. .#.#. ..... ..#.. ..#.. ..#.. ..#.. .....",
            "..#.. ..#.. .#... ##### ...#. ....# #.### .#... .#... ...#. #...# ..... ..... ..#.. ..#.. ..#.. .#...",
            "..... ..... #.... ..... ....# ..##. #.#.# .#... ..#.. ...#. ..... ..... ..... ##... ..#.. ...## #.#.#",
            "..... ..... .#... ##### ...#. ..#.. #.### .#... ...#. ...#. ..... ..... ..... ..#.. ..#.. ..#.. ...#.",
            "..#.. ..#.. ..#.. ..... ..#.. ..... #.... .#... ....# ...#. ..... ..... ..... ..#.. ..#.. ..#.. .....",
            "..... .#... ...#. ..... .#... ..#.. .#### .###. ....# .###. ..... ..... ..... ...## ..#.. ##... .....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ##### ..... ..... ..#.. ..... .....",
            "..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... ..... .....",
        ],
    ),
];

/// Drawn for any character outside printable ASCII.
const MISSING_GLYPH: [&str; 9] = [
    "#####", "#.#.#", "##.##", "#.#.#", "##.##", "#.#.#", "#####", ".....", ".....",
];

fn parse_row(row: &str) -> u8 {
    row.bytes()
        .take(INK_W)
        .fold(0u8, |bits, b| (bits << 1) | u8::from(b == b'#'))
}

fn glyph_table() -> &'static [GlyphRows; GLYPH_COUNT] {
    static TABLE: OnceLock<[GlyphRows; GLYPH_COUNT]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [[0u8; CELL_H as usize]; GLYPH_COUNT];
        for (chars, rows) in SHEETS {
            for (i, c) in chars.chars().enumerate() {
                let Some(slot) = slot_of(c) else { continue };
                for (r, row) in rows.iter().enumerate() {
                    if let Some(chunk) = row.split(' ').nth(i) {
                        table[slot][r] = parse_row(chunk);
                    }
                }
            }
        }
        for (r, row) in MISSING_GLYPH.iter().enumerate() {
            table[MISSING_SLOT][r] = parse_row(row);
        }
        table
    })
}

fn slot_of(c: char) -> Option<usize> {
    let code = c as u32;
    (32..=126).contains(&code).then(|| (code - 32) as usize)
}

/// Whether `c` has a real glyph (printable ASCII). Other characters draw the missing box.
pub fn has_glyph(c: char) -> bool {
    slot_of(c).is_some()
}

/// The pixel rows of a glyph (bit 4 = leftmost column). Mostly useful for tests and tools.
pub fn glyph_rows(c: char) -> GlyphRows {
    glyph_table()[slot_of(c).unwrap_or(MISSING_SLOT)]
}

/// Whether the glyph for `c` has ink at pixel (`x`, `y`) of its 6 x 9 cell.
pub fn glyph_pixel(c: char, x: u32, y: u32) -> bool {
    if x >= INK_W as u32 || y >= CELL_H {
        return false;
    }
    glyph_rows(c)[y as usize] & (1 << (INK_W as u32 - 1 - x)) != 0
}

/// Font atlas: white glyph pixels with full alpha on a transparent background.
pub fn build_atlas() -> TextureData {
    let mut rgba = vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
    for (slot, rows) in glyph_table().iter().enumerate() {
        let (ox, oy) = slot_origin(slot);
        for (y, bits) in rows.iter().enumerate() {
            for x in 0..INK_W as u32 {
                if bits & (1 << (INK_W as u32 - 1 - x)) != 0 {
                    let (px, py) = (ox + x, oy + y as u32);
                    let i = ((py * ATLAS_W + px) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
    }
    TextureData {
        width: ATLAS_W,
        height: ATLAS_H,
        rgba,
    }
}

/// Top-left pixel of a glyph cell inside the atlas.
fn slot_origin(slot: usize) -> (u32, u32) {
    let slot = slot as u32;
    ((slot % COLS) * SLOT_W + 1, (slot / COLS) * SLOT_H + 1)
}

/// UV rectangle (min, max) of the `GLYPH_W` x `GLYPH_H` cell of `c` in the atlas.
pub fn glyph_uv(c: char) -> (Vec2, Vec2) {
    let (x, y) = slot_origin(slot_of(c).unwrap_or(MISSING_SLOT));
    let size = Vec2::new(ATLAS_W as f32, ATLAS_H as f32);
    let min = Vec2::new(x as f32, y as f32) / size;
    let max = Vec2::new((x + CELL_W) as f32, (y + CELL_H) as f32) / size;
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_are_well_formed() {
        for (chars, rows) in SHEETS {
            let n = chars.chars().count();
            for row in rows {
                let chunks: Vec<&str> = row.split(' ').collect();
                assert_eq!(
                    chunks.len(),
                    n,
                    "sheet {chars:?}: row {row:?} has wrong glyph count"
                );
                for chunk in chunks {
                    assert_eq!(chunk.len(), INK_W, "sheet {chars:?}: bad chunk {chunk:?}");
                    assert!(
                        chunk.bytes().all(|b| b == b'#' || b == b'.'),
                        "bad chunk {chunk:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_printable_ascii_char_defined_once() {
        let mut seen = [0u32; GLYPH_COUNT];
        for (chars, _) in SHEETS {
            for c in chars.chars() {
                seen[slot_of(c).expect("sheet char must be printable ASCII")] += 1;
            }
        }
        for code in 32u8..=126 {
            assert_eq!(seen[(code - 32) as usize], 1, "char {:?}", code as char);
        }
    }

    #[test]
    fn glyphs_are_distinct_and_only_space_is_blank() {
        let table = glyph_table();
        for a in 0..GLYPH_COUNT {
            let blank = table[a].iter().all(|&r| r == 0);
            assert_eq!(blank, a == 0, "slot {a} blank={blank}");
            for b in a + 1..GLYPH_COUNT {
                assert_ne!(table[a], table[b], "glyphs {a} and {b} are identical");
            }
        }
    }

    #[test]
    fn caps_and_digits_stay_above_baseline() {
        for c in ('A'..='Z').chain('0'..='9') {
            let rows = glyph_rows(c);
            assert_eq!(rows[7] | rows[8], 0, "{c} has ink below the baseline");
            assert_ne!(rows[0], 0, "{c} does not reach cap height");
        }
        for c in ['g', 'j', 'p', 'q', 'y'] {
            assert_ne!(
                glyph_rows(c)[8] | glyph_rows(c)[7],
                0,
                "{c} needs a descender"
            );
        }
    }

    #[test]
    fn atlas_matches_glyphs_and_uvs() {
        let atlas = build_atlas();
        assert_eq!(atlas.rgba.len(), (atlas.width * atlas.height * 4) as usize);
        for c in [' ', 'A', 'g', '~', '\u{e9}'] {
            let (min, max) = glyph_uv(c);
            let x0 = (min.x * atlas.width as f32).round() as u32;
            let y0 = (min.y * atlas.height as f32).round() as u32;
            assert_eq!(
                ((max.x - min.x) * atlas.width as f32).round() as u32,
                CELL_W
            );
            assert_eq!(
                ((max.y - min.y) * atlas.height as f32).round() as u32,
                CELL_H
            );
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    let i = (((y0 + y) * atlas.width + x0 + x) * 4 + 3) as usize;
                    assert_eq!(
                        atlas.rgba[i] == 255,
                        glyph_pixel(c, x, y),
                        "{c:?} at {x},{y}"
                    );
                }
            }
        }
        // Unknown characters use the missing-glyph box.
        assert_eq!(glyph_uv('\u{e9}'), glyph_uv('\u{2603}'));
        assert!(!has_glyph('\u{e9}'));
    }
}
