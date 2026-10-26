//! Deterministic procedural textures in a crunchy PS1 style.
//!
//! Everything is built from a seeded integer hash (no RNG crate), so the same name always yields
//! the same pixels on every machine. All material noise is *periodic* over the texture so the
//! textures tile seamlessly with repeat addressing.

use glam::{Vec2, Vec3, Vec4};

use crate::gfx::TextureData;

/// Names accepted by [`material_texture`] (anything else gets a checkerboard).
pub const MATERIALS: &[&str] = &[
    "concrete", "brick", "metal", "rust", "grate", "wax", "waxstone", "lava", "tile", "wood",
    "hazard", "glass", "dirt", "panel",
];

/// Old material names still accepted by [`material_texture`] (level files written before the
/// wax re-theme): `(old, current)`.
pub const MATERIAL_ALIASES: &[(&str, &str)] = &[("flesh", "wax"), ("bone", "waxstone")];

/// Sprite / decal textures (with alpha).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpriteTex {
    MuzzleFlash,
    /// A spatter of molten wax droplets, pale ivory (tint it).
    WaxDrops,
    /// Floor/wall decal: a splat of hardened wax with a raised rim, pale ivory (tint it).
    WaxSplat,
    Spark,
    Smoke,
    /// Soft radial glow, white (tint it; draw additive).
    Glow,
    /// Shockwave ring, white.
    Ring,
    /// Mirrored ricochet shard.
    Shard,
    /// An irregular lump of wax or slag with a glowing crack, neutral grey (tint it).
    Lump,
    /// Burn mark decal.
    Scorch,
}

impl SpriteTex {
    pub const ALL: [SpriteTex; 10] = [
        SpriteTex::MuzzleFlash,
        SpriteTex::WaxDrops,
        SpriteTex::WaxSplat,
        SpriteTex::Spark,
        SpriteTex::Smoke,
        SpriteTex::Glow,
        SpriteTex::Ring,
        SpriteTex::Shard,
        SpriteTex::Lump,
        SpriteTex::Scorch,
    ];
}

// ---------------------------------------------------------------------------------------------
// Noise toolkit

