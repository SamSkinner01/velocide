//! The knacks (docs/refit.md "Knacks", area **arsenal-client**): what you see and hear of the
//! Q slot.
//!
//! * The HUD's **knack slot** ([`draw_slot`]): an icon per knack, its name, charge pips and the
//!   recharge of the next charge (it replaces the thorn pips; the Thorn is a knack too).
//! * **Kindle Flask**: the flask tumbling through the air, then the kindling pool where it
//!   broke ([`pool_effects`]: embers rising in the heal colour, a ring at its edge, a warm light,
//!   a soft crackle).
//! * **Snare**: the wax trap on the floor ([`draw_snare`]: its jaws open, the wick lamp dull
//!   while it arms and pulsing once armed, its trigger ring for its owner's side), snapping shut
//!   when it springs ([`snare_spring`]).
//! * **Soot Pot**: the pot bursting into a soot cloud ([`soot_effects`]) dense enough to hide
//!   what is inside or behind it: big, dark, near-opaque billows kept topped up over its whole
//!   volume while it lasts, thinning as it ends.
//! * **Tallow Skin**: a waxy sheen at the screen's edges with drips running down while yours
//!   lasts ([`tallow_hud`]); other players' Shrikes glisten pale and waxy ([`tallow_tint`]).
//! * **Surge**: a streak behind the body (the Lunge's, `melee::MeleeView::streak`) and speed
//!   lines at the screen's edges for you ([`surge_hud`]).
//!
//! [`KnackPreview`] / `InGame::preview_knack` stage any of it client-side (tools:
//! `combat_shots`' `knack_*.png`).

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight, UiDrawList};
use engine::ui::draw::fade;
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4, vec2};

use std::collections::HashMap;

use shared::protocol::{EntityState, NetId, kind, player_flags, projectile_flags};
use shared::sim::PlayerId;
use shared::tuning::CombatTuning;

use super::super::InGame;
use super::super::effects::{DecalKind, Effects, FxTex, Particle};
use super::super::viewmodel::Viewmodel;
use super::Rng;
use super::entities::EntityFrame;
use super::fx::{Beams, FxCtx};
use super::melee::lunge_streak;
use crate::models::{ModelId, ModelLibrary, Tint, knacks as props};

pub use shared::loadout::Knack;

// ---------------------------------------------------------------------------------------------
// HUD

/// What the knack slot shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnackSlot {
    pub knack: Knack,
    /// Charges ready.
    pub charges: u32,
    pub max_charges: u32,
    /// Progress 0..1 of the next charge (0 when full).
    pub recharge: f32,
    /// The knack's effect running on you (Tallow Skin's time left 0..1), drawn as a frame
    /// round the icon.
    pub active: f32,
}

/// The icons, 12 x 12: `#` the main colour, `+` the accent, `.` empty.
fn icon(knack: Knack) -> [&'static str; 12] {
    match knack {
        // A barbed glass spike, point up-right.
        Knack::Thorn => [
            "..........##",
            ".........##.",
            "........##..",
            ".......##...",
            "...+..##....",
            "....+##.....",
            "....##+.....",
            "...##..+....",
            "..##........",
            ".##.........",
            "##..........",
            "#...........",
        ],
        // A round flask in its cage, embers inside.
        Knack::KindleFlask => [
            "....####....",
            ".....##.....",
            ".....##.....",
            "...######...",
            "..#......#..",
            ".#..+..+..#.",
            ".#...++...#.",
            ".#..++++..#.",
            ".#...++...#.",
            "..#......#..",
            "...######...",
            "............",
        ],
        // Open jaws with teeth over a wax disc.
        Knack::Snare => [
            "............",
            "#.#.#..#.#.#",
            "#.#.#..#.#.#",
            "############",
            "#....++....#",
            ".#...++...#.",
            "..##....##..",
            "....####....",
            "............",
            "############",
            ".##########.",
            "............",
        ],
        // A pot with soot billowing from its neck.
        Knack::SootPot => [
            "..++....++..",
            ".++++.+++++.",
            "..++++++++..",
            "....+++.....",
            ".....##.....",
            "....####....",
            "..########..",
            ".##########.",
            ".##########.",
            ".##########.",
            "..########..",
            "...######...",
        ],
        // A drop of tallow over a shield.
        Knack::TallowSkin => [
            ".....++.....",
            "....++++....",
            "...++++++...",
            "..##++++##..",
            ".###++++###.",
            ".####++####.",
            ".##########.",
            ".##########.",
            "..########..",
            "...######...",
            "....####....",
            ".....##.....",
        ],
        // Two chevrons and a streak.
        Knack::Surge => [
            "............",
            "#....#......",
            ".#....#.....",
            "..#....#....",
            "...#....#...",
            "+++##+++##++",
            "+++##+++##++",
            "...#....#...",
            "..#....#....",
            ".#....#.....",
            "#....#......",
            "............",
        ],
    }
}

/// A knack's accent colour (icon details, its world effects' tint where nothing more
/// meaningful applies): the palette's heal for the flask, hazard for the snare.
pub fn accent(knack: Knack) -> Vec4 {
    let p = crate::palette::palette();
    match knack {
        Knack::Thorn => Vec4::new(1.0, 0.8, 0.4, 1.0),
        Knack::KindleFlask => p.heal,
        Knack::Snare => p.hazard,
        Knack::SootPot => Vec4::new(0.62, 0.6, 0.58, 1.0),
        Knack::TallowSkin => Vec4::new(1.0, 0.92, 0.68, 1.0),
        Knack::Surge => Vec4::new(1.0, 0.62, 0.25, 1.0),
    }
}

/// Draws an icon `size` canvas units square at `at` (top left).
pub fn draw_icon(list: &mut UiDrawList, knack: Knack, at: Vec2, size: f32, main: Vec4, acc: Vec4) {
    let px = size / 12.0;
    for (y, row) in icon(knack).iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            let color = match c {
                b'#' => main,
                b'+' => acc,
                _ => continue,
            };
            let min = at + vec2(x as f32, y as f32) * px;
            list.fill(Rect::new(min, min + Vec2::splat(px)), color);
        }
    }
}

