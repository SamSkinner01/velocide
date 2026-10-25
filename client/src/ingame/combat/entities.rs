//! Drawing the combat entity kinds the server replicates, and the client-side bookkeeping that
//! turns their state changes into effects:
//!
//! * **enemies** (kinds 16..64): `ModelLibrary::draw_enemy` with an [`EnemyAnim`] mapped from
//!   the replicated AI state (+ wind-up progress in `aux`), speed and facing; a per-enemy walk
//!   cycle; a white hurt flash + blood when `health` drops; a yellow strobe in the parryable
//!   phase; gibs, blood and a fade when it dies.
//! * **projectiles** (64..72): their model at the trajectory position for the render tick
//!   (`shared::sim::projectiles::replicated_position`), facing their velocity; trails / glows
//!   / lights per kind; `PARRIED` white-hot, `PENDING` hidden, stuck stickies blink.
//! * **blood bursts** (128): red mist while the pool lasts.
//! * **prisms** (160): a spinning mirrored shard with glints.

use std::collections::HashMap;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::protocol::{EntityState, NetId, blood_flags, enemy_flags, kind, projectile_flags};
use shared::sim::AiState;
use shared::sim::projectiles::{ProjectileKind, replicated_position};
use shared::tuning::CombatTuning;

use super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::Rng;
use super::fx::enemy_projectile_color;
use crate::models::enemies::{EnemyAnim, EnemyKind, EnemyState};
use crate::models::{ModelId, ModelLibrary, Tint};

/// Model kind for a replicated enemy kind (unknown kinds draw as Husks).
pub fn enemy_kind(k: u16) -> EnemyKind {
    match k {
        kind::SPITTER => EnemyKind::Spitter,
        kind::GNAT => EnemyKind::Gnat,
        kind::BASTION => EnemyKind::Bastion,
        _ => EnemyKind::Husk,
    }
}

/// Display name of an enemy kind (kill feed).
pub fn enemy_name(k: u16) -> &'static str {
    match k {
        kind::SPITTER => "SPITTER",
        kind::GNAT => "GNAT",
        kind::BASTION => "BASTION",
        _ => "HUSK",
    }
}

/// Approximate hit capsule of an enemy for client-side tracer ends: (radius, height).
pub fn enemy_size(k: u16) -> (f32, f32) {
    match k {
        kind::SPITTER => (0.5, 1.6),
        kind::GNAT => (0.45, 0.9),
        kind::BASTION => (0.9, 3.0),
        _ => (0.45, 1.9),
    }
}

/// Model offset of the enemy's root from its replicated feet position (the Gnat model's
/// origin is its hover centre).
fn root_offset(k: u16) -> Vec3 {
    if k == kind::GNAT {
        Vec3::Y * 0.45
    } else {
        Vec3::ZERO
    }
}

/// The animation input for a replicated enemy.
pub fn enemy_anim(e: &EntityState, cycle: f32, time: f64) -> EnemyAnim {
    let k = enemy_kind(e.kind);
    let progress = (e.aux & 0xFF) as f32 / 255.0;
    let flat = Vec3::new(e.velocity.x, 0.0, e.velocity.z);
    let speed = flat.length();
    let state = match AiState::from_u16(e.state) {
        Some(AiState::Windup) => EnemyState::Windup(progress),
        Some(AiState::Strike) => EnemyState::Attack(progress),
        Some(AiState::Recover) => EnemyState::Recover(progress),
        Some(AiState::Stunned) => EnemyState::Stunned,
        Some(AiState::Dying) => EnemyState::Dying(progress),
        _ if speed > 0.2 => EnemyState::Walk,
        _ => EnemyState::Idle,
    };
    let fwd = shared::input::look_dir(e.yaw, 0.0);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    EnemyAnim {
        kind: k,
        state,
        speed,
        cycle,
        strafe: if speed > 0.1 {
            (flat.dot(right) / speed).clamp(-1.0, 1.0)
        } else {
            0.0
        },
        aim_pitch: 0.0,
        time,
        seed: e.id.0.wrapping_mul(2_654_435_761),
    }
}

