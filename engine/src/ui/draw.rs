//! Rectangles and low-level drawing primitives on top of [`UiDrawList`].

use glam::{Vec2, Vec4};

use crate::gfx::{UiDrawList, UiQuad};

/// Axis-aligned rectangle in canvas units (origin top-left, +y down).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        min: Vec2::ZERO,
        max: Vec2::ZERO,
    };

    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Self {
            min,
            max: min + size,
        }
    }

    pub fn from_center_size(center: Vec2, size: Vec2) -> Self {
        Self::from_min_size(center - size * 0.5, size)
    }

    /// The whole canvas of a draw list.
    pub fn canvas(list: &UiDrawList) -> Self {
        Self::from_min_size(Vec2::ZERO, list.canvas_size)
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    pub fn is_empty(&self) -> bool {
        self.max.x <= self.min.x || self.max.y <= self.min.y
    }

    /// Inclusive of `min`, exclusive of `max`.
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.y >= self.min.y && p.x < self.max.x && p.y < self.max.y
    }

    /// Whether `other` lies completely inside this rectangle.
    pub fn contains_rect(&self, other: Rect) -> bool {
        other.min.x >= self.min.x
            && other.min.y >= self.min.y
            && other.max.x <= self.max.x
            && other.max.y <= self.max.y
    }

    pub fn intersect(&self, other: Rect) -> Rect {
        Rect {
            min: self.min.max(other.min),
            max: self.max.min(other.max),
        }
    }

    pub fn translate(&self, offset: Vec2) -> Rect {
        Rect {
            min: self.min + offset,
            max: self.max + offset,
        }
    }

    /// Shrink on every side (negative grows).
    pub fn shrink(&self, amount: f32) -> Rect {
        self.shrink2(Vec2::splat(amount))
    }

    pub fn shrink2(&self, amount: Vec2) -> Rect {
        Rect {
            min: self.min + amount,
            max: self.max - amount,
        }
    }

    pub fn expand(&self, amount: f32) -> Rect {
        self.shrink(-amount)
    }

    /// Split into a left part `width` wide and the rest.
    pub fn split_left(&self, width: f32) -> (Rect, Rect) {
        let x = (self.min.x + width).min(self.max.x);
        (
            Rect::new(self.min, Vec2::new(x, self.max.y)),
            Rect::new(Vec2::new(x, self.min.y), self.max),
        )
    }

    /// Split into the rest and a right part `width` wide.
    pub fn split_right(&self, width: f32) -> (Rect, Rect) {
        let x = (self.max.x - width).max(self.min.x);
        (
            Rect::new(self.min, Vec2::new(x, self.max.y)),
            Rect::new(Vec2::new(x, self.min.y), self.max),
        )
    }

    /// Split into a top part `height` tall and the rest.
    pub fn split_top(&self, height: f32) -> (Rect, Rect) {
        let y = (self.min.y + height).min(self.max.y);
        (
            Rect::new(self.min, Vec2::new(self.max.x, y)),
            Rect::new(Vec2::new(self.min.x, y), self.max),
        )
    }

    /// Round both corners to whole canvas units (keeps pixel art crisp).
    pub fn round(&self) -> Rect {
        Rect::new(self.min.round(), self.max.round())
    }
}

/// Multiply a colour's alpha.
pub fn fade(color: Vec4, alpha: f32) -> Vec4 {
    Vec4::new(color.x, color.y, color.z, color.w * alpha.clamp(0.0, 1.0))
}

/// Linear blend between two colours.
pub fn mix(a: Vec4, b: Vec4, t: f32) -> Vec4 {
    a.lerp(b, t.clamp(0.0, 1.0))
}

/// Solid rectangle.
pub fn fill(list: &mut UiDrawList, rect: Rect, color: Vec4) {
    if !rect.is_empty() && color.w > 0.0 {
        list.rect(rect.min, rect.max, color);
    }
}

/// Rectangle outline drawn inside `rect`.
pub fn border(list: &mut UiDrawList, rect: Rect, thickness: f32, color: Vec4) {
    let t = thickness.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let (top, rest) = rect.split_top(t);
    let (mid, _) = rest.split_top(rest.height() - t);
    let bottom = Rect::new(Vec2::new(rect.min.x, rect.max.y - t), rect.max);
    fill(list, top, color);
    fill(list, bottom, color);
    let (left, mid_rest) = mid.split_left(t);
    let (_, right) = mid_rest.split_right(t);
    fill(list, left, color);
    fill(list, right, color);
}

/// "Targeting" brackets: an L-shaped mark in each corner of `rect`.
pub fn corner_brackets(list: &mut UiDrawList, rect: Rect, len: f32, thickness: f32, color: Vec4) {
    let (min, max) = (rect.min, rect.max);
    let t = thickness;
    for (corner, dir) in [
        (min, Vec2::new(1.0, 1.0)),
        (Vec2::new(max.x, min.y), Vec2::new(-1.0, 1.0)),
        (Vec2::new(min.x, max.y), Vec2::new(1.0, -1.0)),
        (max, Vec2::new(-1.0, -1.0)),
    ] {
        let horiz = Rect::new(corner, corner + Vec2::new(len * dir.x, t * dir.y));
        let vert = Rect::new(corner, corner + Vec2::new(t * dir.x, len * dir.y));
        for r in [horiz, vert] {
            fill(list, Rect::new(r.min.min(r.max), r.min.max(r.max)), color);
        }
    }
}