/// Integer hash of a lattice point (lowbias32-style mixing).
pub fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f).wrapping_add(0x9e37_79b9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

/// Hash mapped to `[0, 1)`.
pub fn rand01(x: i32, y: i32, seed: u32) -> f32 {
    (hash(x, y, seed) >> 8) as f32 / (1u32 << 24) as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise in `[0, 1]`, periodic over `[0, 1)` with `cells` lattice cells per axis.
pub fn value_noise(u: f32, v: f32, cells: i32, seed: u32) -> f32 {
    value_noise2(u, v, cells, cells, seed)
}

/// Value noise with different cell counts per axis (stretched / brushed patterns).
pub fn value_noise2(u: f32, v: f32, cells_x: i32, cells_y: i32, seed: u32) -> f32 {
    let (x, y) = (u * cells_x as f32, v * cells_y as f32);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - x0), smooth(y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let at = |i: i32, j: i32| rand01(i.rem_euclid(cells_x), j.rem_euclid(cells_y), seed);
    let a = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * fx;
    let b = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * fx;
    a + (b - a) * fy
}

/// Fractal (octaved) value noise in `[0, 1]`, periodic.
pub fn fbm(u: f32, v: f32, cells: i32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut total) = (0.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += value_noise(u, v, cells << o, seed.wrapping_add(o * 1013)) * amp;
        total += amp;
        amp *= 0.5;
    }
    sum / total
}

/// Cellular noise: distances to the nearest and second-nearest jittered feature points
/// (in cell units), periodic with `cells` cells per axis. Also returns the nearest cell's id.
pub fn cellular(u: f32, v: f32, cells: i32, seed: u32) -> (f32, f32, u32) {
    let p = Vec2::new(u, v) * cells as f32;
    let base = p.floor();
    let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, 0);
    for j in -1..=1 {
        for i in -1..=1 {
            let cell = base + Vec2::new(i as f32, j as f32);
            let (cx, cy) = (
                (cell.x as i32).rem_euclid(cells),
                (cell.y as i32).rem_euclid(cells),
            );
            let jitter = Vec2::new(rand01(cx, cy, seed), rand01(cx, cy, seed ^ 0x5f3)) * 0.8 + 0.1;
            let d = (cell + jitter).distance(p);
            if d < f1 {
                f2 = f1;
                f1 = d;
                id = hash(cx, cy, seed);
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, id)
}

/// Linear colour ramp through evenly spaced stops.
pub fn ramp(t: f32, stops: &[Vec3]) -> Vec3 {
    match stops.len() {
        0 => Vec3::ZERO,
        1 => stops[0],
        n => {
            let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (x.floor() as usize).min(n - 2);
            stops[i].lerp(stops[i + 1], x - i as f32)
        }
    }
}

/// `0xRRGGBB` to a colour in `[0, 1]`.
pub fn hex(rgb: u32) -> Vec3 {
    Vec3::new(
        ((rgb >> 16) & 255) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
    )
}

/// RGBA float image that converts into [`TextureData`].
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Vec4>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![Vec4::ZERO; (width * height) as usize],
        }
    }

    /// Sets every pixel from `f(x, y, u, v)` where `u, v` are in `[0, 1)`.
    pub fn fill(&mut self, f: impl Fn(i32, i32, f32, f32) -> Vec4) {
        let (w, h) = (self.width as f32, self.height as f32);
        for y in 0..self.height {
            for x in 0..self.width {
                self.pixels[(y * self.width + x) as usize] =
                    f(x as i32, y as i32, x as f32 / w, y as f32 / h);
            }
        }
    }

    /// Pixel with wrap-around addressing.
    pub fn get(&self, x: i32, y: i32) -> Vec4 {
        let x = x.rem_euclid(self.width as i32) as u32;
        let y = y.rem_euclid(self.height as i32) as u32;
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: i32, y: i32, c: Vec4) {
        let x = x.rem_euclid(self.width as i32) as u32;
        let y = y.rem_euclid(self.height as i32) as u32;
        self.pixels[(y * self.width + x) as usize] = c;
    }

    pub fn into_texture(self) -> TextureData {
        let rgba = self
            .pixels
            .iter()
            .flat_map(|p| {
                p.to_array()
                    .map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
            })
            .collect();
        TextureData {
            width: self.width,
            height: self.height,
            rgba,
        }
    }
}

fn opaque(c: Vec3) -> Vec4 {
    c.extend(1.0)
}

/// Quantises alpha to a few hard steps (crunchy sprite edges).
fn step_alpha(a: f32, steps: f32) -> f32 {
    (a.clamp(0.0, 1.0) * steps).floor() / steps
}

// ---------------------------------------------------------------------------------------------
// Materials

/// Texture for a level material by name. Unknown names get a magenta checkerboard.
pub fn material_texture(name: &str) -> TextureData {
    match name {
        "concrete" => concrete(),
        "brick" => brick(),
        "metal" => metal(),
        "rust" => rust(),
        "grate" => grate(),
        "wax" | "flesh" => wax(),
        "waxstone" | "bone" => waxstone(),
        "lava" => lava(),
        "tile" => tile(),
        "wood" => wood(),
        "hazard" => hazard(),
        "glass" => glass(),
        "dirt" => dirt(),
        "panel" => panel(),
        _ => checker(),
    }
}

fn checker() -> TextureData {
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, _, _| {
        let on = ((x / 8) + (y / 8)) % 2 == 0;
        opaque(if on { hex(0xff00ff) } else { hex(0x101010) })
    });
    c.into_texture()
}

fn concrete() -> TextureData {
    const SEED: u32 = 11;
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let base = fbm(u, v, 4, 4, SEED);
        let stain = value_noise(u, v, 2, SEED + 7);
        let speck = rand01(x, y, SEED + 3);
        let crack = (fbm(u, v, 3, 3, SEED + 9) - 0.5).abs();
        let mut g = 0.36 + base * 0.26 - (stain - 0.6).max(0.0) * 0.35;
        if speck < 0.05 {
            g -= 0.08;
        } else if speck > 0.97 {
            g += 0.07;
        }
        if crack < 0.012 {
            g *= 0.55;
        }
        // Faint formwork seam.
        if y % 32 == 31 {
            g *= 0.85;
        }
        opaque(Vec3::new(g * 1.02, g, g * 0.95))
    });
    c.into_texture()
}