/// The knack slot in the bottom-right corner, above the weapon block (where the thorn pips
/// were): the icon in a frame with its key, the name, a pip per charge (the next one filling
/// as it recharges). Empty: the icon dims and refills from the bottom.
pub fn draw_slot(list: &mut UiDrawList, t: &Theme, c: Rect, slot: &KnackSlot, now: f64) {
    let outline = fade(t.ink, 0.9);
    let size = 14.0;
    let frame = Rect::from_min_size(
        vec2(c.max.x - 12.0 - size - 4.0, c.max.y - 80.0),
        Vec2::splat(size + 4.0),
    );
    let ready = slot.charges > 0;
    list.fill(frame, fade(t.ink, 0.65));
    let edge = if slot.active > 0.0 {
        let pulse = 0.6 + 0.4 * ((now * 6.0).sin() as f32).abs();
        fade(accent(slot.knack), pulse)
    } else if ready {
        fade(t.bone_dim, 0.9)
    } else {
        fade(t.ash, 0.8)
    };
    list.border(frame, 1.0, edge);
    let (main, acc) = if ready {
        (t.bone, accent(slot.knack))
    } else {
        (fade(t.ash, 0.7), fade(accent(slot.knack), 0.4))
    };
    draw_icon(
        list,
        slot.knack,
        frame.min + Vec2::splat(2.0),
        size,
        main,
        acc,
    );
    if !ready && slot.recharge > 0.0 {
        // Refilling from the bottom of the frame.
        let h = (size + 2.0) * slot.recharge.clamp(0.0, 1.0);
        list.fill(
            Rect::new(
                vec2(frame.min.x + 1.0, frame.max.y - 1.0 - h),
                frame.max - Vec2::ONE,
            ),
            fade(accent(slot.knack), 0.25),
        );
    }
    // A running effect drains a bar along the frame's top.
    if slot.active > 0.0 {
        let w = frame.width() * slot.active.clamp(0.0, 1.0);
        list.fill(
            Rect::new(
                frame.min - vec2(0.0, 3.0),
                vec2(frame.min.x + w, frame.min.y - 1.0),
            ),
            accent(slot.knack),
        );
    }
    list.text(
        vec2(frame.min.x - 4.0, frame.min.y),
        &slot.knack.name().to_uppercase(),
        &TextStyle::new(if ready { t.bone } else { t.bone_dim })
            .right()
            .outline(outline),
    );
    list.text(
        frame.max + vec2(-2.0, -7.0),
        "Q",
        &TextStyle::new(fade(t.bone_dim, 0.8)).outline(outline),
    );
    // Pips: one per charge, the next one filling with the recharge.
    let pip = vec2(6.0, 3.0);
    let gap = 3.0;
    let max = slot.max_charges.clamp(1, 6);
    let mut x = frame.min.x - 4.0 - max as f32 * (pip.x + gap) + gap;
    let y = frame.min.y + 10.0;
    for i in 0..max {
        let r = Rect::from_min_size(vec2(x, y), pip);
        list.fill(r.expand(1.0), fade(t.ink, 0.7));
        if i < slot.charges {
            list.fill(r, accent(slot.knack));
        } else if i == slot.charges {
            let w = pip.x * slot.recharge.clamp(0.0, 1.0);
            list.fill(
                Rect::new(r.min, vec2(r.min.x + w, r.max.y)),
                fade(t.bone_dim, 0.8),
            );
        }
        x += pip.x + gap;
    }
}

/// Tallow Skin on you (`strength` = time left 0..1): a pale waxy sheen at the screen's edges,
/// drips of tallow running down from the top and a slow glint.
pub fn tallow_hud(list: &mut UiDrawList, c: Rect, strength: f32, now: f64) {
    if strength <= 0.0 {
        return;
    }
    let wax = Vec4::new(1.0, 0.93, 0.72, 1.0);
    let k = strength.clamp(0.0, 1.0).sqrt();
    let bands = 8;
    for i in 0..bands {
        let inset = i as f32 * 4.0;
        let a = 0.42 * k * (1.0 - i as f32 / bands as f32).powf(1.5);
        let r = Rect::new(c.min + Vec2::splat(inset), c.max - Vec2::splat(inset));
        list.border(r, 4.0, fade(wax, a));
    }
    // Drips: a dozen runs down from the top edge, each at its own pace.
    for d in 0..12 {
        let seed = d as f64 * 1.618;
        let x = c.min.x + (c.width() * ((seed * 0.37).fract() as f32 * 0.9 + 0.05)).round();
        let speed = 18.0 + 22.0 * (seed * 0.71).fract() as f32;
        let len = ((now + seed * 3.0) as f32 * speed).rem_euclid(90.0) * k;
        let w = 2.0 + (d % 3) as f32;
        list.fill(
            Rect::new(vec2(x, c.min.y), vec2(x + w, c.min.y + len)),
            fade(wax, 0.35 * k),
        );
        list.fill(
            Rect::from_center_size(vec2(x + w * 0.5, c.min.y + len), Vec2::splat(w + 2.0)),
            fade(wax, 0.45 * k),
        );
    }
    // A glint travelling round the edge.
    let g = ((now * 0.6).fract() as f32) * 2.0 * (c.width() + c.height());
    let p = if g < c.width() {
        vec2(c.min.x + g, c.min.y + 3.0)
    } else if g < c.width() + c.height() {
        vec2(c.max.x - 3.0, c.min.y + g - c.width())
    } else if g < 2.0 * c.width() + c.height() {
        vec2(c.max.x - (g - c.width() - c.height()), c.max.y - 3.0)
    } else {
        vec2(c.min.x + 3.0, c.max.y - (g - 2.0 * c.width() - c.height()))
    };
    list.fill(
        Rect::from_center_size(p, Vec2::splat(6.0)),
        fade(Vec4::ONE, 0.5 * k),
    );
}

/// Surging (`strength` 1 = just started → 0): speed lines streaming in from the screen's sides.
pub fn surge_hud(list: &mut UiDrawList, c: Rect, strength: f32, now: f64) {
    if strength <= 0.0 {
        return;
    }
    let color = accent(Knack::Surge);
    let center = c.center();
    for i in 0..18 {
        let seed = i as f64 * 2.399;
        let a = (seed % std::f64::consts::TAU) as f32;
        let dir = vec2(a.cos(), a.sin() * 0.6);
        let phase = ((now * 3.5 + seed).fract()) as f32;
        let r0 = c.width() * (0.32 + 0.3 * phase);
        let len = 40.0 + 50.0 * strength;
        let from = center + dir * r0;
        let steps = 10;
        for s in 0..steps {
            let p = from + dir * (len * s as f32 / steps as f32);
            list.fill(
                Rect::from_center_size(p, Vec2::splat(3.0)),
                fade(color, 0.85 * strength * (s as f32 / steps as f32)),
            );
        }
    }
}

