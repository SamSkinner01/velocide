//! Client-side visual effects: billboard particles with simple physics and lifetimes, and a
//! ring buffer of decals. Purely cosmetic (never replicated, never affect gameplay); they run
//! on the *visual* clock, so hitstop freezes them.
//!
//! ```ignore
//! fx.muzzle_flash(muzzle_pos, aim_dir);
//! fx.sparks(hit_point, hit_normal, 12);
//! fx.wax_spray(hit_point, shot_dir, 1.0);      // molten wax; drops that land set as splats
//! fx.lumps(enemy_center, 8);                   // wax / slag lumps + embers (a Mould breaking)
//! fx.smoke(pos, 3);
//! fx.shockwave(pos, radius);                    // slam / explosion ring
//! fx.decal(point, normal, 0.8, DecalKind::Scorch);
//! fx.spawn(Particle { .. });                   // anything custom
//! ```
//!
//! Particles optionally collide with the level (a ray per particle per frame, only for those
//! with `collide`); molten wax that hits a surface sets as a wax splat decal.

use engine::assets::procedural::{SpriteTex, sprite_texture};
use engine::gfx::{Decal, Sprite, TextureId};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use engine::render::Renderer;
use glam::{Vec2, Vec3, Vec4};

/// Live particle cap; the oldest are replaced when exceeded.
pub const MAX_PARTICLES: usize = 2048;
/// Decal ring buffer size (oldest overwritten).
pub const MAX_DECALS: usize = 256;

/// Sprite textures available to effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxTex {
    MuzzleFlash,
    /// Molten wax droplets (pale; tint with the wax colour).
    WaxDrop,
    Spark,
    Smoke,
    Glow,
    Ring,
    Shard,
    /// A lump of wax or slag with a glowing crack (tint it).
    Lump,
}

/// Decal textures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalKind {
    /// Molten wax that hit a surface and set.
    Wax,
    Scorch,
}

/// What happens when a colliding particle hits the level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OnHit {
    /// Bounce with this restitution (0..1).
    Bounce(f32),
    /// Stop and leave a decal of this size, then die.
    Splat(DecalKind, f32),
    Die,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    /// Downward acceleration (m/s²); negative floats up (smoke).
    pub gravity: f32,
    /// Velocity damping per second (0 = none).
    pub drag: f32,
    pub life: f32,
    pub max_life: f32,
    pub size: Vec2,
    /// Size multiplier at the end of life (linear).
    pub end_scale: f32,
    pub color: Vec4,
    /// Alpha multiplier at the end of life.
    pub end_alpha: f32,
    pub texture: FxTex,
    pub additive: bool,
    pub rotation: f32,
    pub spin: f32,
    pub collide: Option<OnHit>,
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            gravity: 0.0,
            drag: 0.0,
            life: 0.0,
            max_life: 0.5,
            size: Vec2::splat(0.3),
            end_scale: 1.0,
            color: Vec4::ONE,
            end_alpha: 0.0,
            texture: FxTex::Glow,
            additive: false,
            rotation: 0.0,
            spin: 0.0,
            collide: None,
        }
    }
}

struct Textures {
    sprites: [TextureId; 8],
    wax_splat: TextureId,
    scorch: TextureId,
}

/// Particle and decal manager.
pub struct Effects {
    tex: Option<Textures>,
    particles: Vec<Particle>,
    next_replace: usize,
    decals: Vec<(Decal, f32)>,
    next_decal: usize,
    rng: u32,
}

impl Effects {
    /// Creates the sprite textures on the GPU.
    pub fn new(renderer: &mut Renderer) -> Self {
        let mut make = |t| renderer.create_texture(&sprite_texture(t));
        let sprites = [
            make(SpriteTex::MuzzleFlash),
            make(SpriteTex::WaxDrops),
            make(SpriteTex::Spark),
            make(SpriteTex::Smoke),
            make(SpriteTex::Glow),
            make(SpriteTex::Ring),
            make(SpriteTex::Shard),
            make(SpriteTex::Lump),
        ];
        let wax_splat = make(SpriteTex::WaxSplat);
        let scorch = make(SpriteTex::Scorch);
        let mut fx = Self::headless();
        fx.tex = Some(Textures {
            sprites,
            wax_splat,
            scorch,
        });
        fx
    }

