//! Built-in bitmap font.
//!
//! CONTRACT (used by the renderer): `build_atlas()` returns the font atlas texture and
//! `glyph_uv(c)` returns the (uv_min, uv_max) of a character inside it. Glyphs are
//! `GLYPH_W` x `GLYPH_H` canvas units at scale 1, advancing `GLYPH_ADVANCE` units.
//! This is a placeholder (solid boxes) until the UI work replaces it with real glyphs.

use glam::Vec2;

use crate::gfx::TextureData;

pub const GLYPH_W: f32 = 6.0;
pub const GLYPH_H: f32 = 8.0;
pub const GLYPH_ADVANCE: f32 = 6.0;

const COLS: u32 = 16;
const ROWS: u32 = 6;
const CELL_W: u32 = 6;
const CELL_H: u32 = 8;

/// Atlas covering ASCII 32..=127 in a 16x6 grid.
pub fn build_atlas() -> TextureData {
    let (w, h) = (COLS * CELL_W, ROWS * CELL_H);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x % CELL_W, y % CELL_H);
            let glyph = (y / CELL_H) * COLS + x / CELL_W;
            let inside = glyph != 0 && (1..CELL_W - 1).contains(&cx) && (1..CELL_H - 1).contains(&cy);
            if inside {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    TextureData { width: w, height: h, rgba }
}

pub fn glyph_uv(c: char) -> (Vec2, Vec2) {
    let code = c as u32;
    let index = if (32..128).contains(&code) { code - 32 } else { '?' as u32 - 32 };
    let (col, row) = (index % COLS, index / COLS);
    let size = Vec2::new((COLS * CELL_W) as f32, (ROWS * CELL_H) as f32);
    let min = Vec2::new((col * CELL_W) as f32, (row * CELL_H) as f32) / size;
    let max = Vec2::new(((col + 1) * CELL_W) as f32, ((row + 1) * CELL_H) as f32) / size;
    (min, max)
}
