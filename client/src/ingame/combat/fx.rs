//! Combat visuals that are not entities: tracer beams (a stretched unlit box mesh, depth
//! tested), temporary point lights (muzzle flashes, explosions), and the playback of the
//! server's cosmetic [`Fx`] events, each played when the render clock reaches its tick so it
//! lines up with the interpolated entities it belongs to.

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
use super::{Rng, attack_style, enemy_fx};

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

/// A point light that fades out.
#[derive(Clone, Copy, Debug)]
struct TempLight {
    position: Vec3,
    color: Vec3,
    radius: f32,
    life: f32,
    max_life: f32,
}

/// Tracers and temporary lights.
pub struct Beams {
    mesh: Option<MeshId>,
    texture: Option<TextureId>,
    beams: Vec<Beam>,
    lights: Vec<TempLight>,
}

/// Most tracers alive at once (oldest replaced).
const MAX_BEAMS: usize = 256;
const MAX_TEMP_LIGHTS: usize = 8;

impl Beams {
    pub fn new(renderer: &mut Renderer) -> Self {
        let mesh = engine::assets::meshgen::cuboid(Vec3::splat(0.5), Vec4::ONE).build();
        Self {
            mesh: Some(renderer.create_mesh(&mesh)),
            texture: Some(renderer.white_texture()),
            beams: Vec::new(),
            lights: Vec::new(),
        }
    }

