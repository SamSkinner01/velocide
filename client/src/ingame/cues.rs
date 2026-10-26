//! Non-colour accessibility cues (`palette::shape_cues()`, settings > ACCESSIBILITY > SHAPE
//! CUES): things colour alone used to say, said again with shapes.
//!
//! - **Player markers**: every player slot has a [`MarkerShape`] drawn wherever its colour
//!   swatch goes (name tags, scoreboard, lobby, results, the team list), see
//!   [`player_swatch`].
//! - **Parry icon**: a strobing `!` diamond over every enemy in its parryable window
//!   ([`parry_icons`]), so the window no longer depends on telling the strobe colour from the
//!   attack's wind-up glow.
//! - Health popups (`+12` / `-8`) live in `hud.rs`; hard damage is always hatched
//!   (`engine::ui::hud::segmented_bar`).

use engine::gfx::{Camera, UiDrawList};
use engine::ui::Rect;
use engine::ui::draw::{fade, fill};
use glam::{Vec2, Vec3, Vec4, vec2};
use shared::protocol::{EntityState, enemy_flags, kind};

use super::hud::project;
use crate::palette::{MarkerShape, Palette, marker_shape, shape_cues};

/// Draws a pixel mask (`rows[y]` bit `w - 1 - x` = pixel (x, y)) with `unit`-sized pixels and
/// a one-unit dark outline around the silhouette (skipped when `outline` is transparent).
fn draw_mask(
    list: &mut UiDrawList,
    top_left: Vec2,
    rows: &[u16],
    w: u32,
    unit: f32,
    color: Vec4,
    outline: Vec4,
) {
    let pixels = || {
        rows.iter().enumerate().flat_map(move |(y, bits)| {
            (0..w)
                .filter(move |x| (bits >> (w - 1 - x)) & 1 == 1)
                .map(move |x| top_left + vec2(x as f32, y as f32) * unit)
        })
    };
    if outline.w > 0.0 {
        for p in pixels() {
            let r = Rect::from_min_size(p - Vec2::ONE, Vec2::splat(unit + 2.0));
            fill(list, r, outline);
        }
    }
    for p in pixels() {
        fill(list, Rect::from_min_size(p, Vec2::splat(unit)), color);
    }
}

/// A player's marker shape, 5x5 pixels of `unit` canvas units at `top_left`, outlined.
pub fn draw_marker(
    list: &mut UiDrawList,
    top_left: Vec2,
    shape: MarkerShape,
    unit: f32,
    color: Vec4,
    outline: Vec4,
) {
    let rows = shape.mask().map(u16::from);
    draw_mask(list, top_left.round(), &rows, 5, unit, color, outline);
}

/// The swatch beside a player's name: a plain `bar` of `color` at `pos`, or, with shape cues
/// on, slot `slot`'s marker shape (5x5 units, centred on the bar's height).
pub fn player_swatch(
    list: &mut UiDrawList,
    pos: Vec2,
    bar: Vec2,
    slot: usize,
    color: Vec4,
    outline: Vec4,
) {
    if shape_cues() {
        let y = pos.y + ((bar.y - 5.0) * 0.5).round();
        draw_marker(
            list,
            vec2(pos.x, y),
            marker_shape(slot),
            1.0,
            color,
            outline,
        );
    } else {
        fill(list, Rect::from_min_size(pos, bar), color);
    }
}

/// `!` in a diamond, 9x9 pixels.
const PARRY_ICON: [u16; 9] = [
    0b000010000,
    0b000111000,
    0b001101100,
    0b011010110,
    0b110010011,
    0b011000110,
    0b001101100,
    0b000111000,
    0b000010000,
];

/// The parry icon centred on `center` (9x9 pixels of `unit`); strobes between the palette's
/// parry colour and white like the enemy's own glow.
pub fn draw_parry_icon(
    list: &mut UiDrawList,
    center: Vec2,
    unit: f32,
    alpha: f32,
    p: &Palette,
    ink: Vec4,
    time: f64,
) {
    let strobe = (time * 18.0).fract() < 0.5;
    let color = if strobe { p.parry_window } else { Vec4::ONE };
    let top_left = (center - Vec2::splat(4.5 * unit)).round();
    draw_mask(
        list,
        top_left,
        &PARRY_ICON,
        9,
        unit,
        fade(color, alpha),
        fade(ink, 0.85 * alpha),
    );
}

/// An enemy in the parryable phase of an attack.
pub fn in_parry_window(e: &EntityState) -> bool {
    kind::is_enemy(e.kind)
        && e.flags & enemy_flags::PARRYABLE != 0
        && e.flags & enemy_flags::DYING == 0
}

/// Icons stay below this canvas y (the level timer and status line sit above it).
const ICON_TOP: f32 = 40.0;