/// Model of a projectile kind.
pub fn projectile_model(k: u16) -> Option<ModelId> {
    Some(match ProjectileKind::from_net_kind(k)? {
        ProjectileKind::Rivet => ModelId::Rivet,
        ProjectileKind::Sawblade => ModelId::Sawblade,
        ProjectileKind::Rocket => ModelId::Rocket,
        ProjectileKind::Flak => ModelId::FlakGrenade,
        ProjectileKind::Sticky => ModelId::StickyCharge,
        ProjectileKind::Mortar => ModelId::MortarShell,
        ProjectileKind::SpitterOrb => ModelId::SpitterOrb,
        ProjectileKind::GnatBolt => ModelId::GnatBolt,
    })
}

/// Velocity of a trajectory-replicated entity at `time_ticks`.
fn replicated_velocity(e: &EntityState, time_ticks: f64, rate: u32, t: &CombatTuning) -> Vec3 {
    let g = if e.flags & projectile_flags::PARRIED != 0 {
        0.0
    } else {
        ProjectileKind::from_net_kind(e.kind).map_or(0.0, |k| k.gravity(t))
    };
    let dt = ((time_ticks - e.aux as f64) / rate.max(1) as f64).max(0.0) as f32;
    e.velocity - Vec3::Y * g * dt
}

