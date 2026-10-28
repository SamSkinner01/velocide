//! Drawing the combat entity kinds the server replicates, and the client-side bookkeeping that
//! turns their state changes into effects:
//!
//! * **enemies** (kinds 16..64): `ModelLibrary::draw_enemy` with an [`EnemyAnim`] mapped from
//!   the replicated AI state (+ wind-up progress and attack index in `aux`), speed and facing;
//!   a per-enemy walk cycle; a white hurt flash + a spray of wax and sparks when `health`
//!   drops; a yellow strobe in the parryable phase; on death one big ember burst and a
//!   smouldering, melting corpse; the Heron's aim laser (`AIMING` / `AIM_LOCKED`), the diving
//!   Harrier's streak, the swat burst, the Gaoler's footsteps (see `enemy_fx.rs`).
//! * **projectiles** (64..128): their model at the trajectory position for the render tick
//!   (`shared::sim::projectiles::replicated_position`), facing their velocity; trails / glows
//!   / lights per kind (bomb fuse sparks, Wisp crackle + hum, the stuck Gravewell's vortex and
//!   hum, ember bombs smoke and drop embers, needles draw a long streak); `PARRIED` white-hot,
//!   `PENDING` hidden.
//! * **ember bursts** (128): heal pools; glowing embers (the palette's heal colour) rising
//!   over a smoky wisp while the pool lasts.
//! * **thorns** (160): the glass spike flying along its path, planted tip-first in the
//!   surface (a pulsing relay glint so it reads at a distance) or lying on the floor.
//! * **brands**: a turning sigil over every branded enemy / player (`BRANDED` flag).
//! * **the third roster** (`enemy_fx.rs`): frenzied / elite looks, the toll's ring, a Snuffer
//!   latched onto the local player drawn clamped below the view, slag lines (kinds 140..144).

use std::collections::HashMap;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use engine::physics::CollisionWorld;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use shared::protocol::{
    EntityState, NetId, ember_flags, enemy_flags, kind, player_flags, projectile_flags, thorn_flags,
};
use shared::sim::AiState;
use shared::sim::projectiles::{ProjectileKind, replicated_position};
use shared::tuning::CombatTuning;

use super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::Rng;
use super::enemy_fx;
use super::fx::{Beams, brand_color, enemy_projectile_color};
use crate::models::enemies::{EnemyAnim, EnemyKind, EnemyState};
use crate::models::projectiles::THORN_TIP;
use crate::models::{ModelId, ModelLibrary, Tint, WeaponKind, WeaponLook as _};
use crate::palette::palette;

/// Model kind for a replicated enemy kind (unknown kinds draw as Gnawers).
pub fn enemy_kind(k: u16) -> EnemyKind {
    enemy_fx::enemy_kind(k)
}

/// Display name of an enemy kind (kill feed).
pub fn enemy_name(k: u16) -> &'static str {
    enemy_fx::enemy_name(k)
}

/// Hit capsule of an enemy for client-side tracer ends: (radius, height).
pub fn enemy_size(k: u16) -> (f32, f32) {
    enemy_fx::enemy_size(k)
}

/// Model offset of the enemy's root from its replicated feet position (the Harrier model's
/// origin is its hover centre).
fn root_offset(k: u16) -> Vec3 {
    Vec3::Y * enemy_kind(k).root_lift()
}

/// The animation input for a replicated enemy.
pub fn enemy_anim(e: &EntityState, cycle: f32, time: f64) -> EnemyAnim {
    let k = enemy_kind(e.kind);
    let progress = (e.aux & 0xFF) as f32 / 255.0;
    let attack = ((e.aux >> 8) & 0xFF) as u8;
    let flat = Vec3::new(e.velocity.x, 0.0, e.velocity.z);
    let speed = flat.length();
    let state = match AiState::from_u16(e.state) {
        Some(AiState::Windup) => EnemyState::Windup(progress),
        Some(AiState::Strike) => EnemyState::Attack(progress),
        Some(AiState::Recover) => EnemyState::Recover(progress),
        Some(AiState::Stunned) => EnemyState::Stunned,
        Some(AiState::Latched) => EnemyState::Latched,
        Some(AiState::Dying) => EnemyState::Dying(progress),
        // A jump on its path: crouched on the ground, then in the air.
        Some(AiState::Leap) if e.flags & enemy_flags::AIRBORNE != 0 => {
            EnemyState::Airborne(progress)
        }
        Some(AiState::Leap) => EnemyState::Crouch(progress),
        Some(AiState::Melt) => EnemyState::Melt(progress),
        Some(AiState::Reform) => EnemyState::Reform(progress),
        _ if speed > 0.2 => EnemyState::Walk,
        _ => EnemyState::Idle,
    };
    let fwd = shared::input::look_dir(e.yaw, 0.0);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    // Heads aim with the replicated laser pitch; a diving Harrier points down its dive, a
    // leaping Mould its flight.
    let aim_pitch = if e.flags & enemy_flags::AIMING != 0 {
        e.pitch
    } else if (k == EnemyKind::Harrier && state == EnemyState::Attack(progress))
        || matches!(state, EnemyState::Airborne(_))
    {
        e.velocity.y.atan2(speed.max(1e-3))
    } else {
        0.0
    };
    EnemyAnim {
        kind: k,
        state,
        attack,
        speed,
        cycle,
        strafe: if speed > 0.1 {
            (flat.dot(right) / speed).clamp(-1.0, 1.0)
        } else {
            0.0
        },
        aim_pitch,
        aim_locked: e.flags & enemy_flags::AIM_LOCKED != 0,
        time,
        seed: e.id.0.wrapping_mul(2_654_435_761),
    }
}

/// Model of a projectile kind.
pub fn projectile_model(k: u16) -> Option<ModelId> {
    Some(match ProjectileKind::from_net_kind(k)? {
        ProjectileKind::Bomb => ModelId::Bomb,
        ProjectileKind::Gravewell => ModelId::Gravewell,
        ProjectileKind::Wisp => ModelId::Wisp,
        ProjectileKind::EmberBomb => ModelId::EmberBomb,
        ProjectileKind::Needle => ModelId::Needle,
        // The expansion arsenal's own (drawn through `arsenal.rs`, which picks the model).
        _ => return super::arsenal::projectile_kind(k).map(|p| p.model()),
    })
}