/// A parry icon over every enemy [`in_parry_window`]. `height(kind)` is the enemy's height:
/// the icon floats above its head, or, when the head is above the screen (a big enemy up
/// close), is held at the top of the view over the enemy. Icons halve in size beyond 14 m
/// and fade out from 40 to 60 m. Call it when `palette::shape_cues()` is on.
pub fn parry_icons<'a>(
    list: &mut UiDrawList,
    camera: &Camera,
    entities: impl IntoIterator<Item = &'a EntityState>,
    height: &dyn Fn(u16) -> Option<f32>,
    p: &Palette,
    ink: Vec4,
    time: f64,
) {
    let canvas = list.canvas_size;
    for e in entities.into_iter().filter(|e| in_parry_window(e)) {
        let h = height(e.kind).unwrap_or(2.0);
        let top = e.position + Vec3::Y * (h + 0.5);
        let body = e.position + Vec3::Y * (h * 0.5);
        let Some(at) = project(camera, canvas, top).or_else(|| project(camera, canvas, body))
        else {
            continue;
        };
        let dist = camera.position.distance(body);
        let alpha = (1.0 - (dist - 40.0) / 20.0).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            continue;
        }
        let unit = if dist < 14.0 { 2.0 } else { 1.0 };
        let half = 4.5 * unit + 1.0;
        let center = vec2(
            at.x.clamp(half, canvas.x - half),
            (at.y - 6.0 * unit).clamp(ICON_TOP + half, canvas.y - half),
        );
        draw_parry_icon(list, center, unit, alpha, p, ink, time);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::color_vision::ColorVisionMode;
    use shared::protocol::NetId;

    fn enemy(position: Vec3, flags: u32) -> EntityState {
        EntityState {
            id: NetId(7),
            kind: kind::ENEMY_FIRST,
            position,
            flags,
            ..Default::default()
        }
    }

    #[test]
    fn markers_draw_their_mask() {
        let mut list = UiDrawList::new(vec2(640.0, 360.0));
        let cross = MarkerShape::Cross;
        draw_marker(
            &mut list,
            vec2(10.0, 10.0),
            cross,
            1.0,
            Vec4::ONE,
            Vec4::ZERO,
        );
        assert_eq!(list.quads.len(), 9, "a plus of 9 pixels, no outline");
        let mut list = UiDrawList::new(vec2(640.0, 360.0));
        let ink = Vec4::new(0.0, 0.0, 0.0, 1.0);
        draw_marker(
            &mut list,
            vec2(10.0, 10.0),
            MarkerShape::Square,
            2.0,
            Vec4::ONE,
            ink,
        );
        assert_eq!(list.quads.len(), 50, "25 outline + 25 fill");
        let right = list.quads.iter().map(|q| q.max.x).fold(0.0, f32::max);
        assert_eq!(
            right, 21.0,
            "the outline reaches one unit past the 10-unit shape"
        );
    }

    #[test]
    fn parry_icon_only_over_enemies_in_their_window() {
        let camera = Camera {
            position: Vec3::new(0.0, 1.6, 0.0),
            ..Default::default()
        };
        let p = Palette::for_mode(ColorVisionMode::Deuteranopia);
        let ahead = Vec3::new(0.0, 0.0, -10.0);
        let count = |entities: &[EntityState]| {
            let mut list = UiDrawList::new(vec2(640.0, 360.0));
            parry_icons(
                &mut list,
                &camera,
                entities,
                &|_| Some(2.0),
                p,
                Vec4::ONE,
                0.0,
            );
            list.quads
        };
        assert!(count(&[enemy(ahead, 0)]).is_empty());
        let icon = count(&[enemy(ahead, enemy_flags::PARRYABLE)]);
        assert!(!icon.is_empty());
        // Strobe phase 0 is the palette colour, above the enemy, centred.
        assert!(icon.iter().any(|q| q.color == p.parry_window));
        let x = icon.iter().map(|q| q.min.x).fold(f32::MAX, f32::min);
        let y = icon.iter().map(|q| q.max.y).fold(0.0, f32::max);
        assert!((x - 320.0).abs() < 12.0 && y < 180.0, "icon at {x}, {y}");
        let dying = enemy_flags::PARRYABLE | enemy_flags::DYING;
        assert!(count(&[enemy(ahead, dying)]).is_empty());
        let behind = enemy(Vec3::new(0.0, 0.0, 10.0), enemy_flags::PARRYABLE);
        assert!(count(&[behind]).is_empty());
        let far = enemy(Vec3::new(0.0, 0.0, -80.0), enemy_flags::PARRYABLE);
        assert!(count(&[far]).is_empty());
        let player = EntityState {
            kind: kind::PLAYER,
            ..enemy(ahead, enemy_flags::PARRYABLE)
        };
        assert!(count(&[player]).is_empty());
        // A 3.7 m heavy right in front: its head is off screen, the icon stays in view and
        // below the timer.
        let close = enemy(Vec3::new(0.0, 0.0, -3.0), enemy_flags::PARRYABLE);
        let mut list = UiDrawList::new(vec2(640.0, 360.0));
        parry_icons(
            &mut list,
            &camera,
            &[close],
            &|_| Some(3.7),
            p,
            Vec4::ONE,
            0.0,
        );
        assert!(!list.quads.is_empty());
        let top = list.quads.iter().map(|q| q.min.y).fold(f32::MAX, f32::min);
        assert!(top >= ICON_TOP, "icon top {top}");
    }
}