fn brick() -> TextureData {
    const SEED: u32 = 23;
    let mut c = Canvas::new(64, 64);
    let palette = [
        hex(0x7a2e22),
        hex(0x8f3a26),
        hex(0x6b2a24),
        hex(0x9a4a30),
        hex(0x5e2420),
    ];
    c.fill(|x, y, u, v| {
        let row = y / 8;
        let offset = if row % 2 == 0 { 0 } else { 8 };
        let col = (x + offset).rem_euclid(64) / 16;
        let (bx, by) = ((x + offset).rem_euclid(16), y % 8);
        let mortar = bx == 15 || by == 7;
        let n = fbm(u, v, 8, 3, SEED);
        if mortar {
            let m = 0.5 + n * 0.2;
            return opaque(Vec3::new(m, m * 0.96, m * 0.88) * 0.8);
        }
        let pick = hash(col, row, SEED) as usize % palette.len();
        let mut color = palette[pick] * (0.8 + n * 0.4);
        // Darker edges just inside the mortar lines, lighter top edge.
        if by == 6 || bx == 14 {
            color *= 0.8;
        } else if by == 0 {
            color *= 1.12;
        }
        if rand01(x, y, SEED + 1) < 0.06 {
            color *= 0.75;
        }
        opaque(color)
    });
    c.into_texture()
}

fn metal() -> TextureData {
    const SEED: u32 = 37;
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let brushed = value_noise2(u, v, 4, 48, SEED);
        let blotch = fbm(u, v, 4, 3, SEED + 5);
        let mut color = hex(0x5d6670) * (0.85 + brushed * 0.25 + (blotch - 0.5) * 0.25);
        // Bevelled panel border.
        if x == 0 || y == 0 {
            color = hex(0x8b96a0);
        } else if x == 63 || y == 63 {
            color = hex(0x2c3238);
        } else if x == 1 || y == 1 {
            color *= 1.15;
        }
        // Rivets in the corners.
        for (rx, ry) in [(5, 5), (58, 5), (5, 58), (58, 58)] {
            let d = Vec2::new((x - rx) as f32, (y - ry) as f32);
            if d.length() < 2.3 {
                color = if d.x + d.y < 0.0 {
                    hex(0xa9b4bd)
                } else {
                    hex(0x3a4148)
                };
            }
        }
        // A few light scratches.
        let s = (x + y * 3 + (hash(x / 16, 0, SEED) % 7) as i32) % 41;
        if s == 0 && rand01(x, y, SEED + 2) < 0.7 {
            color *= 1.2;
        }
        opaque(color)
    });
    c.into_texture()
}

fn rust() -> TextureData {
    const SEED: u32 = 41;
    let stops = [
        hex(0x2a140c),
        hex(0x5a2a12),
        hex(0x8e4a1c),
        hex(0xb8692a),
        hex(0xc98a46),
    ];
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 4, 4, SEED);
        let patches = fbm(u, v, 3, 3, SEED + 9);
        let mut color = ramp(n * 1.1 - 0.05, &stops);
        if patches > 0.62 {
            // Bare metal showing through.
            color = hex(0x59616a) * (0.8 + n * 0.4);
        }
        if rand01(x, y, SEED + 4) < 0.07 {
            color *= 0.6; // pits
        }
        opaque(color)
    });
    c.into_texture()
}

fn grate() -> TextureData {
    const SEED: u32 = 53;
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let (gx, gy) = (x % 16, y % 16);
        let bar_x = gx < 4;
        let bar_y = gy < 4;
        if !bar_x && !bar_y {
            return Vec4::ZERO; // hole (alpha-tested away)
        }
        let n = fbm(u, v, 8, 2, SEED);
        let mut color = hex(0x4a4f55) * (0.8 + n * 0.4);
        let edge = if bar_x { gx } else { gy };
        if edge == 0 {
            color *= 1.35;
        } else if edge == 3 {
            color *= 0.6;
        }
        if bar_x && bar_y {
            color *= 1.1;
        }
        opaque(color)
    });
    c.into_texture()
}

