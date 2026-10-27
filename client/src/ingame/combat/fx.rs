//! Combat visuals that are not entities: tracer beams (a stretched unlit box mesh, depth
//! tested), jagged arcs (Arclash zaps and chains, Wisp zaps), expanding / collapsing rings
//! (Discharge nova, Gravewell implosion), per-frame sigils (brands), temporary point lights
//! (muzzle flashes, explosions), and the playback of the server's cosmetic [`Fx`] events, each
//! played when the render clock reaches its tick so it lines up with the interpolated entities
//! it belongs to.

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, MeshId, PointLight, TextureId};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use engine::render::Renderer;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::protocol::{Fx, FxEvent, NetId, QPos, kind};
use shared::sim::{Attack, PlayerId};

use super::super::effects::{DecalKind, Effects, FxTex, OnHit, Particle};
use super::super::feedback::Feedback;
use super::super::remote_players::RemotePlayers;
use super::arsenal::{self, Blast};
use super::{Rng, attack_style, enemy_fx};
use crate::models::{WeaponKind, WeaponLook as _};
use crate::palette::palette;

/// A fading tracer segment.
#[derive(Clone, Copy, Debug)]
struct Beam {
    a: Vec3,
    b: Vec3,
    width: f32,
    color: Vec4,
    life: f32,
    max_life: f32,
}

/// A ring of beam segments whose radius moves from `r0` to `r1` over its life (in the plane
/// perpendicular to `axis`).
#[derive(Clone, Copy, Debug)]
struct Ring {
    center: Vec3,
    axis: Vec3,
    r0: f32,
    r1: f32,
    width: f32,
    color: Vec4,
    life: f32,
    max_life: f32,
}

/// A point light that fades out.
#[derive(Clone, Copy, Debug)]
struct TempLight {
    position: Vec3,
    color: Vec3,
    radius: f32,
    life: f32,
    max_life: f32,
}

/// Tracers, arcs, rings, sigils and temporary lights.
pub struct Beams {
    mesh: Option<MeshId>,
    texture: Option<TextureId>,
    beams: Vec<Beam>,
    rings: Vec<Ring>,
    /// Drawn once, dropped at the next update (per-frame shapes: brand sigils).
    frame: Vec<Beam>,
    lights: Vec<TempLight>,
}

/// Most tracers alive at once (oldest replaced).
const MAX_BEAMS: usize = 256;
const MAX_RINGS: usize = 16;
const MAX_FRAME_BEAMS: usize = 320;
const MAX_TEMP_LIGHTS: usize = 8;
/// Segments of a ring.
const RING_SEGMENTS: usize = 28;

/// Brand sigil colour (standard palette).
pub const BRAND_COLOR: Vec4 = Vec4::new(2.2, 0.35, 0.6, 0.95);

/// Brand sigil colour in the player's colour-vision palette (`palette().brand`).
pub fn brand_color() -> Vec4 {
    let p = palette();
    p.effect(BRAND_COLOR, p.brand, 1.6)
}
/// Arclash arc colour (zaps, chains, the Discharge nova).
pub const ARC_COLOR: Vec4 = Vec4::new(1.2, 1.7, 2.4, 0.95);

/// A colour from the weapon models' palette, boosted for additive / unlit effects.
fn hot(c: Vec4, k: f32) -> Vec4 {
    (c.truncate() * k).extend(c.w)
}

impl Beams {
    pub fn new(renderer: &mut Renderer) -> Self {
        let mesh = engine::assets::meshgen::cuboid(Vec3::splat(0.5), Vec4::ONE).build();
        Self {
            mesh: Some(renderer.create_mesh(&mesh)),
            texture: Some(renderer.white_texture()),
            ..Self::headless()
        }
    }

    /// Without GPU resources (tests): beams are tracked but not drawn.
    pub fn headless() -> Self {
        Self {
            mesh: None,
            texture: None,
            beams: Vec::new(),
            rings: Vec::new(),
            frame: Vec::new(),
            lights: Vec::new(),
        }
    }

    pub fn free(&mut self, renderer: &mut Renderer) {
        if let Some(m) = self.mesh.take() {
            renderer.free_mesh(m);
        }
    }

    pub fn clear(&mut self) {
        self.beams.clear();
        self.rings.clear();
        self.frame.clear();
        self.lights.clear();
    }

    pub fn beam_count(&self) -> usize {
        self.beams.len()
    }

    /// A tracer from `a` to `b`.
    pub fn beam(&mut self, a: Vec3, b: Vec3, width: f32, color: Vec4, life: f32) {
        if !(a.is_finite() && b.is_finite()) || a.distance_squared(b) < 1e-4 {
            return;
        }
        if self.beams.len() >= MAX_BEAMS {
            self.beams.remove(0);
        }
        self.beams.push(Beam {
            a,
            b,
            width,
            color,
            life: 0.0,
            max_life: life.max(0.01),
        });
    }