/// The body tint of a Shrike under Tallow Skin (`strength` 0..1): paler and glossier, with a
/// slow glisten.
pub fn tallow_tint(strength: f32, time: f64) -> Vec4 {
    let k = strength.clamp(0.0, 1.0);
    let glint = 0.08 * ((time * 5.0).sin() as f32);
    Vec4::new(
        1.0 + (0.32 + glint) * k,
        1.0 + (0.26 + glint) * k,
        1.0 + 0.08 * k,
        1.0,
    )
}

// ---------------------------------------------------------------------------------------------
// The things in the world

/// A thrown knack in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Thrown {
    Flask,
    Snare,
    SootPot,
}

impl Thrown {
    pub fn model(self) -> ModelId {
        match self {
            Thrown::Flask => ModelId::KindleFlask,
            Thrown::Snare => ModelId::Snare,
            Thrown::SootPot => ModelId::SootPot,
        }
    }

    /// Drawn size (the props are modelled at life size; thrown, they read better larger).
    fn scale(self) -> f32 {
        match self {
            Thrown::Snare => 0.6,
            _ => 1.4,
        }
    }
}

/// Draws a thrown knack at `position`, tumbling `age` seconds into its flight.
pub fn draw_thrown(
    lib: &ModelLibrary,
    thing: Thrown,
    position: Vec3,
    velocity: Vec3,
    age: f32,
    out: &mut Vec<DrawItem>,
) {
    let dir = velocity.normalize_or(Vec3::NEG_Z);
    let face = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    let tumble = Quat::from_rotation_x(age * 9.0) * Quat::from_rotation_z(age * 3.0);
    let root =
        Mat4::from_scale_rotation_translation(Vec3::splat(thing.scale()), face * tumble, position);
    let mut pose = lib.get(thing.model()).rest_pose();
    pose.glow = Vec4::new(1.4, 1.4, 1.4, 1.0);
    lib.get(thing.model())
        .draw(&pose, root, &Tint::default(), out);
}