/// Poured wax (drapes, sheets, candles, effigies): smooth ivory with soft vertical drip runs,
/// rounded drip ends, a dull sheen and the odd pit. Pale so level tints colour it.
fn wax() -> TextureData {
    const SEED: u32 = 67;
    let stops = [hex(0xb8aa8c), hex(0xd4c8aa), hex(0xe8dec6), hex(0xf6efdc)];
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 3, 3, SEED);
        // Drip runs: noise stretched along v, each run ending in a slightly darker bead.
        let run = value_noise2(u, v, 12, 2, SEED + 3);
        let ripple = value_noise2(u, v, 12, 6, SEED + 4);
        let mut color = ramp(n * 0.55 + 0.35 + (run - 0.5) * 0.35, &stops);
        if run > 0.62 {
            color *= 1.0 - (run - 0.62) * 0.5 - (ripple - 0.5).abs() * 0.12;
        }
        if rand01(x, y, SEED + 5) < 0.012 && n < 0.5 {
            color *= 0.78; // pits
        }
        if rand01(x, y, SEED + 6) < 0.02 && n > 0.62 {
            color = color * 0.5 + hex(0xfffaf0) * 0.5; // sheen
        }
        opaque(color)
    });
    c.into_texture()
}

/// Set wax-stone (pillars, ribs, columns): wax poured in layers and hardened pale and hard,
/// with faint pour lines, hairline cracks and pits.
fn waxstone() -> TextureData {
    const SEED: u32 = 71;
    let stops = [hex(0x8c7e62), hex(0xb6a886), hex(0xd6caaa), hex(0xebe2c8)];
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 4, 4, SEED);
        let crack = (fbm(u, v, 2, 4, SEED + 2) - 0.5).abs();
        let mut color = ramp(n * 0.8 + 0.25, &stops);
        // Pour lines: four wavering horizontal layers.
        let layer = (v * 4.0 + (value_noise(u, 0.5, 4, SEED + 3) - 0.5) * 0.25).fract();
        if layer < 0.035 {
            color *= 0.82;
        } else if layer < 0.07 {
            color *= 1.06;
        }
        if crack < 0.01 {
            color *= 0.6;
        }
        if rand01(x, y, SEED + 1) < 0.03 {
            color *= 0.84; // pits
        }
        opaque(color)
    });
    c.into_texture()
}

fn lava() -> TextureData {
    const SEED: u32 = 83;
    let stops = [
        hex(0x1c0604),
        hex(0x6a1206),
        hex(0xc8320a),
        hex(0xff8a1a),
        hex(0xffe070),
    ];
    let mut c = Canvas::new(64, 64);
    c.fill(|_, _, u, v| {
        let (f1, f2, _) = cellular(u, v, 4, SEED);
        let n = fbm(u, v, 4, 3, SEED + 1);
        let crack = (f2 - f1).clamp(0.0, 1.0);
        // Glowing seams between dark crust plates.
        let heat = (1.0 - crack * 3.2).max(0.0) * 0.8 + n * 0.35 - f1 * 0.25;
        opaque(ramp(heat, &stops))
    });
    c.into_texture()
}

fn tile() -> TextureData {
    const SEED: u32 = 97;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let (tx, ty) = (x / 16, y / 16);
        let (lx, ly) = (x % 16, y % 16);
        let dirt = fbm(u, v, 4, 3, SEED);
        if lx == 15 || ly == 15 {
            return opaque(hex(0x3e3a34) * (0.8 + dirt * 0.4));
        }
        let tone = 0.85 + rand01(tx, ty, SEED) * 0.15;
        let mut color = hex(0xb9c2c4) * tone * (0.92 + dirt * 0.12);
        if lx == 0 || ly == 0 {
            color *= 1.08;
        } else if lx == 14 || ly == 14 {
            color *= 0.85;
        }
        // Chipped corners.
        if (lx < 2 && ly < 2) && hash(tx, ty, SEED + 1).is_multiple_of(3) {
            color = hex(0x6a6660);
        }
        opaque(color)
    });
    c.into_texture()
}

