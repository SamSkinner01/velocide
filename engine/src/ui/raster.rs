//! CPU rasteriser for [`UiDrawList`]s, so UI can be checked (tests, screenshots) without a
//! GPU. Matches the renderer's contract: canvas scaled to the pixel size, nearest sampling,
//! straight alpha blending in the order quads were pushed. `UiTexture::Texture` quads are drawn
//! magenta (the rasteriser has no access to game textures).

use glam::{Vec2, Vec4};

use crate::gfx::{TextureData, UiDrawList, UiTexture};

/// Rasterise `list` onto a transparent `width_px` x `height_px` image.
pub fn rasterize(
    list: &UiDrawList,
    font_atlas: &TextureData,
    width_px: u32,
    height_px: u32,
) -> TextureData {
    let mut target = TextureData {
        width: width_px,
        height: height_px,
        rgba: vec![0; (width_px * height_px * 4) as usize],
    };
    rasterize_onto(list, font_atlas, &mut target);
    target
}

/// Rasterise `list` on top of the existing pixels of `target` (e.g. a background picture).
pub fn rasterize_onto(list: &UiDrawList, font_atlas: &TextureData, target: &mut TextureData) {
    let (w, h) = (target.width as i32, target.height as i32);
    if w == 0 || h == 0 || list.canvas_size.x <= 0.0 || list.canvas_size.y <= 0.0 {
        return;
    }
    let scale = Vec2::new(w as f32, h as f32) / list.canvas_size;
    for q in &list.quads {
        if q.max.x <= q.min.x || q.max.y <= q.min.y || q.color.w <= 0.0 {
            continue;
        }
        // A pixel is covered when its centre lies inside the quad.
        let p0 = q.min * scale;
        let p1 = q.max * scale;
        let x0 = ((p0.x - 0.5).ceil() as i32).clamp(0, w);
        let x1 = ((p1.x - 0.5).ceil() as i32).clamp(0, w);
        let y0 = ((p0.y - 0.5).ceil() as i32).clamp(0, h);
        let y1 = ((p1.y - 0.5).ceil() as i32).clamp(0, h);
        let size = p1 - p0;
        for py in y0..y1 {
            let ty = (py as f32 + 0.5 - p0.y) / size.y;
            let v = q.uv_min.y + ty * (q.uv_max.y - q.uv_min.y);
            for px in x0..x1 {
                let tx = (px as f32 + 0.5 - p0.x) / size.x;
                let u = q.uv_min.x + tx * (q.uv_max.x - q.uv_min.x);
                let texel = match q.texture {
                    UiTexture::White => Vec4::ONE,
                    UiTexture::Font => sample_nearest(font_atlas, u, v),
                    UiTexture::Texture(_) => Vec4::new(1.0, 0.0, 1.0, 1.0),
                };
                let src = texel * q.color;
                if src.w <= 0.0 {
                    continue;
                }
                let i = ((py * w + px) * 4) as usize;
                blend(&mut target.rgba[i..i + 4], src);
            }
        }
    }
}

fn sample_nearest(tex: &TextureData, u: f32, v: f32) -> Vec4 {
    if tex.width == 0 || tex.height == 0 {
        return Vec4::ZERO;
    }
    let x = ((u * tex.width as f32).floor() as i64).clamp(0, tex.width as i64 - 1) as u32;
    let y = ((v * tex.height as f32).floor() as i64).clamp(0, tex.height as i64 - 1) as u32;
    let i = ((y * tex.width + x) * 4) as usize;
    match tex.rgba.get(i..i + 4) {
        Some(p) => Vec4::new(p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32) / 255.0,
        None => Vec4::ZERO,
    }
}

fn blend(dst: &mut [u8], src: Vec4) {
    let d = Vec4::new(dst[0] as f32, dst[1] as f32, dst[2] as f32, dst[3] as f32) / 255.0;
    // Straight-alpha "over".
    let a = src.w.clamp(0.0, 1.0);
    let out_a = a + d.w * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    let rgb = (src.truncate() * a + d.truncate() * d.w * (1.0 - a)) / out_a;
    let out = rgb.extend(out_a).clamp(Vec4::ZERO, Vec4::ONE) * 255.0;
    dst.copy_from_slice(&[
        out.x.round() as u8,
        out.y.round() as u8,
        out.z.round() as u8,
        out.w.round() as u8,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::font;
    use crate::ui::text::{TextStyle, draw_glyph, draw_text};

    fn px(img: &TextureData, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * img.width + x) * 4) as usize;
        [
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        ]
    }

    #[test]
    fn rect_scaling_and_blending() {
        let atlas = font::build_atlas();
        let mut list = UiDrawList::new(Vec2::new(64.0, 36.0));
        list.rect(
            Vec2::new(1.0, 1.0),
            Vec2::new(3.0, 2.0),
            Vec4::new(1.0, 0.0, 0.0, 1.0),
        );
        list.rect(
            Vec2::new(2.0, 1.0),
            Vec2::new(4.0, 2.0),
            Vec4::new(0.0, 0.0, 1.0, 0.5),
        );
        let img = rasterize(&list, &atlas, 128, 72);
        assert_eq!(px(&img, 1, 2), [0, 0, 0, 0]);
        assert_eq!(px(&img, 2, 2), [255, 0, 0, 255]);
        assert_eq!(px(&img, 3, 3), [255, 0, 0, 255]);
        assert_eq!(px(&img, 5, 2), [128, 0, 128, 255]);
        assert_eq!(px(&img, 7, 3), [0, 0, 255, 128]);
        assert_eq!(px(&img, 8, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn glyph_pixels_match_font() {
        let atlas = font::build_atlas();
        let mut list = UiDrawList::new(Vec2::new(64.0, 36.0));
        draw_glyph(&mut list, Vec2::new(2.0, 3.0), 'R', 1.0, Vec4::ONE);
        // 2 pixels per canvas unit: every glyph pixel becomes a 2x2 block.
        let img = rasterize(&list, &atlas, 128, 72);
        for y in 0..9 {
            for x in 0..6 {
                let ink = font::glyph_pixel('R', x, y);
                for (sx, sy) in [(0, 0), (1, 1)] {
                    let p = px(&img, 4 + x * 2 + sx, 6 + y * 2 + sy);
                    assert_eq!(p[3] == 255, ink, "R pixel {x},{y}");
                }
            }
        }
    }

    #[test]
    fn non_integer_scale_does_not_bleed() {
        // 'w' sits right below 'g' in the atlas; without padding, nearest sampling at odd
        // scales would pull g's descender into w's blank top rows.
        let atlas = font::build_atlas();
        for scale in [1.13, 1.37, 2.71] {
            for offset in [0.0, 0.3, 0.77] {
                let mut list = UiDrawList::new(Vec2::new(640.0, 360.0));
                let pos = Vec2::splat(10.0 + offset);
                draw_text(&mut list, pos, "w", &TextStyle::default().scale(scale));
                let img = rasterize(&list, &atlas, 1280, 720);
                let top = (pos.y * 2.0) as u32;
                let blank_rows_px = (2.0 * scale * 2.0) as u32 - 1;
                for y in top..top + blank_rows_px {
                    for x in 0..img.width {
                        assert_eq!(px(&img, x, y)[3], 0, "scale {scale} offset {offset} y {y}");
                    }
                }
            }
        }
    }
}
