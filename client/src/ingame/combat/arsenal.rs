//! The expansion arsenal's presentation (`docs/expansion.md`, area **arsenal-client**): which
//! build the local player is holding, and the visuals and sounds of the new projectiles,
//! blasts, hazards and status effects.
//!
//! Everything is keyed on small enums so the replicated kinds and flags map onto them in one
//! place: [`ArsenalProjectile`] (arrows, rivets, cinder rockets, flak shells, limpets, mortar
//! shells, flares), [`Blast`] (what a new explosion looks like), [`Hazard`] (burning patches)
//! and [`Status`] (Burn, Pinned). [`ArsenalView`] keeps the per-entity bookkeeping (trails,
//! blink timers, burn embers) and draws them.
//!
//! [`ArsenalPreview`] / `InGame::preview_arsenal` stage any of these client-side (no
//! simulation involved) so tools can look at them: `combat_shots` uses it for the
//! `arsenal_*.png` shots.

use std::collections::HashMap;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::loadout::{Ability, Loadout, WeaponBuild, WeaponKind};
use shared::protocol::{EntityState, NetId};

use super::super::InGame;
use super::super::effects::{DecalKind, Effects, FxTex, OnHit, Particle};
use super::Rng;
use super::entities::enemy_size;
use super::fx::{Beams, FxCtx, shake_at};
use crate::models::{ModelId, ModelLibrary, Tint, ability_color};

// ---------------------------------------------------------------------------------------------
// The held build

impl InGame {
    /// The loadout the local player carries right now. Until the simulation carries loadouts
    /// (`OwnState`'s weapon state), that is the fixed default four on slots 1–4.
    pub(crate) fn carried_loadout(&self) -> Loadout {
        Loadout::default()
    }

    /// The build in the selected slot (an empty slot shows the first weapon carried).
    pub(crate) fn held_build(&self) -> WeaponBuild {
        let loadout = self.carried_loadout();
        loadout
            .slot(self.controls.weapons.current)
            .or_else(|| loadout.builds().next())
            .copied()
            .unwrap_or_default()
    }

    /// The held weapon and its selected ability.
    pub(crate) fn held_weapon(&self) -> (WeaponKind, Ability) {
        let build = self.held_build();
        (build.weapon, build.ability(self.controls.weapons.mode()))
    }

    /// Stages an arsenal visual client-side (tools and screenshots; see [`ArsenalPreview`]).
    #[doc(hidden)]
    pub fn preview_arsenal(&mut self, preview: ArsenalPreview) {
        self.combat.arsenal.preview(preview);
    }
}

// ---------------------------------------------------------------------------------------------
// Projectiles

/// A projectile of the expansion arsenal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArsenalProjectile {
    /// Longwick taper arrow (also Triple Nock's three).
    Arrow,
    PinningArrow,
    EmberArrow,
    /// Riveter hot rivet.
    Rivet,
    /// Cinderbox salvo rocket.
    CinderRocket,
    /// Flak shell (airbursts into a ring of pellets).
    FlakShell,
    /// Limpet sticky charge.
    Limpet,
    MortarShell,
    /// Flare (lands and leaves a burning patch).
    Flare,
}

impl ArsenalProjectile {
    pub const ALL: [ArsenalProjectile; 9] = [
        ArsenalProjectile::Arrow,
        ArsenalProjectile::PinningArrow,
        ArsenalProjectile::EmberArrow,
        ArsenalProjectile::Rivet,
        ArsenalProjectile::CinderRocket,
        ArsenalProjectile::FlakShell,
        ArsenalProjectile::Limpet,
        ArsenalProjectile::MortarShell,
        ArsenalProjectile::Flare,
    ];

    pub fn model(self) -> ModelId {
        match self {
            ArsenalProjectile::Arrow => ModelId::Arrow,
            ArsenalProjectile::PinningArrow => ModelId::PinningArrow,
            ArsenalProjectile::EmberArrow => ModelId::EmberArrow,
            ArsenalProjectile::Rivet => ModelId::Rivet,
            ArsenalProjectile::CinderRocket => ModelId::CinderRocket,
            ArsenalProjectile::FlakShell => ModelId::FlakShell,
            ArsenalProjectile::Limpet => ModelId::Limpet,
            ArsenalProjectile::MortarShell => ModelId::MortarShell,
            ArsenalProjectile::Flare => ModelId::Flare,
        }
    }

    /// Drawn scale (small things a little larger than life so they read at 640×360).
    pub fn scale(self) -> f32 {
        match self {
            ArsenalProjectile::Rivet => 1.5,
            ArsenalProjectile::Limpet => 1.6,
            ArsenalProjectile::FlakShell => 1.2,
            _ => 1.0,
        }
    }

    /// The light it carries (colour, radius).
    pub fn light(self) -> (Vec3, f32) {
        match self {
            ArsenalProjectile::Arrow => (Vec3::new(1.2, 0.8, 0.4), 2.5),
            ArsenalProjectile::PinningArrow => (Vec3::new(0.6, 0.85, 1.4), 3.0),
            ArsenalProjectile::EmberArrow => (Vec3::new(1.8, 0.7, 0.2), 4.0),
            ArsenalProjectile::Rivet => (Vec3::new(1.4, 0.5, 0.2), 2.0),
            ArsenalProjectile::CinderRocket => (Vec3::new(2.0, 0.9, 0.3), 4.5),
            ArsenalProjectile::FlakShell => (Vec3::new(1.4, 0.6, 0.9), 3.0),
            ArsenalProjectile::Limpet => (Vec3::new(1.4, 1.3, 0.4), 2.5),
            ArsenalProjectile::MortarShell => (Vec3::new(1.2, 0.8, 1.5), 4.0),
            ArsenalProjectile::Flare => (Vec3::new(2.6, 0.7, 0.45), 9.0),
        }
    }