fn wood() -> TextureData {
    const SEED: u32 = 101;
    let stops = [hex(0x3a2412), hex(0x5c3a1e), hex(0x7a5030), hex(0x8f6640)];
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let plank = x / 16;
        let lx = x % 16;
        let warp = fbm(u, v, 2, 3, SEED + plank as u32) * 6.0;
        let grain = ((u * 64.0 + warp * 3.0 + plank as f32 * 7.0) * 0.9).sin() * 0.5 + 0.5;
        let tone = rand01(plank, 0, SEED) * 0.25;
        let mut color = ramp(grain * 0.6 + tone + 0.1, &stops);
        if lx == 0 {
            color *= 0.45; // seam
        } else if lx == 15 {
            color *= 0.8;
        }
        // Plank ends at different heights.
        let end = (hash(plank, 1, SEED) % 64) as i32;
        if y == end {
            color *= 0.5;
        }
        // A knot.
        let knot = Vec2::new(
            ((x - plank * 16 - 8) as f32) * 1.6,
            (y - (end + 20) % 64) as f32,
        );
        let d = knot.length();
        if d < 4.0 && hash(plank, 2, SEED).is_multiple_of(2) {
            color = ramp(0.2 + (d * 1.7).sin() * 0.2, &stops);
        }
        opaque(color * (0.92 + rand01(x, y, SEED + 4) * 0.12))
    });
    c.into_texture()
}

fn hazard() -> TextureData {
    const SEED: u32 = 113;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        let stripe = ((x + y) / 8) % 2 == 0;
        let grime = fbm(u, v, 4, 3, SEED);
        let mut color = if stripe { hex(0xe0b020) } else { hex(0x1a1a1a) };
        color *= 0.75 + grime * 0.35;
        // Chipped paint shows metal.
        if rand01(x, y, SEED + 1) < 0.04 + (grime - 0.6).max(0.0) * 0.4 {
            color = hex(0x6a6e72);
        }
        opaque(color)
    });
    c.into_texture()
}

fn glass() -> TextureData {
    const SEED: u32 = 127;
    let mut c = Canvas::new(32, 32);
    c.fill(|x, y, u, v| {
        if x == 0 || y == 0 || x == 31 || y == 31 {
            return opaque(hex(0x3a4a52)); // frame
        }
        let n = fbm(u, v, 4, 2, SEED);
        let streak = (x - y).rem_euclid(24);
        let (color, alpha) = if streak < 2 || streak == 5 {
            (hex(0xe8fbff), 0.7)
        } else {
            (hex(0x7fc6d8) * (0.85 + n * 0.3), 0.35)
        };
        color.extend(alpha)
    });
    c.into_texture()
}

fn dirt() -> TextureData {
    const SEED: u32 = 131;
    let stops = [hex(0x2a1d12), hex(0x4a3420), hex(0x6a4c30), hex(0x806040)];
    let mut c = Canvas::new(64, 64);
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 4, 4, SEED);
        let mut color = ramp(n, &stops);
        let (f1, _, id) = cellular(u, v, 8, SEED + 1);
        if f1 < 0.22 && id % 4 == 0 {
            // Pebbles: lighter with a dark lower edge.
            let shade = if rand01(x, y, SEED + 2) < 0.3 {
                0.8
            } else {
                1.0
            };
            color = hex(0x8a8070) * shade * (0.9 + (id % 7) as f32 * 0.03);
        }
        opaque(color * (0.9 + rand01(x, y, SEED + 3) * 0.15))
    });
    c.into_texture()
}

