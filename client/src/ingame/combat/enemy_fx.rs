//! Presentation of the Moulds (docs/roster.md, docs/combat.md "Client presentation
//! contract"): which model / name / size a replicated enemy kind has, the sounds of their
//! telegraphs and strikes, the Heron's aim laser, and the wax-and-ember effects — a hit
//! cracks a Mould and sprays molten wax and sparks, a kill releases its heat as one big ember
//! burst while the corpse slumps and melts, ember bombs trail smoke and burst into embers,
//! the Gaoler's bash kicks up dust and its sweep draws a burning arc.
//!
//! The client reads the same enemy definitions as the server (the built-ins compiled from
//! `assets/enemies/*.ron`), so attack names and the laser's muzzle come from the data.

use std::sync::OnceLock;

use engine::audio::Sfx;
use engine::physics::{CollisionWorld, MASK_SHOTS};
use glam::{Quat, Vec2, Vec3, Vec4};
use shared::protocol::{EntityState, enemy_flags, kind, player_flags};
use shared::sim::{EnemyDef, EnemyRegistry, HitPart, HitShape};

use super::super::effects::{DecalKind, Effects, FxTex, OnHit, Particle};
use super::Rng;
use crate::models::EnemyKind;
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
        _ => "GNAWER",
    }
}

/// Hit capsule of an enemy (radius, top of the body hitbox) for client-side tracer ends.
pub fn enemy_size(k: u16) -> (f32, f32) {
    def(k).map_or((0.6, 2.0), |d| {
        (d.radius, d.body_top.unwrap_or(d.height).max(2.0 * d.radius))
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
        _ => return None,
    })
}

/// The bark when an enemy notices a player.
pub fn alert_sfx(k: u16) -> Sfx {
    match k {
        kind::CENSER => Sfx::CenserAlert,
        kind::HARRIER => Sfx::HarrierAlert,
        kind::GAOLER => Sfx::GaolerAlert,
        kind::HERON => Sfx::HeronAlert,
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