    /// How far a stuck projectile's front sinks into the surface it hit (arrows, rivets).
    fn embed(self) -> f32 {
        match self {
            ArsenalProjectile::Arrow
            | ArsenalProjectile::PinningArrow
            | ArsenalProjectile::EmberArrow => 0.24,
            ArsenalProjectile::Rivet => 0.06,
            _ => 0.0,
        }
    }

    /// Sound when it hits / lands (blasts have their own, [`Blast::sfx`]).
    pub fn impact_sfx(self) -> Option<Sfx> {
        match self {
            ArsenalProjectile::Arrow => Some(Sfx::ArrowImpact),
            ArsenalProjectile::PinningArrow => Some(Sfx::PinThunk),
            ArsenalProjectile::Rivet => Some(Sfx::RivetImpact),
            ArsenalProjectile::Limpet => Some(Sfx::LimpetStick),
            ArsenalProjectile::Flare => Some(Sfx::FlareBurn),
            _ => None,
        }
    }
}

/// One projectile to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileView {
    pub kind: ArsenalProjectile,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Stuck in the world or a target (arrows, rivets, limpets): drawn along `facing`.
    pub stuck: bool,
    /// Flight direction when it stuck (arrows sink their tip along it; a limpet's claws grip
    /// the surface it faces).
    pub facing: Vec3,
    /// Seconds since it appeared (blink, sputter, quiver).
    pub age: f32,
    pub seed: u32,
}

/// Model transform of a projectile.
pub fn projectile_transform(p: &ProjectileView, time: f64) -> Mat4 {
    let dir = if p.stuck {
        p.facing.normalize_or(Vec3::NEG_Z)
    } else {
        p.velocity.normalize_or(Vec3::NEG_Z)
    };
    let look = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    let t = time as f32 + (p.seed % 97) as f32 * 0.37;
    let spin = match (p.kind, p.stuck) {
        (_, true) => {
            // A fresh arrow quivers for a moment.
            let q = (1.0 - p.age / 0.5).max(0.0);
            Quat::from_rotation_x(0.06 * q * q * (p.age * 60.0).sin())
                * Quat::from_rotation_z((p.seed % 13) as f32)
        }
        (ArsenalProjectile::Rivet | ArsenalProjectile::CinderRocket, false) => {
            Quat::from_rotation_z(t * 18.0)
        }
        (ArsenalProjectile::FlakShell | ArsenalProjectile::Flare, false) => {
            Quat::from_rotation_z(t * 9.0)
        }
        (ArsenalProjectile::MortarShell, false) => Quat::from_rotation_x(0.08 * (t * 7.0).sin()),
        (ArsenalProjectile::Limpet, false) => {
            Quat::from_rotation_x(t * 8.0) * Quat::from_rotation_y(t * 5.0)
        }
        _ => Quat::IDENTITY,
    };
    let sink = if p.stuck { p.kind.embed() } else { 0.0 };
    // The models' fronts sit ~0.36 m (arrows) / 0.12 m (rivet) ahead of their origin.
    let front = match p.kind {
        ArsenalProjectile::Rivet => 0.12,
        ArsenalProjectile::Arrow
        | ArsenalProjectile::PinningArrow
        | ArsenalProjectile::EmberArrow => 0.36,
        _ => 0.0,
    };
    let offset = if p.stuck {
        -dir * (front - sink).max(0.0)
    } else {
        Vec3::ZERO
    };
    Mat4::from_scale_rotation_translation(
        Vec3::splat(p.kind.scale()),
        look * spin,
        p.position + offset,
    )
}

/// Is an armed limpet's lamp lit? It blinks faster the longer it sits (2 → 6 Hz).
pub fn limpet_lamp_on(age: f32) -> bool {
    let rate = 2.0 + (age / 2.0).min(1.0) * 4.0;
    (age * rate).fract() < 0.35
}

/// Draws a projectile and its light.
pub fn draw_projectile(
    lib: &ModelLibrary,
    p: &ProjectileView,
    time: f64,
    draws: &mut Vec<DrawItem>,
    lights: &mut Vec<PointLight>,
) {
    let mut tint = Tint::default();
    let flicker = 0.85 + 0.15 * ((time * 23.0 + p.seed as f64).sin() as f32);
    let mut light = p.kind.light();
    match p.kind {
        ArsenalProjectile::Limpet => {
            let on = limpet_lamp_on(p.age);
            tint.glow = Vec4::splat(if on { 3.0 } else { 0.35 });
            if !on {
                light.0 *= 0.25;
            }
        }
        ArsenalProjectile::Flare => {
            let sputter = 1.6 + 0.9 * ((time * 31.0 + p.seed as f64).sin() as f32).abs();
            tint.glow = Vec4::splat(sputter);
            light.0 *= 0.8 + 0.3 * sputter / 2.5;
        }
        _ => tint.glow = Vec4::splat(1.2 * flicker + 0.4),
    }
    lib.draw_model(p.kind.model(), projectile_transform(p, time), &tint, draws);
    if lights.len() < 48 {
        lights.push(PointLight {
            position: p.position,
            color: light.0 * flicker,
            radius: light.1,
        });
    }
}