fn panel() -> TextureData {
    const SEED: u32 = 149;
    let mut c = Canvas::new(128, 128);
    // Seams of the inset panels (in pixels).
    let seams_x = [0, 64];
    let seams_y = [0, 40, 88];
    c.fill(|x, y, u, v| {
        let n = fbm(u, v, 8, 3, SEED);
        let mut color = hex(0x4c5560) * (0.88 + n * 0.22);
        for &s in &seams_x {
            if x == s {
                color = hex(0x1c2026);
            } else if x == s + 1 {
                color = hex(0x7a8794);
            }
        }
        for &s in &seams_y {
            if y == s {
                color = hex(0x1c2026);
            } else if y == s + 1 {
                color = hex(0x7a8794);
            }
        }
        // Vent slots in the middle band.
        if (44..84).contains(&y) && (10..54).contains(&x) && (y - 44) % 6 < 2 {
            color = if (y - 44) % 6 == 0 {
                hex(0x14181c)
            } else {
                hex(0x2a3038)
            };
        }
        // Indicator strip.
        if (100..104).contains(&y) && (74..118).contains(&x) {
            let lit = (x - 74) / 6 % 3 != 2;
            color = if lit { hex(0x40e0d0) } else { hex(0x103a38) };
        }
        // Screws near panel corners.
        for sx in [6, 58, 70, 122] {
            for sy in [6, 34, 46, 82, 94, 122] {
                let d = Vec2::new((x - sx) as f32, (y - sy) as f32);
                if d.length() < 1.8 {
                    color = if d.x < 0.0 {
                        hex(0x9aa6b2)
                    } else {
                        hex(0x2a3038)
                    };
                }
            }
        }
        opaque(color)
    });
    c.into_texture()
}

// ---------------------------------------------------------------------------------------------
// Sprites

/// Texture for an effect sprite or decal (RGBA with alpha).
pub fn sprite_texture(kind: SpriteTex) -> TextureData {
    match kind {
        SpriteTex::MuzzleFlash => muzzle_flash(),
        SpriteTex::WaxDrops => wax_drops(),
        SpriteTex::WaxSplat => wax_splat(),
        SpriteTex::Spark => spark(),
        SpriteTex::Smoke => smoke(),
        SpriteTex::Glow => glow(),
        SpriteTex::Ring => ring(),
        SpriteTex::Shard => shard(),
        SpriteTex::Lump => lump(),
        SpriteTex::Scorch => scorch(),
    }
}

/// Centred coordinates in `[-1, 1]` for pixel centres.
fn centered(x: i32, y: i32, size: u32) -> Vec2 {
    (Vec2::new(x as f32, y as f32) + 0.5) / size as f32 * 2.0 - 1.0
}

fn muzzle_flash() -> TextureData {
    const SEED: u32 = 211;
    let size = 32;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, _, _| {
        let p = centered(x, y, size);
        let r = p.length();
        let a = p.y.atan2(p.x);
        // Seven uneven spikes around a hot core.
        let spikes = (a * 3.5 + 0.4).cos().abs().powf(5.0);
        let jag = 0.85
            + rand01(
                (a * 7.0 / std::f32::consts::TAU * 7.0).floor() as i32,
                0,
                SEED,
            ) * 0.3;
        let reach = (0.38 + 0.62 * spikes) * jag;
        let t = (1.0 - r / reach).max(0.0);
        if t <= 0.0 {
            return Vec4::ZERO;
        }
        let color = ramp(
            t * 1.4,
            &[hex(0xc83c08), hex(0xff9a1a), hex(0xffe27a), hex(0xfffbe8)],
        );
        color.extend(step_alpha(t * 2.2, 4.0))
    });
    c.into_texture()
}

fn blobs(size: u32, seed: u32, count: i32, spread: f32, radius: (f32, f32)) -> Vec<(Vec2, f32)> {
    (0..count)
        .map(|i| {
            let a = rand01(i, 0, seed) * std::f32::consts::TAU;
            let d = rand01(i, 1, seed).sqrt() * spread;
            let r = radius.0 + rand01(i, 2, seed) * (radius.1 - radius.0);
            (
                Vec2::new(a.cos(), a.sin()) * d * size as f32 * 0.5,
                r * size as f32 * 0.5,
            )
        })
        .collect()
}

/// Molten wax droplets: round beads with a glossy highlight and a slightly darker skin.
fn wax_drops() -> TextureData {
    const SEED: u32 = 223;
    let size = 32;
    let drops = blobs(size, SEED, 7, 0.55, (0.12, 0.3));
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, _, _| {
        let p = Vec2::new(x as f32 - 15.5, y as f32 - 15.5);
        let mut best = f32::MAX;
        for (center, r) in &drops {
            best = best.min(p.distance(*center) / r);
        }
        if best > 1.0 {
            return Vec4::ZERO;
        }
        let highlight = best < 0.4 && p.x + p.y < 0.0;
        let color = if highlight {
            hex(0xfffaf0)
        } else {
            hex(0xe2d8c4) * (1.08 - best * 0.3)
        };
        color.extend(1.0)
    });
    c.into_texture()
}

