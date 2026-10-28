//! Shared look of the menu screens: the VELOCIDE title block, the shaded menu column, hint
//! bars and modal message boxes.

use engine::gfx::UiDrawList;
use engine::ui::draw::{self, fade, mix};
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, vec2};

/// Big title with a stacked crimson extrusion, an underline slab and a tagline. Returns the
/// bottom y of the block.
pub fn title(list: &mut UiDrawList, t: &Theme, pos: Vec2, scale: f32, tagline: &str) -> f32 {
    // Solid extrusion: one copy per canvas unit, darkening towards the back. It reaches past
    // the title's 1-font-pixel outline so it shows.
    let depth = (scale * 2.0).round().max(2.0) as i32;
    for k in (1..=depth).rev() {
        let o = k as f32;
        let shade = mix(t.crimson, t.crimson_dark, k as f32 / depth as f32);
        list.text(
            pos + vec2(o, o),
            shared::GAME_NAME,
            &TextStyle::new(shade).scale(scale),
        );
    }
    let style = TextStyle::new(t.bone)
        .scale(scale)
        .outline(fade(t.ink, 0.9));
    let r = list.text(pos, shared::GAME_NAME, &style);
    let y = r.max.y + scale;
    draw::slanted(
        list,
        Rect::new(vec2(r.min.x, y), vec2(r.max.x + 40.0, y + 4.0)),
        true,
        t.crimson,
    );
    draw::hline(
        list,
        r.min.x,
        r.max.x + 120.0,
        y + 6.0,
        fade(t.crimson_bright, 0.6),
    );
    let tag = TextStyle::new(t.bone_dim).shadow(t.shadow);
    list.text(vec2(r.min.x, y + 12.0), tagline, &tag);
    y + 24.0
}

/// Dark gradient behind the left menu column so text reads over the 3D backdrop, with a
/// crimson edge line.
pub fn left_shade(list: &mut UiDrawList, t: &Theme, canvas: Rect, width: f32) {
    let steps = 12;
    for i in 0..steps {
        let x0 = canvas.min.x + width * i as f32 / steps as f32;
        let x1 = canvas.min.x + width * (i + 1) as f32 / steps as f32;
        let a = 0.78 * (1.0 - (i as f32 / steps as f32).powf(2.2));
        list.fill(
            Rect::new(vec2(x0, canvas.min.y), vec2(x1, canvas.max.y)),
            fade(t.ink, a),
        );
    }
    // Scanline texture over the column.
    let mut y = canvas.min.y;
    while y < canvas.max.y {
        draw::hline(
            list,
            canvas.min.x,
            canvas.min.x + width * 0.7,
            y,
            fade(t.ink, 0.25),
        );
        y += 3.0;
    }
}

/// Bottom hint line (left) and build tag (right).
pub fn hints(list: &mut UiDrawList, t: &Theme, canvas: Rect, left: &str, right: &str) {
    let hint = TextStyle::new(t.ash).shadow(t.shadow);
    list.text(vec2(canvas.min.x + 24.0, canvas.max.y - 16.0), left, &hint);
    list.text(
        vec2(canvas.max.x - 12.0, canvas.max.y - 16.0),
        right,
        &hint.right(),
    );
}

/// Version / build text: `build-42 (1a2b3c4) // PROTOCOL 7` (`dev` for dev builds).
pub fn build_tag() -> String {
    format!(
        "{} // PROTOCOL {}",
        shared::build_info::label(),
        shared::protocol::PROTOCOL_VERSION
    )
}