/// Per-frame trail particles behind a flying projectile (and the occasional spark from a
/// stuck one).
pub fn projectile_trail(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    p: &ProjectileView,
    dt: f32,
) {
    let dir = p.velocity.normalize_or_zero();
    let back = -dir;
    let chance = |rng: &mut Rng, per_second: f32| rng.unit() < dt * per_second;
    if p.stuck {
        match p.kind {
            ArsenalProjectile::Arrow | ArsenalProjectile::EmberArrow if chance(rng, 6.0) => {
                // The taper keeps burning where it stuck.
                effects.spawn(Particle {
                    position: p.position - p.facing.normalize_or_zero() * 0.2,
                    velocity: Vec3::Y * 0.6 + rng.dir() * 0.2,
                    max_life: 0.4,
                    size: Vec2::splat(0.06),
                    color: Vec4::new(1.0, 0.7, 0.3, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
            ArsenalProjectile::Limpet if limpet_lamp_on(p.age) && chance(rng, 3.0) => {
                effects.glow(p.position, Vec4::new(1.0, 0.95, 0.4, 0.8), 0.35, 0.08);
            }
            _ => {}
        }
        return;
    }
    match p.kind {
        ArsenalProjectile::Arrow
        | ArsenalProjectile::PinningArrow
        | ArsenalProjectile::EmberArrow => {
            // A thin streak of wick smoke, sparks from the flame.
            let ember = p.kind == ArsenalProjectile::EmberArrow;
            if chance(rng, if ember { 50.0 } else { 30.0 }) {
                effects.spawn(Particle {
                    position: p.position + back * rng.range(0.0, 0.4),
                    velocity: rng.dir() * 0.3,
                    gravity: -0.3,
                    drag: 2.0,
                    max_life: rng.range(0.25, 0.45),
                    size: Vec2::splat(0.1),
                    end_scale: 2.0,
                    color: Vec4::new(0.55, 0.52, 0.5, 0.35),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
            let color = match p.kind {
                ArsenalProjectile::PinningArrow => Vec4::new(0.7, 0.85, 1.0, 1.0),
                ArsenalProjectile::EmberArrow => Vec4::new(1.0, 0.5, 0.12, 1.0),
                _ => Vec4::new(1.0, 0.75, 0.35, 1.0),
            };
            if chance(rng, if ember { 60.0 } else { 20.0 }) {
                effects.spawn(Particle {
                    position: p.position + dir * 0.25,
                    velocity: rng.dir() * 1.5 + back * 2.0,
                    gravity: 6.0,
                    max_life: 0.25,
                    size: Vec2::splat(0.07),
                    color,
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
        }
        ArsenalProjectile::Rivet => {
            // A short red-hot streak.
            let len = (p.velocity.length() * 0.012).clamp(0.2, 1.2);
            beams.beam(
                p.position - dir * len,
                p.position,
                0.035,
                Vec4::new(2.2, 0.8, 0.3, 0.8),
                0.03,
            );
        }
        ArsenalProjectile::CinderRocket => {
            effects.spawn(Particle {
                position: p.position + back * 0.3,
                velocity: back * 3.0 + rng.dir() * 0.8,
                drag: 3.0,
                max_life: 0.12,
                size: Vec2::splat(0.3),
                end_scale: 0.5,
                color: Vec4::new(1.0, 0.6, 0.25, 1.0),
                texture: FxTex::MuzzleFlash,
                additive: true,
                rotation: rng.angle(),
                ..Default::default()
            });
            if chance(rng, 40.0) {
                effects.spawn(Particle {
                    position: p.position + back * 0.4,
                    gravity: -0.6,
                    drag: 1.5,
                    max_life: rng.range(0.4, 0.7),
                    size: Vec2::splat(0.22),
                    end_scale: 2.6,
                    color: Vec4::new(0.35, 0.32, 0.3, 0.5),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
        }
        ArsenalProjectile::FlakShell => {
            if chance(rng, 35.0) {
                effects.spawn(Particle {
                    position: p.position + back * 0.2,
                    gravity: -0.3,
                    drag: 1.5,
                    max_life: 0.35,
                    size: Vec2::splat(0.14),
                    end_scale: 2.0,
                    color: Vec4::new(0.55, 0.5, 0.52, 0.4),
                    texture: FxTex::Smoke,
                    ..Default::default()
                });
            }
            if chance(rng, 25.0) {
                let rose = ability_color(Ability::Flak);
                effects.glow(p.position, (rose.truncate() * 1.6).extend(0.8), 0.3, 0.05);
            }
        }
        ArsenalProjectile::Limpet => {}
        ArsenalProjectile::MortarShell => {
            if chance(rng, 45.0) {
                effects.spawn(Particle {
                    position: p.position + back * 0.35,
                    gravity: -0.4,
                    drag: 1.2,
                    max_life: rng.range(0.6, 1.0),
                    size: Vec2::splat(0.3),
                    end_scale: 3.0,
                    color: Vec4::new(0.4, 0.37, 0.38, 0.5),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
            if chance(rng, 30.0) {
                effects.spawn(Particle {
                    position: p.position + dir * 0.27,
                    velocity: rng.dir() * 1.5,
                    max_life: 0.15,
                    size: Vec2::splat(0.08),
                    color: Vec4::new(1.0, 0.85, 0.55, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
        }
        ArsenalProjectile::Flare => {
            // Sputtering sparks and a thick smoke tail.
            for _ in 0..2 {
                if chance(rng, 70.0) {
                    effects.spawn(Particle {
                        position: p.position + dir * 0.1,
                        velocity: rng.dir() * 3.0 + back * 1.5,
                        gravity: 9.0,
                        max_life: rng.range(0.2, 0.45),
                        size: Vec2::splat(0.08),
                        color: Vec4::new(1.0, 0.45, 0.3, 1.0),
                        texture: FxTex::Spark,
                        additive: true,
                        collide: Some(OnHit::Die),
                        ..Default::default()
                    });
                }
            }
            if chance(rng, 40.0) {
                effects.spawn(Particle {
                    position: p.position,
                    gravity: -0.8,
                    drag: 1.0,
                    max_life: rng.range(0.8, 1.3),
                    size: Vec2::splat(0.25),
                    end_scale: 3.5,
                    color: Vec4::new(0.6, 0.45, 0.45, 0.45),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
        }
    }
}

/// A projectile hitting or landing: splinters, sparks, its sound. Blasting kinds go through
/// [`blast`] / [`flak_airburst`] instead (call those for them).
pub fn projectile_impact(
    effects: &mut Effects,
    audio: &mut Audio,
    rng: &mut Rng,
    kind: ArsenalProjectile,
    position: Vec3,
    normal: Vec3,
) {
    let n = normal.normalize_or(Vec3::Y);
    match kind {
        ArsenalProjectile::Arrow | ArsenalProjectile::PinningArrow => {
            // Splinters and a puff of dust where it drove in.
            for _ in 0..6 {
                effects.spawn(Particle {
                    position: position + n * 0.05,
                    velocity: (n + rng.dir() * 0.8) * rng.range(2.0, 4.5),
                    gravity: 14.0,
                    max_life: rng.range(0.3, 0.6),
                    size: Vec2::splat(0.07),
                    color: Vec4::new(0.6, 0.45, 0.3, 1.0),
                    texture: FxTex::Shard,
                    rotation: rng.angle(),
                    spin: rng.range(-10.0, 10.0),
                    collide: Some(OnHit::Bounce(0.3)),
                    ..Default::default()
                });
            }
            effects.dust(position, 4, 0.3);
            if kind == ArsenalProjectile::PinningArrow {
                effects.glow(position, Vec4::new(0.7, 0.85, 1.6, 0.9), 0.9, 0.18);
            }
        }
        ArsenalProjectile::Rivet => {
            effects.sparks(position + n * 0.03, n, 7);
            effects.glow(position, Vec4::new(1.6, 0.6, 0.2, 0.9), 0.4, 0.12);
            effects.decal(position, n, 0.1, DecalKind::Scorch);
        }
        ArsenalProjectile::Limpet => {
            effects.sparks(position + n * 0.05, n, 5);
        }
        ArsenalProjectile::Flare => {
            effects.sparks(position + n * 0.05, n, 12);
            effects.glow(position, Vec4::new(1.8, 0.5, 0.3, 1.0), 1.4, 0.3);
        }
        _ => {}
    }
    if let Some(sfx) = kind.impact_sfx() {
        audio.play(sfx, PlayParams::at(position));
    }
}

// ---------------------------------------------------------------------------------------------
// Blasts

/// What a new explosion looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blast {
    /// A Cinderbox rocket (small, three in a row).
    Cinder,
    /// Limpets detonating.
    Limpet,
    /// The mortar shell (the biggest).
    Mortar,
    /// An Ember Arrow bursting into flame.
    EmberArrow,
}

impl Blast {
    pub fn sfx(self) -> Sfx {
        match self {
            Blast::Cinder => Sfx::CinderBlast,
            Blast::Limpet => Sfx::LimpetDetonate,
            Blast::Mortar => Sfx::MortarBlast,
            Blast::EmberArrow => Sfx::EmberArrowBurst,
        }
    }

    /// Fireball colour.
    fn color(self) -> Vec4 {
        match self {
            Blast::Cinder => Vec4::new(1.0, 0.5, 0.18, 1.0),
            Blast::Limpet => Vec4::new(1.0, 0.85, 0.35, 1.0),
            Blast::Mortar => Vec4::new(1.0, 0.65, 0.4, 1.0),
            Blast::EmberArrow => Vec4::new(1.0, 0.45, 0.1, 1.0),
        }
    }
}

/// A blast of radius `radius` at `position`: flash, fireball, smoke, sparks, ring, light,
/// scorch, shake and sound. The Mortar adds a lilac shock ring and heavy debris; the Ember
/// Arrow a fountain of embers that settle and keep glowing.
pub fn blast(ctx: &mut FxCtx, kind: Blast, position: Vec3, radius: f32) {
    let r = radius.max(0.8);
    let hot = kind.color();
    let e = &mut *ctx.effects;
    e.spawn(Particle {
        position,
        max_life: 0.12,
        size: Vec2::splat(r * 0.9),
        end_scale: 1.5,
        color: Vec4::new(1.0, 0.95, 0.8, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    let puffs = match kind {
        Blast::Cinder => 6,
        Blast::Mortar => 16,
        _ => 9,
    };
    for _ in 0..puffs {
        let d = ctx.rng.dir();
        e.spawn(Particle {
            position: position + d * r * 0.15,
            velocity: d * ctx.rng.range(2.0, 6.0) * (r / 4.5) + Vec3::Y * 1.5,
            gravity: -1.0,
            drag: 3.0,
            max_life: ctx.rng.range(0.25, 0.5),
            size: Vec2::splat(r * ctx.rng.range(0.3, 0.5)),
            end_scale: 1.8,
            color: hot,
            texture: FxTex::MuzzleFlash,
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-3.0, 3.0),
            ..Default::default()
        });
    }
    e.smoke(position, if kind == Blast::Mortar { 12 } else { 5 });
    e.sparks(
        position,
        Vec3::Y,
        if kind == Blast::Mortar { 26 } else { 12 },
    );
    e.shockwave(position, r);
    match kind {
        Blast::Mortar => {
            let lilac = ability_color(Ability::Mortar);
            ctx.beams.ring(
                position + Vec3::Y * 0.2,
                Vec3::Y,
                0.5,
                r * 1.1,
                0.12,
                (lilac.truncate() * 1.8).extend(0.9),
                0.35,
            );
            for _ in 0..14 {
                let v = ctx.rng.dir() * ctx.rng.range(6.0, 13.0) + Vec3::Y * 5.0;
                ctx.effects.spawn(Particle {
                    position,
                    velocity: v,
                    gravity: 18.0,
                    drag: 0.6,
                    max_life: ctx.rng.range(0.8, 1.4),
                    size: Vec2::splat(0.18),
                    color: Vec4::new(0.32, 0.3, 0.29, 1.0),
                    end_alpha: 0.4,
                    texture: FxTex::Shard,
                    rotation: ctx.rng.angle(),
                    spin: ctx.rng.range(-8.0, 8.0),
                    collide: Some(OnHit::Bounce(0.3)),
                    ..Default::default()
                });
            }
        }
        Blast::EmberArrow => {
            for _ in 0..22 {
                let v = ctx.rng.dir() * ctx.rng.range(2.0, 7.0) + Vec3::Y * 4.0;
                ctx.effects.spawn(Particle {
                    position,
                    velocity: v,
                    gravity: 9.0,
                    drag: 1.0,
                    max_life: ctx.rng.range(0.8, 1.8),
                    size: Vec2::splat(0.1),
                    color: Vec4::new(1.0, 0.55, 0.15, 1.0),
                    end_alpha: 0.3,
                    texture: FxTex::Spark,
                    additive: true,
                    collide: Some(OnHit::Bounce(0.1)),
                    ..Default::default()
                });
            }
        }
        Blast::Limpet => {
            let lamp = ability_color(Ability::Limpet);
            ctx.beams.ring(
                position,
                Vec3::Y,
                0.3,
                r,
                0.08,
                (lamp.truncate() * 1.6).extend(0.8),
                0.2,
            );
        }
        Blast::Cinder => {}
    }
    if let Some(w) = ctx.world
        && let Some(h) = w.cast_ray(position + Vec3::Y * 0.3, Vec3::NEG_Y, r * 0.6, MASK_SHOTS)
    {
        ctx.effects
            .decal(h.point, h.normal, r * 0.55, DecalKind::Scorch);
    }
    ctx.beams.light(
        position + Vec3::Y * 0.5,
        hot.truncate() * 3.0,
        r * 3.0,
        if kind == Blast::Mortar { 0.5 } else { 0.3 },
    );
    let d = position.distance(ctx.listener);
    let strength = match kind {
        Blast::Mortar => 1.0,
        Blast::Cinder => 0.35,
        _ => 0.6,
    };
    ctx.feedback.shake(shake_at(d, r * 4.0, strength));
    ctx.audio.play(kind.sfx(), PlayParams::at(position));
}

/// The Flak shell's airburst: a flash and a ring of `pellets` hot tracers flung out
/// perpendicular to its flight (`dir`), each ending in a little spark.
pub fn flak_airburst(ctx: &mut FxCtx, position: Vec3, dir: Vec3, pellets: usize, range: f32) {
    let axis = dir.normalize_or(Vec3::NEG_Z);
    let side = axis.any_orthonormal_vector();
    let up = axis.cross(side);
    let rose = ability_color(Ability::Flak);
    let tracer = Vec4::new(2.0, 1.4, 0.9, 0.9);
    let n = pellets.max(1);
    for k in 0..n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU + ctx.rng.range(-0.1, 0.1);
        let out = (side * a.cos() + up * a.sin() + axis * 0.15).normalize();
        let mut end = position + out * range;
        if let Some(w) = ctx.world
            && let Some(h) = w.cast_ray(position, out, range, MASK_SHOTS)
        {
            end = h.point;
            ctx.effects.sparks(h.point + h.normal * 0.03, h.normal, 3);
        }
        ctx.beams.beam(position, end, 0.03, tracer, 0.12);
    }
    ctx.effects
        .glow(position, (rose.truncate() * 2.0).extend(1.0), 1.6, 0.15);
    ctx.effects.smoke(position, 4);
    ctx.beams.ring(
        position,
        axis,
        0.2,
        range * 0.5,
        0.06,
        (rose.truncate() * 1.8).extend(0.9),
        0.18,
    );
    ctx.beams
        .light(position, Vec3::new(2.2, 1.0, 1.4), 6.0, 0.25);
    let d = position.distance(ctx.listener);
    ctx.feedback.shake(shake_at(d, 10.0, 0.3));
    ctx.audio.play(Sfx::FlakBurst, PlayParams::at(position));
}

// ---------------------------------------------------------------------------------------------
// Hazards and status effects

/// A lingering area effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hazard {
    /// A Flare's burning patch (Burn to hostiles inside).
    BurningPatch,
}

/// Per-frame particles of a hazard of `radius` at `position` (`left` = fraction of its life
/// remaining: it dies down at the end).
pub fn hazard_effects(
    effects: &mut Effects,
    rng: &mut Rng,
    hazard: Hazard,
    position: Vec3,
    radius: f32,
    left: f32,
    dt: f32,
) {
    match hazard {
        Hazard::BurningPatch => {
            let r = radius.max(0.5);
            let strength = left.clamp(0.0, 1.0).sqrt();
            // Tongues of flame licking up all over the patch.
            let flames = (dt * 70.0 * r / 3.0 * strength).max(0.0);
            let mut budget = flames;
            while budget > 0.0 {
                if budget < 1.0 && rng.unit() > budget {
                    break;
                }
                budget -= 1.0;
                let a = rng.angle();
                let d = rng.unit().sqrt() * r;
                let at = position + Vec3::new(a.cos() * d, 0.05, a.sin() * d);
                effects.spawn(Particle {
                    position: at,
                    velocity: Vec3::Y * rng.range(1.2, 2.6),
                    drag: 1.0,
                    max_life: rng.range(0.25, 0.5),
                    size: Vec2::splat(rng.range(0.35, 0.6) * (0.6 + 0.4 * strength)),
                    end_scale: 0.3,
                    color: Vec4::new(1.0, rng.range(0.35, 0.6), 0.15, 1.0),
                    texture: FxTex::MuzzleFlash,
                    additive: true,
                    rotation: rng.angle(),
                    spin: rng.range(-2.0, 2.0),
                    ..Default::default()
                });
            }
            if rng.unit() < dt * 14.0 * r / 3.0 {
                let a = rng.angle();
                let at = position + Vec3::new(a.cos(), 0.1, a.sin()) * rng.range(0.0, r);
                effects.spawn(Particle {
                    position: at,
                    velocity: Vec3::Y * rng.range(2.0, 4.0) + rng.dir() * 0.6,
                    gravity: -0.5,
                    max_life: rng.range(0.6, 1.2),
                    size: Vec2::splat(0.07),
                    color: Vec4::new(1.0, 0.6, 0.2, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
            if rng.unit() < dt * 6.0 * strength {
                let a = rng.angle();
                let at = position + Vec3::new(a.cos(), 0.5, a.sin()) * rng.range(0.0, r * 0.7);
                effects.spawn(Particle {
                    position: at,
                    velocity: Vec3::Y * 1.2,
                    gravity: -0.4,
                    drag: 0.8,
                    max_life: rng.range(1.0, 1.6),
                    size: Vec2::splat(0.4),
                    end_scale: 3.0,
                    color: Vec4::new(0.3, 0.27, 0.26, 0.45),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
        }
    }
}

/// The light a hazard casts.
pub fn hazard_light(
    hazard: Hazard,
    position: Vec3,
    radius: f32,
    left: f32,
    time: f64,
) -> PointLight {
    match hazard {
        Hazard::BurningPatch => {
            let flicker = 0.8
                + 0.12 * ((time * 13.0).sin() as f32)
                + 0.08 * ((time * 29.0 + 1.3).sin() as f32);
            PointLight {
                position: position + Vec3::Y * 0.6,
                color: Vec3::new(2.4, 1.0, 0.3) * flicker * left.clamp(0.0, 1.0).sqrt(),
                radius: radius * 2.2,
            }
        }
    }
}

/// A status effect shown on an enemy or a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Burning: ember flicker, little flames, smoke.
    Burning,
    /// Pinned: steel-blue shackle ring at the feet with three chains to stakes.
    Pinned,
}

/// Body tint of something burning: an orange, flickering glow over its own colours.
pub fn burn_tint(time: f64, seed: u32) -> Vec4 {
    let t = time + (seed % 101) as f64 * 0.173;
    let f = 0.5 + 0.3 * (t * 11.0).sin() as f32 + 0.2 * (t * 27.0).sin() as f32;
    Vec4::new(1.0 + 0.45 * f, 0.88 + 0.12 * f, 0.72, 1.0)
}

/// Per-frame visuals of a status on a body standing at `feet`, `height` tall and `radius`
/// wide.
#[allow(clippy::too_many_arguments)]
pub fn status_effects(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    status: Status,
    feet: Vec3,
    height: f32,
    radius: f32,
    dt: f32,
    time: f64,
) {
    match status {
        Status::Burning => {
            let on_body = |rng: &mut Rng| {
                let a = rng.angle();
                feet + Vec3::new(
                    a.cos() * radius * 0.8,
                    rng.range(0.15, 0.95) * height,
                    a.sin() * radius * 0.8,
                )
            };
            if rng.unit() < dt * 30.0 {
                effects.spawn(Particle {
                    position: on_body(rng),
                    velocity: Vec3::Y * rng.range(1.0, 2.2),
                    drag: 1.0,
                    max_life: rng.range(0.2, 0.4),
                    size: Vec2::splat(rng.range(0.22, 0.4)),
                    end_scale: 0.3,
                    color: Vec4::new(1.0, rng.range(0.4, 0.6), 0.15, 1.0),
                    texture: FxTex::MuzzleFlash,
                    additive: true,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
            if rng.unit() < dt * 40.0 {
                effects.spawn(Particle {
                    position: on_body(rng),
                    velocity: Vec3::Y * rng.range(1.5, 3.5) + rng.dir() * 0.7,
                    gravity: -0.6,
                    max_life: rng.range(0.5, 1.0),
                    size: Vec2::splat(0.06),
                    color: Vec4::new(1.0, 0.65, 0.25, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
            if rng.unit() < dt * 8.0 {
                effects.spawn(Particle {
                    position: feet + Vec3::Y * height * rng.range(0.7, 1.1),
                    velocity: Vec3::Y * 1.0,
                    gravity: -0.5,
                    drag: 0.8,
                    max_life: rng.range(0.8, 1.3),
                    size: Vec2::splat(0.3),
                    end_scale: 2.5,
                    color: Vec4::new(0.3, 0.28, 0.27, 0.4),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
        }
        Status::Pinned => {
            let steel = Vec4::new(0.9, 1.3, 2.4, 0.95);
            let pulse = 0.8 + 0.2 * ((time * 6.0).sin() as f32);
            let ring_r = radius + 0.35;
            let ring = |k: usize| {
                let a = k as f32 / 8.0 * std::f32::consts::TAU + (time * 0.6) as f32;
                feet + Vec3::new(a.cos() * ring_r, 0.08, a.sin() * ring_r)
            };
            for k in 0..8 {
                beams.frame_beam(ring(k), ring(k + 1), 0.05, steel * pulse);
            }
            // Three chains from the waist down to stakes on the ring.
            let waist = feet + Vec3::Y * height * 0.45;
            for k in 0..3 {
                let stake = ring(k * 8 / 3);
                beams.frame_beam(waist, stake, 0.035, steel * (0.7 * pulse));
                beams.frame_beam(stake, stake + Vec3::Y * 0.45, 0.06, steel * pulse);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Bookkeeping and previews

/// Something staged client-side by [`InGame::preview_arsenal`] (tools: look at the visuals
/// without the simulation making them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArsenalPreview {
    /// A projectile flying from `from` (falling with `gravity`) that sticks or bursts where it
    /// meets the level (or after `life` seconds).
    Projectile {
        kind: ArsenalProjectile,
        from: Vec3,
        velocity: Vec3,
        gravity: f32,
        life: f32,
    },
    BurningPatch {
        position: Vec3,
        radius: f32,
        seconds: f32,
    },
    /// A status on a replicated entity (enemy or player).
    Status {
        entity: NetId,
        status: Status,
        seconds: f32,
    },
    Blast {
        blast: Blast,
        position: Vec3,
        radius: f32,
    },
    FlakBurst {
        position: Vec3,
        dir: Vec3,
    },
}

#[derive(Clone, Copy, Debug)]
struct Flying {
    view: ProjectileView,
    gravity: f32,
    life: f32,
}

#[derive(Clone, Copy, Debug)]
struct Patch {
    position: Vec3,
    radius: f32,
    age: f32,
    seconds: f32,
}

/// Arsenal visuals that need state from frame to frame: staged previews (projectiles,
/// patches, statuses) and pending blasts.
#[derive(Default)]
pub struct ArsenalView {
    flying: Vec<Flying>,
    patches: Vec<Patch>,
    statuses: HashMap<(NetId, u8), (Status, f32)>,
    /// Blasts waiting for an `FxCtx` (staged, or a preview projectile that burst).
    blasts: Vec<(Blast, Vec3, f32)>,
    flak: Vec<(Vec3, Vec3)>,
    /// Seconds since the next burn sizzle per burning entity.
    sizzle: HashMap<NetId, f32>,
    time: f64,
}

/// Burning-patch radius a Flare leaves (docs/expansion.md: 3 m).
const FLARE_PATCH_RADIUS: f32 = 3.0;

impl ArsenalView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self {
            time: self.time,
            ..Self::default()
        };
    }

    fn preview(&mut self, p: ArsenalPreview) {
        match p {
            ArsenalPreview::Projectile {
                kind,
                from,
                velocity,
                gravity,
                life,
            } => self.flying.push(Flying {
                view: ProjectileView {
                    kind,
                    position: from,
                    velocity,
                    stuck: false,
                    facing: velocity,
                    age: 0.0,
                    seed: self.flying.len() as u32 * 7919 + 13,
                },
                gravity,
                life,
            }),
            ArsenalPreview::BurningPatch {
                position,
                radius,
                seconds,
            } => self.patches.push(Patch {
                position,
                radius,
                age: 0.0,
                seconds,
            }),
            ArsenalPreview::Status {
                entity,
                status,
                seconds,
            } => {
                self.statuses
                    .insert((entity, status as u8), (status, seconds));
            }
            ArsenalPreview::Blast {
                blast,
                position,
                radius,
            } => self.blasts.push((blast, position, radius)),
            ArsenalPreview::FlakBurst { position, dir } => self.flak.push((position, dir)),
        }
    }

    /// Advances the staged projectiles (sticking / bursting where they meet the level) and
    /// patches, spawns trails, flames and status visuals.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        entities: &[EntityState],
        world: Option<&CollisionWorld>,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
        dt: f32,
    ) {
        self.time += dt as f64;
        let mut landed = Vec::new();
        for f in &mut self.flying {
            let v = &mut f.view;
            v.age += dt;
            if v.stuck {
                projectile_trail(effects, beams, rng, v, dt);
                continue;
            }
            let step = v.velocity * dt;
            let hit = world.and_then(|w| {
                let len = step.length();
                (len > 1e-5)
                    .then(|| w.cast_ray(v.position, step / len, len, MASK_SHOTS))
                    .flatten()
            });
            if let Some(h) = hit {
                v.position = h.point;
                v.facing = match v.kind {
                    // A limpet's claws grip the surface.
                    ArsenalProjectile::Limpet => -h.normal,
                    _ => v.velocity,
                };
                v.stuck = true;
                v.age = 0.0;
                landed.push((v.kind, h.point, h.normal, v.velocity));
                continue;
            }
            v.position += step;
            v.velocity.y -= f.gravity * dt;
            projectile_trail(effects, beams, rng, v, dt);
            if v.age > f.life {
                // Out of time in the air: bursting kinds burst there, the rest stop there.
                landed.push((
                    v.kind,
                    v.position,
                    -v.velocity.normalize_or(Vec3::Y),
                    v.velocity,
                ));
                v.facing = v.velocity;
                v.stuck = true;
                v.age = 0.0;
            }
        }
        for (kind, at, normal, vel) in landed {
            match kind {
                ArsenalProjectile::CinderRocket => self.blasts.push((Blast::Cinder, at, 2.6)),
                ArsenalProjectile::MortarShell => self.blasts.push((Blast::Mortar, at, 6.0)),
                ArsenalProjectile::EmberArrow => self.blasts.push((Blast::EmberArrow, at, 3.0)),
                ArsenalProjectile::FlakShell => self.flak.push((at, vel)),
                ArsenalProjectile::Flare => {
                    projectile_impact(effects, audio, rng, kind, at, normal);
                    self.patches.push(Patch {
                        position: at,
                        radius: FLARE_PATCH_RADIUS,
                        age: 0.0,
                        seconds: 4.0,
                    });
                }
                _ => projectile_impact(effects, audio, rng, kind, at, normal),
            }
        }
        // Bursting kinds and flares that landed are gone; arrows, rivets and limpets stay.
        self.flying.retain(|f| {
            let gone = matches!(
                f.view.kind,
                ArsenalProjectile::CinderRocket
                    | ArsenalProjectile::MortarShell
                    | ArsenalProjectile::EmberArrow
                    | ArsenalProjectile::FlakShell
                    | ArsenalProjectile::Flare
            ) && f.view.stuck;
            !gone && f.view.age < 30.0
        });
        for p in &mut self.patches {
            if p.age == 0.0 {
                effects.decal(p.position, Vec3::Y, p.radius * 0.9, DecalKind::Scorch);
            }
            p.age += dt;
            let left = 1.0 - p.age / p.seconds.max(0.1);
            hazard_effects(
                effects,
                rng,
                Hazard::BurningPatch,
                p.position,
                p.radius,
                left,
                dt,
            );
        }
        self.patches.retain(|p| p.age < p.seconds);
        // Statuses on entities.
        for ((id, _), (status, left)) in &mut self.statuses {
            *left -= dt;
            let Some(e) = entities.iter().find(|e| e.id == *id) else {
                continue;
            };
            let (r, h) = body_size(e);
            status_effects(
                effects, beams, rng, *status, e.position, h, r, dt, self.time,
            );
            if *status == Status::Burning {
                let t = self.sizzle.entry(*id).or_insert(0.0);
                *t -= dt;
                if *t <= 0.0 {
                    *t = 0.55;
                    audio.play(
                        Sfx::BurnSizzle,
                        PlayParams::at(e.position + Vec3::Y * h * 0.5),
                    );
                }
            }
        }
        self.statuses.retain(|_, (_, left)| *left > 0.0);
        let burning: Vec<NetId> = self.statuses.keys().map(|(id, _)| *id).collect();
        self.sizzle.retain(|id, _| burning.contains(id));
    }

    /// Blasts are waiting for [`ArsenalView::play_pending`].
    pub fn has_pending(&self) -> bool {
        !self.blasts.is_empty() || !self.flak.is_empty()
    }

    /// Plays staged blasts (they need the Fx context: shake, lights, sounds).
    pub fn play_pending(&mut self, ctx: &mut FxCtx) {
        for (kind, at, r) in std::mem::take(&mut self.blasts) {
            blast(ctx, kind, at, r);
        }
        for (at, dir) in std::mem::take(&mut self.flak) {
            flak_airburst(ctx, at, dir, 10, 6.0);
        }
    }

    /// The status an entity shows (staged previews; the replicated flags in a later wiring).
    pub fn status_of(&self, id: NetId) -> impl Iterator<Item = Status> + '_ {
        self.statuses
            .iter()
            .filter(move |((e, _), _)| *e == id)
            .map(|(_, (s, _))| *s)
    }

    pub fn draw(
        &self,
        lib: Option<&ModelLibrary>,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        if let Some(lib) = lib {
            for f in &self.flying {
                draw_projectile(lib, &f.view, self.time, draws, lights);
            }
        }
        for p in &self.patches {
            let left = 1.0 - p.age / p.seconds.max(0.1);
            if lights.len() < 48 {
                lights.push(hazard_light(
                    Hazard::BurningPatch,
                    p.position,
                    p.radius,
                    left,
                    self.time,
                ));
            }
        }
    }
}

/// Body size (radius, height) of a replicated enemy or player for status visuals.
fn body_size(e: &EntityState) -> (f32, f32) {
    if e.kind == shared::protocol::kind::PLAYER {
        (0.45, 1.8)
    } else {
        enemy_size(e.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_projectile_has_its_own_model_and_a_sane_transform() {
        let mut models: Vec<ModelId> = ArsenalProjectile::ALL.iter().map(|p| p.model()).collect();
        models.sort_unstable();
        models.dedup();
        assert_eq!(models.len(), ArsenalProjectile::ALL.len());
        for kind in ArsenalProjectile::ALL {
            for stuck in [false, true] {
                let p = ProjectileView {
                    kind,
                    position: Vec3::new(1.0, 2.0, 3.0),
                    velocity: if stuck {
                        Vec3::ZERO
                    } else {
                        Vec3::new(0.0, -3.0, -40.0)
                    },
                    stuck,
                    facing: Vec3::NEG_Z,
                    age: 0.1,
                    seed: 7,
                };
                let m = projectile_transform(&p, 12.5);
                assert!(m.is_finite(), "{kind:?}");
                if !stuck && kind != ArsenalProjectile::Limpet {
                    // Flying: the model's nose (-Z) points along the velocity (a limpet tumbles).
                    let nose = m.transform_vector3(Vec3::NEG_Z).normalize();
                    assert!(nose.dot(p.velocity.normalize()) > 0.95, "{kind:?}");
                }
            }
        }
        // A stuck arrow sinks its tip into the surface: the origin stays in front of it.
        let arrow = ProjectileView {
            kind: ArsenalProjectile::Arrow,
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            stuck: true,
            facing: Vec3::NEG_Z,
            age: 5.0,
            seed: 0,
        };
        let origin = projectile_transform(&arrow, 0.0).transform_point3(Vec3::ZERO);
        assert!(origin.z > 0.05 && origin.z < 0.2, "{origin}");
    }

    #[test]
    fn limpets_blink_faster_with_time_and_burn_tints_stay_warm() {
        let blinks = |from: f32| {
            (0..200)
                .map(|i| limpet_lamp_on(from + i as f32 * 0.005))
                .collect::<Vec<_>>()
                .windows(2)
                .filter(|w| w[0] != w[1])
                .count()
        };
        assert!(blinks(5.0) > blinks(0.0), "faster once armed a while");
        for i in 0..50 {
            let t = burn_tint(i as f64 * 0.07, i);
            assert!(t.is_finite() && t.x >= t.y && t.y >= t.z, "{t}");
        }
    }

    #[test]
    fn previews_fly_stick_burst_and_expire() {
        let mut view = ArsenalView::new();
        let mut effects = Effects::headless();
        let mut beams = Beams::headless();
        let mut audio = Audio::disabled();
        let mut rng = Rng::default();
        view.preview(ArsenalPreview::Projectile {
            kind: ArsenalProjectile::MortarShell,
            from: Vec3::ZERO,
            velocity: Vec3::new(0.0, 5.0, -10.0),
            gravity: 20.0,
            life: 0.5,
        });
        view.preview(ArsenalPreview::Projectile {
            kind: ArsenalProjectile::Arrow,
            from: Vec3::ZERO,
            velocity: Vec3::new(0.0, 0.0, -60.0),
            gravity: 0.0,
            life: 0.2,
        });
        view.preview(ArsenalPreview::BurningPatch {
            position: Vec3::ZERO,
            radius: 3.0,
            seconds: 1.0,
        });
        for _ in 0..40 {
            view.update(
                &[],
                None,
                &mut effects,
                &mut beams,
                &mut audio,
                &mut rng,
                1.0 / 60.0,
            );
        }
        // The mortar timed out and is waiting to blast; the arrow (no level to stick in) is
        // still around; the patch burns.
        assert_eq!(view.blasts.len(), 1);
        assert_eq!(view.flying.len(), 1);
        assert_eq!(view.patches.len(), 1);
        assert!(effects.particle_count() > 20, "trails and flames");
        for _ in 0..60 {
            view.update(
                &[],
                None,
                &mut effects,
                &mut beams,
                &mut audio,
                &mut rng,
                1.0 / 60.0,
            );
        }
        assert!(view.patches.is_empty(), "the patch burnt out");
    }
}