/// A splat of wax that hit a surface and set: a lumpy pool with a raised, darker rim, a few
/// satellite beads, a dull sheen and the odd skin wrinkle. No spatter streaks.
fn wax_splat() -> TextureData {
    const SEED: u32 = 227;
    let size = 64;
    let beads = blobs(size, SEED, 9, 0.8, (0.04, 0.09));
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, u, v| {
        let p = centered(x, y, size);
        let r = p.length();
        let a = p.y.atan2(p.x);
        let edge = 0.48 + value_noise(a / std::f32::consts::TAU + 0.5, 0.0, 6, SEED) * 0.2;
        let n = fbm(u, v, 4, 3, SEED + 1);
        let px = Vec2::new(x as f32 - 31.5, y as f32 - 31.5);
        let bead = beads
            .iter()
            .map(|(c, rad)| px.distance(*c) / rad)
            .fold(f32::MAX, f32::min);
        if r >= edge && bead >= 1.0 {
            return Vec4::ZERO;
        }
        let mut color = ramp(n, &[hex(0xb4a994), hex(0xd2c8b2), hex(0xece4d0)]);
        if r < edge {
            // The rim cools first and stands proud; a wrinkle ring sits just inside it.
            let rim = (r - (edge - 0.07)).max(0.0) / 0.07;
            color *= 1.0 - rim * 0.3;
            if (r - (edge - 0.13)).abs() < 0.012 && n > 0.45 {
                color *= 0.85;
            }
            // Dull sheen towards the top-left.
            if (p - Vec2::new(-0.15, -0.15)).length() < 0.12 + n * 0.08 {
                color = color * 0.6 + hex(0xfffaf0) * 0.4;
            }
            color.extend(1.0)
        } else {
            (color * (1.05 - bead * 0.2)).extend(1.0)
        }
    });
    c.into_texture()
}

fn spark() -> TextureData {
    let size = 16;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, _, _| {
        let p = centered(x, y, size).abs();
        // Four-pointed star.
        let t = (1.0 - (p.x * p.y).sqrt() * 3.0 - p.length() * 0.6).max(0.0);
        if t <= 0.0 {
            return Vec4::ZERO;
        }
        let color = ramp(t, &[hex(0xffa020), hex(0xfff0a0), hex(0xffffff)]);
        color.extend(step_alpha(t * 2.0, 4.0))
    });
    c.into_texture()
}

fn smoke() -> TextureData {
    const SEED: u32 = 239;
    let size = 32;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, u, v| {
        let p = centered(x, y, size);
        let n = fbm(u, v, 4, 3, SEED);
        let r = p.length() + (n - 0.5) * 0.5;
        let t = (1.0 - r).max(0.0);
        let shade = 0.45 + n * 0.35 - p.y * 0.1;
        Vec3::splat(shade).extend(step_alpha(t * 1.6, 5.0) * 0.85)
    });
    c.into_texture()
}

fn glow() -> TextureData {
    let size = 32;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, _, _| {
        let r = centered(x, y, size).length();
        let t = (1.0 - r).max(0.0);
        Vec3::ONE.extend(step_alpha(t * t * 1.2, 8.0))
    });
    c.into_texture()
}

fn ring() -> TextureData {
    let size = 64;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, _, _| {
        let r = centered(x, y, size).length();
        let t = (1.0 - (r - 0.8).abs() / 0.14).max(0.0);
        Vec3::ONE.extend(step_alpha(t, 4.0))
    });
    c.into_texture()
}

fn shard() -> TextureData {
    let size = 32;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, _, _| {
        let p = centered(x, y, size);
        // Elongated diamond tilted 30 degrees.
        let (s, co) = (0.5f32).sin_cos();
        let q = Vec2::new(p.x * co + p.y * s, -p.x * s + p.y * co);
        let d = q.x.abs() / 0.38 + q.y.abs() / 0.92;
        if d > 1.0 {
            return Vec4::ZERO;
        }
        let outline = d > 0.86;
        // Two facets split along the long axis, plus a glint.
        let color = if outline {
            hex(0x1a3a52)
        } else if q.x < 0.0 {
            hex(0xbff4ff)
        } else {
            hex(0x5ab8e0)
        };
        let glint = (q.y + 0.35).abs() < 0.06 && q.x < 0.05;
        (if glint { Vec3::ONE } else { color }).extend(1.0)
    });
    c.into_texture()
}