    /// Without GPU textures (tests): simulation only, nothing is drawn.
    pub fn headless() -> Self {
        Self {
            tex: None,
            particles: Vec::with_capacity(256),
            next_replace: 0,
            decals: Vec::with_capacity(MAX_DECALS),
            next_decal: 0,
            rng: 0x1234_5678,
        }
    }

    pub fn free(self, renderer: &mut Renderer) {
        if let Some(t) = self.tex {
            for id in t.sprites {
                renderer.free_texture(id);
            }
            renderer.free_texture(t.wax_splat);
            renderer.free_texture(t.scorch);
        }
    }

    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    pub fn decal_count(&self) -> usize {
        self.decals.len()
    }

    /// Removes everything (level change, restart).
    pub fn clear(&mut self) {
        self.particles.clear();
        self.decals.clear();
        self.next_decal = 0;
    }

    /// Uniform random in [0, 1) (xorshift; cosmetic only).
    pub fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    fn rand_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand()
    }

    /// Random unit vector.
    pub fn rand_dir(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(
                self.rand() * 2.0 - 1.0,
                self.rand() * 2.0 - 1.0,
                self.rand() * 2.0 - 1.0,
            );
            let l = v.length_squared();
            if l > 1e-4 && l <= 1.0 {
                return v / l.sqrt();
            }
        }
    }

    /// Random direction within a cone around `dir` (`spread` 0..1, 1 = hemisphere).
    fn rand_cone(&mut self, dir: Vec3, spread: f32) -> Vec3 {
        let d = dir.normalize_or(Vec3::Y);
        (d + self.rand_dir() * spread).normalize_or(d)
    }

    /// Adds a particle (replaces the oldest when full).
    pub fn spawn(&mut self, p: Particle) {
        if !p.position.is_finite() || !p.velocity.is_finite() {
            return;
        }
        if self.particles.len() < MAX_PARTICLES {
            self.particles.push(p);
        } else {
            self.next_replace = (self.next_replace + 1) % MAX_PARTICLES;
            self.particles[self.next_replace] = p;
        }
    }

    /// Adds a decal (ring buffer of [`MAX_DECALS`]).
    pub fn decal(&mut self, position: Vec3, normal: Vec3, size: f32, kind: DecalKind) {
        let Some(tex) = &self.tex else {
            // Still track it for headless tests.
            self.push_decal(position, normal, size, kind, TextureId::default());
            return;
        };
        let texture = match kind {
            DecalKind::Wax => tex.wax_splat,
            DecalKind::Scorch => tex.scorch,
        };
        self.push_decal(position, normal, size, kind, texture);
    }

    fn push_decal(
        &mut self,
        position: Vec3,
        normal: Vec3,
        size: f32,
        kind: DecalKind,
        texture: TextureId,
    ) {
        if !position.is_finite() || !normal.is_finite() {
            return;
        }
        let rotation = self.rand() * std::f32::consts::TAU;
        let color = match kind {
            DecalKind::Wax => Vec4::new(0.86, 0.82, 0.74, 0.95),
            DecalKind::Scorch => Vec4::new(1.0, 1.0, 1.0, 0.85),
        };
        let d = Decal {
            position,
            normal: normal.normalize_or(Vec3::Y),
            size,
            rotation,
            color,
            texture,
        };
        if self.decals.len() < MAX_DECALS {
            self.decals.push((d, 0.0));
        } else {
            self.decals[self.next_decal] = (d, 0.0);
            self.next_decal = (self.next_decal + 1) % MAX_DECALS;
        }
    }

    // ---- presets ------------------------------------------------------------------------------

    pub fn muzzle_flash(&mut self, position: Vec3, dir: Vec3) {
        let rotation = self.rand() * std::f32::consts::TAU;
        let p = Particle {
            position: position + dir * 0.05,
            max_life: 0.06,
            size: Vec2::splat(0.45),
            end_scale: 1.3,
            color: Vec4::new(1.0, 0.85, 0.55, 1.0),
            end_alpha: 0.0,
            texture: FxTex::MuzzleFlash,
            additive: true,
            rotation,
            ..Default::default()
        };
        self.spawn(p);
    }

    /// Bright sparks bouncing off a surface.
    pub fn sparks(&mut self, position: Vec3, normal: Vec3, count: usize) {
        for _ in 0..count {
            let v = self.rand_cone(normal, 0.9) * self.rand_range(4.0, 12.0);
            let p = Particle {
                position,
                velocity: v,
                gravity: 18.0,
                drag: 1.5,
                max_life: self.rand_range(0.15, 0.4),
                size: Vec2::splat(0.08),
                end_scale: 0.3,
                color: Vec4::new(1.0, 0.8, 0.4, 1.0),
                end_alpha: 0.3,
                texture: FxTex::Spark,
                additive: true,
                collide: Some(OnHit::Bounce(0.4)),
                ..Default::default()
            };
            self.spawn(p);
        }
    }

    /// Molten wax thrown along `dir` (`amount` ~1 for a normal hit), with a few glowing drops;
    /// every third drop that lands sets as a wax splat decal.
    pub fn wax_spray(&mut self, position: Vec3, dir: Vec3, amount: f32) {
        let n = (amount * 10.0).clamp(1.0, 60.0) as usize;
        for i in 0..n {
            let v = self.rand_cone(dir, 0.7) * self.rand_range(2.0, 7.0) + Vec3::Y * 1.5;
            let size = self.rand_range(0.1, 0.24);
            let (collide, color, additive) = if i % 3 == 0 {
                (
                    OnHit::Splat(DecalKind::Wax, self.rand_range(0.4, 0.9)),
                    Vec4::new(0.9, 0.86, 0.76, 1.0),
                    false,
                )
            } else if i % 3 == 1 {
                (OnHit::Die, Vec4::new(0.86, 0.8, 0.68, 1.0), false)
            } else {
                // Still molten: glows on the way out.
                (OnHit::Die, Vec4::new(1.0, 0.62, 0.25, 1.0), true)
            };
            let rotation = self.rand() * std::f32::consts::TAU;
            let p = Particle {
                position,
                velocity: v,
                gravity: 14.0,
                drag: 0.6,
                max_life: self.rand_range(0.5, 1.0),
                size: Vec2::splat(size),
                end_scale: 0.6,
                color,
                end_alpha: 0.6,
                texture: if additive { FxTex::Glow } else { FxTex::WaxDrop },
                additive,
                rotation,
                spin: self.rand_range(-4.0, 4.0),
                collide: Some(collide),
                ..Default::default()
            };
            self.spawn(p);
        }
    }

    /// A Mould breaking apart: lumps of wax and slag that bounce around, a fountain of embers
    /// and molten wax that sets where it lands.
    pub fn lumps(&mut self, position: Vec3, count: usize) {
        for k in 0..count {
            let v = self.rand_cone(Vec3::Y, 1.0) * self.rand_range(4.0, 10.0);
            let rotation = self.rand() * std::f32::consts::TAU;
            // Mostly pale wax, every third one dark slag.
            let color = if k % 3 == 2 {
                Vec4::new(0.42, 0.38, 0.35, 1.0)
            } else {
                Vec4::new(0.9, 0.86, 0.78, 1.0)
            };
            let p = Particle {
                position,
                velocity: v,
                gravity: 20.0,
                drag: 0.3,
                max_life: self.rand_range(1.5, 3.0),
                size: Vec2::splat(self.rand_range(0.2, 0.4)),
                end_scale: 0.8,
                color,
                end_alpha: 0.0,
                texture: FxTex::Lump,
                rotation,
                spin: self.rand_range(-8.0, 8.0),
                collide: Some(OnHit::Bounce(0.25)),
                ..Default::default()
            };
            self.spawn(p);
        }
        for _ in 0..count * 2 {
            let v = self.rand_cone(Vec3::Y, 0.8) * self.rand_range(2.0, 6.0);
            let p = Particle {
                position,
                velocity: v,
                gravity: -1.0,
                drag: 1.2,
                max_life: self.rand_range(0.6, 1.4),
                size: Vec2::splat(0.08),
                end_scale: 0.3,
                color: Vec4::new(1.0, 0.7, 0.3, 1.0),
                end_alpha: 0.0,
                texture: FxTex::Spark,
                additive: true,
                ..Default::default()
            };
            self.spawn(p);
        }
        self.wax_spray(position, Vec3::Y, 2.0);
    }

    pub fn smoke(&mut self, position: Vec3, count: usize) {
        for _ in 0..count {
            let v = self.rand_dir() * 0.6 + Vec3::Y * self.rand_range(0.4, 1.2);
            let rotation = self.rand() * std::f32::consts::TAU;
            let p = Particle {
                position: position + self.rand_dir() * 0.2,
                velocity: v,
                gravity: -0.5,
                drag: 1.2,
                max_life: self.rand_range(0.8, 1.6),
                size: Vec2::splat(self.rand_range(0.5, 0.9)),
                end_scale: 2.4,
                color: Vec4::new(0.55, 0.5, 0.5, 0.55),
                end_alpha: 0.0,
                texture: FxTex::Smoke,
                rotation,
                spin: self.rand_range(-0.8, 0.8),
                ..Default::default()
            };
            self.spawn(p);
        }
    }

    /// Ground dust kicked up by a landing / slide.
    pub fn dust(&mut self, position: Vec3, count: usize, spread: f32) {
        for _ in 0..count {
            let mut d = self.rand_dir();
            d.y = d.y.abs() * 0.3;
            let rotation = self.rand() * std::f32::consts::TAU;
            let p = Particle {
                position: position + Vec3::Y * 0.1,
                velocity: d * spread,
                gravity: -0.3,
                drag: 3.0,
                max_life: self.rand_range(0.4, 0.8),
                size: Vec2::splat(self.rand_range(0.3, 0.6)),
                end_scale: 2.0,
                color: Vec4::new(0.6, 0.55, 0.5, 0.45),
                end_alpha: 0.0,
                texture: FxTex::Smoke,
                rotation,
                ..Default::default()
            };
            self.spawn(p);
        }
    }

    /// Expanding shockwave ring + glow (slam impacts, explosions).
    pub fn shockwave(&mut self, position: Vec3, radius: f32) {
        let p = Particle {
            position: position + Vec3::Y * 0.3,
            max_life: 0.35,
            size: Vec2::splat(radius * 0.4),
            end_scale: 5.0,
            color: Vec4::new(1.0, 0.75, 0.6, 0.9),
            end_alpha: 0.0,
            texture: FxTex::Ring,
            additive: true,
            ..Default::default()
        };
        self.spawn(p);
        let p = Particle {
            position: position + Vec3::Y * 0.4,
            max_life: 0.18,
            size: Vec2::splat(radius * 0.6),
            end_scale: 1.6,
            color: Vec4::new(1.0, 0.55, 0.3, 0.8),
            end_alpha: 0.0,
            texture: FxTex::Glow,
            additive: true,
            ..Default::default()
        };
        self.spawn(p);
        self.dust(position, 14, radius * 2.0);
    }

    /// Short glow pulse (jump pads, pickups).
    pub fn glow(&mut self, position: Vec3, color: Vec4, size: f32, life: f32) {
        let p = Particle {
            position,
            max_life: life,
            size: Vec2::splat(size),
            end_scale: 1.8,
            color,
            end_alpha: 0.0,
            texture: FxTex::Glow,
            additive: true,
            ..Default::default()
        };
        self.spawn(p);
    }

    // ---- simulation / drawing -----------------------------------------------------------------

    /// Advances particles by the *visual* frame time. `world` enables particle collisions.
    pub fn update(&mut self, dt: f32, world: Option<&CollisionWorld>) {
        if dt <= 0.0 {
            return;
        }
        let mut new_decals: Vec<(Vec3, Vec3, f32, DecalKind)> = Vec::new();
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            p.life += dt;
            if p.life >= p.max_life {
                self.particles.swap_remove(i);
                continue;
            }
            p.velocity.y -= p.gravity * dt;
            if p.drag > 0.0 {
                p.velocity *= (-p.drag * dt).exp();
            }
            p.rotation += p.spin * dt;
            let motion = p.velocity * dt;
            let mut dead = false;
            match (p.collide, world) {
                (Some(on_hit), Some(world)) if motion.length_squared() > 1e-10 => {
                    let len = motion.length();
                    if let Some(hit) = world.cast_ray(p.position, motion / len, len, MASK_SHOTS) {
                        match on_hit {
                            OnHit::Bounce(e) => {
                                p.position = hit.point + hit.normal * 0.02;
                                let vn = p.velocity.dot(hit.normal);
                                p.velocity = (p.velocity - hit.normal * vn * (1.0 + e)) * 0.8;
                                p.spin *= 0.5;
                            }
                            OnHit::Splat(kind, size) => {
                                new_decals.push((hit.point, hit.normal, size, kind));
                                dead = true;
                            }
                            OnHit::Die => dead = true,
                        }
                    } else {
                        p.position += motion;
                    }
                }
                _ => p.position += motion,
            }
            if dead {
                self.particles.swap_remove(i);
            } else {
                i += 1;
            }
        }
        for (pos, n, size, kind) in new_decals {
            self.decal(pos, n, size, kind);
        }
        for (_, age) in &mut self.decals {
            *age += dt;
        }
    }

    /// Appends sprites and decals for this frame.
    pub fn draw(&self, sprites: &mut Vec<Sprite>, decals: &mut Vec<Decal>) {
        let Some(tex) = &self.tex else {
            return;
        };
        for p in &self.particles {
            let t = (p.life / p.max_life).clamp(0.0, 1.0);
            let scale = 1.0 + (p.end_scale - 1.0) * t;
            let alpha = 1.0 + (p.end_alpha - 1.0) * t;
            let texture = tex.sprites[p.texture as usize];
            sprites.push(Sprite {
                position: p.position,
                size: p.size * scale,
                color: Vec4::new(p.color.x, p.color.y, p.color.z, p.color.w * alpha),
                texture,
                uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                rotation: p.rotation,
                additive: p.additive,
            });
        }
        for (d, age) in &self.decals {
            // Splats "land" with a quick grow.
            let grow = (age / 0.08).clamp(0.4, 1.0);
            decals.push(Decal {
                size: d.size * grow,
                ..*d
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decals_are_a_ring_buffer() {
        let mut fx = Effects::headless();
        for i in 0..(MAX_DECALS + 10) {
            fx.decal(
                Vec3::new(i as f32, 0.0, 0.0),
                Vec3::Y,
                1.0,
                DecalKind::Wax,
            );
        }
        assert_eq!(fx.decal_count(), MAX_DECALS);
    }

    #[test]
    fn particles_expire_and_are_capped() {
        let mut fx = Effects::headless();
        for _ in 0..(MAX_PARTICLES + 100) {
            fx.spawn(Particle::default());
        }
        assert_eq!(fx.particle_count(), MAX_PARTICLES);
        fx.update(1.0, None);
        assert_eq!(fx.particle_count(), 0);
    }

    #[test]
    fn molten_wax_sets_as_decals() {
        let mut world = CollisionWorld::new();
        world.add_box(
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.5, 20.0),
            glam::Quat::IDENTITY,
            engine::physics::GROUP_WORLD,
            0,
        );
        let mut fx = Effects::headless();
        fx.wax_spray(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Y, 3.0);
        for _ in 0..120 {
            fx.update(1.0 / 60.0, Some(&world));
        }
        assert!(
            fx.decal_count() > 0,
            "wax drops should set on the floor"
        );
        assert_eq!(fx.particle_count(), 0);
    }

    #[test]
    fn hitstop_frame_does_nothing() {
        let mut fx = Effects::headless();
        fx.spawn(Particle {
            velocity: Vec3::X,
            ..Default::default()
        });
        fx.update(0.0, None);
        assert_eq!(fx.particles[0].position, Vec3::ZERO);
    }
}