    /// Without GPU resources (tests): beams are tracked but not drawn.
    pub fn headless() -> Self {
        Self {
            mesh: None,
            texture: None,
            beams: Vec::new(),
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
        for b in &self.beams {
            let f = 1.0 - b.life / b.max_life;
            let d = b.b - b.a;
            let len = d.length();
            let dir = d / len;
            let rot = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
            let w = b.width * (0.4 + 0.6 * f);
            let transform =
                Mat4::from_scale_rotation_translation(Vec3::new(w, w, len), rot, (b.a + b.b) * 0.5);
            let mut color = b.color;
            color.w *= f;
            draws.push(DrawItem {
                mesh,
                texture,
                transform,
                tint: color,
                unlit: true,
            });
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
    /// Local player's chest (heal streak target).
    pub local_chest: Option<Vec3>,
    /// Last drawn position of prisms (for `PrismExpired`).
    pub prism_position: &'a dyn Fn(NetId) -> Option<Vec3>,
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

/// Fireball, smoke, sparks, ring, light, scorch and shake for an explosion.
pub fn explosion(ctx: &mut FxCtx, position: Vec3, radius: f32, attack: Attack) {
    let e = &mut *ctx.effects;
    let r = radius.max(1.0);
    let hot = match attack {
        Attack::Flak => Vec4::new(1.0, 0.75, 0.35, 1.0),
        Attack::Sticky => Vec4::new(1.0, 0.55, 0.25, 1.0),
        Attack::Mortar => Vec4::new(1.0, 0.5, 0.2, 1.0),
        _ => Vec4::new(1.0, 0.62, 0.28, 1.0),
    };
    // Core flash.
    e.spawn(Particle {
        position,
        max_life: 0.16,
        size: Vec2::splat(r * 0.9),
        end_scale: 1.5,
        color: Vec4::new(1.0, 0.95, 0.8, 1.0),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    // Fireball puffs.
    for _ in 0..10 {
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
            end_alpha: 0.0,
            texture: FxTex::MuzzleFlash,
            additive: true,
            rotation: ctx.rng.angle(),
            spin: ctx.rng.range(-3.0, 3.0),
            ..Default::default()
        });
    }
    e.smoke(position, 6);
    let up = Vec3::Y;
    e.sparks(position, up, 14);
    e.shockwave(position, r);
    // Embers that bounce.
    for _ in 0..6 {
        let v = ctx.rng.dir() * ctx.rng.range(5.0, 11.0) + Vec3::Y * 3.0;
        e.spawn(Particle {
            position,
            velocity: v,
            gravity: 16.0,
            drag: 0.8,
            max_life: ctx.rng.range(0.5, 1.0),
            size: Vec2::splat(0.1),
            color: Vec4::new(1.0, 0.6, 0.2, 1.0),
            end_alpha: 0.2,
            texture: FxTex::Spark,
            additive: true,
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
    ctx.audio.play(
        Sfx::Explosion,
        PlayParams::at(position).with_pitch(if r > 6.0 { 0.8 } else { 1.0 }),
    );
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
            ctx.effects.muzzle_flash(muzzle, dir);
            ctx.beams.light(muzzle, Vec3::new(1.6, 1.1, 0.6), 4.0, 0.06);
            ctx.audio.play(style.sfx, PlayParams::at(muzzle));
            for end in ends {
                let end = q(*end);
                ctx.beams.beam(
                    muzzle,
                    end,
                    style.beam_width,
                    style.beam_color,
                    style.beam_life,
                );
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
            // Every point but the last is a shattering prism.
            for (i, p) in pts.iter().enumerate().take(pts.len().saturating_sub(1)) {
                prism_shatter(ctx, *p);
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
            ctx.beams.light(p + Vec3::Y * 0.4, Vec3::new(2.6, 1.3, 0.4), r * 2.6, 0.3);
            let d = p.distance(ctx.listener);
            ctx.feedback.shake(shake_at(d, r * 3.0, 0.35));
            ctx.audio.play(Sfx::EmberBurst, PlayParams::at(p));
        }
        Fx::Explosion {
            position,
            radius_dm,
            attack,
            ..
        } => explosion(ctx, q(*position), *radius_dm as f32 / 10.0, *attack),
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
                ctx.entities
                    .iter()
                    .find(|e| e.id == NetId::player(*player))
                    .map(|e| e.position + Vec3::Y * 1.0)
            };
            let Some(to) = to else {
                return;
            };
            heal_streak(ctx, from, to, *amount_tenths as f32 / 10.0);
            if Some(*player) == ctx.local {
                let amount = *amount_tenths as f32 / 10.0;
                ctx.feedback.flash(
                    Vec3::new(0.95, 0.15, 0.2),
                    (0.08 + amount / 120.0).min(0.25),
                );
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
        Fx::PrismThrown { player } => {
            if Some(*player) == ctx.local {
                return;
            }
            let vis = ctx.remote.visual_mut(NetId::player(*player));
            vis.punching = 0.01;
            let at = vis.head;
            ctx.audio.play(Sfx::PrismThrow, PlayParams::at(at));
        }
        Fx::PrismExpired { id } => {
            if let Some(p) = (ctx.prism_position)(*id) {
                ctx.effects
                    .glow(p, Vec4::new(0.7, 0.9, 1.0, 0.6), 0.6, 0.25);
                ctx.effects.sparks(p, Vec3::Y, 4);
            }
        }
        Fx::StickiesDetonated { player } => {
            if Some(*player) == ctx.local {
                return;
            }
            if let Some(e) = ctx.entities.iter().find(|e| e.id == NetId::player(*player)) {
                ctx.audio
                    .play(Sfx::StickyDetonate, PlayParams::at(e.position + Vec3::Y));
            }
        }
        Fx::ProjectileBounce { kind: k, position } => {
            let p = q(*position);
            ctx.effects.sparks(p, Vec3::Y, 8);
            if *k == kind::SAWBLADE {
                ctx.audio.play(Sfx::SawBounce, PlayParams::at(p));
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
            ctx.audio.play(
                Sfx::ProjectileImpact,
                PlayParams::at(p).with_volume(if *k == kind::RIVET { 0.5 } else { 1.0 }),
            );
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
                let c = if *parryable {
                    Vec4::new(1.0, 0.9, 0.2, 0.9)
                } else {
                    enemy_fx::telegraph_color(e.kind)
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
            let projectile = if k == kind::HERON { kind::NEEDLE } else { kind::EMBER_BOMB };
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
            let (lift, size) = (model.root_lift() + model.height() * 0.5, model.height() * 1.2);
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

/// Glass shards and a flash where a prism breaks.
pub fn prism_shatter(ctx: &mut FxCtx, p: Vec3) {
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

/// Red droplets flowing from a blood burst into the healed player.
pub fn heal_streak(ctx: &mut FxCtx, from: Vec3, to: Vec3, amount: f32) {
    let n = (amount * 0.6).clamp(3.0, 14.0) as usize;
    let time = 0.22;
    for i in 0..n {
        let start = from + ctx.rng.dir() * 0.4;
        let delay = i as f32 / n as f32 * 0.1;
        ctx.effects.spawn(Particle {
            position: start,
            velocity: (to - start) / time + ctx.rng.dir() * 0.5,
            drag: 0.0,
            max_life: time + delay,
            size: Vec2::splat(0.16),
            end_scale: 0.4,
            color: Vec4::new(1.0, 0.1, 0.12, 1.0),
            end_alpha: 0.6,
            texture: FxTex::Glow,
            additive: true,
            ..Default::default()
        });
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
            attack: Attack::Rocket,
            owner,
        }
    }

    #[test]
    fn fx_play_at_their_render_tick_and_own_explosions_at_once() {
        let mut q = FxQueue::default();
        q.push(vec![
            ev(3, 105, explosion(1)),
            ev(1, 100, Fx::PrismExpired { id: NetId(5) }),
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