/// A slab whose right end is cut at 45 degrees with 1-unit pixel steps (`/` shaped end when
/// `lean_right`, `\` otherwise). Used for title bars, tabs and highlights.
pub fn slanted(list: &mut UiDrawList, rect: Rect, lean_right: bool, color: Vec4) {
    let h = rect.height().round().max(1.0) as i32;
    let cut = h as f32;
    let body = Rect::new(rect.min, Vec2::new(rect.max.x - cut, rect.max.y));
    fill(list, body, color);
    for i in 0..h {
        // Row i of the cut region: the further down, the longer (lean right) or shorter.
        let len = if lean_right {
            (h - i) as f32
        } else {
            (i + 1) as f32
        };
        let y = rect.min.y + i as f32;
        let x0 = rect.max.x - cut;
        fill(
            list,
            Rect::new(
                Vec2::new(x0, y),
                Vec2::new(x0 + len, (y + 1.0).min(rect.max.y)),
            ),
            color,
        );
    }
}

/// A `/`-leaning parallelogram `w` wide and `h` tall (whole units) with its bottom-left
/// corner at (`x`, `y + h`). Drawn as 1-unit rows for a pixel-art diagonal.
pub fn parallelogram(list: &mut UiDrawList, x: f32, y: f32, w: f32, h: f32, color: Vec4) {
    let rows = h.round().max(1.0) as i32;
    for i in 0..rows {
        let dx = (rows - 1 - i) as f32;
        fill(
            list,
            Rect::from_min_size(Vec2::new(x + dx, y + i as f32), Vec2::new(w, 1.0)),
            color,
        );
    }
}

/// Vertical hatching: 1-unit stripes every `period` units.
pub fn hatch(list: &mut UiDrawList, rect: Rect, period: f32, color: Vec4) {
    let period = period.max(2.0);
    let mut x = rect.min.x;
    while x < rect.max.x {
        let x1 = (x + 1.0).min(rect.max.x);
        fill(
            list,
            Rect::new(Vec2::new(x, rect.min.y), Vec2::new(x1, rect.max.y)),
            color,
        );
        x += period;
    }
}

/// Horizontal 1-unit line.
pub fn hline(list: &mut UiDrawList, x0: f32, x1: f32, y: f32, color: Vec4) {
    fill(
        list,
        Rect::new(Vec2::new(x0, y), Vec2::new(x1, y + 1.0)),
        color,
    );
}

/// Clip every quad from index `start` onwards to `clip`, adjusting UVs and dropping quads that
/// end up empty. Used for scroll areas (the renderer has no scissor support for UI).
pub fn clip_quads(list: &mut UiDrawList, start: usize, clip: Rect) {
    if start >= list.quads.len() {
        return;
    }
    let tail: Vec<UiQuad> = list.quads.split_off(start);
    list.quads
        .extend(tail.into_iter().filter_map(|q| clip_quad(q, clip)));
}

fn clip_quad(q: UiQuad, clip: Rect) -> Option<UiQuad> {
    let min = q.min.max(clip.min);
    let max = q.max.min(clip.max);
    if max.x <= min.x || max.y <= min.y {
        return None;
    }
    if min == q.min && max == q.max {
        return Some(q);
    }
    let size = q.max - q.min;
    let uv_size = q.uv_max - q.uv_min;
    let to_uv = |p: Vec2| q.uv_min + (p - q.min) / size * uv_size;
    Some(UiQuad {
        min,
        max,
        uv_min: to_uv(min),
        uv_max: to_uv(max),
        ..q
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::UiTexture;

    #[test]
    fn clip_adjusts_uvs_and_drops_outside() {
        let mut list = UiDrawList::new(Vec2::new(100.0, 100.0));
        list.quads.push(UiQuad {
            min: Vec2::new(0.0, 0.0),
            max: Vec2::new(10.0, 10.0),
            uv_min: Vec2::ZERO,
            uv_max: Vec2::ONE,
            color: Vec4::ONE,
            texture: UiTexture::Font,
        });
        list.rect(Vec2::new(50.0, 50.0), Vec2::new(60.0, 60.0), Vec4::ONE);
        clip_quads(
            &mut list,
            0,
            Rect::new(Vec2::new(5.0, 0.0), Vec2::new(40.0, 8.0)),
        );
        assert_eq!(list.quads.len(), 1);
        let q = list.quads[0];
        assert_eq!(q.min, Vec2::new(5.0, 0.0));
        assert_eq!(q.max, Vec2::new(10.0, 8.0));
        assert!((q.uv_min.x - 0.5).abs() < 1e-6 && (q.uv_max.y - 0.8).abs() < 1e-6);
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::from_min_size(Vec2::new(10.0, 20.0), Vec2::new(100.0, 50.0));
        assert!(r.contains(Vec2::new(10.0, 20.0)) && !r.contains(Vec2::new(110.0, 30.0)));
        let (l, rest) = r.split_left(30.0);
        assert_eq!(l.width(), 30.0);
        assert_eq!(rest.width(), 70.0);
        assert!(
            r.intersect(Rect::new(Vec2::ZERO, Vec2::splat(5.0)))
                .is_empty()
        );
    }
}
