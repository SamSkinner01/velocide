//! Presentation of the Moulds (docs/roster.md, docs/enemies.md, docs/combat.md "Client
//! presentation contract"): which model / name / size a replicated enemy kind has, the sounds
//! of their telegraphs and strikes, the Heron's aim laser, and the wax-and-ember effects — a
//! hit cracks a Mould and sprays molten wax and sparks, a kill releases its heat as one big
//! ember burst while the corpse slumps and melts, ember bombs trail smoke and burst into
//! embers, the Gaoler's bash kicks up dust and its sweep draws a burning arc. The third roster
//! (the end of this file): the Bellwether's toll rings out in a shockwave and frenzied Moulds
//! burn blue-white (`palette().frenzy`), elites wear a searing glow and a crown of embers, a
//! latched Snuffer smothers the view, the Smelter's slag lines glow on the floor first and
//! then burn, the Effigy thunks.
//!
//! The client reads the same enemy definitions as the server (the built-ins compiled from
//! `assets/enemies/*.ron`), so attack names and the laser's muzzle come from the data.

use std::collections::HashMap;
use std::sync::OnceLock;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use glam::{Quat, Vec2, Vec3, Vec4};
use shared::protocol::{EntityState, NetId, enemy_flags, kind, player_flags, slag_flags};
use shared::sim::{AiState, EnemyDef, EnemyRegistry, HitPart, HitShape};

use super::super::effects::{DecalKind, Effects, FxTex, OnHit, Particle};
use super::Rng;
use crate::models::enemies::slag;
use crate::models::{EnemyKind, ModelId, ModelLibrary, Tint};
use crate::palette::palette;

/// The enemy definitions the client knows (the built-ins: the same files the server loads).
pub fn defs() -> &'static EnemyRegistry {
    static DEFS: OnceLock<EnemyRegistry> = OnceLock::new();
    DEFS.get_or_init(EnemyRegistry::builtin)
}

pub fn def(k: u16) -> Option<std::sync::Arc<EnemyDef>> {
    defs().by_kind(k)
}

/// Model kind for a replicated enemy kind (unknown kinds draw as Gnawers).
pub fn enemy_kind(k: u16) -> EnemyKind {
    match k {
        kind::CENSER => EnemyKind::Censer,
        kind::HARRIER => EnemyKind::Harrier,
        kind::GAOLER => EnemyKind::Gaoler,
        kind::HERON => EnemyKind::Heron,
        kind::BELLWETHER => EnemyKind::Bellwether,
        kind::SNUFFER => EnemyKind::Snuffer,
        kind::SMELTER => EnemyKind::Smelter,
        kind::EFFIGY => EnemyKind::Effigy,
        _ => EnemyKind::Gnawer,
    }
}

/// Display name of an enemy kind (kill feed): the definition's name in capitals.
pub fn enemy_name(k: u16) -> &'static str {
    match k {
        kind::CENSER => "CENSER",
        kind::HARRIER => "HARRIER",
        kind::GAOLER => "GAOLER",
        kind::HERON => "HERON",
        kind::BELLWETHER => "BELLWETHER",
        kind::SNUFFER => "SNUFFER",
        kind::SMELTER => "SMELTER",
        kind::EFFIGY => "EFFIGY",
        _ => "GNAWER",
    }
}

/// Hit capsule of an enemy (radius, top of the body hitbox) for client-side tracer ends.
pub fn enemy_size(k: u16) -> (f32, f32) {
    def(k).map_or((0.6, 2.0), |d| {
        let r = d.hit_radius.unwrap_or(d.radius);
        (r, d.body_top.unwrap_or(d.height).max(2.0 * r))
    })
}

/// Name of attack `index` of enemy kind `k` ("bite", "volley", ...).
pub fn attack_name(k: u16, index: u8) -> Option<String> {
    def(k).and_then(|d| d.attacks.get(index as usize).map(|a| a.name.clone()))
}

/// Telegraph (`strike == false`) or strike sound of an enemy attack.
pub fn attack_sfx(k: u16, index: u8, strike: bool) -> Option<Sfx> {
    let name = attack_name(k, index)?;
    Some(match (name.as_str(), strike) {
        ("bite", false) => Sfx::GnawerBiteWindup,
        ("bite", true) => Sfx::GnawerBite,
        ("pounce", false) => Sfx::GnawerPounceWindup,
        ("pounce", true) => Sfx::GnawerPounce,
        ("volley", false) => Sfx::CenserWhirl,
        // The lobs themselves sound on `EnemyFired`.
        ("volley", true) => return None,
        ("swing", false) => Sfx::CenserSwingWindup,
        ("swing", true) => Sfx::CenserSwing,
        ("dive", false) => Sfx::HarrierShriek,
        ("dive", true) => Sfx::HarrierDive,
        ("bash", false) => Sfx::GaolerBashWindup,
        ("bash", true) => Sfx::GaolerBash,
        ("sweep", false) => Sfx::GaolerChain,
        ("sweep", true) => Sfx::GaolerSweep,
        ("rush", false) => Sfx::GaolerRushWindup,
        ("rush", true) => Sfx::GaolerRush,
        ("needle", false) => Sfx::HeronLaser,
        ("needle", true) => return None,
        ("kick", false) => Sfx::HeronKickWindup,
        ("kick", true) => Sfx::HeronKick,
        // The third roster. The latch's leap is silent: it clanks on when it lands
        // (`SnufferLatch`); the pour's line ignites with the strike.
        ("toll", false) => Sfx::BellwetherTollWindup,
        ("toll", true) => Sfx::BellwetherToll,
        ("clapper", false) => Sfx::BellwetherClapperWindup,
        ("clapper", true) => Sfx::BellwetherClapper,
        ("latch" | "nip", false) => Sfx::SnufferPounceWindup,
        ("latch", true) => return None,
        ("nip", true) => Sfx::SnufferNip,
        ("pour", false) => Sfx::SmelterPourWindup,
        ("pour", true) => Sfx::SmelterPour,
        ("heave", false) => Sfx::SmelterSwingWindup,
        ("heave", true) => Sfx::SmelterSwing,
        _ => return None,
    })
}

/// A hit on an enemy of kind `k` (the training dummy thunks).
pub fn hurt_sfx(k: u16) -> Sfx {
    match k {
        kind::EFFIGY => Sfx::EffigyHit,
        _ => Sfx::EnemyHurt,
    }
}

/// The heavy death sound (Gaoler, Smelter) or the ordinary one.
pub fn death_sfx(k: u16) -> Sfx {
    match k {
        kind::GAOLER | kind::SMELTER => Sfx::GaolerDeath,
        _ => Sfx::EnemyDeath,
    }
}