    /// A jagged electric arc from `a` to `b`: a zig-zag of beams with random sideways kinks
    /// (bigger on longer arcs), plus a thin bright core.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&mut self, a: Vec3, b: Vec3, width: f32, color: Vec4, life: f32, rng: &mut Rng) {
        let d = b - a;
        let len = d.length();
        if !(a.is_finite() && b.is_finite()) || len < 0.05 {
            return;
        }
        let dir = d / len;
        let side = dir.any_orthonormal_vector();
        let up = dir.cross(side);
        let segments = ((len / 0.7) as usize).clamp(3, 16);
        let jag = (0.08 + len * 0.03).min(0.5);
        let mut prev = a;
        for i in 1..=segments {
            let t = i as f32 / segments as f32;
            let p = if i == segments {
                b
            } else {
                // Kinks fade out towards both ends (calm at the muzzle and the target).
                let taper = (t * (1.0 - t) * 4.0).min(1.0);
                let off = side * rng.range(-1.0, 1.0) + up * rng.range(-1.0, 1.0);
                a + d * t + off * jag * taper
            };
            self.beam(prev, p, width, color, life);
            self.beam(
                prev,
                p,
                width * 0.35,
                Vec4::new(2.5, 2.5, 2.6, color.w),
                life * 0.7,
            );
            prev = p;
        }
    }

    /// A ring around `center` in the plane perpendicular to `axis`, its radius moving from
    /// `r0` to `r1` over `life` seconds (Discharge nova: out; Gravewell implosion: in).
    #[allow(clippy::too_many_arguments)]
    pub fn ring(
        &mut self,
        center: Vec3,
        axis: Vec3,
        r0: f32,
        r1: f32,
        width: f32,
        color: Vec4,
        life: f32,
    ) {
        if !center.is_finite() {
            return;
        }
        if self.rings.len() >= MAX_RINGS {
            self.rings.remove(0);
        }
        self.rings.push(Ring {
            center,
            axis: axis.normalize_or(Vec3::Y),
            r0,
            r1,
            width,
            color,
            life: 0.0,
            max_life: life.max(0.01),
        });
    }

    /// A brand sigil for this frame only, facing `normal` at `center`: a diamond of `radius`
    /// turning one way and a point-down triangle inside it turning the other (a brand mark,
    /// not any real-world symbol).
    pub fn sigil(&mut self, center: Vec3, normal: Vec3, radius: f32, color: Vec4, angle: f32) {
        if !center.is_finite() || self.frame.len() + 7 > MAX_FRAME_BEAMS {
            return;
        }
        let n = normal.normalize_or(Vec3::Y);
        // Keep "down" pointing down on screen: the in-plane axes follow world up.
        let up = (Vec3::Y - n * n.dot(Vec3::Y)).normalize_or(n.any_orthonormal_vector());
        let right = up.cross(n);
        let point = |a: f32, r: f32| center + (right * a.sin() + up * a.cos()) * r;
        let mut edge = |a: Vec3, b: Vec3, width: f32| {
            self.frame.push(Beam {
                a,
                b,
                width,
                color,
                life: 0.0,
                max_life: 1.0,
            });
        };
        let quarter = std::f32::consts::FRAC_PI_2;
        for i in 0..4 {
            let a = angle * 0.35 + i as f32 * quarter;
            edge(point(a, radius), point(a + quarter, radius), radius * 0.1);
        }
        let third = std::f32::consts::TAU / 3.0;
        let tilt = std::f32::consts::PI + (angle * 2.0).sin() * 0.25;
        for i in 0..3 {
            let a = tilt + i as f32 * third;
            edge(
                point(a, radius * 0.6),
                point(a + third, radius * 0.6),
                radius * 0.08,
            );
        }
    }

    /// A beam drawn this frame only (per-frame shapes: the Pinned shackles).
    pub fn frame_beam(&mut self, a: Vec3, b: Vec3, width: f32, color: Vec4) {
        if !(a.is_finite() && b.is_finite()) || self.frame.len() >= MAX_FRAME_BEAMS {
            return;
        }
        self.frame.push(Beam {
            a,
            b,
            width,
            color,
            life: 0.0,
            max_life: 1.0,
        });
    }

    /// A light that fades over `life` seconds.
    pub fn light(&mut self, position: Vec3, color: Vec3, radius: f32, life: f32) {
        if self.lights.len() >= MAX_TEMP_LIGHTS {
            self.lights.remove(0);
        }
        self.lights.push(TempLight {
            position,
            color,
            radius,
            life: 0.0,
            max_life: life.max(0.01),
        });
    }

    pub fn update(&mut self, dt: f32) {
        for b in &mut self.beams {
            b.life += dt;
        }
        self.beams.retain(|b| b.life < b.max_life);
        for r in &mut self.rings {
            r.life += dt;
        }
        self.rings.retain(|r| r.life < r.max_life);
        self.frame.clear();
        for l in &mut self.lights {
            l.life += dt;
        }
        self.lights.retain(|l| l.life < l.max_life);
    }

    pub fn draw(&self, draws: &mut Vec<DrawItem>, lights: &mut Vec<PointLight>) {
        for l in &self.lights {
            let f = 1.0 - l.life / l.max_life;
            lights.push(PointLight {
                position: l.position,
                color: l.color * f,
                radius: l.radius * (0.6 + 0.4 * f),
            });
        }
        let (Some(mesh), Some(texture)) = (self.mesh, self.texture) else {
            return;
        };
        let mut segment = |a: Vec3, b: Vec3, w: f32, color: Vec4| {
            let d = b - a;
            let len = d.length();
            if len < 1e-4 || !len.is_finite() {
                return;
            }
            let rot = Quat::from_rotation_arc(Vec3::NEG_Z, d / len);
            let transform =
                Mat4::from_scale_rotation_translation(Vec3::new(w, w, len), rot, (a + b) * 0.5);
            draws.push(DrawItem {
                mesh,
                texture,
                transform,
                tint: color,
                unlit: true,
            });
        };
        for b in self.beams.iter().chain(&self.frame) {
            let f = 1.0 - b.life / b.max_life;
            let mut color = b.color;
            color.w *= f;
            segment(b.a, b.b, b.width * (0.4 + 0.6 * f), color);
        }
        for r in &self.rings {
            let k = r.life / r.max_life;
            let ease = 1.0 - (1.0 - k) * (1.0 - k);
            let radius = r.r0 + (r.r1 - r.r0) * ease;
            let u = r.axis.any_orthonormal_vector();
            let v = r.axis.cross(u);
            let mut color = r.color;
            color.w *= 1.0 - k * k;
            let at = |i: usize| {
                let a = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
                r.center + (u * a.cos() + v * a.sin()) * radius
            };
            for i in 0..RING_SEGMENTS {
                segment(at(i), at(i + 1), r.width * (1.0 - 0.6 * k), color);
            }
        }
    }
}

/// Fx events waiting for the render clock.
#[derive(Default)]
pub struct FxQueue {
    pending: Vec<FxEvent>,
    pub played: u64,
}

impl FxQueue {
    pub fn push(&mut self, events: Vec<FxEvent>) {
        self.pending.extend(events);
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Events due at `render_tick` (and anything very old), in sequence order. Explosions of
    /// `local`'s own projectiles are due immediately: those projectiles are drawn at the
    /// present time, not at the render tick.
    pub fn due(&mut self, render_tick: f64, local: Option<PlayerId>) -> Vec<FxEvent> {
        let mut due = Vec::new();
        let own = local.map(|p| p.0 as u16);
        self.pending.retain(|e| {
            let t = e.tick as f64;
            let mine = matches!(e.fx, Fx::Explosion { owner, .. } if Some(owner) == own);
            // Due when displayed; stale ones (render clock jumped) are played anyway.
            if mine || t <= render_tick + 0.5 || t > render_tick + 120.0 {
                due.push(e.clone());
                false
            } else {
                true
            }
        });
        due.sort_by_key(|e| e.seq);
        self.played += due.len() as u64;
        due
    }
}

/// Everything an Fx needs to play.
pub struct FxCtx<'a> {
    pub audio: &'a mut Audio,
    pub effects: &'a mut Effects,
    pub feedback: &'a mut Feedback,
    pub beams: &'a mut Beams,
    pub remote: &'a mut RemotePlayers,
    pub world: Option<&'a CollisionWorld>,
    /// Interpolated entities (enemy positions for their sounds).
    pub entities: &'a [shared::protocol::EntityState],
    pub local: Option<PlayerId>,
    pub listener: Vec3,
    /// Local player's chest lantern (kindling stream target).
    pub local_chest: Option<Vec3>,
    /// Last drawn position of thorns (for `ThornExpired`).
    pub thorn_position: &'a dyn Fn(NetId) -> Option<Vec3>,
    pub rng: &'a mut Rng,
}