/// Velocity of a trajectory-replicated entity at `time_ticks`.
pub(crate) fn replicated_velocity(
    e: &EntityState,
    time_ticks: f64,
    rate: u32,
    t: &CombatTuning,
) -> Vec3 {
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
    /// Replicated state last frame (state-change effects).
    state: u16,
    locked: bool,
    /// Seconds since death (the heat-release light), and until the next smoulder puff.
    dead_for: f32,
    smoulder: f32,
    /// The aim laser this frame: (origin, end, locked).
    laser: Option<(Vec3, Vec3, bool)>,
    /// Frenzy / elite / latch effects state.
    extra: enemy_fx::ExtraVis,
    /// In the air on a leap last frame (takeoff / landing dust), and seconds to the next steam
    /// puff while melting or rising.
    airborne: bool,
    steam: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ProjVis {
    trail: f32,
    seen: bool,
    position: Vec3,
    /// Where it was first drawn (the needle's streak doesn't reach back past its muzzle).
    start: Vec3,
    age: f32,
    /// Seconds to the next hum (Wisp) / the hum already started (stuck Gravewell).
    hum: f32,
    hummed: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct EmberVis {
    puff: f32,
    seen: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct ThornVis {
    glint: f32,
    seen: bool,
    position: Vec3,
}

/// Thorn colour (glints, relay glow).
const THORN_COLOR: Vec4 = Vec4::new(0.75, 0.95, 1.0, 1.0);

/// Inputs shared by update and draw.
pub struct EntityFrame<'a> {
    /// Interpolated entities (drawn at `render_tick`).
    pub entities: &'a [EntityState],
    pub render_tick: f64,
    pub tick_rate: u32,
    pub tuning: &'a CombatTuning,
    /// The local player's own projectiles from the newest snapshot: the ones not flown from
    /// the shot's view time (deflected ones, the orbiting Wisp, the steered Kite, the
    /// returning Hurl) are drawn from here at `own_time` (the present);
    /// the rest from `entities` at the render tick, their stand-ins covering the start
    /// (`stand_in.rs`).
    pub own: &'a [EntityState],
    pub own_time: f64,
    /// Owner code of the local player.
    pub local_owner: Option<u16>,
    /// Muzzle minus eye of the local weapon: own projectiles leave the drawn barrel and blend
    /// onto their true (eye-based) path over their first ~0.15 s.
    pub own_offset: Vec3,
    /// Stand-ins for own projectiles the server hasn't reported yet, and the handovers of the
    /// ones it just did (`stand_in.rs`).
    pub stand_ins: Option<&'a super::stand_in::StandIns>,
    /// Camera position (nothing is drawn inside the camera) ...
    pub camera: Vec3,
    /// ... and where it looks (parryable projectiles flare in punch reach in front of it).
    pub view: Vec3,
    /// The level (aim lasers stop at walls).
    pub world: Option<&'a CollisionWorld>,
}

/// Seconds over which an own projectile blends from the muzzle onto its true path.
const MUZZLE_BLEND: f32 = 0.15;

impl EntityFrame<'_> {
    /// Every projectile to show: the entity, the time to evaluate its trajectory at and a
    /// visual offset (own projectiles start at the muzzle). The local player's own are drawn at
    /// the render tick like the rest when the server flies them from the shot's view time
    /// (`stand_in::on_render_clock`), else at `own_time`.
    pub(crate) fn projectiles(&self) -> impl Iterator<Item = (&EntityState, f64, Vec3)> {
        let own = self.local_owner;
        let stand_ins = self.stand_ins;
        self.entities
            .iter()
            .filter(move |e| {
                kind::is_projectile(e.kind)
                    && (Some(e.owner) != own
                        || (super::stand_in::on_render_clock(e)
                            && !stand_ins.is_some_and(|s| s.hidden(e.id))))
            })
            .map(move |e| {
                // Just handed over from a stand-in: drawn where the stand-in was, easing onto
                // its own path.
                match stand_ins.and_then(|s| s.handover(e.id)) {
                    Some((ahead, offset)) if Some(e.owner) == own => {
                        (e, self.render_tick + ahead, offset)
                    }
                    _ => (e, self.render_tick, Vec3::ZERO),
                }
            })
            .chain(
                self.own
                    .iter()
                    .filter(|e| kind::is_projectile(e.kind) && !super::stand_in::on_render_clock(e))
                    .map(|e| {
                        // Just handed over from a stand-in: drawn where the stand-in was,
                        // easing onto its own path.
                        if let Some((ahead, offset)) = self.stand_ins.and_then(|s| s.handover(e.id))
                        {
                            return (e, self.own_time + ahead, offset);
                        }
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
            .chain(
                self.stand_ins
                    .into_iter()
                    .flat_map(|s| s.drawn(self.own_time, self.render_tick)),
            )
    }
}

#[derive(Default)]
pub struct EntityView {
    enemies: HashMap<NetId, EnemyVis>,
    projectiles: HashMap<NetId, ProjVis>,
    embers: HashMap<NetId, EmberVis>,
    thorns: HashMap<NetId, ThornVis>,
    /// The Smelter's slag lines.
    slag: enemy_fx::SlagView,
    time: f64,
}

impl EntityView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.enemies.clear();
        self.projectiles.clear();
        self.embers.clear();
        self.thorns.clear();
        self.slag.clear();
    }

    /// Stand-ins handed over to the server's projectiles: their per-projectile state (trail,
    /// age) carries over to the real id.
    pub fn adopt(&mut self, claims: &[(NetId, NetId)]) {
        for (from, to) in claims {
            if let Some(v) = self.projectiles.remove(from) {
                self.projectiles.insert(*to, v);
            }
        }
    }

    /// Last drawn position of a thorn.
    pub fn thorn_position(&self, id: NetId) -> Option<Vec3> {
        self.thorns.get(&id).map(|p| p.position)
    }

    /// A turning brand sigil over every branded enemy and other player this frame, facing the
    /// camera (the local player's own brand shows on the HUD).
    pub fn brand_sigils(
        &self,
        entities: &[EntityState],
        local: Option<shared::sim::PlayerId>,
        camera: Vec3,
        beams: &mut Beams,
    ) {
        let angle = (self.time * 1.8 % std::f64::consts::TAU) as f32;
        for e in entities {
            let height = if kind::is_enemy(e.kind) {
                if e.flags & enemy_flags::BRANDED == 0 || e.flags & enemy_flags::DYING != 0 {
                    continue;
                }
                enemy_size(e.kind).1
            } else if e.kind == kind::PLAYER {
                if e.flags & player_flags::BRANDED == 0
                    || e.flags & player_flags::DEAD != 0
                    || e.id.as_player() == local
                {
                    continue;
                }
                2.0
            } else {
                continue;
            };
            let pulse = 0.85 + 0.15 * ((self.time * 7.0).sin() as f32);
            let above = e.position + Vec3::Y * (height + 0.7);
            let facing = (camera - above).normalize_or(Vec3::Z);
            let brand = brand_color();
            beams.sigil(above, facing, 0.5 * pulse, brand, angle);
            beams.sigil(above, facing, 0.28, brand * 1.2, -angle * 1.6);
        }
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
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        f: &EntityFrame,
        dt: f32,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
        beams: &mut Beams,
    ) {
        self.time += dt as f64;
        for v in self.enemies.values_mut() {
            v.seen = false;
        }
        for v in self.projectiles.values_mut() {
            v.seen = false;
        }
        for v in self.embers.values_mut() {
            v.seen = false;
        }
        for v in self.thorns.values_mut() {
            v.seen = false;
        }
        self.slag.begin();
        for e in f.entities {
            if kind::is_enemy_hazard(e.kind) {
                self.slag.update(e, f.world, dt, effects, audio, rng);
            } else if kind::is_enemy(e.kind) {
                self.update_enemy(e, f, dt, effects, audio, rng, beams);
            } else if e.kind == kind::EMBER_BURST {
                let v = self.embers.entry(e.id).or_default();
                let first = !v.seen && v.puff == 0.0;
                v.seen = true;
                let radius = e.state as f32 / 10.0;
                let big = e.flags & ember_flags::BIG != 0;
                if first {
                    let n = if big {
                        9
                    } else {
                        (3.0 + e.health as f32 / 4.0).min(8.0) as usize
                    };
                    for _ in 0..n {
                        ember_puff(effects, rng, e.position, radius, big);
                    }
                    v.puff = 0.001;
                }
                v.puff += dt;
                if e.health > 0 && v.puff > if big { 0.25 } else { 0.15 } {
                    v.puff = 0.001;
                    ember_puff(effects, rng, e.position, radius, big);
                }
            } else if e.kind == kind::THORN {
                let pos = replicated_position(e, f.render_tick, f.tick_rate, f.tuning);
                let v = self.thorns.entry(e.id).or_default();
                v.seen = true;
                v.position = pos;
                let planted = e.flags & thorn_flags::PLANTED != 0;
                // Planted thorns are lit relays you placed on purpose: a steady halo and a
                // pulse ring so they read from across the arena.
                if planted {
                    effects.spawn(Particle {
                        position: pos,
                        max_life: dt.max(0.016) * 1.5,
                        size: Vec2::splat(0.55),
                        color: Vec4::new(0.6, 0.85, 1.0, 0.55),
                        end_alpha: 1.0,
                        texture: FxTex::Glow,
                        additive: true,
                        ..Default::default()
                    });
                }
                v.glint -= dt;
                if v.glint <= 0.0 {
                    v.glint = if planted { 0.6 } else { rng.range(0.15, 0.3) };
                    if planted {
                        let facing = e.velocity.normalize_or(Vec3::Y);
                        beams.ring(pos, facing, 0.1, 0.7, 0.03, THORN_COLOR * 1.6, 0.4);
                    }
                    effects.glow(pos + rng.dir() * 0.05, THORN_COLOR, 0.35, 0.35);
                }
            }
        }
        for (e, time, offset) in f.projectiles() {
            self.update_projectile(e, time, offset, f, dt, effects, audio, rng, beams);
        }
        self.enemies.retain(|_, v| v.seen);
        self.projectiles.retain(|_, v| v.seen);
        self.embers.retain(|_, v| v.seen);
        self.thorns.retain(|_, v| v.seen);
        self.slag.end();
    }

    #[allow(clippy::too_many_arguments)]
    fn update_enemy(
        &mut self,
        e: &EntityState,
        f: &EntityFrame,
        dt: f32,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
        beams: &mut Beams,
    ) {
        let k = enemy_kind(e.kind);
        let height = k.height();
        let center = e.position
            + Vec3::Y * (k.root_lift() + height * if k == EnemyKind::Harrier { 0.0 } else { 0.5 });
        let dying = e.flags & enemy_flags::DYING != 0 || e.state == AiState::Dying as u16;
        let (wax, heat) = (enemy_fx::wax_color(e.kind), enemy_fx::heat_color(e.kind));
        let v = self.enemies.entry(e.id).or_insert(EnemyVis {
            last_health: e.health,
            died: dying,
            state: e.state,
            ..Default::default()
        });
        v.seen = true;
        v.position = center;
        let speed = Vec3::new(e.velocity.x, 0.0, e.velocity.z).length();
        let before = v.cycle;
        v.cycle = k.advance_cycle(v.cycle, speed, dt);
        v.flash = (v.flash - dt * 6.0).max(0.0);
        if e.health < v.last_health && !dying {
            // A hit cracks the wax: chips, molten droplets, sparks.
            let lost = (v.last_health - e.health) as f32;
            v.flash = 1.0;
            let dir = (rng.dir() + Vec3::Y * 0.5).normalize_or(Vec3::Y);
            enemy_fx::wax_hit(
                effects,
                rng,
                center,
                dir,
                (0.6 + lost / 20.0).min(3.5),
                wax,
                heat,
            );
            audio.play(enemy_fx::hurt_sfx(e.kind), PlayParams::at(center));
        }
        v.last_health = e.health;
        if dying && !v.died {
            // The heat leaves it as one big ember burst; the body slumps and melts.
            v.died = true;
            let size = (height / 2.2).clamp(0.7, 1.8);
            enemy_fx::ember_burst(effects, rng, center, size, wax, heat);
            let death = enemy_fx::death_sfx(e.kind);
            audio.play(death, PlayParams::at(center).with_pitch(1.25 / size.sqrt()));
            audio.play(Sfx::EmberRelease, PlayParams::at(center));
        }
        if dying {
            v.dead_for += dt;
            v.smoulder -= dt;
            if v.smoulder <= 0.0 {
                v.smoulder = rng.range(0.12, 0.3);
                let spread = enemy_size(e.kind).0;
                enemy_fx::smoulder(effects, rng, e.position + Vec3::Y * 0.3, spread, heat);
            }
        }
        let state = AiState::from_u16(e.state);
        let entered = |s: AiState| v.state != s as u16 && state == Some(s);
        // A swatted Harrier: wax shards and a crunch.
        if k == EnemyKind::Harrier && entered(AiState::Stunned) {
            enemy_fx::swat_burst(effects, rng, center, wax, heat);
            audio.play(Sfx::HarrierSwat, PlayParams::at(center));
        }
        // The Heron's laser locks: a click and a flare at the lens.
        let locked = e.flags & enemy_flags::AIM_LOCKED != 0;
        v.laser = enemy_fx::aim_laser(e).map(|(origin, dir, locked)| {
            (
                origin,
                enemy_fx::laser_end(f.world, origin, dir, f.entities, f.camera),
                locked,
            )
        });
        if let Some((origin, end, locked)) = v.laser {
            let p = palette();
            let c = if locked {
                Vec4::new(1.0, 0.85, 0.8, 1.0)
            } else {
                // A dim role colour: these sprites overlap and add up (a bright blue would sum to
                // white, like the locked glare).
                p.effect(Vec4::new(1.0, 0.15, 0.08, 0.9), p.aim_laser, 0.45)
            };
            // The dot where it lands, and a glare on the lens.
            effects.glow(end, c, if locked { 0.45 } else { 0.32 }, 0.05);
            effects.glow(origin, c, if locked { 0.9 } else { 0.55 }, 0.05);
        }
        if locked
            && !v.locked
            && let Some((origin, _, _)) = v.laser
        {
            audio.play(Sfx::HeronLock, PlayParams::at(origin));
            effects.glow(origin, Vec4::new(1.0, 0.95, 0.9, 1.0), 1.4, 0.12);
        }
        v.locked = locked;
        let attack = enemy_fx::attack_name(e.kind, ((e.aux >> 8) & 0xFF) as u8);
        match (k, state, attack.as_deref()) {
            // Diving: a hot streak behind it.
            (EnemyKind::Harrier, Some(AiState::Strike), _) => {
                let back = -e.velocity.normalize_or_zero();
                enemy_fx::dive_streak(effects, rng, center, back);
            }
            // Whirling the censer overhead: embers flung off the whirl.
            (EnemyKind::Censer, Some(AiState::Windup | AiState::Strike), Some("volley")) => {
                if rng.unit() < dt * 30.0 {
                    let a = rng.angle();
                    let at = e.position + Vec3::new(a.cos() * 0.9, 3.0, a.sin() * 0.9);
                    effects.spawn(Particle {
                        position: at,
                        velocity: Vec3::new(-a.sin(), 0.3, a.cos()) * rng.range(2.0, 5.0),
                        gravity: 6.0,
                        max_life: rng.range(0.3, 0.6),
                        size: Vec2::splat(0.09),
                        end_scale: 0.3,
                        color: palette().effect(
                            Vec4::new(1.0, 0.6, 0.2, 1.0),
                            palette().telegraph,
                            1.0,
                        ),
                        texture: FxTex::Spark,
                        additive: true,
                        ..Default::default()
                    });
                }
            }
            // Spinning the chain up low: red sparks thrown off a ring at knee height.
            (EnemyKind::Gaoler, Some(AiState::Windup), Some("sweep")) => {
                for _ in 0..3 {
                    if rng.unit() < dt * 30.0 {
                        let a = rng.angle();
                        let r = rng.range(1.7, 2.7);
                        let at =
                            e.position + Vec3::new(a.cos() * r, rng.range(0.5, 1.1), a.sin() * r);
                        effects.spawn(Particle {
                            position: at,
                            velocity: Vec3::new(-a.sin(), 0.15, a.cos()) * rng.range(3.0, 7.0),
                            gravity: 4.0,
                            max_life: rng.range(0.3, 0.6),
                            size: Vec2::splat(0.2),
                            end_scale: 0.3,
                            color: palette().effect(
                                Vec4::new(1.0, 0.25, 0.08, 1.0),
                                palette().telegraph,
                                1.0,
                            ),
                            texture: FxTex::Spark,
                            additive: true,
                            ..Default::default()
                        });
                    }
                }
            }
            // The rush: dust thrown up behind it.
            (EnemyKind::Gaoler, Some(AiState::Strike), Some("rush")) if rng.unit() < dt * 20.0 => {
                effects.dust(e.position, 2, 2.0);
            }
            _ => {}
        }
        // The Gaoler's heavy footfalls.
        if k == EnemyKind::Gaoler
            && !dying
            && speed > 0.5
            && (v.cycle * 2.0).floor() != (before * 2.0).floor()
        {
            audio.play(Sfx::GaolerStep, PlayParams::at(e.position));
            effects.dust(e.position, 3, 1.2);
        }
        // Getting there: dust where a leap takes off and lands; a melt slops wax out and
        // steams, the puddle where it re-forms bubbles up and it rises with a splash ring.
        let size = enemy_size(e.kind).0;
        let airborne = state == Some(AiState::Leap) && e.flags & enemy_flags::AIRBORNE != 0;
        let heavy = matches!(k, EnemyKind::Gaoler | EnemyKind::Smelter);
        let pitch = (1.1 / size.max(0.4)).clamp(0.6, 1.4);
        if airborne && !v.airborne {
            enemy_fx::leap_dust(effects, rng, e.position, size);
            audio.play(
                Sfx::Jump,
                PlayParams::at(e.position)
                    .with_pitch(pitch * 0.7)
                    .with_volume(0.8),
            );
        }
        if v.airborne && !airborne && !dying && e.flags & enemy_flags::AIRBORNE == 0 {
            enemy_fx::leap_dust(effects, rng, e.position, size * 1.2);
            let land = if heavy { Sfx::HeavyLand } else { Sfx::Land };
            audio.play(land, PlayParams::at(e.position).with_pitch(pitch * 0.8));
        }
        v.airborne = airborne && !dying;
        if entered(AiState::Melt) {
            enemy_fx::melt_burst(effects, rng, e.position, size, wax);
            audio.play(Sfx::MouldMelt, PlayParams::at(center).with_pitch(pitch));
        }
        if entered(AiState::Reform) {
            enemy_fx::rise_burst(effects, rng, e.position, size, wax);
            audio.play(Sfx::MouldRise, PlayParams::at(e.position).with_pitch(pitch));
        }
        if v.state == AiState::Reform as u16 && state != Some(AiState::Reform) && !dying {
            beams.ring(
                e.position + Vec3::Y * 0.08,
                Vec3::Y,
                size * 0.8,
                size * 2.6,
                0.08,
                wax * Vec4::new(1.2, 1.2, 1.2, 0.8),
                0.4,
            );
        }
        if matches!(state, Some(AiState::Melt | AiState::Reform)) {
            v.steam -= dt;
            if v.steam <= 0.0 {
                v.steam = rng.range(0.08, 0.18);
                enemy_fx::melt_steam(effects, rng, e.position, size);
            }
        }
        let frame = enemy_fx::ExtraFrame {
            center,
            height,
            camera: f.camera,
            local_owner: f.local_owner,
            dt,
            cycle_before: before,
            cycle: v.cycle,
        };
        enemy_fx::enemy_extras(e, v.state, &mut v.extra, &frame, effects, audio, rng);
        v.state = e.state;
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
        audio: &mut Audio,
        rng: &mut Rng,
        beams: &mut Beams,
    ) {
        let pos = replicated_position(e, time, f.tick_rate, f.tuning) + offset;
        let v = self.projectiles.entry(e.id).or_insert_with(|| {
            if e.kind == kind::EMBER_BOMB && e.flags & projectile_flags::PARRIED == 0 {
                audio.play(Sfx::EmberFlight, PlayParams::at(pos));
            }
            ProjVis {
                start: pos,
                ..Default::default()
            }
        });
        v.seen = true;
        v.position = pos;
        v.age += dt;
        if e.flags & projectile_flags::PENDING != 0 || pos.distance(f.camera) < 1.0 {
            return;
        }
        // Halo, core and streak: every projectile reads at a glance (docs/feel.md).
        let vel = replicated_velocity(e, time, f.tick_rate, f.tuning);
        projectile_glow(e, pos, vel, v.start, v.age, f, self.time, effects, beams);
        // The expansion arsenal's projectiles have their own trails (arsenal.rs).
        if e.flags & projectile_flags::PARRIED == 0 {
            let vel = replicated_velocity(e, time, f.tick_rate, f.tuning);
            if let Some(view) = super::arsenal::projectile_view(e, pos, vel, v.age, f.world) {
                super::arsenal::projectile_trail(effects, beams, rng, &view, dt);
                return;
            }
        }
        let stuck = e.flags & projectile_flags::STUCK != 0;
        let parried = e.flags & projectile_flags::PARRIED != 0;
        // Hums: the Wisp crackles as it drifts, a stuck Gravewell drones once while it pulls.
        v.hum -= dt;
        if e.kind == kind::WISP && v.hum <= 0.0 {
            v.hum = 0.45;
            audio.play(Sfx::WispHum, PlayParams::at(pos));
        }
        if e.kind == kind::GRAVEWELL && stuck && !v.hummed {
            v.hummed = true;
            audio.play(Sfx::GravewellHum, PlayParams::at(pos));
        }
        if e.kind == kind::WISP && rng.unit() < dt * 14.0 {
            let violet = WeaponKind::Arclash.glow_color(0);
            let to = pos + rng.dir() * rng.range(0.4, 0.9);
            beams.arc(
                pos,
                to,
                0.03,
                (violet.truncate() * 1.8).extend(0.9),
                0.07,
                rng,
            );
        }
        v.trail += dt;
        let interval = match e.kind {
            kind::GRAVEWELL if stuck => 0.012,
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
                color: palette().effect(Vec4::new(1.0, 0.95, 0.85, 0.9), palette().deflected, 1.0),
                texture: FxTex::Glow,
                additive: true,
                ..Default::default()
            });
            return;
        }
        match e.kind {
            kind::BOMB => {
                // The fuse spits sparks; a faint smoke line follows the bounces.
                effects.spawn(Particle {
                    position: pos + Vec3::Y * 0.2,
                    velocity: rng.dir() * 2.5 + Vec3::Y * 1.5,
                    gravity: 10.0,
                    max_life: 0.18,
                    size: Vec2::splat(0.09),
                    color: Vec4::new(1.0, 0.65, 0.2, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    collide: Some(OnHit::Die),
                    ..Default::default()
                });
                if rng.unit() < 0.4 {
                    effects.spawn(Particle {
                        position: pos + back * 0.2,
                        gravity: -0.4,
                        drag: 1.5,
                        max_life: rng.range(0.3, 0.5),
                        size: Vec2::splat(0.18),
                        end_scale: 2.4,
                        color: Vec4::new(0.45, 0.42, 0.4, 0.4),
                        texture: FxTex::Smoke,
                        rotation: rng.angle(),
                        ..Default::default()
                    });
                }
            }
            kind::GRAVEWELL if stuck => {
                // Vortex: motes drawn in from the pull radius, spiralling to the core.
                let green = WeaponKind::Tumbler.glow_color(1);
                let r = f.tuning.tumbler.gravewell_pull_radius * rng.range(0.4, 1.0);
                let d = rng.dir();
                let start = pos + Vec3::new(d.x, d.y.abs() * 0.6, d.z) * r;
                let to = pos - start;
                let swirl = to.cross(Vec3::Y).normalize_or_zero() * to.length() * 0.6;
                let time = rng.range(0.4, 0.7);
                effects.spawn(Particle {
                    position: start,
                    velocity: (to + swirl) / time,
                    drag: 0.8,
                    max_life: time,
                    size: Vec2::splat(rng.range(0.12, 0.22)),
                    end_scale: 0.3,
                    color: (green.truncate() * 1.6).extend(0.8),
                    end_alpha: 1.0,
                    texture: FxTex::Glow,
                    additive: true,
                    ..Default::default()
                });
                if rng.unit() < 0.15 {
                    effects.glow(pos, Vec4::new(0.05, 0.12, 0.06, 1.0), 1.6, 0.3);
                    effects.glow(pos, (green.truncate() * 1.4).extend(0.9), 0.9, 0.2);
                }
            }
            kind::GRAVEWELL => {
                let green = WeaponKind::Tumbler.glow_color(1);
                effects.spawn(Particle {
                    position: pos,
                    max_life: 0.25,
                    size: Vec2::splat(0.3),
                    end_scale: 0.3,
                    color: (green.truncate() * 1.5).extend(0.8),
                    texture: FxTex::Glow,
                    additive: true,
                    ..Default::default()
                });
            }
            kind::WISP => {
                let violet = WeaponKind::Arclash.glow_color(0);
                effects.spawn(Particle {
                    position: pos + rng.dir() * 0.15,
                    max_life: 0.25,
                    size: Vec2::splat(0.6),
                    end_scale: 0.4,
                    color: (violet.truncate() * 1.4).extend(0.7),
                    texture: FxTex::Glow,
                    additive: true,
                    ..Default::default()
                });
            }
            kind::EMBER_BOMB => enemy_fx::ember_trail(effects, rng, pos, back),
            kind::NEEDLE => {
                // Hot sparks shed along the needle's line (its streak is drawn in `draw`).
                let c = enemy_projectile_color(e.kind);
                effects.spawn(Particle {
                    position: pos + back * rng.range(0.5, 3.0),
                    velocity: rng.dir() * 0.6,
                    max_life: 0.25,
                    size: Vec2::splat(0.12),
                    end_scale: 0.3,
                    color: c,
                    texture: FxTex::Spark,
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
                // Colour-blind palettes: telegraph glows and weak points in their roles (the
                // standard palette keeps each Mould's own colours).
                let pal = palette();
                tint.glow_color = pal.model_glow(pal.telegraph);
                tint.core_color = pal.model_glow(pal.weak_point);
                if e.flags & enemy_flags::PARRYABLE != 0 {
                    let on = (time * 18.0).fract() < 0.5;
                    if pal.recolours_world() {
                        // Strobe the parry colour between full and dim (brightness is capped
                        // so the hue survives).
                        tint.glow_color = Some(pal.parry_window);
                        tint.glow = Vec4::splat(if on { 1.0 } else { 0.55 });
                    } else {
                        let strobe = if on { 3.0 } else { 1.6 };
                        tint.glow = Vec4::new(strobe, strobe * 0.9, 0.4, 1.0);
                    }
                }
                if let EnemyState::Dying(p) = anim.state
                    && p > 0.7
                {
                    tint.body.w = ((1.0 - p) / 0.3).clamp(0.05, 1.0);
                }
                enemy_fx::decorate_tint(e, time, &mut tint);
                // Burning (arsenal): an ember flicker over the body.
                if e.flags & enemy_flags::BURNING != 0 {
                    tint.body *= super::arsenal::burn_tint(time, e.id.0);
                }
                // A Snuffer latched onto you is drawn clamped below your view (its replicated
                // position trails the predicted camera).
                let root = if enemy_fx::latched_on_local(e, f.local_owner) {
                    enemy_fx::latched_root(e, f.camera)
                } else {
                    EnemyAnim::root(e.position + root_offset(e.kind), e.yaw)
                };
                lib.draw_enemy(&anim, root, &tint, draws);
                // The puddle it melts into / rises out of.
                if let EnemyState::Melt(p) | EnemyState::Reform(p) = anim.state {
                    let melt = matches!(anim.state, EnemyState::Melt(_));
                    let size = enemy_size(e.kind).0;
                    let r = enemy_fx::puddle_radius(melt, p, size);
                    enemy_fx::draw_puddle(lib, e, r, draws);
                }
                if let Some((scale, shell)) = enemy_fx::elite_shell(e, time) {
                    let pivot = Vec3::Y * (enemy_kind(e.kind).height() * 0.5);
                    let grow = Mat4::from_translation(pivot)
                        * Mat4::from_scale(Vec3::splat(scale))
                        * Mat4::from_translation(-pivot);
                    lib.draw_enemy(&anim, root * grow, &shell, draws);
                }
                enemy_fx::enemy_lights(e, v.position, time, lights);
                // The aim laser (every client draws it from the replicated aim).
                if let Some((origin, end, locked)) = v.laser {
                    draw_beam(
                        lib,
                        origin,
                        end,
                        if locked { 0.07 } else { 0.045 },
                        laser_tint(locked, time),
                        draws,
                    );
                    let tracking = pal.light(Vec3::new(1.6, 0.25, 0.15), pal.aim_laser, 1.6);
                    lights.push(PointLight {
                        position: origin,
                        color: if locked {
                            Vec3::new(2.0, 1.2, 1.0)
                        } else {
                            tracking
                        },
                        radius: 4.0,
                    });
                }
                // The heat leaving a fresh corpse lights up the room for a moment.
                if v.died && v.dead_for < 0.6 {
                    let fade = 1.0 - v.dead_for / 0.6;
                    let heat = enemy_fx::heat_color(e.kind).truncate();
                    lights.push(PointLight {
                        position: v.position,
                        color: heat * 2.5 * fade,
                        radius: 4.0 + 4.0 * enemy_kind(e.kind).height() / 3.7,
                    });
                }
            } else if kind::is_enemy_hazard(e.kind) {
                self.slag.draw(lib, e, time, draws, lights);
            } else if e.kind == kind::THORN {
                let (rot, pos) = thorn_transform(e, f, time);
                let pulse = 1.4 + 0.6 * ((time * 5.0 + e.id.0 as f64).sin() as f32);
                let tint = Tint {
                    glow: Vec4::splat(pulse),
                    ..Default::default()
                };
                // Planted / lying thorns are drawn a little larger so the relay reads.
                let scale = if e.flags & thorn_flags::PLANTED != 0 {
                    1.5
                } else {
                    1.2
                };
                lib.draw_model(
                    ModelId::Thorn,
                    Mat4::from_scale_rotation_translation(Vec3::splat(scale), rot, pos),
                    &tint,
                    draws,
                );
                if e.flags & thorn_flags::PLANTED != 0 && lights.len() < 48 {
                    lights.push(PointLight {
                        position: pos,
                        color: Vec3::new(0.5, 0.75, 0.9) * (0.6 + 0.3 * pulse),
                        radius: 2.5,
                    });
                }
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
            // The expansion arsenal's projectiles (arsenal.rs); a parried one goes the
            // white-hot way below.
            if !parried {
                let age = self.projectiles.get(&e.id).map_or(0.0, |v| v.age);
                if let Some(view) = super::arsenal::projectile_view(e, pos, vel, age, f.world) {
                    super::arsenal::draw_projectile(lib, &view, self.time, draws, lights);
                    continue;
                }
            }
            let stuck = e.flags & projectile_flags::STUCK != 0;
            let rot = if e.kind == kind::GRAVEWELL && stuck {
                // Claws (-Z) bitten into the floor, the core turning.
                Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::NEG_Y)
                    * Quat::from_rotation_z((time * 3.0) as f32)
            } else if e.kind == kind::WISP {
                Quat::from_rotation_y((time * 7.0) as f32)
                    * Quat::from_rotation_x((time * 4.3) as f32)
            } else if stuck || vel.length_squared() < 1e-4 {
                Quat::from_rotation_y((e.id.0 % 7) as f32)
            } else {
                // Bombs tumble end over end.
                let spin = if e.kind == kind::BOMB {
                    Quat::from_rotation_x((time * -11.0) as f32)
                } else {
                    Quat::IDENTITY
                };
                Quat::from_rotation_arc(Vec3::NEG_Z, vel.normalize()) * spin
            };
            let mut tint = Tint::default();
            let pal = palette();
            let hostile = matches!(e.kind, kind::EMBER_BOMB | kind::NEEDLE) && !parried;
            if parried {
                tint.body = Vec4::new(2.6, 2.5, 2.3, 1.0);
                tint.glow = Vec4::splat(3.0);
            } else if hostile {
                // Enemy projectiles in the colour-blind palettes' hostile colour.
                tint.glow_color = pal.model_glow(pal.hostile_projectile);
            } else if e.kind == kind::BOMB {
                // The fuse band flickers faster as it burns down.
                let age = ((time - e.aux as f64) / f.tick_rate.max(1) as f64) as f32;
                let rate = 6.0 + age * 14.0;
                tint.glow = Vec4::splat(1.2 + 1.2 * ((age * rate).sin() * 0.5 + 0.5));
            } else if e.kind == kind::GRAVEWELL && stuck {
                tint.glow = Vec4::splat(1.6 + 0.8 * ((time * 9.0).sin() as f32));
            }
            // Drawn bigger than life so they read in flight (docs/feel.md); needles mostly
            // longer (a fat hexagon head-on would hide what's behind it).
            let scale = if stuck { 1.3 } else { projectile_scale(e.kind) };
            let scale = if parried { scale * 1.25 } else { scale };
            let scale = if e.kind == kind::NEEDLE {
                Vec3::new(scale * 0.7, scale * 0.7, scale)
            } else {
                Vec3::splat(scale)
            };
            lib.draw_model(
                model,
                Mat4::from_scale_rotation_translation(scale, rot, pos),
                &tint,
                draws,
            );
            if e.kind == kind::NEEDLE
                && let Some(pv) = self.projectiles.get(&e.id)
                && vel.length_squared() > 1.0
            {
                // A long glowing streak back along its flight (never past where it was first
                // seen).
                let dir = vel.normalize();
                let len = (pos - pv.start).dot(dir).clamp(0.0, 7.0);
                if len > 0.3 {
                    let tint = if parried {
                        glow_tint(Vec4::new(2.4, 2.3, 2.2, 0.9), None)
                    } else {
                        glow_tint(
                            Vec4::new(2.6, 0.9, 0.55, 0.85),
                            pal.model_glow(pal.hostile_projectile),
                        )
                    };
                    draw_beam(lib, pos - dir * len, pos, 0.12, tint, draws);
                }
            }
            let light = match e.kind {
                _ if parried => Some((Vec3::new(2.2, 2.2, 2.0), 7.0)),
                kind::BOMB => Some((Vec3::new(1.6, 0.8, 0.25), 4.5)),
                kind::GRAVEWELL if stuck => Some((Vec3::new(0.5, 2.0, 0.7), 7.0)),
                kind::GRAVEWELL => Some((Vec3::new(0.3, 1.2, 0.4), 3.0)),
                kind::WISP => Some((Vec3::new(1.2, 0.8, 2.2), 6.0)),
                kind::EMBER_BOMB => Some((
                    pal.light(Vec3::new(2.2, 1.1, 0.35), pal.hostile_projectile, 2.2),
                    6.5,
                )),
                kind::NEEDLE => Some((
                    pal.light(Vec3::new(1.8, 0.6, 0.35), pal.hostile_projectile, 1.8),
                    5.0,
                )),
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

/// How big a projectile model is drawn (× life size) in flight: at least twice, more for the
/// small fast ones (`docs/feel.md`). Stuck ones are drawn a little over life size.
pub fn projectile_scale(k: u16) -> f32 {
    match k {
        kind::NEEDLE => 3.0,
        kind::RIVET => 3.2,
        kind::ARROW
        | kind::PIN_ARROW
        | kind::BRAND_ARROW
        | kind::BLINK_ARROW
        | kind::BEACON_ARROW => 2.2,
        kind::FLAK_SHELL | kind::LIMPET => 2.4,
        _ => 2.0,
    }
}

/// How a projectile glows in flight: colour (standard palette), halo diameter (m) and how many
/// seconds of its flight its streak shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectileLook {
    pub color: Vec4,
    pub halo: f32,
    pub streak: f32,
}

/// The glow of a projectile kind (`parried`: white-hot, bigger, a long streak).
pub fn projectile_look(k: u16, parried: bool) -> ProjectileLook {
    let look = |r: f32, g: f32, b: f32, halo: f32, streak: f32| ProjectileLook {
        color: Vec4::new(r, g, b, 1.0),
        halo,
        streak,
    };
    if parried {
        return look(1.0, 0.97, 0.9, 1.6, 0.09);
    }
    match k {
        kind::BOMB => look(1.0, 0.55, 0.15, 1.0, 0.03),
        kind::GRAVEWELL => look(0.45, 1.0, 0.55, 1.1, 0.04),
        kind::WISP => look(0.75, 0.5, 1.0, 1.5, 0.08),
        kind::EMBER_BOMB => look(1.0, 0.55, 0.15, 1.5, 0.05),
        kind::NEEDLE => look(1.0, 0.35, 0.2, 1.0, 0.07),
        kind::ARROW => look(1.0, 0.8, 0.42, 0.9, 0.05),
        kind::PIN_ARROW => look(0.55, 0.75, 1.0, 0.9, 0.05),
        kind::BLINK_ARROW => look(0.7, 1.0, 0.35, 0.9, 0.05),
        kind::BEACON_ARROW => look(1.0, 0.55, 0.16, 1.1, 0.06),
        kind::BRAND_ARROW => look(1.0, 0.25, 0.45, 1.0, 0.05),
        kind::RIVET => look(1.0, 0.42, 0.18, 0.8, 0.035),
        kind::CINDER_ROCKET => look(1.0, 0.6, 0.25, 1.1, 0.06),
        kind::FLAK_SHELL => look(1.0, 0.42, 0.68, 1.0, 0.04),
        kind::LIMPET => look(0.95, 1.0, 0.3, 0.8, 0.03),
        // The rework's (client-a / client-b give them their own looks): the colours of their
        // abilities.
        kind::KITE => look(0.82, 0.6, 1.0, 1.3, 0.05),
        kind::BOLAS => look(1.0, 0.5, 0.15, 1.0, 0.04),
        kind::LANTERN_TOSS => look(1.0, 0.28, 0.2, 1.2, 0.04),
        // Melee and knacks (melee.rs, knacks.rs draw the pieces): the Hurl in its violet, the
        // flask in kindling gold-green, the snare's amber lamp, the pot's spitting fuse.
        kind::HURL => look(0.78, 0.5, 1.0, 1.1, 0.06),
        kind::KINDLE_FLASK => look(0.85, 1.0, 0.55, 0.9, 0.04),
        kind::SNARE => look(1.0, 0.72, 0.3, 0.55, 0.03),
        kind::SOOT_POT => look(1.0, 0.6, 0.25, 0.7, 0.04),
        _ => look(1.0, 0.85, 0.5, 0.8, 0.04),
    }
}

/// Is a projectile one the local player can parry (an enemy's parryable kind, not yet
/// deflected)?
fn parryable_by_me(e: &EntityState, local: Option<u16>) -> bool {
    let theirs = Some(e.owner) != local;
    theirs
        && e.flags & projectile_flags::PARRIED == 0
        && ProjectileKind::from_net_kind(e.kind).is_some_and(|k| k.parryable())
}

/// Seconds a "now in punch reach" flare takes to build (it pulses while it lasts).
const REACH_FLARE: f32 = 0.08;

/// One frame of a projectile's glow: a halo and a white-hot core (one-frame billboards, so
/// they follow it exactly and hitstop can't stack them), a streak back along its flight (never
/// past where it was first seen), and for a parryable one a ring in the parry colour that
/// flares and tightens once it is in punch reach in front of you.
#[allow(clippy::too_many_arguments)]
fn projectile_glow(
    e: &EntityState,
    pos: Vec3,
    vel: Vec3,
    start: Vec3,
    age: f32,
    f: &EntityFrame,
    time: f64,
    effects: &mut Effects,
    beams: &mut Beams,
) {
    let pal = palette();
    let parried = e.flags & projectile_flags::PARRIED != 0;
    let stuck = e.flags & projectile_flags::STUCK != 0;
    let hostile = e.flags & projectile_flags::ENEMY != 0 && !parried;
    let look = projectile_look(e.kind, parried);
    let color = if parried {
        pal.effect(look.color, pal.deflected, 1.0)
    } else if hostile {
        pal.effect(enemy_projectile_color(e.kind), pal.hostile_projectile, 1.0)
    } else {
        look.color
    };
    let distance = pos.distance(f.camera);
    // Never smaller than a few pixels far away.
    let halo = look.halo.max(distance * 0.022) * if stuck { 0.5 } else { 1.0 };
    let flicker = 0.85 + 0.15 * ((time * 31.0 + e.id.0 as f64 * 0.7).sin() as f32);
    effects.billboard(
        pos,
        halo * flicker,
        (color.truncate() * 0.9).extend(0.55),
        FxTex::Glow,
        0.0,
    );
    let white = color.truncate().lerp(Vec3::ONE, 0.6) * 1.6;
    effects.billboard(pos, halo * 0.36, white.extend(1.0), FxTex::Glow, 0.0);

    // The streak: how far it flew in the last `look.streak` seconds, back along its velocity.
    let speed = vel.length();
    if !stuck && speed > 2.0 {
        let back = -vel / speed;
        let flown = (pos - start).dot(-back).max(0.0);
        // Never reaching back into the camera, and faded when seen end-on (a streak flying
        // straight away from you or at you would otherwise be a disc over the target).
        let mut len = (speed * look.streak).min(flown).min(9.0);
        let to_camera = (f.camera - pos).normalize_or_zero();
        if back.dot(to_camera) > 0.0 {
            len = len.min((distance - 2.0).max(0.0));
        }
        let side = ((1.0 - back.dot(to_camera).abs()) / 0.3).clamp(0.0, 1.0);
        if len > 0.15 && side > 0.02 {
            let tail = pos + back * len;
            let width = 0.05 + 0.08 * look.halo;
            let core = color.truncate().lerp(Vec3::ONE, 0.45) * 2.0;
            beams.frame_beam(tail, pos, width, core.extend(0.9 * side.max(0.35)));
            beams.frame_beam(
                tail,
                pos,
                width * 3.0,
                (color.truncate() * 1.3).extend(0.3 * side),
            );
        }
    }

    // Parryable: a ring in the parry colour; in punch reach in front of you it flares (bigger,
    // brighter, a fast pulse) — "punch now".
    if parryable_by_me(e, f.local_owner) && !stuck {
        let to = pos - f.camera;
        let d = to.length();
        let t = f.tuning;
        let in_front = to.normalize_or_zero().dot(f.view.normalize_or(Vec3::NEG_Z));
        let half_cone = (t.parry_cone_degrees * 0.5).to_radians().cos();
        let reach = (d <= t.parry_range && in_front >= half_cone)
            || (d <= t.parry_close_range && in_front > 0.0);
        // A little early, so the cue leads the window rather than trailing it.
        let soon = d <= t.parry_range + speed * REACH_FLARE && in_front >= half_cone;
        let ring = pal.parry_window;
        let (size, alpha, pulse_hz) = if reach || soon {
            (halo * 1.25, 1.0, 18.0)
        } else {
            (halo * 1.6, 0.75, 4.0)
        };
        let pulse = 0.8 + 0.2 * ((time * pulse_hz).sin() as f32);
        let level = if reach { 2.4 } else { 1.5 };
        effects.billboard(
            pos,
            size * pulse,
            (ring.truncate() * level).extend(alpha),
            FxTex::Ring,
            (time * 3.0) as f32,
        );
        if reach {
            effects.billboard(
                pos,
                halo * 0.8,
                Vec4::new(2.2, 2.2, 2.0, 0.9),
                FxTex::Glow,
                0.0,
            );
        }
    }
    let _ = age;
}

/// Tint of an aim laser: red while tracking, white-hot and pulsing once locked. The
/// colour-blind palettes draw tracking in `aim_laser`; locked stays white-hot in every palette,
/// so tracking vs locked always differs in colour as well as width and pulse.
fn laser_tint(locked: bool, time: f64) -> Tint {
    if locked {
        let pulse = 0.8 + 0.4 * ((time * 30.0).sin() as f32).abs();
        glow_tint(Vec4::new(3.0 * pulse, 2.4 * pulse, 2.2 * pulse, 1.0), None)
    } else {
        let p = palette();
        glow_tint(Vec4::new(2.4, 0.25, 0.15, 0.9), p.model_glow(p.aim_laser))
    }
}

/// Tint for a glowing model: `glow` multiplies its own colours, or `color` (a palette role)
/// replaces them.
fn glow_tint(glow: Vec4, color: Option<Vec4>) -> Tint {
    Tint {
        glow,
        glow_color: color,
        ..Default::default()
    }
}

/// A glowing beam (the `AimLaser` unit beam model stretched from `a` to `b`).
fn draw_beam(
    lib: &ModelLibrary,
    a: Vec3,
    b: Vec3,
    width: f32,
    tint: Tint,
    draws: &mut Vec<DrawItem>,
) {
    let d = b - a;
    let len = d.length();
    if !(len > 1e-3 && a.is_finite() && b.is_finite()) {
        return;
    }
    let rot = Quat::from_rotation_arc(Vec3::NEG_Z, d / len);
    lib.draw_model(
        ModelId::AimLaser,
        Mat4::from_scale_rotation_translation(Vec3::new(width, width, len), rot, a),
        &tint,
        draws,
    );
}

/// Where and how a thorn is drawn: along its flight while flying, tumbling while dropping,
/// tip-first in the surface while planted, flat on the floor while lying.
fn thorn_transform(e: &EntityState, f: &EntityFrame, time: f64) -> (Quat, Vec3) {
    let pos = replicated_position(e, f.render_tick, f.tick_rate, f.tuning);
    let facing = e.velocity.normalize_or(Vec3::NEG_Z);
    if e.flags & thorn_flags::LYING != 0 {
        let flat = Vec3::new(facing.x, 0.0, facing.z).normalize_or(Vec3::X);
        // Lying on its side, a little off the floor.
        return (
            Quat::from_rotation_arc(Vec3::NEG_Z, flat),
            pos + Vec3::Y * 0.04,
        );
    }
    if e.flags & thorn_flags::PLANTED != 0 {
        // The model's tip is THORN_TIP ahead of its origin: drawn at the planted position the
        // tip sinks into the surface. Quiver for a moment after planting.
        let since = ((f.render_tick - e.aux as f64) / f.tick_rate.max(1) as f64).max(0.0) as f32;
        let quiver = (-since * 6.0).exp() * (since * 70.0).sin() * 0.12;
        let side = facing.any_orthonormal_vector();
        let dir = (facing + side * quiver).normalize_or(facing);
        return (
            Quat::from_rotation_arc(Vec3::NEG_Z, dir),
            pos - facing * (THORN_TIP * 0.2),
        );
    }
    let g = f.tuning.thorn.gravity;
    let dt = ((f.render_tick - e.aux as f64) / f.tick_rate.max(1) as f64).max(0.0) as f32;
    let vel = e.velocity - Vec3::Y * g * dt;
    let along = Quat::from_rotation_arc(Vec3::NEG_Z, vel.normalize_or(Vec3::NEG_Z));
    if e.flags & thorn_flags::DROPPING != 0 {
        return (along * Quat::from_rotation_x((time * 14.0) as f32), pos);
    }
    (along * Quat::from_rotation_z((time * 9.0) as f32), pos)
}

/// One puff of a live ember burst: a few glowing embers (the palette's heal colour, so the
/// heal reads the same in every colour-vision mode) lifting off a faint warm glow, over a
/// wisp of grey smoke.
fn ember_puff(effects: &mut Effects, rng: &mut Rng, center: Vec3, radius: f32, big: bool) {
    let ember = palette().heal;
    let r = radius.max(0.5);
    let p = center + rng.dir() * r * 0.3;
    for _ in 0..if big { 4 } else { 3 } {
        effects.spawn(Particle {
            position: p + rng.dir() * r * 0.15,
            velocity: rng.dir() * 0.7 + Vec3::Y * rng.range(0.5, 1.4),
            gravity: -0.6,
            drag: 1.0,
            max_life: rng.range(0.5, 1.0),
            size: Vec2::splat(rng.range(0.07, 0.13)),
            end_scale: 0.4,
            color: ember,
            end_alpha: 0.0,
            texture: FxTex::Spark,
            additive: true,
            rotation: rng.angle(),
            spin: rng.range(-3.0, 3.0),
            ..Default::default()
        });
    }
    effects.spawn(Particle {
        position: p,
        velocity: Vec3::Y * 0.2,
        max_life: rng.range(0.3, 0.5),
        size: Vec2::splat(r * rng.range(0.16, 0.24)),
        end_scale: 1.4,
        color: ember * Vec4::new(1.0, 1.0, 1.0, if big { 0.3 } else { 0.38 }),
        end_alpha: 0.0,
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    effects.spawn(Particle {
        position: p,
        velocity: rng.dir() * 0.4 + Vec3::Y * 0.5,
        gravity: -0.3,
        drag: 1.0,
        max_life: rng.range(0.6, 1.0),
        size: Vec2::splat(r * rng.range(0.14, 0.22)),
        end_scale: 2.0,
        color: Vec4::new(0.34, 0.31, 0.29, if big { 0.2 } else { 0.26 }),
        end_alpha: 0.0,
        texture: FxTex::Smoke,
        rotation: rng.angle(),
        spin: rng.range(-0.5, 0.5),
        ..Default::default()
    });
}