/// The bark when an enemy notices a player.
pub fn alert_sfx(k: u16) -> Sfx {
    match k {
        kind::CENSER => Sfx::CenserAlert,
        kind::HARRIER => Sfx::HarrierAlert,
        kind::GAOLER => Sfx::GaolerAlert,
        kind::HERON => Sfx::HeronAlert,
        kind::BELLWETHER => Sfx::BellwetherAlert,
        kind::SNUFFER => Sfx::SnufferAlert,
        kind::SMELTER => Sfx::SmelterAlert,
        _ => Sfx::GnawerAlert,
    }
}

/// Sound of an enemy projectile leaving (`EnemyFired`).
pub fn fired_sfx(k: u16) -> Sfx {
    match k {
        kind::HERON => Sfx::HeronNeedle,
        _ => Sfx::CenserLob,
    }
}

/// Glow colour of an enemy projectile kind (`palette().hostile_projectile` in the colour-blind
/// palettes).
pub fn projectile_color(k: u16) -> Vec4 {
    let standard = match k {
        kind::NEEDLE => Vec4::new(1.0, 0.35, 0.22, 1.0),
        _ => Vec4::new(1.0, 0.58, 0.16, 1.0),
    };
    let p = palette();
    p.effect(standard, p.hostile_projectile, 1.0)
}

/// Telegraph glint colour of an enemy kind (`colors.telegraph`).
pub fn telegraph_color(k: u16) -> Vec4 {
    def(k).map_or(Vec4::new(1.0, 0.2, 0.08, 0.9), |d| {
        d.colors.telegraph.extend(0.9)
    })
}

/// Heat colour of an enemy kind's insides (`colors.glow`): hit sprays and death bursts.
pub fn heat_color(k: u16) -> Vec4 {
    def(k).map_or(Vec4::new(1.0, 0.5, 0.15, 1.0), |d| {
        d.colors.glow.extend(1.0)
    })
}

/// Wax colour of an enemy kind (`colors.body`).
pub fn wax_color(k: u16) -> Vec4 {
    def(k).map_or(Vec4::new(0.8, 0.77, 0.7, 1.0), |d| {
        (d.colors.body * 1.25).min(Vec3::ONE).extend(1.0)
    })
}

/// Origin and direction of a replicated enemy's aim laser (while `AIMING`), and whether it has
/// locked. The origin is the laser attack's muzzle (the Heron's lens) at the replicated yaw;
/// the direction is the exact replicated yaw / pitch.
pub fn aim_laser(e: &EntityState) -> Option<(Vec3, Vec3, bool)> {
    if e.flags & enemy_flags::AIMING == 0 || e.flags & enemy_flags::DYING != 0 {
        return None;
    }
    let d = def(e.kind)?;
    let attack = d.attacks.get(((e.aux >> 8) & 0xFF) as usize)?;
    let muzzle = attack
        .projectile
        .as_ref()
        .map_or(Vec3::Y * d.eye_height, |p| p.muzzle);
    let origin = e.position + Quat::from_rotation_y(e.yaw) * muzzle;
    let dir = shared::input::look_dir(e.yaw, e.pitch);
    Some((origin, dir, e.flags & enemy_flags::AIM_LOCKED != 0))
}

/// Longest laser drawn (the Heron's sight range).
pub const LASER_RANGE: f32 = 90.0;

/// Where an aim laser stops: the level, a player's body, or just short of the camera (the
/// player being aimed at sees it come right at them).
pub fn laser_end(
    world: Option<&CollisionWorld>,
    origin: Vec3,
    dir: Vec3,
    entities: &[EntityState],
    camera: Vec3,
) -> Vec3 {
    let mut end = world
        .and_then(|w| w.cast_ray(origin, dir, LASER_RANGE, MASK_SHOTS))
        .map_or(LASER_RANGE, |h| h.distance);
    for p in entities
        .iter()
        .filter(|p| p.kind == kind::PLAYER && p.flags & player_flags::DEAD == 0)
    {
        let shape = HitShape::capsule(
            HitPart::Body,
            p.position + Vec3::Y * 0.4,
            p.position + Vec3::Y * 1.4,
            0.4,
        );
        if let Some(d) = shape.ray_hit(origin, dir, end) {
            end = d;
        }
    }
    // The local player isn't in `entities`: stop just before the eye.
    let to_cam = camera - origin;
    let along = to_cam.dot(dir);
    if along > 0.0 && along < end && (to_cam - dir * along).length() < 0.45 {
        end = (along - 0.35).max(0.0);
    }
    origin + dir * end
}

// ---- particles -----------------------------------------------------------------------------

/// A hit cracks a Mould: pale wax droplets flying off along the shot (some set as wax splats
/// where they land), molten droplets and sparks (`amount` ~1 for a normal hit).
pub fn wax_hit(
    effects: &mut Effects,
    rng: &mut Rng,
    at: Vec3,
    dir: Vec3,
    amount: f32,
    wax: Vec4,
    heat: Vec4,
) {
    let n = (amount * 5.0).clamp(2.0, 18.0) as usize;
    for k in 0..n {
        let v = (dir + rng.dir() * 0.8).normalize_or(Vec3::Y) * rng.range(2.5, 7.0) + Vec3::Y * 2.0;
        // Every fourth drop sets where it lands: a wax splat on the floor or wall.
        let (texture, collide) = if k % 4 == 0 {
            (
                FxTex::WaxDrop,
                OnHit::Splat(DecalKind::Wax, rng.range(0.25, 0.5)),
            )
        } else {
            (FxTex::Glow, OnHit::Die)
        };
        effects.spawn(Particle {
            position: at,
            velocity: v,
            gravity: 16.0,
            drag: 0.5,
            max_life: rng.range(0.4, 0.9),
            size: Vec2::splat(rng.range(0.1, 0.2)),
            end_scale: 0.7,
            color: wax,
            end_alpha: 0.6,
            texture,
            collide: Some(collide),
            ..Default::default()
        });
    }
    for _ in 0..(n / 2 + 1) {
        let v = (dir * 0.6 + rng.dir()).normalize_or(Vec3::Y) * rng.range(2.0, 5.0) + Vec3::Y * 1.5;
        effects.spawn(Particle {
            position: at,
            velocity: v,
            gravity: 14.0,
            drag: 0.8,
            max_life: rng.range(0.3, 0.6),
            size: Vec2::splat(rng.range(0.06, 0.11)),
            end_scale: 0.5,
            color: heat,
            end_alpha: 0.2,
            texture: FxTex::Glow,
            additive: true,
            collide: Some(OnHit::Die),
            ..Default::default()
        });
    }
    effects.sparks(at, -dir, (n / 2).max(3));
    effects.glow(at, heat * Vec4::new(1.0, 1.0, 1.0, 0.6), 0.5, 0.1);
}