fn q(p: QPos) -> Vec3 {
    p.to_vec3()
}

/// Screen-shake trauma for something at `distance` with a felt radius.
pub fn shake_at(distance: f32, felt_radius: f32, strength: f32) -> f32 {
    ((1.0 - distance / felt_radius.max(0.1)).max(0.0)).powf(1.5) * strength
}

/// Impact sparks + a small scorch where a shot hit the level.
pub fn world_impact(
    effects: &mut Effects,
    world: Option<&CollisionWorld>,
    from: Vec3,
    end: Vec3,
    sparks: usize,
) {
    let dir = (end - from).normalize_or_zero();
    let normal = world
        .and_then(|w| w.cast_ray(end - dir * 0.3, dir, 0.6, MASK_SHOTS))
        .map(|h| h.normal);
    match normal {
        Some(n) => {
            effects.sparks(end + n * 0.03, n, sparks);
            effects.decal(end, n, 0.18, DecalKind::Scorch);
        }
        None => effects.sparks(end, -dir, sparks / 2),
    }
}

/// What an explosion looks and sounds like, by the attack that caused it. `own`: the local
/// player's (their predicted shot already played the Breach's bang).
pub fn explosion(ctx: &mut FxCtx, position: Vec3, radius: f32, attack: Attack, own: bool) {
    match attack {
        Attack::Gravewell => implosion(ctx, position, radius),
        Attack::Wisp => wisp_pop(ctx, position, radius),
        // The expansion arsenal's blasts (arsenal.rs).
        Attack::Cinderbox => arsenal::blast(ctx, Blast::Cinder, position, radius),
        Attack::Limpet => arsenal::blast(ctx, Blast::Limpet, position, radius),
        Attack::Mortar => arsenal::blast(ctx, Blast::Mortar, position, radius),
        Attack::EmberArrow => arsenal::blast(ctx, Blast::EmberArrow, position, radius),
        Attack::Flak => {
            // The ring flies out across the line of sight (no flight direction in the Fx).
            let across = (position - ctx.listener).normalize_or(Vec3::NEG_Z);
            arsenal::flak_airburst(ctx, position, across, 10, radius.max(4.0));
        }
        _ => fireball(ctx, position, radius, attack, own),
    }
}