/// A thrown knack's wake: kindling sparks behind the flask, a sputter from the pot's fuse.
pub fn thrown_trail(effects: &mut Effects, rng: &mut Rng, thing: Thrown, position: Vec3, dt: f32) {
    let (color, rate) = match thing {
        Thrown::Flask => (accent(Knack::KindleFlask), 40.0),
        Thrown::SootPot => (Vec4::new(1.0, 0.7, 0.3, 1.0), 30.0),
        Thrown::Snare => return,
    };
    if rng.unit() < dt * rate {
        effects.spawn(Particle {
            position,
            velocity: rng.dir() * 0.8,
            gravity: -0.5,
            max_life: 0.35,
            size: Vec2::splat(0.06),
            color,
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// The flask breaking at `at`: glass, a burst of kindling in the heal colour, the sound.
pub fn flask_shatter(effects: &mut Effects, audio: &mut Audio, rng: &mut Rng, at: Vec3) {
    let heal = accent(Knack::KindleFlask);
    for _ in 0..14 {
        effects.spawn(Particle {
            position: at + Vec3::Y * 0.1,
            velocity: (rng.dir() + Vec3::Y * 0.8) * rng.range(1.5, 4.0),
            gravity: 12.0,
            max_life: rng.range(0.3, 0.6),
            size: Vec2::splat(0.05),
            color: Vec4::new(0.85, 1.0, 0.9, 1.0),
            end_alpha: 0.3,
            texture: FxTex::Shard,
            additive: true,
            rotation: rng.angle(),
            spin: rng.range(-10.0, 10.0),
            ..Default::default()
        });
    }
    effects.glow(at + Vec3::Y * 0.3, heal, 2.4, 0.35);
    audio.play(Sfx::FlaskShatter, PlayParams::at(at));
}

#[allow(clippy::too_many_arguments)]
/// Per-frame particles of a kindling pool of `radius` at `position` (on the floor; `left` =
/// fraction of its life remaining: it dies down at the end), and its rim.
pub fn pool_effects(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    position: Vec3,
    radius: f32,
    left: f32,
    dt: f32,
    time: f64,
) {
    let heal = accent(Knack::KindleFlask);
    let r = radius.max(0.5);
    let strength = left.clamp(0.0, 1.0).sqrt();
    // Kindling embers drifting up all over the pool.
    let mut budget = (dt * 110.0 * r / 3.5 * strength).max(0.0);
    while budget > 0.0 {
        if budget < 1.0 && rng.unit() > budget {
            break;
        }
        budget -= 1.0;
        let a = rng.angle();
        let d = rng.unit().sqrt() * r;
        effects.spawn(Particle {
            position: position + Vec3::new(a.cos() * d, 0.05, a.sin() * d),
            velocity: Vec3::Y * rng.range(0.6, 1.6) + rng.dir() * 0.2,
            gravity: -0.4,
            drag: 0.6,
            max_life: rng.range(0.6, 1.2),
            size: Vec2::splat(rng.range(0.05, 0.1)),
            color: heal,
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    // A soft glowing haze low over it.
    if rng.unit() < dt * 10.0 * strength {
        let a = rng.angle();
        let d = rng.unit().sqrt() * r * 0.8;
        effects.spawn(Particle {
            position: position + Vec3::new(a.cos() * d, 0.25, a.sin() * d),
            velocity: Vec3::Y * 0.3,
            max_life: rng.range(0.8, 1.4),
            size: Vec2::splat(r * 0.5),
            end_scale: 1.4,
            color: heal.truncate().extend(0.3 * strength),
            texture: FxTex::Glow,
            additive: true,
            ..Default::default()
        });
    }
    // The rim: a slowly turning ring of light on the floor.
    let rim = (heal.truncate() * 1.6).extend(0.75 * strength);
    let n = 20;
    let turn = (time * 0.4) as f32;
    let point = |k: usize| {
        let a = k as f32 / n as f32 * std::f32::consts::TAU + turn;
        position + Vec3::new(a.cos() * r, 0.06, a.sin() * r)
    };
    for k in 0..n {
        if k % 2 == 0 {
            beams.frame_beam(point(k), point(k + 1), 0.06, rim);
        }
    }
}

/// The light a kindling pool casts.
pub fn pool_light(position: Vec3, radius: f32, left: f32, time: f64) -> PointLight {
    let heal = accent(Knack::KindleFlask).truncate();
    let breathe = 0.85 + 0.15 * ((time * 2.3).sin() as f32);
    PointLight {
        position: position + Vec3::Y * 0.6,
        color: heal * 1.6 * breathe * left.clamp(0.0, 1.0).sqrt(),
        radius: radius * 2.0,
    }
}

/// Per-frame billows of a soot cloud of `radius` round `center` (`left` = life remaining
/// 0..1). Kept dense enough to hide players: a steady population of big, dark, near-opaque
/// billows filling the sphere (each lives ~1.5 s, so ~40 a second at full radius keep ~60
/// alive), plus darker ones at the core; it thins out over its last fifth.
pub fn soot_effects(
    effects: &mut Effects,
    rng: &mut Rng,
    center: Vec3,
    radius: f32,
    left: f32,
    dt: f32,
) {
    let r = radius.max(1.0);
    let thin = (left.clamp(0.0, 1.0) / 0.2).min(1.0);
    let rate = 40.0 * (r / 5.0).powi(2) * thin;
    let mut budget = (dt * rate).max(0.0);
    while budget > 0.0 {
        if budget < 1.0 && rng.unit() > budget {
            break;
        }
        budget -= 1.0;
        // Inside the sphere, biased towards the middle and the ground.
        let d = rng.dir() * rng.unit().powf(0.7) * r * 0.85;
        let at = center + Vec3::new(d.x, d.y.abs() * 0.55, d.z);
        let core = d.length() < r * 0.4;
        let shade = if core {
            rng.range(0.03, 0.06)
        } else {
            rng.range(0.06, 0.11)
        };
        effects.spawn(Particle {
            position: at,
            velocity: rng.dir() * 0.25 + Vec3::Y * 0.15,
            gravity: -0.05,
            drag: 0.5,
            max_life: rng.range(1.3, 1.9),
            size: Vec2::splat(rng.range(1.8, 2.8) * (r / 5.0).max(0.4)),
            end_scale: 1.35,
            color: Vec4::new(shade, shade * 0.95, shade * 0.92, 0.94 * thin.max(0.3)),
            end_alpha: 0.0,
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            spin: rng.range(-0.3, 0.3),
            ..Default::default()
        });
    }
}

/// The pot bursting at `center`: shards, a dark puff and the first billows thrown out.
pub fn soot_burst(effects: &mut Effects, audio: &mut Audio, rng: &mut Rng, center: Vec3) {
    for _ in 0..10 {
        effects.spawn(Particle {
            position: center,
            velocity: (rng.dir() + Vec3::Y) * rng.range(2.0, 5.0),
            gravity: 14.0,
            max_life: rng.range(0.4, 0.8),
            size: Vec2::splat(0.08),
            color: Vec4::new(0.25, 0.2, 0.18, 1.0),
            texture: FxTex::Lump,
            rotation: rng.angle(),
            spin: rng.range(-8.0, 8.0),
            ..Default::default()
        });
    }
    for _ in 0..24 {
        let d = rng.dir();
        effects.spawn(Particle {
            position: center + d * 0.3,
            velocity: d * rng.range(4.0, 8.0) + Vec3::Y,
            drag: 3.0,
            max_life: rng.range(1.2, 1.8),
            size: Vec2::splat(rng.range(1.0, 1.6)),
            end_scale: 2.2,
            color: Vec4::new(0.07, 0.065, 0.06, 0.9),
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            ..Default::default()
        });
    }
    effects.decal(center, Vec3::Y, 2.5, DecalKind::Scorch);
    audio.play(Sfx::SootBurst, PlayParams::at(center));
}

/// Draws a snare on the floor at `position` facing `yaw`: `armed` 0..1 (its arming time),
/// `sprung` 0..1 (the jaws snapping shut).
pub fn draw_snare(
    lib: &ModelLibrary,
    position: Vec3,
    yaw: f32,
    armed: f32,
    sprung: f32,
    time: f64,
    out: &mut Vec<DrawItem>,
) {
    let pose = props::snare_pose(armed, sprung, time);
    let root = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), position);
    let tint = Tint {
        glow_color: Some(accent(Knack::Snare)),
        ..Tint::default()
    };
    lib.get(ModelId::Snare).draw(&pose, root, &tint, out);
}

/// An armed snare's trigger ring (shown to its owner's side): a dashed circle of `radius`
/// slowly pulsing.
pub fn snare_ring(beams: &mut Beams, position: Vec3, radius: f32, time: f64) {
    let c = accent(Knack::Snare);
    let pulse = 0.5 + 0.3 * ((time * 3.0).sin() as f32);
    let n = 24;
    let point = |k: usize| {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        position + Vec3::new(a.cos() * radius, 0.05, a.sin() * radius)
    };
    for k in (0..n).step_by(2) {
        beams.frame_beam(
            point(k),
            point(k + 1),
            0.035,
            (c.truncate() * 1.3).extend(pulse),
        );
    }
}

/// A snare springing on what stood at `position`: a flash, chains whipping up, the sound.
pub fn snare_spring(effects: &mut Effects, beams: &mut Beams, audio: &mut Audio, position: Vec3) {
    let c = accent(Knack::Snare);
    effects.glow(position + Vec3::Y * 0.3, c, 1.6, 0.2);
    effects.sparks(position + Vec3::Y * 0.2, Vec3::Y, 12);
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU;
        let foot = position + Vec3::new(a.cos(), 0.05, a.sin()) * 0.5;
        beams.beam(
            foot,
            position + Vec3::Y * 0.9,
            0.04,
            (c.truncate() * 1.6).extend(0.9),
            0.25,
        );
    }
    audio.play(Sfx::SnareSpring, PlayParams::at(position));
}

// ---------------------------------------------------------------------------------------------
// Bookkeeping and previews

/// Something staged client-side by `InGame::preview_knack` (tools: look at the visuals without
/// the simulation making them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KnackPreview {
    /// A thrown knack flying from `from` (gravity `gravity`) that bursts / settles where it
    /// meets the level: a flask leaves a pool, a pot a cloud, a snare lies down and arms.
    Throw {
        thing: Thrown,
        from: Vec3,
        velocity: Vec3,
        gravity: f32,
    },
    Pool {
        position: Vec3,
        radius: f32,
        seconds: f32,
    },
    Cloud {
        position: Vec3,
        radius: f32,
        seconds: f32,
    },
    /// A snare on the floor that arms after `arm` seconds and springs after `spring`.
    Snare {
        position: Vec3,
        arm: f32,
        spring: Option<f32>,
    },
    /// Tallow Skin on you for `seconds`.
    TallowSelf { seconds: f32 },
    /// Surging (you): the speed lines for `seconds`.
    SurgeSelf { seconds: f32 },
}

#[derive(Clone, Copy, Debug)]
struct Flying {
    thing: Thrown,
    position: Vec3,
    velocity: Vec3,
    gravity: f32,
    age: f32,
}

#[derive(Clone, Copy, Debug)]
struct Area {
    position: Vec3,
    radius: f32,
    age: f32,
    seconds: f32,
}

#[derive(Clone, Copy, Debug)]
struct Trap {
    position: Vec3,
    yaw: f32,
    age: f32,
    arm: f32,
    spring: Option<f32>,
    armed_sound: bool,
}

/// A replicated knack entity seen last frame.
#[derive(Clone, Copy, Debug)]
struct Seen {
    armed: bool,
    still: bool,
}

/// A replicated hazard on screen this frame (its light).
#[derive(Clone, Copy, Debug)]
struct LiveArea {
    position: Vec3,
    radius: f32,
    left: f32,
}

/// Knack visuals that live across frames.
#[derive(Default)]
pub struct KnackView {
    flying: Vec<Flying>,
    pools: Vec<Area>,
    clouds: Vec<Area>,
    snares: Vec<Trap>,
    staged: Vec<KnackPreview>,
    /// Replicated flasks, snares, pots, pools and clouds seen last frame (a pool or cloud
    /// appearing is where a flask or pot broke; a snare arming ticks).
    seen: HashMap<NetId, Seen>,
    /// Replicated pools this frame (lights).
    live_pools: Vec<LiveArea>,
    /// Snares that just sprang (the jaws shut for a moment where the entity was).
    sprung: Vec<(Vec3, f32)>,
    /// Your Tallow Skin / Surge (staged), seconds left and total.
    tallow: (f32, f32),
    surge: (f32, f32),
    /// Seconds to the next pool crackle.
    crackle: f32,
    time: f64,
}

/// What [`KnackView::update`] needs.
pub struct UpdateCtx<'a> {
    pub effects: &'a mut Effects,
    pub beams: &'a mut Beams,
    pub audio: &'a mut Audio,
    pub rng: &'a mut Rng,
    pub world: Option<&'a engine::physics::CollisionWorld>,
    pub dt: f32,
}

/// Radius a snare springs within (docs/refit.md: 1.8 m).
const SNARE_RADIUS: f32 = 1.8;
/// A Kindle pool's radius and a soot cloud's (docs/refit.md: 3.5 m, 5 m).
const POOL_RADIUS: f32 = 3.5;
const CLOUD_RADIUS: f32 = 5.0;

impl KnackView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self {
            time: self.time,
            ..Self::default()
        };
    }

    /// A predicted knack of the local player (`FireIntent::Knack`): the left hand throws its
    /// thing (a Thorn, the flask, the snare, the pot) or Tallow Skin / Surge start on you.
    pub fn local_knack(
        &mut self,
        knack: Knack,
        viewmodel: &mut Viewmodel,
        audio: &mut Audio,
        t: &CombatTuning,
    ) {
        match knack {
            Knack::Thorn => {
                viewmodel.throw_thorn();
                audio.play_2d(Sfx::ThornThrow);
            }
            Knack::KindleFlask | Knack::Snare | Knack::SootPot => {
                viewmodel.throw_item(thrown_of(knack).map(Thrown::model));
                audio.play_2d(Sfx::KnackThrow);
            }
            Knack::TallowSkin => {
                let d = t.knacks.tallow_skin.duration.max(0.1);
                self.tallow = (d, d);
                audio.play_2d(Sfx::TallowSkin);
            }
            // Felt, not watched: speed lines (others see the streak).
            Knack::Surge => {
                self.surge = (0.35, 0.35);
                audio.play_2d(Sfx::Surge);
            }
        }
    }

    /// Your Tallow Skin (0..1 left), for the HUD sheen.
    pub fn tallow_self(&self) -> f32 {
        if self.tallow.0 > 0.0 {
            self.tallow.0 / self.tallow.1.max(0.01)
        } else {
            0.0
        }
    }

    /// Your Surge (1 = just started), for the HUD's speed lines.
    pub fn surge_self(&self) -> f32 {
        if self.surge.0 > 0.0 {
            self.surge.0 / self.surge.1.max(0.01)
        } else {
            0.0
        }
    }

    fn preview(&mut self, p: KnackPreview, ctx: &mut UpdateCtx) {
        match p {
            KnackPreview::Throw {
                thing,
                from,
                velocity,
                gravity,
            } => {
                self.flying.push(Flying {
                    thing,
                    position: from,
                    velocity,
                    gravity,
                    age: 0.0,
                });
                ctx.audio.play(Sfx::KnackThrow, PlayParams::at(from));
            }
            KnackPreview::Pool {
                position,
                radius,
                seconds,
            } => self.pools.push(Area {
                position,
                radius,
                age: 0.0,
                seconds,
            }),
            KnackPreview::Cloud {
                position,
                radius,
                seconds,
            } => {
                soot_burst(ctx.effects, ctx.audio, ctx.rng, position);
                self.clouds.push(Area {
                    position,
                    radius,
                    age: 0.0,
                    seconds,
                });
            }
            KnackPreview::Snare {
                position,
                arm,
                spring,
            } => {
                ctx.audio.play(Sfx::SnareSet, PlayParams::at(position));
                self.snares.push(Trap {
                    position,
                    yaw: ctx.rng.angle(),
                    age: 0.0,
                    arm,
                    spring,
                    armed_sound: false,
                });
            }
            KnackPreview::TallowSelf { seconds } => {
                self.tallow = (seconds, seconds);
                ctx.audio.play_2d(Sfx::TallowSkin);
            }
            KnackPreview::SurgeSelf { seconds } => {
                self.surge = (seconds, seconds);
                ctx.audio.play_2d(Sfx::Surge);
            }
        }
    }

    /// Advances staged things: throws landing (a flask breaks into its pool, a pot into its
    /// cloud, a snare lies down), pools, clouds and snares burning / arming / springing.
    pub fn update(&mut self, ctx: &mut UpdateCtx) {
        let dt = ctx.dt.max(0.0);
        self.time += dt as f64;
        for p in std::mem::take(&mut self.staged) {
            self.preview(p, ctx);
        }
        self.tallow.0 = (self.tallow.0 - dt).max(0.0);
        self.surge.0 = (self.surge.0 - dt).max(0.0);

        let mut landed = Vec::new();
        for f in &mut self.flying {
            f.age += dt;
            let step = f.velocity * dt;
            let hit = ctx.world.and_then(|w| {
                let len = step.length();
                (len > 1e-5)
                    .then(|| w.cast_ray(f.position, step / len, len, engine::physics::MASK_SHOTS))
                    .flatten()
            });
            if let Some(h) = hit {
                landed.push((f.thing, h.point));
                f.age = f32::INFINITY;
                continue;
            }
            f.position += step;
            f.velocity.y -= f.gravity * dt;
            thrown_trail(ctx.effects, ctx.rng, f.thing, f.position, dt);
        }
        self.flying.retain(|f| f.age < 4.0);
        for (thing, at) in landed {
            match thing {
                Thrown::Flask => {
                    flask_shatter(ctx.effects, ctx.audio, ctx.rng, at);
                    self.pools.push(Area {
                        position: at,
                        radius: POOL_RADIUS,
                        age: 0.0,
                        seconds: 4.0,
                    });
                }
                Thrown::SootPot => {
                    soot_burst(ctx.effects, ctx.audio, ctx.rng, at);
                    self.clouds.push(Area {
                        position: at,
                        radius: CLOUD_RADIUS,
                        age: 0.0,
                        seconds: 7.0,
                    });
                }
                Thrown::Snare => {
                    ctx.audio.play(Sfx::SnareSet, PlayParams::at(at));
                    self.snares.push(Trap {
                        position: at,
                        yaw: ctx.rng.angle(),
                        age: 0.0,
                        arm: 0.6,
                        spring: None,
                        armed_sound: false,
                    });
                }
            }
        }

        self.crackle -= dt;
        let crackle = self.crackle <= 0.0 && !self.pools.is_empty();
        if crackle {
            self.crackle = 1.2;
        }
        for p in &mut self.pools {
            p.age += dt;
            let left = 1.0 - p.age / p.seconds.max(0.1);
            pool_effects(
                ctx.effects,
                ctx.beams,
                ctx.rng,
                p.position,
                p.radius,
                left,
                dt,
                self.time,
            );
            if crackle {
                ctx.audio.play(Sfx::KindlePool, PlayParams::at(p.position));
            }
        }
        self.pools.retain(|p| p.age < p.seconds);
        for c in &mut self.clouds {
            c.age += dt;
            let left = 1.0 - c.age / c.seconds.max(0.1);
            soot_effects(ctx.effects, ctx.rng, c.position, c.radius, left, dt);
        }
        self.clouds.retain(|c| c.age < c.seconds);
        for s in &mut self.snares {
            s.age += dt;
            if s.age >= s.arm && !s.armed_sound {
                s.armed_sound = true;
                ctx.audio.play(Sfx::SnareArmed, PlayParams::at(s.position));
            }
            if let Some(at) = s.spring
                && s.age >= at
                && s.age - dt < at
            {
                snare_spring(ctx.effects, ctx.beams, ctx.audio, s.position);
            }
            let sprung = s.spring.is_some_and(|at| s.age >= at);
            if s.age >= s.arm && !sprung {
                snare_ring(ctx.beams, s.position, SNARE_RADIUS, self.time);
            }
        }
        self.snares.retain(|s| s.age < 20.0);
    }

    /// The replicated knacks this frame: thrown things' wakes, a pool or cloud appearing
    /// (the flask / pot breaking there), pools burning and clouds billowing, a snare arming,
    /// Tallow Skin glinting on other players.
    pub fn update_replicated(&mut self, f: &EntityFrame, ctx: &mut UpdateCtx) {
        let dt = ctx.dt.max(0.0);
        for s in self.seen.values_mut() {
            s.still = false;
        }
        self.live_pools.clear();
        let rate = f.tick_rate.max(1) as f32;
        for e in f.entities {
            match e.kind {
                kind::KINDLE_POOL | kind::SOOT_CLOUD => {
                    let radius = (e.state as f32 / 10.0).max(0.5);
                    let (full, first) = if e.kind == kind::KINDLE_POOL {
                        (f.tuning.knacks.kindle_flask.duration, Thrown::Flask)
                    } else {
                        (f.tuning.knacks.soot_pot.duration, Thrown::SootPot)
                    };
                    let left = ((e.aux as f64 - f.render_tick) as f32 / (full * rate).max(1.0))
                        .clamp(0.0, 1.0);
                    if !self.seen.contains_key(&e.id) {
                        match first {
                            Thrown::Flask => {
                                flask_shatter(ctx.effects, ctx.audio, ctx.rng, e.position)
                            }
                            _ => soot_burst(ctx.effects, ctx.audio, ctx.rng, e.position),
                        }
                    }
                    if e.kind == kind::KINDLE_POOL {
                        pool_effects(
                            ctx.effects,
                            ctx.beams,
                            ctx.rng,
                            e.position,
                            radius,
                            left.max(0.15),
                            dt,
                            self.time,
                        );
                        self.live_pools.push(LiveArea {
                            position: e.position,
                            radius,
                            left: left.max(0.15),
                        });
                    } else {
                        soot_effects(ctx.effects, ctx.rng, e.position, radius, left, dt);
                    }
                    self.seen.insert(
                        e.id,
                        Seen {
                            armed: false,
                            still: true,
                        },
                    );
                }
                kind::SNARE if e.flags & projectile_flags::STUCK != 0 => {
                    let armed = e.flags & projectile_flags::ARMED != 0;
                    let was = self.seen.get(&e.id).copied();
                    if was.is_none() {
                        // Just landed (first seen lying on the floor).
                        ctx.audio.play(Sfx::SnareSet, PlayParams::at(e.position));
                    }
                    if armed && was.is_some_and(|s| !s.armed) {
                        ctx.audio.play(Sfx::SnareArmed, PlayParams::at(e.position));
                    }
                    if armed && f.local_owner == Some(e.owner) {
                        snare_ring(ctx.beams, e.position, SNARE_RADIUS, self.time);
                    }
                    self.seen.insert(e.id, Seen { armed, still: true });
                }
                // Tallow Skin: drips of wax glinting on the body.
                kind::PLAYER
                    if e.flags & player_flags::TALLOW != 0 && ctx.rng.unit() < dt * 10.0 =>
                {
                    let a = ctx.rng.angle();
                    ctx.effects.spawn(Particle {
                        position: e.position
                            + Vec3::new(a.cos() * 0.4, ctx.rng.range(0.4, 1.8), a.sin() * 0.4),
                        velocity: Vec3::NEG_Y * 0.4,
                        max_life: 0.4,
                        size: Vec2::splat(0.08),
                        color: accent(Knack::TallowSkin),
                        texture: FxTex::Spark,
                        additive: true,
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }
        // Thrown things in flight: their wakes (at the time they are drawn).
        for (e, time, offset) in f.projectiles() {
            let Some(thing) = thrown_kind(e.kind) else {
                continue;
            };
            if e.flags & projectile_flags::STUCK != 0 {
                continue;
            }
            let at = shared::sim::projectiles::replicated_position(e, time, f.tick_rate, f.tuning)
                + offset;
            thrown_trail(ctx.effects, ctx.rng, thing, at, dt);
        }
        self.seen.retain(|_, s| s.still);
        for s in &mut self.sprung {
            s.1 += dt;
        }
        self.sprung.retain(|s| s.1 < 0.8);
    }

    /// A snare sprang at `position` (`Fx::SnareSprung`): its jaws shut there for a moment.
    pub fn snare_sprung(&mut self, position: Vec3) {
        self.sprung.push((position, 0.0));
    }

    /// Draws the replicated thrown things (in flight, a snare on the floor) at the time the
    /// entity frame shows them.
    pub fn draw_replicated(
        &self,
        lib: Option<&ModelLibrary>,
        f: &EntityFrame,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        if let Some(lib) = lib {
            for (e, time, offset) in f.projectiles() {
                let Some(thing) = thrown_kind(e.kind) else {
                    continue;
                };
                let pos =
                    shared::sim::projectiles::replicated_position(e, time, f.tick_rate, f.tuning)
                        + offset;
                if pos.distance(f.camera) < 0.5 {
                    continue;
                }
                if thing == Thrown::Snare && e.flags & projectile_flags::STUCK != 0 {
                    let armed = if e.flags & projectile_flags::ARMED != 0 {
                        1.0
                    } else {
                        0.4
                    };
                    let yaw = (e.id.0 % 13) as f32 * 0.48;
                    draw_snare(lib, pos, yaw, armed, 0.0, self.time, draws);
                    continue;
                }
                let age = ((time - e.aux as f64) / f.tick_rate.max(1) as f64) as f32;
                draw_thrown(lib, thing, pos, e.velocity, age.max(0.0), draws);
            }
            for (at, age) in &self.sprung {
                draw_snare(lib, *at, 0.0, 1.0, (age / 0.06).min(1.0), self.time, draws);
            }
        }
        for p in &self.live_pools {
            if lights.len() < 48 {
                lights.push(pool_light(p.position, p.radius, p.left, self.time));
            }
        }
    }

    pub fn draw(
        &self,
        lib: Option<&ModelLibrary>,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        if let Some(lib) = lib {
            for f in &self.flying {
                draw_thrown(lib, f.thing, f.position, f.velocity, f.age, draws);
            }
            for s in &self.snares {
                let armed = (s.age / s.arm.max(0.01)).min(1.0);
                let sprung = s
                    .spring
                    .map_or(0.0, |at| ((s.age - at) / 0.06).clamp(0.0, 1.0));
                draw_snare(lib, s.position, s.yaw, armed, sprung, self.time, draws);
            }
        }
        for p in &self.pools {
            if lights.len() < 48 {
                let left = 1.0 - p.age / p.seconds.max(0.1);
                lights.push(pool_light(p.position, p.radius, left, self.time));
            }
        }
    }

    /// How strongly your Tallow Skin shows (0..1, its time left): the predicted timer, kept
    /// up while the server's `TALLOW` flag (`flag`) is on; a timer the server never confirmed
    /// is dropped after its first moments.
    pub fn tallow_strength(&self, flag: bool) -> f32 {
        let own = self.tallow_self();
        if flag {
            own.max(0.3)
        } else if own > 0.8 {
            own
        } else {
            0.0
        }
    }

    /// Your Tallow Skin's sheen (`tallow`: [`KnackView::tallow_strength`]) and the speed lines
    /// of a Surge or a Lunge (`rush`: `MeleeView::rush`) (the combat HUD calls it).
    pub fn hud(&self, list: &mut UiDrawList, c: Rect, tallow: f32, rush: f32, now: f64) {
        tallow_hud(list, c, tallow, now);
        surge_hud(list, c, self.surge_self().max(rush), now);
    }
}

/// The thrown thing of a knack (`None`: Tallow Skin, Surge and the Thorn aren't these).
pub fn thrown_of(knack: Knack) -> Option<Thrown> {
    match knack {
        Knack::KindleFlask => Some(Thrown::Flask),
        Knack::Snare => Some(Thrown::Snare),
        Knack::SootPot => Some(Thrown::SootPot),
        _ => None,
    }
}

/// The thrown thing a replicated projectile kind is.
pub fn thrown_kind(k: u16) -> Option<Thrown> {
    match k {
        kind::KINDLE_FLASK => Some(Thrown::Flask),
        kind::SNARE => Some(Thrown::Snare),
        kind::SOOT_POT => Some(Thrown::SootPot),
        _ => None,
    }
}

/// Another player used a knack (`Fx::Knack`): the throw (their left arm), Tallow Skin's
/// sound (the sheen is their `TALLOW` flag), the Surge's streak and rush.
pub fn remote_knack(ctx: &mut FxCtx, player: PlayerId, knack: Knack) {
    let id = NetId::player(player);
    let Some(e) = ctx.entities.iter().find(|e| e.id == id).copied() else {
        return;
    };
    let at = e.position + Vec3::Y * 1.2;
    match knack {
        Knack::KindleFlask | Knack::Snare | Knack::SootPot | Knack::Thorn => {
            ctx.remote.visual_mut(id).punching = 0.01;
            ctx.audio.play(Sfx::KnackThrow, PlayParams::at(at));
        }
        Knack::TallowSkin => ctx.audio.play(Sfx::TallowSkin, PlayParams::at(at)),
        Knack::Surge => {
            ctx.audio.play(Sfx::Surge, PlayParams::at(at));
            let dir = shared::input::look_dir(e.yaw, 0.0);
            lunge_streak(ctx.beams, e.position, dir * 4.0, accent(Knack::Surge));
        }
    }
}

/// The viewpoint's own knack in a replay (first person): the throw with its thing in hand, or
/// Tallow Skin's / the Surge's sound.
pub fn first_person_knack(vm: &mut Viewmodel, audio: &mut Audio, knack: Knack) {
    match thrown_of(knack) {
        Some(thing) => {
            vm.throw_item(Some(thing.model()));
            audio.play_2d(Sfx::KnackThrow);
        }
        None => audio.play_2d(match knack {
            Knack::TallowSkin => Sfx::TallowSkin,
            Knack::Surge => Sfx::Surge,
            _ => Sfx::ThornThrow,
        }),
    }
}

/// A heal from a Kindle pool (`Fx::Heal` from the pool's centre, every quarter second while
/// someone stands in it)?
pub fn is_pool_heal(entities: &[EntityState], from: Vec3) -> bool {
    entities
        .iter()
        .any(|e| e.kind == kind::KINDLE_POOL && e.position.distance(from) < 0.3)
}

/// A Kindle pool healing `player`: kindling drawn up from the floor at their feet into their
/// lantern, a faint warm flash for you; quiet (the pool's own crackle carries the sound, a
/// kindling chime every quarter second would drown everything).
pub fn pool_heal(ctx: &mut FxCtx, player: PlayerId, pool: Vec3, amount: f32) {
    use crate::models::shrike::{LANTERN_HEIGHT, REMOTE_SCALE};
    let local = Some(player) == ctx.local;
    let to = if local {
        ctx.local_chest
    } else {
        ctx.entities
            .iter()
            .find(|e| e.id == NetId::player(player))
            .map(|e| e.position + Vec3::Y * LANTERN_HEIGHT * REMOTE_SCALE)
    };
    let Some(to) = to else {
        return;
    };
    let feet = Vec3::new(to.x, pool.y + 0.1, to.z);
    let heal = accent(Knack::KindleFlask);
    for _ in 0..4 {
        let start = feet + Vec3::new(ctx.rng.range(-0.3, 0.3), 0.0, ctx.rng.range(-0.3, 0.3));
        ctx.effects.spawn(Particle {
            position: start,
            velocity: (to - start) / 0.35,
            max_life: 0.35,
            size: Vec2::splat(0.07),
            color: heal,
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    if local {
        ctx.feedback.flash(
            crate::palette::palette().flash_heal,
            (amount / 60.0).min(0.06),
        );
    }
}

/// A snare springing (`Fx::SnareSprung`): the flash, chains and snap.
pub fn remote_snare_sprung(ctx: &mut FxCtx, position: Vec3) {
    snare_spring(ctx.effects, ctx.beams, ctx.audio, position);
}

/// Whether a replicated entity is drawn by this module (thrown knacks: the entity drawers
/// skip them).
pub fn draws_entity(e: &EntityState) -> bool {
    thrown_kind(e.kind).is_some()
}

impl InGame {
    /// Stages a knack visual client-side (tools and screenshots; see [`KnackPreview`]).
    #[doc(hidden)]
    pub fn preview_knack(&mut self, preview: KnackPreview) {
        self.combat.knacks.staged.push(preview);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_knack_has_a_full_icon_and_a_name() {
        for k in Knack::ALL {
            let rows = icon(k);
            assert!(rows.iter().all(|r| r.len() == 12), "{k:?}");
            assert!(rows.iter().any(|r| r.contains('#')), "{k:?}");
            assert!(!k.name().is_empty());
        }
        // Different icons for every knack.
        for a in Knack::ALL {
            for b in Knack::ALL {
                assert!(a == b || icon(a) != icon(b));
            }
        }
    }

    #[test]
    fn throws_land_and_leave_their_area() {
        let mut view = KnackView::new();
        let mut effects = Effects::headless();
        let mut beams = Beams::headless();
        let mut audio = Audio::disabled();
        let mut rng = Rng::default();
        view.staged.push(KnackPreview::Cloud {
            position: Vec3::ZERO,
            radius: 5.0,
            seconds: 1.0,
        });
        view.staged.push(KnackPreview::Pool {
            position: Vec3::X * 4.0,
            radius: 3.5,
            seconds: 0.5,
        });
        view.staged.push(KnackPreview::TallowSelf { seconds: 3.0 });
        let mut step = |view: &mut KnackView, frames: usize| {
            for _ in 0..frames {
                let mut ctx = UpdateCtx {
                    effects: &mut effects,
                    beams: &mut beams,
                    audio: &mut audio,
                    rng: &mut rng,
                    world: None,
                    dt: 1.0 / 60.0,
                };
                view.update(&mut ctx);
            }
        };
        step(&mut view, 30);
        assert_eq!(view.clouds.len(), 1);
        assert!(view.tallow_self() > 0.7 && view.tallow_self() < 1.0);
        step(&mut view, 60);
        assert!(view.clouds.is_empty() && view.pools.is_empty());
        // The cloud kept a dense population of billows while it lasted.
        assert!(
            effects.particle_count() > 30,
            "{}",
            effects.particle_count()
        );
    }

    #[test]
    fn tallow_tint_brightens_and_stays_finite() {
        let t = tallow_tint(1.0, 3.0);
        assert!(t.x > 1.1 && t.y > 1.1 && t.is_finite());
        assert_eq!(tallow_tint(0.0, 3.0), Vec4::ONE);
    }
}