/// A dead Mould releases its heat: a flash, a fountain of embers, wax chunks and smoke, and a
/// scorch mark on the floor under it. `size` ~1 for a Gnawer, ~2 for the Gaoler.
pub fn ember_burst(
    effects: &mut Effects,
    rng: &mut Rng,
    center: Vec3,
    size: f32,
    wax: Vec4,
    heat: Vec4,
) {
    effects.glow(center, Vec4::new(1.0, 0.75, 0.4, 1.0), 2.2 * size, 0.35);
    let embers = (26.0 * size) as usize;
    for _ in 0..embers {
        let v = Vec3::new(
            rng.range(-1.0, 1.0),
            rng.range(0.6, 1.6),
            rng.range(-1.0, 1.0),
        ) * rng.range(2.0, 7.0)
            * size.sqrt();
        effects.spawn(Particle {
            position: center + rng.dir() * 0.4 * size,
            velocity: v,
            gravity: -1.5,
            drag: 1.2,
            max_life: rng.range(0.8, 1.8),
            size: Vec2::splat(rng.range(0.06, 0.14)),
            end_scale: 0.3,
            color: if rng.unit() < 0.5 {
                heat
            } else {
                Vec4::new(1.0, 0.85, 0.45, 1.0)
            },
            end_alpha: 0.0,
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    // Lumps of wax thrown off as it splits (irregular blobs, not chunks of anything).
    for _ in 0..(8.0 * size) as usize {
        let v = rng.dir() * rng.range(2.0, 6.0) + Vec3::Y * 3.0;
        effects.spawn(Particle {
            position: center,
            velocity: v,
            gravity: 18.0,
            drag: 0.4,
            max_life: rng.range(1.0, 2.0),
            size: Vec2::splat(rng.range(0.18, 0.34) * size.sqrt()),
            end_scale: 0.6,
            color: wax,
            end_alpha: 0.0,
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            spin: rng.range(-6.0, 6.0),
            collide: Some(OnHit::Bounce(0.15)),
            ..Default::default()
        });
    }
    effects.smoke(center, (4.0 * size) as usize);
    // An invisible drop straight down leaves the scorch where it meets the floor.
    effects.spawn(Particle {
        position: center,
        velocity: Vec3::NEG_Y * 6.0,
        gravity: 30.0,
        max_life: 1.5,
        color: Vec4::ZERO,
        collide: Some(OnHit::Splat(DecalKind::Scorch, 1.6 * size)),
        ..Default::default()
    });
}

/// The corpse melting: wisps of smoke and the odd rising ember (call a few times a second).
pub fn smoulder(effects: &mut Effects, rng: &mut Rng, at: Vec3, spread: f32, heat: Vec4) {
    effects.spawn(Particle {
        position: at + Vec3::new(rng.range(-spread, spread), 0.0, rng.range(-spread, spread)),
        velocity: Vec3::new(0.0, rng.range(0.5, 1.2), 0.0),
        gravity: -0.4,
        drag: 1.0,
        max_life: rng.range(0.9, 1.6),
        size: Vec2::splat(rng.range(0.3, 0.5)),
        end_scale: 2.2,
        color: Vec4::new(0.32, 0.29, 0.27, 0.45),
        end_alpha: 0.0,
        texture: FxTex::Smoke,
        rotation: rng.angle(),
        ..Default::default()
    });
    effects.spawn(Particle {
        position: at + rng.dir() * spread * 0.5,
        velocity: Vec3::new(
            rng.range(-0.3, 0.3),
            rng.range(1.0, 2.5),
            rng.range(-0.3, 0.3),
        ),
        gravity: -0.5,
        max_life: rng.range(0.6, 1.2),
        size: Vec2::splat(0.08),
        end_scale: 0.3,
        color: heat,
        texture: FxTex::Spark,
        additive: true,
        ..Default::default()
    });
}

/// An ember bomb in flight: a smoke trail, falling embers, a hot glow.
pub fn ember_trail(effects: &mut Effects, rng: &mut Rng, pos: Vec3, back: Vec3) {
    effects.spawn(Particle {
        position: pos + back * 0.25,
        velocity: rng.dir() * 0.3,
        gravity: -0.5,
        drag: 1.5,
        max_life: rng.range(0.6, 1.0),
        size: Vec2::splat(0.3),
        end_scale: 2.6,
        color: Vec4::new(0.35, 0.31, 0.29, 0.55),
        end_alpha: 0.0,
        texture: FxTex::Smoke,
        rotation: rng.angle(),
        ..Default::default()
    });
    let p = palette();
    effects.spawn(Particle {
        position: pos,
        max_life: 0.12,
        size: Vec2::splat(0.75),
        end_scale: 0.5,
        color: p.effect(Vec4::new(1.0, 0.6, 0.2, 0.9), p.hostile_projectile, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    if rng.unit() < 0.6 {
        effects.spawn(Particle {
            position: pos + rng.dir() * 0.2,
            velocity: back * 1.5 + rng.dir() * 1.0,
            gravity: 9.0,
            max_life: rng.range(0.3, 0.6),
            size: Vec2::splat(0.07),
            end_scale: 0.4,
            color: p.effect(Vec4::new(1.0, 0.7, 0.3, 1.0), p.hostile_projectile, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// An ember bomb bursting (`radius` = its blast radius): a hot flash, a spray of embers that
/// bounce, a puff of smoke and a scorch ring of glowing bits on the floor.
pub fn ember_bomb_burst(effects: &mut Effects, rng: &mut Rng, pos: Vec3, radius: f32) {
    effects.glow(pos, Vec4::new(1.0, 0.8, 0.45, 0.9), radius * 0.7, 0.15);
    effects.spawn(Particle {
        position: pos,
        max_life: 0.22,
        size: Vec2::splat(radius * 0.35),
        end_scale: 1.8,
        color: Vec4::new(1.0, 0.55, 0.18, 0.8),
        end_alpha: 0.0,
        texture: FxTex::MuzzleFlash,
        additive: true,
        rotation: rng.angle(),
        ..Default::default()
    });
    for _ in 0..24 {
        let v = (rng.dir() + Vec3::Y * 0.6).normalize_or(Vec3::Y) * rng.range(3.0, 9.0);
        effects.spawn(Particle {
            position: pos,
            velocity: v,
            gravity: 15.0,
            drag: 0.7,
            max_life: rng.range(0.5, 1.1),
            size: Vec2::splat(rng.range(0.07, 0.13)),
            end_scale: 0.3,
            color: Vec4::new(1.0, 0.62, 0.22, 1.0),
            end_alpha: 0.2,
            texture: FxTex::Spark,
            additive: true,
            collide: Some(OnHit::Bounce(0.3)),
            ..Default::default()
        });
    }
    effects.smoke(pos, 2);
    effects.spawn(Particle {
        position: pos + Vec3::Y * 0.2,
        max_life: 0.35,
        size: Vec2::splat(radius * 0.45),
        end_scale: 4.0,
        color: Vec4::new(1.0, 0.5, 0.15, 0.85),
        end_alpha: 0.0,
        texture: FxTex::Ring,
        additive: true,
        ..Default::default()
    });
}

/// The Gaoler's chain sweep: a burning arc of sparks at knee height around it plus floor dust
/// (telegraph colour in the colour-blind palettes).
pub fn sweep_arc(effects: &mut Effects, rng: &mut Rng, feet: Vec3, yaw: f32, radius: f32) {
    for k in 0..30 {
        // 300 degrees, starting behind its right shoulder and coming round the front.
        let a = yaw + (k as f32 / 29.0) * 300f32.to_radians() - 60f32.to_radians();
        let dir = Vec3::new(-a.sin(), 0.0, -a.cos());
        let r = radius * rng.range(0.75, 1.0);
        let tangent = Vec3::new(-dir.z, 0.0, dir.x);
        effects.spawn(Particle {
            position: feet + dir * r + Vec3::Y * rng.range(0.4, 0.9),
            velocity: tangent * rng.range(4.0, 8.0) + Vec3::Y * rng.range(0.5, 2.0),
            gravity: 8.0,
            drag: 1.5,
            max_life: rng.range(0.25, 0.5),
            size: Vec2::splat(rng.range(0.1, 0.2)),
            end_scale: 0.4,
            color: palette().effect(Vec4::new(1.0, 0.3, 0.1, 1.0), palette().telegraph, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    effects.dust(feet, 18, radius * 1.4);
}

/// A diving Harrier's streak: hot red air behind it (the whole dive is parryable, so the
/// colour-blind palettes draw it in the parry colour).
pub fn dive_streak(effects: &mut Effects, rng: &mut Rng, at: Vec3, back: Vec3) {
    effects.spawn(Particle {
        position: at + back * rng.range(0.3, 0.8),
        velocity: back * 3.0,
        drag: 3.0,
        max_life: 0.18,
        size: Vec2::new(0.35, 0.35),
        end_scale: 0.2,
        color: palette().effect(Vec4::new(1.0, 0.25, 0.12, 0.7), palette().parry_window, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
}

/// A Harrier swatted out of the air: a burst of wax shards and sparks.
pub fn swat_burst(effects: &mut Effects, rng: &mut Rng, at: Vec3, wax: Vec4, heat: Vec4) {
    wax_hit(effects, rng, at, Vec3::NEG_Y, 3.0, wax, heat);
    effects.glow(at, Vec4::new(1.0, 1.0, 0.9, 1.0), 1.5, 0.15);
}

// ---- the third roster: tolls, frenzy, elites, latches, slag lines ----------------------------

/// Client-side state of one enemy for the third roster's effects (kept in `EnemyVis`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtraVis {
    /// Replicated flags last frame (flag changes).
    flags: u32,
    /// Seconds to the next frenzy aura spark, elite crown ember, smother puff / sound.
    aura: f32,
    crown: f32,
    smother: f32,
    smother_sound: f32,
}

/// What [`enemy_extras`] needs to know about the frame.
pub struct ExtraFrame {
    /// Middle of the enemy's body.
    pub center: Vec3,
    /// Model height.
    pub height: f32,
    pub camera: Vec3,
    /// Owner code of the local player (a Snuffer latched on them smothers the view).
    pub local_owner: Option<u16>,
    pub dt: f32,
    /// Walk cycle before and after this frame (footfalls).
    pub cycle_before: f32,
    pub cycle: f32,
}

/// World position of weak point `name` of a replicated enemy.
fn weak_point_at(e: &EntityState, name: &str) -> Option<Vec3> {
    let d = def(e.kind)?;
    let w = d.weak_points.iter().find(|w| w.name == name)?;
    Some(e.position + Quat::from_rotation_y(e.yaw) * w.offset)
}

/// Is this enemy a Snuffer clinging to the local player?
pub fn latched_on_local(e: &EntityState, local_owner: Option<u16>) -> bool {
    e.flags & enemy_flags::LATCHED != 0 && local_owner == Some(e.owner)
}

/// Where to draw a Snuffer latched onto the local player: clinging to their chest, belly
/// against it, its body hanging below the view and the cap clamped just under the eyes, legs
/// reaching up the sides (the replicated position trails the predicted camera, so this is
/// placed from the camera).
pub fn latched_root(e: &EntityState, camera: Vec3) -> glam::Mat4 {
    let to = e.position + Vec3::Y * 0.7 - camera;
    let dir = Vec3::new(to.x, 0.0, to.z).normalize_or(Vec3::NEG_Z);
    // Local −Z (the cap's facing) points up and back at the camera, local +Y (its back) away.
    let (a, b) = (0.35, 0.94);
    let fwd = -dir * a + Vec3::Y * b;
    let up = dir * b + Vec3::Y * a;
    let x = up.cross(-fwd);
    let rot = glam::Mat3::from_cols(x, up, -fwd);
    // Drawn a little smaller than life so the cap sits along the bottom of the view (that is
    // net of the model's drawn scale, which `draw_enemy` adds on top of this root).
    let scale = 0.75;
    let center = camera + dir * 0.75 - Vec3::Y * 1.65;
    let origin = center - rot * Vec3::new(0.0, 0.65, 0.1) * scale;
    glam::Mat4::from_translation(origin)
        * glam::Mat4::from_mat3(rot)
        * glam::Mat4::from_scale(Vec3::splat(scale / enemy_kind(e.kind).scale()))
}

/// State / flag changes and continuous effects of the third roster: frenzy flares and auras,
/// elite crowns, the toll's ring and a cracked bell, a Snuffer clamping on (and smothering the
/// view of the player it holds) or thrown off, the Smelter's footfalls, dripping crucible and
/// the slag it spills when it dies.
pub fn enemy_extras(
    e: &EntityState,
    prev_state: u16,
    x: &mut ExtraVis,
    f: &ExtraFrame,
    effects: &mut Effects,
    audio: &mut Audio,
    rng: &mut Rng,
) {
    let p = palette();
    let dt = f.dt;
    let state = AiState::from_u16(e.state);
    let prev = AiState::from_u16(prev_state);
    let entered = |s: AiState| prev != Some(s) && state == Some(s);
    let dying = e.flags & enemy_flags::DYING != 0;
    let attack = attack_name(e.kind, ((e.aux >> 8) & 0xFF) as u8);
    let attack = attack.as_deref();
    let progress = (e.aux & 0xFF) as f32 / 255.0;

    // Frenzy: a flare when the toll takes, a blue-white aura while it lasts.
    let frenzied = e.flags & enemy_flags::FRENZIED != 0 && !dying;
    if frenzied && x.flags & enemy_flags::FRENZIED == 0 {
        let c = p.effect(Vec4::new(0.55, 0.8, 1.0, 1.0), p.frenzy, 1.2);
        effects.glow(f.center, c, 1.6 + 0.4 * f.height, 0.3);
        for _ in 0..14 {
            spark(
                effects,
                f.center + rng.dir() * 0.5,
                rng.dir() * 4.0 + Vec3::Y * 2.0,
                c,
                rng,
            );
        }
        audio.play(Sfx::Frenzied, PlayParams::at(f.center));
    }
    if frenzied {
        x.aura -= dt;
        while x.aura <= 0.0 {
            x.aura += 0.03;
            // Blue-white flame tongues licking up its body, and sparks.
            let c = p.effect(Vec4::new(0.5, 0.78, 1.0, 1.0), p.frenzy, 1.1);
            let r = 0.35 + 0.15 * f.height;
            let a = rng.angle();
            let at =
                e.position + Vec3::new(a.cos() * r, rng.range(0.1, f.height * 0.9), a.sin() * r);
            effects.spawn(Particle {
                position: at,
                velocity: Vec3::Y * rng.range(1.5, 3.0),
                gravity: -1.0,
                drag: 1.5,
                max_life: rng.range(0.25, 0.45),
                size: Vec2::new(0.22, 0.4) * (0.6 + 0.25 * f.height),
                end_scale: 0.2,
                color: Vec4::new(c.x, c.y, c.z, 0.7),
                texture: FxTex::Glow,
                additive: true,
                ..Default::default()
            });
            spark(
                effects,
                at,
                Vec3::new(0.0, rng.range(1.5, 3.5), 0.0),
                c,
                rng,
            );
        }
    }
    // Elites: a crown of searing embers circling over the head.
    if e.flags & enemy_flags::ELITE != 0 && !dying {
        x.crown -= dt;
        while x.crown <= 0.0 {
            x.crown += 0.03;
            let c = p.effect(Vec4::new(1.0, 0.85, 0.45, 1.0), p.elite, 1.3);
            let r = 0.25 + 0.12 * f.height;
            let a = rng.angle();
            let top = e.position + Vec3::Y * (f.height + 0.25);
            let at = top + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
            let swirl = Vec3::new(-a.sin(), 0.0, a.cos()) * 1.5;
            spark(effects, at, swirl + Vec3::Y * rng.range(1.0, 2.2), c, rng);
        }
    }

    match e.kind {
        kind::BELLWETHER => {
            let bell = weak_point_at(e, "bell").unwrap_or(f.center);
            if attack == Some("toll") {
                // Heat shimmering off the bell as it is hauled back, more and more.
                if state == Some(AiState::Windup) && rng.unit() < dt * (8.0 + 40.0 * progress) {
                    let c = p.effect(Vec4::new(1.0, 0.7, 0.3, 1.0), p.telegraph, 1.0);
                    let at = bell + rng.dir() * 0.5;
                    spark(effects, at, Vec3::Y * rng.range(1.0, 2.5), c, rng);
                }
                if entered(AiState::Strike) {
                    let radius = def(e.kind)
                        .and_then(|d| d.attacks.iter().find_map(|a| a.toll.clone()))
                        .map_or(18.0, |t| t.radius);
                    toll_ring(effects, rng, bell, e.position, radius);
                }
                // Shot mid-swing: the bell cracks, the toll is cancelled.
                if entered(AiState::Stunned) && prev == Some(AiState::Windup) {
                    effects.glow(bell, Vec4::new(1.0, 0.9, 0.7, 1.0), 1.8, 0.15);
                    effects.sparks(bell, Vec3::Y, 16);
                    for _ in 0..8 {
                        effects.spawn(Particle {
                            position: bell,
                            velocity: rng.dir() * rng.range(3.0, 7.0) + Vec3::Y * 2.0,
                            gravity: 16.0,
                            max_life: rng.range(0.6, 1.1),
                            size: Vec2::splat(rng.range(0.08, 0.16)),
                            color: Vec4::new(0.75, 0.52, 0.25, 1.0),
                            end_alpha: 0.3,
                            texture: FxTex::Shard,
                            rotation: rng.angle(),
                            spin: rng.range(-12.0, 12.0),
                            collide: Some(OnHit::Bounce(0.3)),
                            ..Default::default()
                        });
                    }
                    audio.play(Sfx::BellCrack, PlayParams::at(bell));
                }
            }
            if entered(AiState::Dying) {
                // The bell comes down with it.
                audio.play(Sfx::BellCrack, PlayParams::at(bell).with_pitch(0.8));
            }
        }
        kind::SNUFFER => {
            if entered(AiState::Latched) {
                effects.glow(f.center, Vec4::new(1.0, 0.6, 0.25, 0.8), 1.0, 0.15);
                effects.smoke(f.center, 2);
                audio.play(Sfx::SnufferLatch, PlayParams::at(f.center));
            }
            if state == Some(AiState::Latched) && local_owner_is(e, f.local_owner) {
                // Your wick being smothered: soot closing in from below, a muffled gutter.
                x.smother -= dt;
                while x.smother <= 0.0 {
                    x.smother += 0.05;
                    let to = Vec3::new(e.position.x - f.camera.x, 0.0, e.position.z - f.camera.z)
                        .normalize_or(Vec3::NEG_Z);
                    let side = Vec3::new(-to.z, 0.0, to.x);
                    let at = f.camera + to * 0.55 + side * rng.range(-0.45, 0.45)
                        - Vec3::Y * rng.range(0.2, 0.45);
                    effects.spawn(Particle {
                        position: at,
                        velocity: Vec3::Y * rng.range(0.1, 0.4),
                        max_life: rng.range(0.3, 0.6),
                        size: Vec2::splat(rng.range(0.25, 0.4)),
                        end_scale: 1.8,
                        color: Vec4::new(0.06, 0.05, 0.05, 0.5),
                        end_alpha: 0.0,
                        texture: FxTex::Smoke,
                        rotation: rng.angle(),
                        ..Default::default()
                    });
                }
                x.smother_sound -= dt;
                if x.smother_sound <= 0.0 {
                    x.smother_sound = 0.55;
                    audio.play(Sfx::SnufferSmother, PlayParams::default());
                }
            } else if state == Some(AiState::Latched) && rng.unit() < dt * 12.0 {
                // Seen from outside: soot and sparks squeezed out from under the cap.
                effects.smoke(f.center, 1);
                effects.sparks(f.center, Vec3::Y, 2);
            }
            if entered(AiState::Stunned) && prev == Some(AiState::Latched) {
                effects.sparks(f.center, Vec3::Y, 12);
                audio.play(Sfx::SnufferThrown, PlayParams::at(f.center));
            }
        }
        kind::SMELTER => {
            let speed = Vec3::new(e.velocity.x, 0.0, e.velocity.z).length();
            if !dying && speed > 0.4 && (f.cycle * 2.0).floor() != (f.cycle_before * 2.0).floor() {
                audio.play(Sfx::SmelterStep, PlayParams::at(e.position));
                effects.dust(e.position, 3, 1.4);
            }
            if attack == Some("pour")
                && matches!(state, Some(AiState::Windup | AiState::Strike))
                && rng.unit()
                    < dt * if state == Some(AiState::Strike) {
                        60.0
                    } else {
                        15.0
                    }
            {
                // Slag slopping over the lip as it tips, then a stream as it pours.
                let lip = e.position + Quat::from_rotation_y(e.yaw) * Vec3::new(0.0, 1.9, -1.55);
                let c = p.effect(Vec4::new(1.0, 0.6, 0.15, 1.0), p.hazard, 1.0);
                effects.spawn(Particle {
                    position: lip + rng.dir() * 0.15,
                    velocity: Quat::from_rotation_y(e.yaw) * Vec3::new(0.0, -1.0, -1.5)
                        + rng.dir() * 0.5,
                    gravity: 14.0,
                    max_life: rng.range(0.3, 0.6),
                    size: Vec2::splat(rng.range(0.14, 0.26)),
                    end_scale: 0.6,
                    color: c,
                    texture: FxTex::Glow,
                    additive: true,
                    collide: Some(OnHit::Splat(DecalKind::Scorch, 0.5)),
                    ..Default::default()
                });
            }
            if entered(AiState::Dying) {
                // The crucible tips out of its fists: a spill of slag and glowing lumps.
                let spill = e.position + Quat::from_rotation_y(e.yaw) * Vec3::new(0.0, 0.8, -1.4);
                let c = p.effect(Vec4::new(1.0, 0.55, 0.12, 1.0), p.hazard, 1.0);
                effects.glow(spill, c, 2.5, 0.5);
                for _ in 0..14 {
                    effects.spawn(Particle {
                        position: spill,
                        velocity: rng.dir() * rng.range(2.0, 5.0) + Vec3::Y * 3.0,
                        gravity: 16.0,
                        drag: 0.3,
                        max_life: rng.range(1.0, 2.0),
                        size: Vec2::splat(rng.range(0.15, 0.3)),
                        end_scale: 0.7,
                        color: c,
                        end_alpha: 0.2,
                        texture: FxTex::Lump,
                        rotation: rng.angle(),
                        spin: rng.range(-6.0, 6.0),
                        collide: Some(OnHit::Splat(DecalKind::Scorch, 0.9)),
                        ..Default::default()
                    });
                }
                audio.play(Sfx::SlagBurn, PlayParams::at(spill));
            }
        }
        _ => {}
    }
    x.flags = e.flags;
}

fn local_owner_is(e: &EntityState, local_owner: Option<u16>) -> bool {
    local_owner == Some(e.owner)
}

/// A short-lived additive spark.
fn spark(effects: &mut Effects, at: Vec3, velocity: Vec3, color: Vec4, rng: &mut Rng) {
    effects.spawn(Particle {
        position: at,
        velocity,
        gravity: -0.5,
        drag: 1.0,
        max_life: rng.range(0.35, 0.7),
        size: Vec2::splat(rng.range(0.07, 0.13)),
        end_scale: 0.3,
        color,
        texture: FxTex::Spark,
        additive: true,
        ..Default::default()
    });
}

/// The toll rings out: a flash at the bell, a ring racing out to the toll's reach at bell
/// height and a shockwave along the floor (the frenzy colour: this is what frenzies them).
pub fn toll_ring(effects: &mut Effects, rng: &mut Rng, bell: Vec3, feet: Vec3, radius: f32) {
    let p = palette();
    let c = p.effect(Vec4::new(1.0, 0.82, 0.45, 0.9), p.frenzy, 1.0);
    effects.glow(bell, Vec4::new(1.0, 0.95, 0.8, 1.0), 2.4, 0.25);
    for (life, size) in [(0.5, 2.0), (0.8, 1.2)] {
        effects.spawn(Particle {
            position: bell,
            max_life: life,
            size: Vec2::splat(size),
            end_scale: radius * 2.0 / size,
            color: c,
            end_alpha: 0.0,
            texture: FxTex::Ring,
            additive: true,
            ..Default::default()
        });
    }
    effects.shockwave(feet, radius * 0.5);
    for _ in 0..20 {
        let v = (rng.dir() + Vec3::Y * 0.4).normalize_or(Vec3::Y) * rng.range(4.0, 9.0);
        spark(effects, bell, v, c, rng);
    }
}

/// Elite and frenzy looks on an enemy's draw tint: an elite's wax runs hot (warm body, hotter
/// glows); a frenzied Mould's telegraph parts flare and pulse.
pub fn decorate_tint(e: &EntityState, time: f64, tint: &mut Tint) {
    if e.flags & enemy_flags::DYING != 0 {
        return;
    }
    if e.flags & enemy_flags::ELITE != 0 {
        tint.body *= Vec4::new(1.3, 1.1, 0.88, 1.0);
        tint.glow *= 1.35;
    }
    if e.flags & enemy_flags::FRENZIED != 0 {
        let pulse = 0.5 + 0.5 * ((time * 9.0 + (e.id.0 % 13) as f64).sin() as f32);
        tint.glow *= 1.3 + 0.7 * pulse;
        tint.body *= Vec4::new(
            0.95 + 0.15 * pulse,
            0.95 + 0.2 * pulse,
            1.0 + 0.3 * pulse,
            1.0,
        );
    }
}

/// An elite's searing aura: the same pose drawn again a little larger, translucent and lit in
/// the elite colour.
pub fn elite_shell(e: &EntityState, time: f64) -> Option<(f32, Tint)> {
    if e.flags & enemy_flags::ELITE == 0 || e.flags & enemy_flags::DYING != 0 {
        return None;
    }
    let p = palette();
    let c = p.effect(Vec4::new(1.0, 0.7, 0.3, 1.0), p.elite, 1.0);
    let flicker = 0.22 + 0.06 * ((time * 13.0 + e.id.0 as f64).sin() as f32);
    let tint = Tint {
        body: (c.truncate() * 2.2).extend(flicker),
        glow: Vec4::new(1.0, 1.0, 1.0, flicker),
        ..Default::default()
    };
    Some((1.07, tint))
}

/// Extra lights: an elite sears, a frenzied Mould pulses in the frenzy colour.
pub fn enemy_lights(e: &EntityState, center: Vec3, time: f64, lights: &mut Vec<PointLight>) {
    if e.flags & enemy_flags::DYING != 0 || lights.len() >= 48 {
        return;
    }
    let p = palette();
    if e.flags & enemy_flags::ELITE != 0 {
        lights.push(PointLight {
            position: center,
            color: p.light(Vec3::new(2.0, 1.4, 0.6), p.elite, 1.8),
            radius: 5.0,
        });
    }
    if e.flags & enemy_flags::FRENZIED != 0 {
        let pulse = 0.6 + 0.4 * ((time * 9.0).sin() as f32).abs();
        lights.push(PointLight {
            position: center,
            color: p.light(Vec3::new(0.8, 1.3, 2.2), p.frenzy, 1.6) * pulse,
            radius: 4.5,
        });
    }
}

// ---- slag lines (`kind::SLAG_LINE`) ------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct SlagVis {
    /// Floor points (traced like the server's).
    points: Vec<Vec3>,
    seen: bool,
    lit: bool,
}

/// The Smelter's slag lines: a glowing telegraph line on the floor during the pour's wind-up,
/// then a burning strip that smokes and spits embers, cooling for its last second and leaving
/// scorch marks.
#[derive(Default)]
pub struct SlagView {
    lines: HashMap<NetId, SlagVis>,
}

/// Half-width (m) of a replicated slag line (`state` = width in cm).
fn slag_half_width(e: &EntityState) -> f32 {
    (e.state as f32 / 200.0).max(0.1)
}

impl SlagView {
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Call before updating this frame's lines.
    pub fn begin(&mut self) {
        for v in self.lines.values_mut() {
            v.seen = false;
        }
    }

    /// Forgets lines that are gone (after [`SlagView::update`] for every line).
    pub fn end(&mut self) {
        self.lines.retain(|_, v| v.seen);
    }

    /// One replicated slag line this frame: its floor path (first sight), ignition (embers,
    /// scorch marks, a sizzle), embers and smoke while it burns.
    pub fn update(
        &mut self,
        e: &EntityState,
        world: Option<&CollisionWorld>,
        dt: f32,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
    ) {
        let v = self.lines.entry(e.id).or_insert_with(|| {
            let flat = Vec3::new(e.velocity.x, 0.0, e.velocity.z);
            let points = match world {
                Some(w) => {
                    shared::sim::enemy::trace_floor(w, e.position, flat, flat.length() + 0.05)
                }
                None => Vec::new(),
            };
            SlagVis {
                points: if points.len() >= 2 {
                    points
                } else {
                    vec![e.position, e.position + e.velocity]
                },
                ..Default::default()
            }
        });
        v.seen = true;
        let live = e.flags & slag_flags::LIVE != 0;
        let cooling = e.flags & slag_flags::COOLING != 0;
        let p = palette();
        let n = v.points.len();
        let at = |rng: &mut Rng, pts: &[Vec3]| {
            let i = ((rng.unit() * (pts.len() - 1) as f32) as usize).min(pts.len() - 2);
            pts[i].lerp(pts[i + 1], rng.unit())
        };
        let half = slag_half_width(e);
        if live && !v.lit {
            v.lit = true;
            let mid = v.points[n / 2];
            audio.play(Sfx::SlagBurn, PlayParams::at(mid));
            for (i, pt) in v.points.iter().enumerate() {
                if i % 2 == 0 {
                    effects.decal(*pt + Vec3::Y * 0.02, Vec3::Y, half * 2.2, DecalKind::Scorch);
                }
            }
            let c = p.effect(Vec4::new(1.0, 0.65, 0.2, 1.0), p.hazard, 1.2);
            for _ in 0..(n * 2).min(40) {
                let pt = at(rng, &v.points);
                spark(effects, pt, rng.dir() * 2.0 + Vec3::Y * 3.0, c, rng);
            }
        }
        if !live {
            // The telegraph: sparks skipping along the line.
            if rng.unit() < dt * (6.0 + 0.6 * n as f32) {
                let c = p.effect(Vec4::new(1.0, 0.3, 0.1, 1.0), p.telegraph, 1.0);
                let pt = at(rng, &v.points) + Vec3::Y * 0.05;
                spark(effects, pt, Vec3::Y * rng.range(0.8, 2.0), c, rng);
            }
            return;
        }
        let rate = if cooling { 0.3 } else { 1.0 };
        if rng.unit() < dt * rate * (10.0 + 1.5 * n as f32) {
            let c = p.effect(Vec4::new(1.0, 0.62, 0.2, 1.0), p.hazard, 1.0);
            let side = Vec3::new(rng.range(-half, half), 0.05, rng.range(-half, half)) * 0.7;
            spark(
                effects,
                at(rng, &v.points) + side,
                Vec3::Y * rng.range(1.0, 3.0) + rng.dir() * 0.4,
                c,
                rng,
            );
        }
        if rng.unit() < dt * (if cooling { 6.0 } else { 2.0 }) {
            let pt = at(rng, &v.points);
            effects.spawn(Particle {
                position: pt + Vec3::Y * 0.2,
                velocity: Vec3::Y * rng.range(0.6, 1.2),
                gravity: -0.3,
                drag: 1.0,
                max_life: rng.range(1.0, 1.8),
                size: Vec2::splat(rng.range(0.4, 0.7)),
                end_scale: 2.4,
                color: Vec4::new(0.3, 0.27, 0.25, 0.4),
                end_alpha: 0.0,
                texture: FxTex::Smoke,
                rotation: rng.angle(),
                ..Default::default()
            });
        }
    }

    /// Draws a replicated slag line: the telegraph line (its rims, pulsing in the telegraph
    /// colour, over a faint strip), then the molten strip (the hazard colour in the
    /// colour-blind palettes) with a few lights along it.
    pub fn draw(
        &self,
        lib: &ModelLibrary,
        e: &EntityState,
        time: f64,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        let Some(v) = self.lines.get(&e.id) else {
            return;
        };
        let rig = lib.get(ModelId::SlagStrip);
        let p = palette();
        let live = e.flags & slag_flags::LIVE != 0;
        let cooling = e.flags & slag_flags::COOLING != 0;
        let width = slag_half_width(e) * 2.0;
        let flicker = 0.85 + 0.15 * ((time * 11.0 + e.id.0 as f64).sin() as f32);
        let pulse = 0.5 + 0.5 * ((time * 8.0).sin() as f32);
        let mut strip = rig.rest_pose();
        let mut rims = rig.rest_pose();
        strip.hide(slag::RIMS);
        rims.hide(slag::STRIP);
        let (strip_tint, rim_tint) = if !live {
            let c = p.telegraph;
            (
                Tint {
                    glow: Vec4::new(0.45 + 0.3 * pulse, 0.45 + 0.3 * pulse, 0.45, 0.45),
                    glow_color: Some(c),
                    ..Default::default()
                },
                Tint {
                    glow: Vec4::splat(1.0),
                    glow_color: Some(c),
                    ..Default::default()
                },
            )
        } else {
            let heat = if cooling { 0.55 } else { 1.7 * flicker };
            let strip = Tint {
                glow: Vec4::new(heat, heat * if cooling { 0.6 } else { 1.0 }, heat, 1.0),
                glow_color: p.model_glow(p.hazard),
                ..Default::default()
            };
            (strip, strip)
        };
        for w in v.points.windows(2) {
            // Lifted clear of the floor (no depth fighting at grazing angles).
            let (a, b) = (w[0] + Vec3::Y * 0.08, w[1] + Vec3::Y * 0.08);
            let d = b - a;
            let len = d.length();
            if len < 1e-3 {
                continue;
            }
            let root = glam::Mat4::from_scale_rotation_translation(
                Vec3::new(width, 1.0, len * 1.04),
                Quat::from_rotation_arc(Vec3::NEG_Z, d / len),
                a,
            );
            rig.draw(&strip, root, &strip_tint, draws);
            if !live {
                rig.draw(&rims, root, &rim_tint, draws);
            }
        }
        if live && v.points.len() >= 2 {
            let n = v.points.len();
            let heat = if cooling { 0.4 } else { 1.0 };
            for i in [n / 6, n / 2, (5 * n) / 6] {
                if lights.len() >= 48 {
                    break;
                }
                lights.push(PointLight {
                    position: v.points[i.min(n - 1)] + Vec3::Y * 0.6,
                    color: p.light(Vec3::new(2.2, 1.0, 0.3), p.hazard, 1.8) * heat * flicker,
                    radius: 4.5,
                });
            }
        }
    }
}

// ---- getting there: leaps, melting and re-forming (docs/enemies.md) -------------------------

/// Dust and grit thrown up where a Mould takes off on a leap or lands from one (`size`: about
/// its body radius).
pub fn leap_dust(effects: &mut Effects, rng: &mut Rng, feet: Vec3, size: f32) {
    for _ in 0..8 {
        let a = rng.angle();
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        effects.spawn(Particle {
            position: feet + dir * size * 0.6 + Vec3::Y * 0.12,
            velocity: dir * rng.range(1.5, 3.5) + Vec3::Y * rng.range(0.3, 1.2),
            gravity: 1.5,
            drag: 2.5,
            max_life: rng.range(0.35, 0.6),
            size: Vec2::splat(rng.range(0.3, 0.5) * size.max(0.6)),
            end_scale: 2.2,
            color: Vec4::new(0.5, 0.46, 0.42, 0.45),
            end_alpha: 0.0,
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            ..Default::default()
        });
    }
}

/// A Mould slumping into its puddle (`AiState::Melt` starting): molten wax slopping out over
/// the floor (setting into splats), its heat steaming off.
pub fn melt_burst(effects: &mut Effects, rng: &mut Rng, feet: Vec3, size: f32, wax: Vec4) {
    for _ in 0..10 {
        let a = rng.angle();
        let dir = Vec3::new(a.cos(), 0.0, a.sin());
        effects.spawn(Particle {
            position: feet + Vec3::Y * rng.range(0.3, 1.2),
            velocity: dir * rng.range(1.0, 3.0) + Vec3::Y * rng.range(0.5, 2.0),
            gravity: 14.0,
            max_life: 1.2,
            size: Vec2::splat(rng.range(0.07, 0.14)),
            color: wax,
            end_alpha: 1.0,
            texture: FxTex::WaxDrop,
            collide: Some(OnHit::Splat(DecalKind::Wax, rng.range(0.3, 0.6) * size)),
            ..Default::default()
        });
    }
    melt_steam(effects, rng, feet, size);
}

/// Steam and heat leaving a melting or rising Mould (call a few times a second).
pub fn melt_steam(effects: &mut Effects, rng: &mut Rng, feet: Vec3, size: f32) {
    effects.spawn(Particle {
        position: feet
            + Vec3::new(
                rng.range(-size, size) * 0.7,
                0.15,
                rng.range(-size, size) * 0.7,
            ),
        velocity: Vec3::new(0.0, rng.range(0.6, 1.4), 0.0),
        gravity: -0.6,
        drag: 1.2,
        max_life: rng.range(0.7, 1.2),
        size: Vec2::splat(rng.range(0.35, 0.6)),
        end_scale: 2.4,
        color: Vec4::new(0.78, 0.76, 0.72, 0.3),
        end_alpha: 0.0,
        texture: FxTex::Smoke,
        rotation: rng.angle(),
        ..Default::default()
    });
}

/// The puddle welling up where a Mould re-forms (`AiState::Reform` starting): wax bubbling up
/// out of the floor.
pub fn rise_burst(effects: &mut Effects, rng: &mut Rng, feet: Vec3, size: f32, wax: Vec4) {
    for _ in 0..8 {
        let a = rng.angle();
        let r = rng.range(0.1, 0.8) * size;
        effects.spawn(Particle {
            position: feet + Vec3::new(a.cos() * r, 0.05, a.sin() * r),
            velocity: Vec3::new(0.0, rng.range(1.5, 3.5), 0.0),
            gravity: 12.0,
            max_life: 0.6,
            size: Vec2::splat(rng.range(0.06, 0.12)),
            color: wax,
            end_alpha: 1.0,
            texture: FxTex::WaxDrop,
            ..Default::default()
        });
    }
    melt_steam(effects, rng, feet, size);
}

/// Radius (m) of the wax puddle under a Mould melting / re-forming at `progress` of its state
/// (`melt` true for `Melt`), for a Mould whose body is `size` across (its hit radius): it
/// spreads as the body sinks and drains away at the very end of the slump; where it re-forms it
/// wells up first and is drawn back into the body as it rises. Zero at both ends of the
/// teleport between the two, so the move never shows.
pub fn puddle_radius(melt: bool, progress: f32, size: f32) -> f32 {
    let s = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let p = progress.clamp(0.0, 1.0);
    let k = if melt {
        s(p / 0.6) * (1.0 - s((p - 0.86) / 0.14))
    } else {
        s(p / 0.22) * (1.0 - s((p - 0.55) / 0.45))
    };
    k * size * 1.8
}

/// Draws the wax puddle under a melting / re-forming Mould: tinted with its wax, its molten
/// heart in its heat colour (dimmer as the body has sunk away from it).
pub fn draw_puddle(lib: &ModelLibrary, e: &EntityState, radius: f32, draws: &mut Vec<DrawItem>) {
    if radius < 0.02 {
        return;
    }
    let spin = (e.id.0 as f32 * 2.399).rem_euclid(std::f32::consts::TAU);
    let root = glam::Mat4::from_scale_rotation_translation(
        Vec3::new(radius, radius.min(1.0), radius),
        Quat::from_rotation_y(spin),
        e.position + Vec3::Y * 0.02,
    );
    let heat = heat_color(e.kind);
    let p = palette();
    let tint = Tint {
        body: wax_color(e.kind),
        glow: Vec4::new(heat.x * 1.6, heat.y * 1.6, heat.z * 1.6, 1.0),
        glow_color: p.model_glow(p.weak_point),
        ..Default::default()
    };
    lib.draw_model(ModelId::WaxPuddle, root, &tint, draws);
}