/// Fireball, smoke, sparks, ring, light, scorch and shake (bombs, Breach, enemy blasts).
fn fireball(ctx: &mut FxCtx, position: Vec3, radius: f32, attack: Attack, own: bool) {
    let e = &mut *ctx.effects;
    let r = radius.max(1.0);
    let breach = attack == Attack::Breach;
    let hot = if breach {
        Vec4::new(1.0, 0.8, 0.5, 1.0)
    } else {
        Vec4::new(1.0, 0.62, 0.28, 1.0)
    };
    // Core flash (white-hot and tight for a Breach).
    e.spawn(Particle {
        position,
        max_life: if breach { 0.1 } else { 0.16 },
        size: Vec2::splat(r * if breach { 1.1 } else { 0.9 }),
        end_scale: 1.5,
        color: Vec4::new(1.0, 0.95, 0.8, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    // Fireball puffs.
    for _ in 0..if breach { 7 } else { 10 } {
        let d = ctx.rng.dir();
        e.spawn(Particle {
            position: position + d * r * 0.15,
            velocity: d * ctx.rng.range(2.0, 6.0) * (r / 4.5) + Vec3::Y * 1.5,
            gravity: -1.0,
            drag: 3.0,
            max_life: ctx.rng.range(0.25, 0.5) * if breach { 0.7 } else { 1.0 },
            size: Vec2::splat(r * ctx.rng.range(0.3, 0.5)),
            end_scale: 1.8,
            color: hot,
            end_alpha: 0.0,
            texture: FxTex::MuzzleFlash,
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-3.0, 3.0),
            ..Default::default()
        });
    }
    e.smoke(position, 6);
    e.sparks(position, Vec3::Y, if breach { 22 } else { 14 });
    e.shockwave(position, r);
    // Embers (a Breach throws grey debris too).
    for i in 0..if breach { 10 } else { 6 } {
        let v = ctx.rng.dir() * ctx.rng.range(5.0, 11.0) + Vec3::Y * 3.0;
        let debris = breach && i % 2 == 0;
        e.spawn(Particle {
            position,
            velocity: v,
            gravity: 16.0,
            drag: 0.8,
            max_life: ctx.rng.range(0.5, 1.0),
            size: Vec2::splat(if debris { 0.16 } else { 0.1 }),
            color: if debris {
                Vec4::new(0.35, 0.32, 0.3, 1.0)
            } else {
                Vec4::new(1.0, 0.6, 0.2, 1.0)
            },
            end_alpha: 0.2,
            texture: if debris { FxTex::Shard } else { FxTex::Spark },
            additive: !debris,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-8.0, 8.0),
            collide: Some(OnHit::Bounce(0.3)),
            ..Default::default()
        });
    }
    // Scorch on the ground / wall below it.
    if let Some(w) = ctx.world
        && let Some(h) = w.cast_ray(position + Vec3::Y * 0.3, Vec3::NEG_Y, r * 0.6, MASK_SHOTS)
    {
        e.decal(h.point, h.normal, r * 0.55, DecalKind::Scorch);
    }
    ctx.beams.light(
        position + Vec3::Y * 0.5,
        Vec3::new(3.0, 1.6, 0.6),
        r * 3.0,
        0.35,
    );
    let d = position.distance(ctx.listener);
    ctx.feedback.shake(shake_at(d, r * 4.0, 0.7));
    if breach {
        if !own {
            ctx.audio.play(Sfx::BreachBlast, PlayParams::at(position));
        }
    } else {
        ctx.audio.play(
            Sfx::Explosion,
            PlayParams::at(position).with_pitch(if r > 6.0 { 0.8 } else { 1.0 }),
        );
    }
}

/// The Gravewell collapsing: motes rushing in, a green flash, a ring closing on the centre.
fn implosion(ctx: &mut FxCtx, position: Vec3, radius: f32) {
    let green = WeaponKind::Tumbler.glow_color(1);
    let r = radius.max(1.0);
    for _ in 0..24 {
        let d = ctx.rng.dir();
        let start = position + d * r * ctx.rng.range(0.6, 1.0);
        let time = ctx.rng.range(0.12, 0.2);
        ctx.effects.spawn(Particle {
            position: start,
            velocity: (position - start) / time,
            max_life: time,
            size: Vec2::splat(0.22),
            end_scale: 0.4,
            color: hot(green, 1.6),
            end_alpha: 0.8,
            texture: FxTex::Glow,
            additive: true,
            ..Default::default()
        });
    }
    ctx.effects.spawn(Particle {
        position,
        max_life: 0.3,
        size: Vec2::splat(r * 0.5),
        end_scale: 2.6,
        color: Vec4::new(0.9, 1.0, 0.9, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    ctx.effects.sparks(position, Vec3::Y, 18);
    ctx.effects.dust(position, 16, r * 1.5);
    ctx.beams.ring(
        position + Vec3::Y * 0.3,
        Vec3::Y,
        r * 1.2,
        0.2,
        0.12,
        hot(green, 2.0),
        0.25,
    );
    ctx.beams.ring(
        position + Vec3::Y * 0.3,
        Vec3::Y,
        0.3,
        r,
        0.08,
        hot(green, 1.4),
        0.4,
    );
    ctx.beams.light(
        position + Vec3::Y * 0.5,
        Vec3::new(0.8, 2.4, 1.0),
        r * 3.0,
        0.4,
    );
    let d = position.distance(ctx.listener);
    ctx.feedback.shake(shake_at(d, r * 4.0, 0.8));
    ctx.audio
        .play(Sfx::GravewellImplode, PlayParams::at(position));
}

/// A Wisp popping: violet flash, a burst of arcs, a crackle.
fn wisp_pop(ctx: &mut FxCtx, position: Vec3, radius: f32) {
    let violet = WeaponKind::Arclash.glow_color(0);
    let r = radius.max(1.0);
    ctx.effects.glow(position, hot(violet, 1.8), r * 0.8, 0.25);
    ctx.effects.sparks(position, Vec3::Y, 12);
    for _ in 0..6 {
        let to = position + ctx.rng.dir() * r * ctx.rng.range(0.5, 1.0);
        ctx.beams
            .arc(position, to, 0.05, hot(violet, 1.6), 0.18, ctx.rng);
    }
    ctx.beams
        .ring(position, Vec3::Y, 0.3, r, 0.07, hot(violet, 1.6), 0.3);
    ctx.beams
        .light(position, Vec3::new(1.4, 0.9, 2.4), r * 3.0, 0.3);
    let d = position.distance(ctx.listener);
    ctx.feedback.shake(shake_at(d, r * 3.0, 0.4));
    ctx.audio.play(
        Sfx::DischargeRelease,
        PlayParams::at(position).with_pitch(1.35).with_volume(0.7),
    );
}

/// Bellows Gust: a cone of fast-slowing air puffs and a ring blown forward.
pub fn gust_puff(effects: &mut Effects, beams: &mut Beams, rng: &mut Rng, origin: Vec3, aim: Vec3) {
    let air = WeaponKind::Bellows.glow_color(0);
    let (right, up) = shared::sim::weapons::aim_basis(aim);
    for _ in 0..18 {
        let spread = 0.55 * rng.unit().sqrt();
        let a = rng.angle();
        let d = (aim + (right * a.cos() + up * a.sin()) * spread).normalize_or(aim);
        effects.spawn(Particle {
            position: origin + d * 0.3,
            velocity: d * rng.range(14.0, 26.0),
            drag: 4.5,
            max_life: rng.range(0.3, 0.5),
            size: Vec2::splat(rng.range(0.25, 0.45)),
            end_scale: 3.5,
            color: Vec4::new(0.85, 0.92, 1.0, 0.35),
            end_alpha: 0.0,
            texture: FxTex::Smoke,
            rotation: rng.angle(),
            spin: rng.range(-2.0, 2.0),
            ..Default::default()
        });
    }
    let ring = (air.truncate() * 1.1).extend(0.35);
    beams.ring(origin + aim * 2.5, aim, 0.3, 1.6, 0.025, ring, 0.2);
    beams.ring(origin + aim * 5.0, aim, 0.5, 2.6, 0.03, ring, 0.26);
}

/// Arclash Discharge nova around `center` (the shooter's body centre, radius `radius`): rings
/// rushing out along the floor from their feet, arcs to the edge, a flash.
pub fn discharge_nova(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    center: Vec3,
    radius: f32,
) {
    let r = radius.max(1.0);
    let feet = center - Vec3::Y * 0.8;
    let blue = Vec4::new(0.7, 1.1, 2.0, 0.9);
    beams.ring(feet + Vec3::Y * 0.12, Vec3::Y, 1.2, r, 0.09, blue, 0.32);
    beams.ring(
        feet + Vec3::Y * 0.4,
        Vec3::Y,
        1.0,
        r * 0.8,
        0.05,
        Vec4::new(1.6, 1.8, 2.4, 0.8),
        0.26,
    );
    for _ in 0..9 {
        let a = rng.angle();
        let from = feet + Vec3::new(a.cos(), 0.0, a.sin()) * 1.6 + Vec3::Y * 0.2;
        let to = feet
            + Vec3::new(a.cos(), 0.0, a.sin()) * r * rng.range(0.6, 1.0)
            + Vec3::Y * rng.range(0.1, 1.2);
        beams.arc(from, to, 0.04, ARC_COLOR, 0.2, rng);
    }
    effects.glow(center, Vec4::new(0.7, 0.85, 1.0, 0.8), 1.2, 0.18);
    effects.dust(feet, 18, r * 1.8);
    beams.light(center, Vec3::new(1.2, 1.8, 3.0), r * 2.5, 0.3);
}

/// Glassy chips where a thorn planted.
pub fn thorn_plant(ctx: &mut FxCtx, p: Vec3) {
    ctx.effects.sparks(p, Vec3::Y, 5);
    for _ in 0..5 {
        let v = ctx.rng.dir() * ctx.rng.range(1.5, 4.0);
        ctx.effects.spawn(Particle {
            position: p,
            velocity: v,
            gravity: 12.0,
            max_life: ctx.rng.range(0.3, 0.6),
            size: Vec2::splat(0.06),
            color: Vec4::new(0.8, 0.95, 1.0, 1.0),
            end_alpha: 0.2,
            texture: FxTex::Shard,
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-10.0, 10.0),
            collide: Some(OnHit::Die),
            ..Default::default()
        });
    }
    ctx.audio.play(Sfx::ThornPlant, PlayParams::at(p));
}

/// A muzzle flash (`docs/feel.md`): a big star-shaped flash, a white-hot core, a tongue of
/// flame along the shot, a few sparks thrown forward and a bright, short light. `carry` is the
/// shooter's velocity (the flash rides along with the gun); `size` the flash's width (m).
pub fn muzzle_flash(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    muzzle: Vec3,
    aim: Vec3,
    carry: Vec3,
    size: f32,
) {
    let aim = aim.normalize_or(Vec3::NEG_Z);
    let carry = if carry.is_finite() { carry } else { Vec3::ZERO };
    effects.spawn(Particle {
        position: muzzle + aim * 0.06,
        velocity: carry,
        max_life: 0.07,
        size: Vec2::splat(size * 1.3),
        end_scale: 1.4,
        color: Vec4::new(1.0, 0.85, 0.55, 1.0),
        texture: FxTex::MuzzleFlash,
        additive: true,
        rotation: rng.angle(),
        ..Default::default()
    });
    effects.spawn(Particle {
        position: muzzle + aim * (0.06 + size * 0.55),
        velocity: carry + aim * 3.0,
        max_life: 0.05,
        size: Vec2::splat(size * 0.9),
        end_scale: 1.6,
        color: Vec4::new(1.0, 0.7, 0.35, 0.9),
        texture: FxTex::MuzzleFlash,
        additive: true,
        rotation: rng.angle(),
        ..Default::default()
    });
    effects.spawn(Particle {
        position: muzzle + aim * 0.04,
        velocity: carry,
        max_life: 0.05,
        size: Vec2::splat(size * 0.6),
        end_scale: 0.6,
        color: Vec4::new(1.8, 1.7, 1.4, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    for _ in 0..3 {
        effects.spawn(Particle {
            position: muzzle + aim * 0.08,
            velocity: carry + (aim + rng.dir() * 0.35) * rng.range(7.0, 13.0),
            gravity: 9.0,
            drag: 2.0,
            max_life: rng.range(0.08, 0.16),
            size: Vec2::splat(0.05),
            end_scale: 0.4,
            color: Vec4::new(1.0, 0.85, 0.45, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    beams.light(muzzle + aim * 0.3, Vec3::new(2.4, 1.6, 0.9), 6.5, 0.08);
}

/// A hitscan tracer from `muzzle` to `end` in `style`: a bright core and a wide faint glow
/// around it (`docs/feel.md`: thicker and longer-lived than they were). `local`: the shooter's
/// own, which starts right in front of the camera, so its first metres stay thin.
pub fn tracer(beams: &mut Beams, muzzle: Vec3, end: Vec3, style: &super::AttackStyle, local: bool) {
    if style.beam_width <= 0.0 {
        return;
    }
    let along = (end - muzzle).normalize_or_zero();
    let length = (end - muzzle).length();
    let mid = if local {
        muzzle + along * length.min(3.0)
    } else {
        muzzle
    };
    let (w, c, life) = (style.beam_width, style.beam_color, style.beam_life);
    if local {
        beams.beam(muzzle, mid, w.min(0.03), c, life);
    }
    beams.beam(mid, end, w, c, life);
    // The glow: wider, faint, a little shorter-lived.
    let glow = (c.truncate() * 0.8).extend(c.w * 0.28);
    beams.beam(mid, end, w * 2.8, glow, life * 0.8);
}

/// Impact on a body (a Mould or a Shrike): sparks and a puff of ash.
pub fn body_impact(effects: &mut Effects, p: Vec3, dir: Vec3, amount: f32) {
    let n = (4.0 + amount * 8.0).min(16.0) as usize;
    effects.sparks(p, dir, n);
    effects.spawn(Particle {
        position: p,
        velocity: dir * 1.5 + Vec3::Y * 0.5,
        gravity: -0.3,
        drag: 2.0,
        max_life: 0.5,
        size: Vec2::splat(0.3 + 0.2 * amount.min(2.0)),
        end_scale: 2.2,
        color: Vec4::new(0.45, 0.43, 0.42, 0.5),
        end_alpha: 0.0,
        texture: FxTex::Smoke,
        ..Default::default()
    });
}

/// Plays one Fx event.
pub fn play(ctx: &mut FxCtx, ev: &FxEvent) {
    match &ev.fx {
        Fx::Shot {
            shooter,
            attack,
            origin,
            ends,
        } => {
            if Some(*shooter) == ctx.local {
                return; // predicted locally
            }
            let id = NetId::player(*shooter);
            let vis = ctx.remote.visual_mut(id);
            vis.firing = 1.0;
            let muzzle = if vis.muzzle != Vec3::ZERO {
                vis.muzzle
            } else {
                q(*origin)
            };
            let dir = vis.muzzle_dir;
            let style = attack_style(*attack);
            ctx.audio.play(style.sfx, PlayParams::at(muzzle));
            if *attack == Attack::Gust {
                gust_puff(ctx.effects, ctx.beams, ctx.rng, muzzle, dir);
                return;
            }
            // The Kiln sprays embers; the bow only twangs (its arrows are entities).
            if *attack == Attack::Kiln {
                arsenal::kiln_spray(ctx.effects, ctx.rng, muzzle, dir, 13.0, 16.0);
                return;
            }
            if matches!(
                attack,
                Attack::Longwick | Attack::TripleNock | Attack::PinningArrow | Attack::EmberArrow
            ) {
                return;
            }
            if *attack == Attack::Arc {
                ctx.effects.glow(muzzle, ARC_COLOR, 0.3, 0.06);
            } else {
                let carry = ctx
                    .entities
                    .iter()
                    .find(|e| e.id == id)
                    .map_or(Vec3::ZERO, |e| e.velocity);
                muzzle_flash(
                    ctx.effects,
                    ctx.beams,
                    ctx.rng,
                    muzzle,
                    dir,
                    carry,
                    style.flash,
                );
            }
            for end in ends {
                let end = q(*end);
                if style.jagged {
                    ctx.beams.arc(
                        muzzle,
                        end,
                        style.beam_width,
                        style.beam_color,
                        style.beam_life,
                        ctx.rng,
                    );
                } else {
                    tracer(ctx.beams, muzzle, end, &style, false);
                }
                world_impact(ctx.effects, ctx.world, muzzle, end, 4);
            }
        }
        Fx::Ricochet { points, .. } => {
            let pts: Vec<Vec3> = points.iter().map(|p| q(*p)).collect();
            for (i, w) in pts.windows(2).enumerate() {
                let color = Vec4::new(1.6, 1.5, 0.9, 0.95);
                ctx.beams
                    .beam(w[0], w[1], 0.07 + 0.02 * i as f32, color, 0.35);
            }
            // Every point but the last is a shattering thorn.
            for (i, p) in pts.iter().enumerate().take(pts.len().saturating_sub(1)) {
                thorn_shatter(ctx, *p);
                ctx.audio.play(
                    Sfx::Ricochet,
                    PlayParams::at(*p).with_pitch(1.0 + 0.18 * i as f32),
                );
            }
            if let Some(last) = pts.last() {
                ctx.effects.sparks(*last, Vec3::Y, 10);
            }
        }
        // Ember bombs burst into embers rather than a fireball (enemy_fx.rs).
        Fx::Explosion {
            position,
            radius_dm,
            attack: Attack::Enemy { .. },
            ..
        } => {
            let p = q(*position);
            let r = *radius_dm as f32 / 10.0;
            enemy_fx::ember_bomb_burst(ctx.effects, ctx.rng, p, r);
            ctx.beams
                .light(p + Vec3::Y * 0.4, Vec3::new(2.6, 1.3, 0.4), r * 2.6, 0.3);
            let d = p.distance(ctx.listener);
            ctx.feedback.shake(shake_at(d, r * 3.0, 0.35));
            ctx.audio.play(Sfx::EmberBurst, PlayParams::at(p));
        }
        Fx::Explosion {
            position,
            radius_dm,
            attack,
            owner,
        } => {
            let own = ctx.local.is_some_and(|p| p.0 as u16 == *owner);
            explosion(ctx, q(*position), *radius_dm as f32 / 10.0, *attack, own)
        }
        Fx::Punch { player, hit } => {
            if Some(*player) == ctx.local {
                return;
            }
            let vis = ctx.remote.visual_mut(NetId::player(*player));
            vis.punching = 0.01;
            let at = if vis.head != Vec3::ZERO {
                vis.head
            } else {
                vis.muzzle
            };
            ctx.audio.play(
                if *hit { Sfx::PunchHit } else { Sfx::Punch },
                PlayParams::at(at),
            );
        }
        Fx::Heal {
            player,
            amount_tenths,
            from,
        } => {
            let from = q(*from);
            let to = if Some(*player) == ctx.local {
                ctx.local_chest
            } else {
                // Into the chest lantern of their (scaled-up) Shrike.
                use crate::models::shrike::{LANTERN_HEIGHT, REMOTE_SCALE};
                ctx.entities
                    .iter()
                    .find(|e| e.id == NetId::player(*player))
                    .map(|e| e.position + Vec3::Y * LANTERN_HEIGHT * REMOTE_SCALE)
            };
            let Some(to) = to else {
                return;
            };
            let local = Some(*player) == ctx.local;
            heal_streak(ctx, from, to, *amount_tenths as f32 / 10.0, !local);
            if local {
                let amount = *amount_tenths as f32 / 10.0;
                ctx.feedback.heal_flash(amount);
                ctx.audio.play(
                    Sfx::Heal,
                    PlayParams::default().with_volume((0.4 + amount / 30.0).min(1.0)),
                );
            } else {
                ctx.audio
                    .play(Sfx::Heal, PlayParams::at(to).with_volume(0.5));
            }
        }
        Fx::SlamShockwave {
            player,
            position,
            power,
            radius_dm,
        } => {
            if Some(*player) == ctx.local {
                return; // the predicted landing already showed it
            }
            let p = q(*position);
            let power = *power as f32 / 255.0;
            ctx.effects
                .shockwave(p, *radius_dm as f32 / 10.0 * (0.4 + 0.6 * power));
            ctx.audio.play(Sfx::SlamImpact, PlayParams::at(p));
            let d = p.distance(ctx.listener);
            ctx.feedback.shake(shake_at(d, 12.0, 0.3 * power));
        }
        Fx::ThornThrown { player } => {
            if Some(*player) == ctx.local {
                return;
            }
            let vis = ctx.remote.visual_mut(NetId::player(*player));
            vis.punching = 0.01;
            let at = vis.head;
            ctx.audio.play(Sfx::ThornThrow, PlayParams::at(at));
        }
        Fx::ThornPlanted { position } => thorn_plant(ctx, q(*position)),
        Fx::ThornExpired { id } => {
            if let Some(p) = (ctx.thorn_position)(*id) {
                ctx.effects
                    .glow(p, Vec4::new(0.7, 0.9, 1.0, 0.6), 0.6, 0.25);
                ctx.effects.sparks(p, Vec3::Y, 4);
                ctx.audio
                    .play(Sfx::ThornShatter, PlayParams::at(p).with_volume(0.6));
            }
        }
        Fx::Splinters { origin, ends, .. } => {
            let o = q(*origin);
            let ice = hot(WeaponKind::Quill.glow_color(1), 1.8);
            for end in ends {
                let end = q(*end);
                ctx.beams.beam(o, end, 0.022, ice, 0.16);
                world_impact(ctx.effects, ctx.world, o, end, 3);
            }
            ctx.effects.glow(o, ice, 0.7, 0.15);
            for _ in 0..8 {
                let v = ctx.rng.dir() * ctx.rng.range(3.0, 7.0);
                ctx.effects.spawn(Particle {
                    position: o,
                    velocity: v,
                    gravity: 10.0,
                    max_life: ctx.rng.range(0.2, 0.45),
                    size: Vec2::splat(0.07),
                    color: Vec4::new(0.8, 1.0, 1.0, 1.0),
                    end_alpha: 0.2,
                    texture: FxTex::Shard,
                    additive: true,
                    rotation: ctx.rng.angle(),
                    spin: ctx.rng.range(-12.0, 12.0),
                    ..Default::default()
                });
            }
            ctx.audio.play(Sfx::SplinterBurst, PlayParams::at(o));
        }
        // The Flak ring (or a Gust-deflected shell bursting): the arsenal's flak burst.
        Fx::Burst { origin, ends, .. } => {
            let ends: Vec<Vec3> = ends.iter().map(|e| q(*e)).collect();
            arsenal::flak_burst(ctx, q(*origin), &ends);
        }
        Fx::ArcChain { points, .. } => {
            let pts: Vec<Vec3> = points.iter().map(|p| q(*p)).collect();
            for (i, w) in pts.windows(2).enumerate() {
                ctx.beams.arc(w[0], w[1], 0.08, ARC_COLOR, 0.22, ctx.rng);
                ctx.beams.arc(w[0], w[1], 0.04, ARC_COLOR, 0.16, ctx.rng);
                ctx.effects.glow(w[0], ARC_COLOR, 0.8, 0.2);
                ctx.effects.glow(w[1], ARC_COLOR, 0.9, 0.22);
                ctx.effects.sparks(w[1], Vec3::Y, 5);
                ctx.audio.play(
                    Sfx::ArcChain,
                    PlayParams::at(w[1]).with_pitch(1.0 + 0.12 * i as f32),
                );
            }
        }
        Fx::WispZap { from, to } => {
            let (a, b) = (q(*from), q(*to));
            let violet = hot(WeaponKind::Arclash.glow_color(0), 1.8);
            ctx.beams.arc(a, b, 0.05, violet, 0.12, ctx.rng);
            ctx.effects.glow(b, violet, 0.6, 0.12);
            ctx.effects.sparks(b, (a - b).normalize_or(Vec3::Y), 4);
            ctx.audio.play(Sfx::WispZap, PlayParams::at(b));
        }
        Fx::Branded { position } => {
            let p = q(*position);
            ctx.effects.glow(p, brand_color(), 1.4, 0.3);
            ctx.effects.sparks(p, Vec3::Y, 8);
            ctx.audio.play(Sfx::Branded, PlayParams::at(p));
        }
        Fx::Deflected { position, .. } => {
            let p = q(*position);
            let pal = palette();
            let c = pal.effect(Vec4::new(1.0, 1.0, 0.95, 1.0), pal.deflected, 1.0);
            ctx.effects.glow(p, c, 1.2, 0.15);
            ctx.effects.sparks(p, Vec3::Y, 12);
            ctx.audio.play(Sfx::Deflect, PlayParams::at(p));
        }
        Fx::Discharge {
            player,
            position,
            radius_dm,
        } => {
            if Some(*player) == ctx.local {
                return; // predicted locally
            }
            let p = q(*position);
            let r = *radius_dm as f32 / 10.0;
            discharge_nova(ctx.effects, ctx.beams, ctx.rng, p, r);
            ctx.audio.play(Sfx::DischargeRelease, PlayParams::at(p));
            let d = p.distance(ctx.listener);
            ctx.feedback.shake(shake_at(d, r * 2.5, 0.4));
        }
        Fx::ProjectileBounce { kind: k, position } => {
            let p = q(*position);
            ctx.effects.sparks(p, Vec3::Y, 8);
            if *k == kind::BOMB {
                ctx.audio.play(Sfx::BombBounce, PlayParams::at(p));
            }
        }
        Fx::ProjectileImpact {
            kind: k,
            position,
            normal,
        } => {
            let p = q(*position);
            let n = q(*normal).normalize_or(Vec3::Y);
            if *k == kind::NEEDLE || *k == kind::EMBER_BOMB {
                let c = enemy_projectile_color(*k);
                ctx.effects.glow(p, c, 1.1, 0.2);
                ctx.effects.sparks(p, n, 10);
                ctx.effects.decal(p, n, 0.12, DecalKind::Scorch);
            } else {
                ctx.effects.sparks(p, n, 5);
                ctx.effects.decal(p, n, 0.14, DecalKind::Scorch);
            }
            ctx.audio.play(Sfx::ProjectileImpact, PlayParams::at(p));
        }
        Fx::EnemyAttack {
            enemy,
            attack,
            strike,
            parryable,
        } => {
            let Some(e) = ctx.entities.iter().find(|e| e.id == *enemy) else {
                return;
            };
            let (_, height) = super::entities::enemy_size(e.kind);
            let lift = super::entities::enemy_kind(e.kind).root_lift();
            let at = e.position + Vec3::Y * (lift + height * 0.6);
            if let Some(sfx) = enemy_fx::attack_sfx(e.kind, *attack, *strike) {
                ctx.audio.play(sfx, PlayParams::at(at));
            }
            if !*strike {
                let pal = palette();
                let c = if *parryable {
                    pal.effect(Vec4::new(1.0, 0.9, 0.2, 0.9), pal.parry_window, 1.0)
                } else {
                    pal.effect(enemy_fx::telegraph_color(e.kind), pal.telegraph, 1.0)
                };
                ctx.effects.glow(at + Vec3::Y * 0.4, c, 1.2, 0.25);
                return;
            }
            // Strikes that shake the floor.
            let fwd = shared::input::look_dir(e.yaw, 0.0);
            match enemy_fx::attack_name(e.kind, *attack).as_deref() {
                Some("bash") => {
                    // The door slams: dust and a shock ring in front of it.
                    let p = e.position + fwd * 2.6;
                    ctx.effects.shockwave(p, 3.0);
                    ctx.effects.dust(p, 10, 4.0);
                    let d = p.distance(ctx.listener);
                    ctx.feedback.shake(shake_at(d, 10.0, 0.45));
                }
                Some("sweep") => {
                    enemy_fx::sweep_arc(ctx.effects, ctx.rng, e.position, e.yaw, 6.5);
                    let d = e.position.distance(ctx.listener);
                    ctx.feedback.shake(shake_at(d, 10.0, 0.3));
                }
                Some("pounce") | Some("rush") => ctx.effects.dust(e.position, 8, 3.0),
                Some("kick") => ctx.effects.dust(e.position + fwd * 1.0, 4, 1.5),
                _ => {}
            }
        }
        Fx::EnemyFired { enemy, muzzle, .. } => {
            let p = q(*muzzle);
            let k = ctx
                .entities
                .iter()
                .find(|e| e.id == *enemy)
                .map_or(kind::CENSER, |e| e.kind);
            let projectile = if k == kind::HERON {
                kind::NEEDLE
            } else {
                kind::EMBER_BOMB
            };
            let c = enemy_projectile_color(projectile);
            ctx.effects.glow(p, c, 1.3, 0.15);
            ctx.effects.sparks(p, Vec3::Y, 6);
            ctx.audio.play(enemy_fx::fired_sfx(k), PlayParams::at(p));
        }
        Fx::EnemyShockwave {
            position,
            radius_dm,
            ..
        } => {
            let p = q(*position);
            let r = *radius_dm as f32 / 10.0;
            ctx.effects.shockwave(p, r);
            let d = p.distance(ctx.listener);
            ctx.feedback.shake(shake_at(d, r * 3.0, 0.6));
        }
        // An enemy appeared (encounter wave / dev spawn): flash + `EnemySpawn`.
        Fx::EnemySpawned {
            kind: k, position, ..
        } => {
            let feet = q(*position);
            let model = super::entities::enemy_kind(*k);
            let (lift, size) = (
                model.root_lift() + model.height() * 0.5,
                model.height() * 1.2,
            );
            let center = feet + Vec3::Y * lift;
            ctx.effects
                .glow(center, glam::Vec4::new(1.0, 0.25, 0.12, 1.0), size, 0.35);
            ctx.effects.smoke(feet, 6);
            ctx.audio.play(Sfx::EnemySpawn, PlayParams::at(center));
        }
        Fx::EnemyAlert { enemy } => {
            if let Some(e) = ctx.entities.iter().find(|e| e.id == *enemy) {
                ctx.audio.play(
                    enemy_fx::alert_sfx(e.kind),
                    PlayParams::at(e.position + Vec3::Y * 1.5),
                );
            }
        }
    }
}

pub fn enemy_projectile_color(k: u16) -> Vec4 {
    enemy_fx::projectile_color(k)
}

/// Glass shards and a flash where a thorn breaks.
pub fn thorn_shatter(ctx: &mut FxCtx, p: Vec3) {
    ctx.effects
        .glow(p, Vec4::new(0.8, 0.95, 1.0, 1.0), 1.4, 0.18);
    for _ in 0..10 {
        let v = ctx.rng.dir() * ctx.rng.range(3.0, 8.0);
        ctx.effects.spawn(Particle {
            position: p,
            velocity: v,
            gravity: 14.0,
            drag: 0.6,
            max_life: ctx.rng.range(0.4, 0.8),
            size: Vec2::splat(ctx.rng.range(0.06, 0.14)),
            color: Vec4::new(0.85, 0.95, 1.0, 1.0),
            end_alpha: 0.2,
            texture: FxTex::Shard,
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-10.0, 10.0),
            collide: Some(OnHit::Bounce(0.4)),
            ..Default::default()
        });
    }
}

/// Kindling: a stream of sparks drawn out of an ember burst (`from`) into the healed Shrike's
/// chest lantern (`to`), in the palette's heal colour. `flare` also brightens the lantern
/// (other players; your own heal shows as the screen flash instead).
pub fn heal_streak(ctx: &mut FxCtx, from: Vec3, to: Vec3, amount: f32, flare: bool) {
    let color = palette().heal;
    let n = (amount * 0.8).clamp(5.0, 18.0) as usize;
    // Every spark reaches the lantern within `time` and goes out there; staggering their starts
    // along the path draws the burst out into a stream instead of one puff.
    let time = 0.24;
    for i in 0..n {
        let along = i as f32 / n as f32 * 0.55;
        let start = from.lerp(to, along) + ctx.rng.dir() * 0.45 * (1.0 - along);
        let life = time * (1.0 - along) + 0.02;
        let spark = i % 3 != 0;
        ctx.effects.spawn(Particle {
            position: start,
            velocity: (to - start) / life,
            max_life: life,
            size: Vec2::splat(if spark { 0.11 } else { 0.2 }),
            end_scale: if spark { 0.5 } else { 0.3 },
            color: if spark {
                color
            } else {
                color * Vec4::new(1.0, 1.0, 1.0, 0.7)
            },
            end_alpha: 0.8,
            texture: if spark { FxTex::Spark } else { FxTex::Glow },
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-8.0, 8.0),
            ..Default::default()
        });
    }
    if flare {
        ctx.effects
            .glow(to, color * Vec4::new(1.0, 1.0, 1.0, 0.8), 0.45, 0.25);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u32, tick: u32, fx: Fx) -> FxEvent {
        FxEvent { seq, tick, fx }
    }

    fn explosion(owner: u16) -> Fx {
        Fx::Explosion {
            position: QPos::default(),
            radius_dm: 45,
            attack: Attack::Tumble,
            owner,
        }
    }

    #[test]
    fn fx_play_at_their_render_tick_and_own_explosions_at_once() {
        let mut q = FxQueue::default();
        q.push(vec![
            ev(3, 105, explosion(1)),
            ev(1, 100, Fx::ThornExpired { id: NetId(5) }),
            ev(2, 104, explosion(0)),
        ]);
        // Render clock at 100: the tick-100 event and our (player 0) explosion are due.
        let due = q.due(100.0, Some(PlayerId(0)));
        assert_eq!(due.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(q.len(), 1);
        assert!(q.due(104.0, Some(PlayerId(0))).is_empty());
        assert_eq!(q.due(105.0, Some(PlayerId(0))).len(), 1);
        // Stale events (render clock far behind, e.g. after a stall) are not kept forever.
        q.push(vec![ev(9, 1000, explosion(2))]);
        assert_eq!(q.due(10.0, None).len(), 1);
        assert_eq!(q.played, 4);
    }

    #[test]
    fn arcs_rings_and_sigils() {
        let mut b = Beams::headless();
        let mut rng = Rng::default();
        b.arc(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -10.0),
            0.05,
            ARC_COLOR,
            0.1,
            &mut rng,
        );
        let n = b.beam_count();
        assert!(n >= 6, "a zig-zag of segments (+ cores): {n}");
        b.ring(Vec3::ZERO, Vec3::Y, 0.5, 6.0, 0.1, ARC_COLOR, 0.3);
        b.sigil(Vec3::Y, Vec3::Y, 0.5, BRAND_COLOR, 0.0);
        assert_eq!(b.frame.len(), 7, "a diamond and a triangle");
        b.update(0.016);
        assert!(b.frame.is_empty(), "sigils last one frame");
        assert_eq!(b.rings.len(), 1);
        b.update(0.5);
        assert!(b.rings.is_empty() && b.beam_count() == 0);
        // Degenerate input is ignored.
        b.arc(Vec3::ZERO, Vec3::ZERO, 0.05, ARC_COLOR, 0.1, &mut rng);
        b.ring(Vec3::NAN, Vec3::Y, 1.0, 2.0, 0.1, ARC_COLOR, 0.2);
        assert_eq!(b.beam_count() + b.rings.len(), 0);
    }

    #[test]
    fn beams_fade_and_cap() {
        let mut b = Beams::headless();
        for i in 0..300 {
            b.beam(Vec3::ZERO, Vec3::X * (1.0 + i as f32), 0.05, Vec4::ONE, 0.1);
        }
        assert_eq!(b.beam_count(), MAX_BEAMS);
        b.beam(Vec3::ZERO, Vec3::ZERO, 0.05, Vec4::ONE, 0.1);
        assert_eq!(b.beam_count(), MAX_BEAMS, "degenerate beams ignored");
        b.update(0.2);
        assert_eq!(b.beam_count(), 0);
    }

    #[test]
    fn shake_falls_off_with_distance() {
        assert!(shake_at(0.0, 10.0, 0.7) > 0.69);
        assert!(shake_at(5.0, 10.0, 0.7) < 0.3);
        assert_eq!(shake_at(12.0, 10.0, 0.7), 0.0);
    }
}