/// A lump of wax or slag broken off a Mould: a faceted, irregular blob, lit from the top, with
/// a thin glowing crack where the heat still shows.
fn lump() -> TextureData {
    const SEED: u32 = 251;
    let size = 32;
    let mut c = Canvas::new(size, size);
    c.fill(|x, y, u, v| {
        let p = centered(x, y, size);
        let a = p.y.atan2(p.x);
        let edge = 0.58 + value_noise(a / std::f32::consts::TAU + 0.5, 0.0, 5, SEED) * 0.32;
        let r = p.length();
        if r > edge {
            return Vec4::ZERO;
        }
        if r > edge - 0.1 {
            return hex(0x2c2824).extend(1.0);
        }
        let n = fbm(u, v, 4, 2, SEED + 1);
        let crack = (fbm(u, v, 2, 2, SEED + 2) - 0.5).abs();
        if crack < 0.03 && r < edge - 0.18 {
            return hex(0xffb868).extend(1.0);
        }
        ramp(
            n * 0.6 + (0.45 - p.y * 0.35),
            &[hex(0x4c4842), hex(0x8a847a), hex(0xd0c8b8)],
        )
        .extend(1.0)
    });
    c.into_texture()
}

fn scorch() -> TextureData {
    const SEED: u32 = 263;
    let size = 64;
    let mut c = Canvas::new(size, size);
    c.fill(|_, _, u, v| {
        let p = centered((u * size as f32) as i32, (v * size as f32) as i32, size);
        let n = fbm(u, v, 4, 4, SEED);
        let r = p.length() + (n - 0.5) * 0.45;
        let t = (1.0 - r).max(0.0);
        if t <= 0.0 {
            return Vec4::ZERO;
        }
        let color = hex(0x120c08) * (0.6 + n * 0.8);
        color.extend(step_alpha(t * 2.0, 5.0) * 0.92)
    });
    c.into_texture()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_periodic_and_bounded() {
        for i in 0..50 {
            let (u, v) = (rand01(i, 0, 1), rand01(i, 1, 1));
            let a = fbm(u, v, 4, 4, 9);
            let b = fbm(u + 1.0, v - 1.0, 4, 4, 9);
            assert!((a - b).abs() < 1e-4, "fbm not periodic");
            assert!((0.0..=1.0).contains(&a));
            let (f1, f2, _) = cellular(u, v, 4, 3);
            assert!(f1 <= f2);
        }
    }

    #[test]
    fn all_textures_are_valid_and_deterministic() {
        for name in MATERIALS.iter().copied().chain(["unknown"]) {
            let t = material_texture(name);
            assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize, "{name}");
            assert!([32, 64, 128].contains(&t.width), "{name}");
            assert_eq!(
                material_texture(name).rgba,
                t.rgba,
                "{name} not deterministic"
            );
        }
        for kind in SpriteTex::ALL {
            let t = sprite_texture(kind);
            assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize);
            // Sprites have transparent borders and some opaque-ish content.
            assert_eq!(t.rgba[3], 0, "{kind:?} corner should be transparent");
            assert!(t.rgba.chunks(4).any(|p| p[3] > 128), "{kind:?} is empty");
        }
    }

    #[test]
    fn old_material_names_still_load() {
        for &(old, current) in MATERIAL_ALIASES {
            assert!(MATERIALS.contains(&current), "{current}");
            assert!(!MATERIALS.contains(&old), "{old}");
            assert_eq!(material_texture(old).rgba, material_texture(current).rgba, "{old}");
        }
    }

    #[test]
    fn grate_has_holes_and_glass_is_translucent() {
        let grate = material_texture("grate");
        assert!(grate.rgba.chunks(4).any(|p| p[3] == 0));
        let glass = material_texture("glass");
        assert!(glass.rgba.chunks(4).any(|p| p[3] > 0 && p[3] < 200));
    }
}