#[derive(Clone, Copy, Debug, Default)]
struct EnemyVis {
    cycle: f32,
    last_health: u16,
    flash: f32,
    died: bool,
    seen: bool,
    position: Vec3,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProjVis {
    trail: f32,
    seen: bool,
    position: Vec3,
}

#[derive(Clone, Copy, Debug, Default)]
struct BloodVis {
    puff: f32,
    seen: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct PrismVis {
    glint: f32,
    seen: bool,
    position: Vec3,
}

/// Inputs shared by update and draw.
pub struct EntityFrame<'a> {
    /// Interpolated entities (drawn at `render_tick`).
    pub entities: &'a [EntityState],
    pub render_tick: f64,
    pub tick_rate: u32,
    pub tuning: &'a CombatTuning,
    /// The local player's own projectiles from the newest snapshot, drawn at `own_time` (the
    /// present) so a shot leaves the barrel immediately instead of an interpolation delay
    /// later. Own projectiles in `entities` are skipped.
    pub own: &'a [EntityState],
    pub own_time: f64,
    /// Owner code of the local player.
    pub local_owner: Option<u16>,
    /// Muzzle minus eye of the local weapon: own projectiles leave the drawn barrel and blend
    /// onto their true (eye-based) path over their first ~0.15 s.
    pub own_offset: Vec3,
    /// Camera position (nothing is drawn inside the camera).
    pub camera: Vec3,
}

/// Seconds over which an own projectile blends from the muzzle onto its true path.
const MUZZLE_BLEND: f32 = 0.15;

impl EntityFrame<'_> {
    /// Every projectile to show: the entity, the time to evaluate its trajectory at and a
    /// visual offset (own projectiles start at the muzzle).
    fn projectiles(&self) -> impl Iterator<Item = (&EntityState, f64, Vec3)> {
        let own = self.local_owner;
        self.entities
            .iter()
            .filter(move |e| kind::is_projectile(e.kind) && Some(e.owner) != own)
            .map(|e| (e, self.render_tick, Vec3::ZERO))
            .chain(
                self.own
                    .iter()
                    .filter(|e| kind::is_projectile(e.kind))
                    .map(|e| {
                        let age =
                            ((self.own_time - e.aux as f64) / self.tick_rate.max(1) as f64) as f32;
                        let fresh = e.flags & projectile_flags::PARRIED == 0;
                        let k = if fresh {
                            (1.0 - age / MUZZLE_BLEND).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        (e, self.own_time, self.own_offset * k)
                    }),
            )
    }
}

#[derive(Default)]
pub struct EntityView {
    enemies: HashMap<NetId, EnemyVis>,
    projectiles: HashMap<NetId, ProjVis>,
    blood: HashMap<NetId, BloodVis>,
    prisms: HashMap<NetId, PrismVis>,
    time: f64,
}

impl EntityView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.enemies.clear();
        self.projectiles.clear();
        self.blood.clear();
        self.prisms.clear();
    }

    /// Last drawn position of a prism.
    pub fn prism_position(&self, id: NetId) -> Option<Vec3> {
        self.prisms.get(&id).map(|p| p.position)
    }

    /// Living enemies within `radius` of `p` that have a target (music intensity).
    pub fn alert_enemies_near(&self, entities: &[EntityState], p: Vec3, radius: f32) -> usize {
        entities
            .iter()
            .filter(|e| {
                kind::is_enemy(e.kind)
                    && e.flags & enemy_flags::ALERT != 0
                    && e.flags & enemy_flags::DYING == 0
                    && e.position.distance(p) < radius
            })
            .count()
    }

    /// Advances animation state and spawns the effects / sounds of state changes. Runs on
    /// the visual clock (`dt`).
    pub fn update(
        &mut self,
        f: &EntityFrame,
        dt: f32,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
    ) {
        self.time += dt as f64;
        for v in self.enemies.values_mut() {
            v.seen = false;
        }
        for v in self.projectiles.values_mut() {
            v.seen = false;
        }
        for v in self.blood.values_mut() {
            v.seen = false;
        }
        for v in self.prisms.values_mut() {
            v.seen = false;
        }
        for e in f.entities {
            if kind::is_enemy(e.kind) {
                self.update_enemy(e, dt, effects, audio, rng);
            } else if e.kind == kind::BLOOD_BURST {
                let v = self.blood.entry(e.id).or_default();
                let first = !v.seen && v.puff == 0.0;
                v.seen = true;
                let radius = e.state as f32 / 10.0;
                let big = e.flags & blood_flags::BIG != 0;
                if first {
                    let n = if big {
                        9
                    } else {
                        (3.0 + e.health as f32 / 4.0).min(8.0) as usize
                    };
                    for _ in 0..n {
                        blood_mist(effects, rng, e.position, radius, big);
                    }
                    v.puff = 0.001;
                }
                v.puff += dt;
                if e.health > 0 && v.puff > if big { 0.25 } else { 0.15 } {
                    v.puff = 0.001;
                    blood_mist(effects, rng, e.position, radius, big);
                }
            } else if e.kind == kind::PRISM {
                let pos = replicated_position(e, f.render_tick, f.tick_rate, f.tuning);
                let v = self.prisms.entry(e.id).or_default();
                v.seen = true;
                v.position = pos;
                v.glint -= dt;
                if v.glint <= 0.0 {
                    v.glint = rng.range(0.25, 0.7);
                    let off = rng.dir() * 0.15;
                    effects.glow(pos + off, Vec4::new(0.9, 1.0, 1.0, 0.9), 0.25, 0.12);
                }
            }
        }
        for (e, time, offset) in f.projectiles() {
            self.update_projectile(e, time, offset, f, dt, effects, rng);
        }
        self.enemies.retain(|_, v| v.seen);
        self.projectiles.retain(|_, v| v.seen);
        self.blood.retain(|_, v| v.seen);
        self.prisms.retain(|_, v| v.seen);
    }

    fn update_enemy(
        &mut self,
        e: &EntityState,
        dt: f32,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
    ) {
        let (_, height) = enemy_size(e.kind);
        let center = e.position + Vec3::Y * height * 0.55;
        let dying = e.flags & enemy_flags::DYING != 0 || e.state == AiState::Dying as u16;
        let v = self.enemies.entry(e.id).or_insert(EnemyVis {
            last_health: e.health,
            died: dying,
            ..Default::default()
        });
        v.seen = true;
        v.position = center;
        let speed = Vec3::new(e.velocity.x, 0.0, e.velocity.z).length();
        v.cycle = enemy_kind(e.kind).advance_cycle(v.cycle, speed, dt);
        v.flash = (v.flash - dt * 6.0).max(0.0);
        if e.health < v.last_health && !dying {
            let lost = (v.last_health - e.health) as f32;
            v.flash = 1.0;
            effects.blood(center, rng.dir() + Vec3::Y, (0.5 + lost / 25.0).min(3.0));
            audio.play(Sfx::EnemyHurt, PlayParams::at(center));
        }
        v.last_health = e.health;
        if dying && !v.died {
            v.died = true;
            let n = if e.kind == kind::BASTION { 16 } else { 9 };
            effects.gibs(center, n);
            effects.blood(center, Vec3::Y, 3.0);
            audio.play(Sfx::EnemyDeath, PlayParams::at(center));
            audio.play(Sfx::Gib, PlayParams::at(center));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_projectile(
        &mut self,
        e: &EntityState,
        time: f64,
        offset: Vec3,
        f: &EntityFrame,
        dt: f32,
        effects: &mut Effects,
        rng: &mut Rng,
    ) {
        let pos = replicated_position(e, time, f.tick_rate, f.tuning) + offset;
        let v = self.projectiles.entry(e.id).or_default();
        v.seen = true;
        v.position = pos;
        if e.flags & projectile_flags::PENDING != 0 || pos.distance(f.camera) < 1.0 {
            return;
        }
        let stuck = e.flags & projectile_flags::STUCK != 0;
        let parried = e.flags & projectile_flags::PARRIED != 0;
        v.trail += dt;
        let interval = match e.kind {
            kind::RIVET => 0.05,
            kind::STICKY if stuck => 0.5,
            _ => 0.025,
        };
        if v.trail < interval {
            return;
        }
        v.trail = 0.0;
        let vel = replicated_velocity(e, time, f.tick_rate, f.tuning);
        let back = -vel.normalize_or_zero();
        if parried {
            effects.spawn(Particle {
                position: pos,
                max_life: 0.25,
                size: Vec2::splat(0.35),
                end_scale: 0.3,
                color: Vec4::new(1.0, 0.95, 0.85, 0.9),
                texture: FxTex::Glow,
                additive: true,
                ..Default::default()
            });
            return;
        }
        match e.kind {
            kind::ROCKET | kind::MORTAR => {
                effects.spawn(Particle {
                    position: pos + back * 0.3,
                    velocity: rng.dir() * 0.4,
                    gravity: -0.6,
                    drag: 1.5,
                    max_life: rng.range(0.5, 0.9),
                    size: Vec2::splat(0.25),
                    end_scale: 3.0,
                    color: Vec4::new(0.5, 0.47, 0.45, 0.55),
                    texture: FxTex::Smoke,
                    rotation: rng.angle(),
                    ..Default::default()
                });
                effects.spawn(Particle {
                    position: pos + back * 0.25,
                    max_life: 0.08,
                    size: Vec2::splat(0.35),
                    end_scale: 0.4,
                    color: Vec4::new(1.0, 0.65, 0.3, 1.0),
                    texture: FxTex::MuzzleFlash,
                    additive: true,
                    rotation: rng.angle(),
                    ..Default::default()
                });
            }
            kind::STICKY if stuck => {
                effects.glow(pos, Vec4::new(1.0, 0.15, 0.1, 1.0), 0.45, 0.15);
            }
            kind::FLAK | kind::STICKY => {
                effects.spawn(Particle {
                    position: pos,
                    max_life: 0.15,
                    size: Vec2::splat(0.12),
                    color: Vec4::new(1.0, 0.7, 0.3, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
            kind::SAWBLADE => {
                if rng.unit() < 0.3 {
                    effects.spawn(Particle {
                        position: pos,
                        velocity: rng.dir() * 4.0,
                        gravity: 14.0,
                        max_life: 0.2,
                        size: Vec2::splat(0.06),
                        color: Vec4::new(1.0, 0.8, 0.4, 1.0),
                        texture: FxTex::Spark,
                        additive: true,
                        collide: Some(OnHit::Die),
                        ..Default::default()
                    });
                }
            }
            kind::SPITTER_ORB | kind::GNAT_BOLT => {
                let c = enemy_projectile_color(e.kind);
                effects.spawn(Particle {
                    position: pos,
                    max_life: 0.3,
                    size: Vec2::splat(if e.kind == kind::SPITTER_ORB {
                        0.6
                    } else {
                        0.25
                    }),
                    end_scale: 0.2,
                    color: c,
                    texture: FxTex::Glow,
                    additive: true,
                    ..Default::default()
                });
            }
            _ => {
                effects.spawn(Particle {
                    position: pos,
                    max_life: 0.1,
                    size: Vec2::splat(0.08),
                    color: Vec4::new(1.0, 0.85, 0.5, 0.7),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
        }
    }

    /// Appends models and lights for every combat entity.
    pub fn draw(
        &self,
        lib: Option<&ModelLibrary>,
        f: &EntityFrame,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        let Some(lib) = lib else {
            return;
        };
        let time = self.time;
        for e in f.entities {
            if kind::is_enemy(e.kind) {
                let v = self.enemies.get(&e.id).copied().unwrap_or_default();
                let anim = enemy_anim(e, v.cycle, time);
                let mut tint = Tint::default();
                let flash = 1.0 + 2.0 * v.flash;
                tint.body = Vec4::new(flash, flash * 0.9, flash * 0.9, 1.0);
                if e.flags & enemy_flags::PARRYABLE != 0 {
                    let strobe = if (time * 18.0).fract() < 0.5 {
                        3.0
                    } else {
                        1.6
                    };
                    tint.glow = Vec4::new(strobe, strobe * 0.9, 0.4, 1.0);
                }
                if let EnemyState::Dying(p) = anim.state
                    && p > 0.7
                {
                    tint.body.w = ((1.0 - p) / 0.3).clamp(0.05, 1.0);
                }
                let root = EnemyAnim::root(e.position + root_offset(e.kind), e.yaw);
                lib.draw_enemy(&anim, root, &tint, draws);
            } else if e.kind == kind::PRISM {
                let pos = replicated_position(e, f.render_tick, f.tick_rate, f.tuning);
                let rot = Quat::from_rotation_y((time * 3.0) as f32 + e.id.0 as f32)
                    * Quat::from_rotation_z(0.25);
                let tint = Tint {
                    glow: Vec4::splat(1.4 + 0.6 * ((time * 5.0).sin() as f32)),
                    ..Default::default()
                };
                lib.draw_model(
                    ModelId::Prism,
                    Mat4::from_rotation_translation(rot, pos),
                    &tint,
                    draws,
                );
            }
        }
        for (e, time, offset) in f.projectiles() {
            if e.flags & projectile_flags::PENDING != 0 {
                continue;
            }
            let Some(model) = projectile_model(e.kind) else {
                continue;
            };
            let pos = replicated_position(e, time, f.tick_rate, f.tuning) + offset;
            if pos.distance(f.camera) < 0.6 {
                continue;
            }
            let vel = replicated_velocity(e, time, f.tick_rate, f.tuning);
            let parried = e.flags & projectile_flags::PARRIED != 0;
            let stuck = e.flags & projectile_flags::STUCK != 0;
            let rot = if e.kind == kind::SAWBLADE {
                let flat = Vec3::new(vel.x, 0.0, vel.z);
                let yaw = if flat.length_squared() > 1e-4 {
                    (-flat.x).atan2(-flat.z)
                } else {
                    0.0
                };
                Quat::from_rotation_y(yaw + (time * 40.0) as f32)
            } else if stuck || vel.length_squared() < 1e-4 {
                Quat::from_rotation_y((e.id.0 % 7) as f32)
            } else {
                let spin = if e.kind == kind::FLAK || e.kind == kind::MORTAR {
                    Quat::from_rotation_z((time * 12.0) as f32)
                } else {
                    Quat::IDENTITY
                };
                Quat::from_rotation_arc(Vec3::NEG_Z, vel.normalize()) * spin
            };
            let mut tint = Tint::default();
            if parried {
                tint.body = Vec4::new(2.6, 2.5, 2.3, 1.0);
                tint.glow = Vec4::splat(3.0);
            }
            if stuck && e.kind == kind::STICKY && (time * 4.0).fract() < 0.5 {
                tint.glow = Vec4::new(3.0, 0.4, 0.3, 1.0);
            }
            lib.draw_model(
                model,
                Mat4::from_rotation_translation(rot, pos),
                &tint,
                draws,
            );
            let light = match e.kind {
                _ if parried => Some((Vec3::new(1.5, 1.5, 1.4), 5.0)),
                kind::ROCKET | kind::MORTAR => Some((Vec3::new(1.8, 0.9, 0.3), 5.0)),
                kind::SPITTER_ORB => Some((Vec3::new(0.5, 1.4, 0.3), 5.0)),
                kind::GNAT_BOLT => Some((Vec3::new(1.2, 0.3, 1.2), 3.0)),
                _ => None,
            };
            if let Some((color, radius)) = light {
                lights.push(PointLight {
                    position: pos,
                    color,
                    radius,
                });
            }
        }
    }
}

/// One slowly drifting puff of red mist.
fn blood_mist(effects: &mut Effects, rng: &mut Rng, center: Vec3, radius: f32, big: bool) {
    let r = radius.max(0.5);
    let p = center + rng.dir() * r * 0.35;
    effects.spawn(Particle {
        position: p,
        velocity: rng.dir() * 0.6 + Vec3::Y * 0.3,
        gravity: -0.2,
        drag: 1.0,
        max_life: rng.range(0.5, 0.9),
        size: Vec2::splat(r * rng.range(0.2, 0.32)),
        end_scale: 1.8,
        color: Vec4::new(0.6, 0.03, 0.05, if big { 0.22 } else { 0.32 }),
        end_alpha: 0.0,
        texture: FxTex::Smoke,
        rotation: rng.angle(),
        spin: rng.range(-0.5, 0.5),
        ..Default::default()
    });
}
