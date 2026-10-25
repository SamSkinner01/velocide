//! Combat presentation (the "client presentation contract" of `docs/combat.md`).
//!
//! * [`weapon_feel`]: local weapon prediction with the shared `step_weapons` → instant muzzle
//!   flash, viewmodel kick, punch / throw animations, sounds and tracers for our own shots.
//! * [`fx`]: tracer beams, temporary lights, and playback of the server's cosmetic `Fx` events
//!   (remote shots, explosions, ricochets, heals, enemy telegraphs...) at their render tick.
//! * [`entities`]: enemies, projectiles, blood bursts and prisms.
//! * [`hud`]: hit markers, damage numbers, damage direction, vignette, death screen, prisms,
//!   stickies, cooldowns.
//!
//! The `impl InGame` block at the bottom is the glue the in-game screen calls each frame:
//! `combat_game_event` (reliable `GameEvent`s), `combat_after_ticks` (predicted shots),
//! `combat_update` (Fx playback, entity bookkeeping, music), `combat_draw`, `combat_hud`.

pub mod entities;
pub mod fx;
pub mod hud;
pub mod weapon_feel;

use engine::audio::{PlayParams, Sfx};
use engine::gfx::UiDrawList;
use engine::physics::MASK_SHOTS;
use engine::render::Renderer;
use glam::{Vec3, Vec4};
use shared::input::{Buttons, look_dir};
use shared::protocol::{EntityState, GameEvent, NetId, kind, player_flags};
use shared::sim::{Attack, Blocked, DamageFlags, FireIntent, HitTarget, KillInfo, Owner};

use super::{InGame, effects};
use crate::app::Common;
use crate::frame::FrameBuffers;
use crate::net::{NetClient, Session};
use entities::{EntityFrame, EntityView, enemy_name, enemy_size};
use fx::{Beams, FxCtx, FxQueue};
use hud::{CombatHud, CombatHudInfo, MarkerKind};
use weapon_feel::{PredictedShot, WeaponPredictor, hitscan_dirs};

/// Small deterministic RNG for cosmetic variation.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Default for Rng {
    fn default() -> Self {
        Rng(0x9E37_79B9)
    }
}

impl Rng {
    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// Random angle in `[0, τ)`.
    pub fn angle(&mut self) -> f32 {
        self.unit() * std::f32::consts::TAU
    }

    /// Random unit vector.
    pub fn dir(&mut self) -> Vec3 {
        for _ in 0..8 {
            let v = Vec3::new(
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
            );
            let l = v.length_squared();
            if l > 0.01 && l <= 1.0 {
                return v / l.sqrt();
            }
        }
        Vec3::Y
    }
}

/// How an attack looks and sounds when fired.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackStyle {
    pub sfx: Sfx,
    /// Viewmodel kick 0..1.
    pub kick: f32,
    /// Tracer width (m) and colour (hitscan); 0 = no tracer.
    pub beam_width: f32,
    pub beam_color: Vec4,
    pub beam_life: f32,
    /// Screen shake trauma for the shooter.
    pub shake: f32,
    /// Muzzle flash size (m).
    pub flash: f32,
}

pub fn attack_style(a: Attack) -> AttackStyle {
    let s = |sfx, kick, w, c: Vec4, life, shake| AttackStyle {
        sfx,
        kick,
        beam_width: w,
        beam_color: c,
        beam_life: life,
        shake,
        flash: 0.25 + 0.3 * kick,
    };
    let teal = Vec4::new(0.6, 1.6, 1.7, 0.9);
    let amber = Vec4::new(1.6, 1.1, 0.5, 0.75);
    match a {
        Attack::Lancet => s(Sfx::LancetFire, 0.55, 0.035, teal, 0.09, 0.04),
        Attack::Overcharge => s(
            Sfx::LancetBeam,
            1.0,
            0.16,
            Vec4::new(1.4, 1.0, 2.0, 0.95),
            0.3,
            0.35,
        ),
        Attack::Fan => s(
            Sfx::LancetFan,
            0.7,
            0.03,
            Vec4::new(0.9, 0.7, 1.8, 0.9),
            0.1,
            0.08,
        ),
        Attack::Pellet => s(Sfx::ThresherFire, 1.0, 0.022, amber, 0.07, 0.22),
        Attack::Slug => s(
            Sfx::ThresherSlug,
            1.0,
            0.07,
            Vec4::new(1.8, 0.7, 0.3, 0.95),
            0.18,
            0.32,
        ),
        Attack::Flak => s(Sfx::FlakLaunch, 0.8, 0.0, Vec4::ONE, 0.0, 0.1),
        Attack::Rivet => s(Sfx::RiveterFire, 0.3, 0.0, Vec4::ONE, 0.0, 0.0),
        Attack::Sawblade => s(Sfx::SawLaunch, 0.7, 0.0, Vec4::ONE, 0.0, 0.08),
        Attack::Volley => s(Sfx::VolleyFire, 0.9, 0.0, Vec4::ONE, 0.0, 0.15),
        Attack::Rocket => s(Sfx::CinderFire, 1.0, 0.0, Vec4::ONE, 0.0, 0.14),
        Attack::Sticky => s(Sfx::StickyArm, 0.5, 0.0, Vec4::ONE, 0.0, 0.04),
        Attack::Mortar => s(Sfx::MortarFire, 1.0, 0.0, Vec4::ONE, 0.0, 0.2),
        Attack::Fist => s(Sfx::Punch, 0.0, 0.0, Vec4::ONE, 0.0, 0.0),
        _ => s(Sfx::LancetFire, 0.5, 0.0, Vec4::ONE, 0.0, 0.0),
    }
}

/// Everything combat keeps on the client.
pub struct CombatClient {
    pub predictor: WeaponPredictor,
    pub beams: Beams,
    pub queue: FxQueue,
    pub entities: EntityView,
    pub hud: CombatHud,
    rng: Rng,
    /// Predicted shots of this frame's ticks (played after the tick loop).
    shots: Vec<PredictedShot>,
    charging: bool,
    /// Recent combat intensity for the music (decays).
    heat: f32,
    /// Smoothed music intensity (the style rank can raise it: `mode_hud::mode_music_intensity`).
    pub(crate) music: f32,
    killed_by: Option<String>,
}

impl CombatClient {
    pub fn new(renderer: &mut Renderer) -> Self {
        Self::with_beams(Beams::new(renderer))
    }

    /// Without GPU resources (tests).
    pub fn headless() -> Self {
        Self::with_beams(Beams::headless())
    }

    fn with_beams(beams: Beams) -> Self {
        Self {
            predictor: WeaponPredictor::new(),
            beams,
            queue: FxQueue::default(),
            entities: EntityView::new(),
            hud: CombatHud::new(),
            rng: Rng::default(),
            shots: Vec::new(),
            charging: false,
            heat: 0.0,
            music: 0.2,
            killed_by: None,
        }
    }

    pub fn free(&mut self, renderer: &mut Renderer) {
        self.beams.free(renderer);
    }

    /// New level / restart: drop every transient visual.
    pub fn clear(&mut self) {
        self.beams.clear();
        self.queue.clear();
        self.entities.clear();
        self.hud.clear();
        self.predictor.reset();
        self.killed_by = None;
        self.heat = 0.0;
    }

    /// Records the input `NetClient::tick` just predicted (call right after it).
    pub fn after_tick(&mut self, net: &NetClient) {
        if let Some(shot) = self.predictor.after_tick(net) {
            self.shots.push(shot);
        }
    }

    pub fn music_intensity(&self) -> f32 {
        self.music
    }
}

/// Name of whoever caused something, for the kill feed.
fn owner_name(net: &NetClient, owner: Owner, attack: Attack) -> String {
    match owner {
        Owner::Player(id) if Some(id) == net.local_player_id() => "YOU".into(),
        Owner::Player(id) => net.player_name(id).unwrap_or("?").to_uppercase(),
        Owner::Enemy(_) => match attack {
            Attack::Enemy { kind, .. } => enemy_name(kind).into(),
            _ => "ENEMY".into(),
        },
        Owner::World if attack == Attack::World => "THE PIT".into(),
        Owner::World => "SERVER".into(),
    }
}

fn victim_name(net: &NetClient, k: &KillInfo) -> String {
    match k.victim {
        HitTarget::Player(id) if Some(id) == net.local_player_id() => "YOU".into(),
        HitTarget::Player(id) => net.player_name(id).unwrap_or("?").to_uppercase(),
        HitTarget::Entity(_) => enemy_name(k.victim_kind).into(),
    }
}

/// "YOU [LANCET] HUSK  HEADSHOT" style kill feed line.
fn kill_line(net: &NetClient, k: &KillInfo) -> String {
    let mut s = format!(
        "{} [{}] {}",
        owner_name(net, k.killer, k.attack),
        k.attack.name().to_uppercase(),
        victim_name(net, k)
    );
    if k.flags.contains(DamageFlags::HEADSHOT) {
        s.push_str("  HEADSHOT");
    }
    if k.parried_projectile {
        s.push_str("  RETURNED");
    }
    if k.ricochets > 0 {
        s.push_str(&format!("  RICOCHET x{}", k.ricochets));
    }
    if k.killer_airborne {
        s.push_str("  AIR");
    }
    s
}

/// The local player's projectiles from the newest snapshot, the (present) time to draw them
/// at, and the local owner code.
fn own_projectiles(net: &NetClient) -> (Vec<EntityState>, f64, Option<u16>) {
    let Some(me) = net.local_player_id().map(|p| p.0 as u16) else {
        return (Vec::new(), 0.0, None);
    };
    let Some((latest, frame)) = net.frames().latest() else {
        return (Vec::new(), 0.0, Some(me));
    };
    let own = frame
        .states()
        .filter(|e| kind::is_projectile(e.kind) && e.owner == me)
        .collect();
    // Present time, but never far past what the server has confirmed (impacts we haven't
    // heard of yet).
    let now = net
        .server_tick_estimate()
        .unwrap_or(latest as f64)
        .min(latest as f64 + 6.0)
        .max(net.render_tick());
    (own, now, Some(me))
}

/// Approximate displayed hit capsules for client-side tracer ends: (entity, a, b, radius).
fn entity_capsule(e: &EntityState) -> Option<(Vec3, Vec3, f32)> {
    if e.kind == kind::PLAYER {
        if e.flags & player_flags::DEAD != 0 {
            return None;
        }
        let h = if e.flags & player_flags::LOW_PROFILE != 0 {
            0.9
        } else {
            1.8
        };
        return Some((
            e.position + Vec3::Y * 0.4,
            e.position + Vec3::Y * (h - 0.4),
            0.4,
        ));
    }
    if kind::is_enemy(e.kind) {
        if e.flags & shared::protocol::enemy_flags::DYING != 0 {
            return None;
        }
        let (r, h) = enemy_size(e.kind);
        return Some((
            e.position + Vec3::Y * r,
            e.position + Vec3::Y * (h - r).max(r),
            r,
        ));
    }
    None
}

impl InGame {
    /// Local player's health, max and hard damage for the HUD.
    pub(super) fn combat_health(&self) -> (f32, f32, f32) {
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return (100.0, 100.0, 0.0);
        };
        let t = &net.tuning().combat;
        match net.own_combat() {
            Some(c) => (c.combat.health(t), t.max_health, c.combat.hard),
            None => (t.max_health, t.max_health, 0.0),
        }
    }

    /// Reliable combat events.
    pub(super) fn combat_game_event(&mut self, ev: GameEvent, common: &mut Common) {
        let now = self.time;
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        let theme = common.theme.clone();
        let local = net.local_player_id();
        let me = net
            .local_state()
            .map_or(Vec3::ZERO, |s| s.position + Vec3::Y);
        let max = net.tuning().combat.max_health;
        match ev {
            GameEvent::Damaged(r) => {
                if r.blocked == Some(Blocked::Invulnerable) {
                    self.feedback.flash(Vec3::new(0.7, 0.9, 1.0), 0.12);
                    return;
                }
                if r.amount <= 0.0 {
                    return;
                }
                self.feedback.damage_flash(r.amount / max);
                self.feedback.shake(0.12 + r.amount / 70.0);
                common.audio.play(
                    Sfx::Hurt,
                    PlayParams::default().with_volume((0.5 + r.amount / 40.0).min(1.0)),
                );
                self.combat.heat = (self.combat.heat + 0.25).min(1.0);
                if !r.flags.contains(DamageFlags::SELF) {
                    let source = match r.source {
                        Owner::Player(id) => {
                            net.remote_entity(NetId::player(id)).map(|e| e.position)
                        }
                        Owner::Enemy(id) => net.remote_entity(id).map(|e| e.position),
                        Owner::World => None,
                    };
                    let from = match source {
                        Some(p) => p + Vec3::Y - me,
                        None => -r.direction,
                    };
                    self.combat
                        .hud
                        .damage_from(from, 0.4 + r.amount / 30.0, now);
                }
            }
            GameEvent::HitConfirm(h) => {
                let kind = if h.killed {
                    MarkerKind::Kill
                } else if h.flags.contains(DamageFlags::HEADSHOT)
                    || h.flags.contains(DamageFlags::WEAK_POINT)
                {
                    MarkerKind::Head
                } else if h.flags.contains(DamageFlags::ARMOURED) {
                    MarkerKind::Armour
                } else {
                    MarkerKind::Body
                };
                self.combat.hud.hit_marker(kind, now);
                let color = match kind {
                    MarkerKind::Head | MarkerKind::Kill => theme.blood_bright,
                    MarkerKind::Armour => theme.ash,
                    MarkerKind::Body => theme.bone,
                };
                self.combat.hud.damage_number(
                    h.point,
                    h.damage,
                    color,
                    kind == MarkerKind::Head,
                    now,
                );
                match kind {
                    MarkerKind::Kill => {
                        common.audio.play_2d(Sfx::KillConfirm);
                        self.feedback.hitstop(0.045);
                        self.feedback.shake(0.08);
                    }
                    MarkerKind::Head => common.audio.play_2d(Sfx::Headshot),
                    _ => common.audio.play(
                        Sfx::Hitmarker,
                        PlayParams::default().with_pitch(if kind == MarkerKind::Armour {
                            0.7
                        } else {
                            1.0
                        }),
                    ),
                }
                self.combat.heat = (self.combat.heat + 0.15).min(1.0);
            }
            GameEvent::Killed(k) => {
                let involves_player = matches!(k.killer, Owner::Player(_))
                    || matches!(k.victim, HitTarget::Player(_));
                let mine = k.killer.player() == local && local.is_some();
                let me_dead = k.victim.player() == local && local.is_some();
                if involves_player {
                    let color = if mine || me_dead {
                        theme.blood_bright
                    } else {
                        theme.bone_dim
                    };
                    let line = kill_line(net, &k);
                    self.hud.event(line, color, now);
                }
                if me_dead {
                    let by = match k.killer {
                        Owner::World if k.attack == Attack::World => {
                            "FELL TO YOUR DEATH".to_string()
                        }
                        Owner::World => "STRUCK DOWN".to_string(),
                        Owner::Player(id) if Some(id) == local => {
                            format!("KILLED YOURSELF [{}]", k.attack.name().to_uppercase())
                        }
                        other => format!(
                            "KILLED BY {} [{}]",
                            owner_name(net, other, k.attack),
                            k.attack.name().to_uppercase()
                        ),
                    };
                    self.combat.killed_by = Some(by);
                    common.audio.play_2d(Sfx::Death);
                    self.feedback.flash(Vec3::new(0.6, 0.0, 0.0), 0.5);
                }
            }
            GameEvent::Parried(p) => {
                if Some(p.player) == local {
                    self.feedback.hitstop(0.12);
                    self.feedback.parry_flash();
                    self.feedback.shake(0.15);
                    common.audio.play_2d(Sfx::Parry);
                    if p.healed > 0.5 {
                        common
                            .audio
                            .play(Sfx::Heal, PlayParams::default().with_volume(0.6));
                    }
                    self.hud.banner("PARRY", theme.bone, now);
                    self.combat.heat = (self.combat.heat + 0.3).min(1.0);
                } else {
                    common.audio.play_at(Sfx::Parry, p.position);
                }
                self.effects
                    .glow(p.position, Vec4::new(1.0, 1.0, 0.95, 1.0), 2.2, 0.2);
                self.effects.sparks(p.position, Vec3::Y, 16);
            }
            GameEvent::Respawned {
                player, position, ..
            } => {
                if Some(player) == local {
                    self.combat.killed_by = None;
                    self.combat.predictor.reset();
                }
                self.effects
                    .glow(position + Vec3::Y, Vec4::new(1.0, 0.3, 0.3, 0.9), 2.5, 0.4);
            }
            // Style and game-mode events are handled in `on_net_event` (style_hud / mode_hud).
            GameEvent::Style(_) | GameEvent::Mode(_) => {}
            // Arena encounters: banners / sounds in `encounter_hud` (ingame/mod.rs).
            GameEvent::EncounterStarted { .. }
            | GameEvent::WaveStarted { .. }
            | GameEvent::EncounterCleared { .. } => {}
        }
    }

    /// Plays the local player's predicted shots of this frame's ticks.
    pub(super) fn combat_after_ticks(&mut self, common: &mut Common) {
        let InGame {
            session,
            combat,
            effects,
            viewmodel,
            feedback,
            last_camera,
            ..
        } = self;
        let shots = std::mem::take(&mut combat.shots);
        let Some(net) = session.as_ref().map(Session::client) else {
            return;
        };
        let t = &net.tuning().combat;
        let world = net.world().map(|w| &w.collision.world);
        let cam = *last_camera;
        let vm = viewmodel.last_draw();
        for shot in shots {
            let input = &shot.input;
            let aim = look_dir(input.yaw, input.pitch);
            let muzzle = vm.map_or(cam.position + aim * 0.6, |d| {
                cam.position + cam.rotation * d.muzzle
            });
            for intent in &shot.intents {
                match intent {
                    FireIntent::Primary(_) | FireIntent::Alt { .. } => {
                        let Some(attack) = intent.attack() else {
                            continue;
                        };
                        let style = attack_style(attack);
                        common.audio.play_2d(style.sfx);
                        viewmodel.fire(style.kick);
                        let angle = combat.rng.angle();
                        effects.spawn(effects::Particle {
                            position: muzzle + aim * 0.06,
                            max_life: 0.06,
                            size: glam::Vec2::splat(style.flash),
                            end_scale: 1.3,
                            color: Vec4::new(1.0, 0.85, 0.55, 1.0),
                            texture: effects::FxTex::MuzzleFlash,
                            additive: true,
                            rotation: angle,
                            ..Default::default()
                        });
                        combat
                            .beams
                            .light(muzzle + aim * 0.3, Vec3::new(1.8, 1.2, 0.7), 5.0, 0.05);
                        let charge = match intent {
                            FireIntent::Alt { charge, .. } => *charge,
                            _ => 1.0,
                        };
                        feedback.shake(
                            style.shake
                                * if attack == Attack::Overcharge {
                                    0.4 + 0.6 * charge
                                } else {
                                    1.0
                                },
                        );
                        combat.heat = (combat.heat + 0.05).min(1.0);
                        let dirs = hitscan_dirs(intent, input, t, &mut combat.rng);
                        for dir in dirs {
                            let range = if attack == Attack::Pellet {
                                t.thresher.pellet_range
                            } else {
                                t.hitscan_range
                            };
                            let world_hit = world
                                .and_then(|w| w.cast_ray(cam.position, dir, range, MASK_SHOTS));
                            let mut end_t = world_hit.map_or(range, |h| h.distance);
                            let mut hit_entity = false;
                            if attack != Attack::Overcharge {
                                for e in net.remote_entities() {
                                    if let Some((a, b, r)) = entity_capsule(e) {
                                        let shape = shared::sim::HitShape::capsule(
                                            shared::sim::HitPart::Body,
                                            a,
                                            b,
                                            r,
                                        );
                                        if let Some(d) = shape.ray_hit(cam.position, dir, end_t) {
                                            end_t = d;
                                            hit_entity = true;
                                        }
                                    }
                                }
                            }
                            let end = cam.position + dir * end_t;
                            let width = if attack == Attack::Overcharge {
                                style.beam_width * (0.4 + 0.6 * charge)
                            } else {
                                style.beam_width
                            };
                            // Thin near the gun (it is right in front of the camera), full
                            // width further out.
                            let along = (end - muzzle).normalize_or_zero();
                            let mid = muzzle + along * (end - muzzle).length().min(3.0);
                            combat.beams.beam(
                                muzzle,
                                mid,
                                width.min(0.03),
                                style.beam_color,
                                style.beam_life,
                            );
                            combat
                                .beams
                                .beam(mid, end, width, style.beam_color, style.beam_life);
                            if hit_entity {
                                effects.blood(end, -dir, 0.4);
                            } else if world_hit.is_some() {
                                fx::world_impact(effects, world, cam.position, end, 6);
                            }
                        }
                    }
                    FireIntent::Punch => {
                        viewmodel.punch();
                        common.audio.play_2d(Sfx::Punch);
                    }
                    FireIntent::ThrowPrism => {
                        viewmodel.throw_prism();
                        common.audio.play_2d(Sfx::PrismThrow);
                    }
                    FireIntent::DetonateStickies => {
                        common.audio.play_2d(Sfx::StickyDetonate);
                    }
                }
            }
        }
    }

    /// Per frame (visual time): Fx playback, entity bookkeeping, remote player combat
    /// visuals, viewmodel charge / spin, music intensity.
    pub(super) fn combat_update(&mut self, common: &mut Common, vdt: f32) {
        let InGame {
            session,
            combat,
            effects,
            viewmodel,
            feedback,
            remote,
            last_camera,
            ..
        } = self;
        let Some(session) = session.as_mut() else {
            return;
        };
        let fx = session.client_mut().drain_fx();
        combat.queue.push(fx);
        let net = session.client();
        if !net.is_in_game() {
            return;
        }
        combat.predictor.sync(net);
        let t = &net.tuning().combat;
        let rate = net.tick_rate();
        let render_tick = net.render_tick();
        let entities = net.remote_entities();
        let local = net.local_player_id();
        let listener = last_camera.position;
        let local_chest = net.local_state().map(|s| s.position + Vec3::Y * 1.1);

        // Remote players: dead / charging / protected visuals.
        for e in entities.iter().filter(|e| e.kind == kind::PLAYER) {
            let vis = remote.visual_mut(e.id);
            vis.dead = e.flags & player_flags::DEAD != 0;
            if e.flags & player_flags::CHARGING != 0 && vis.muzzle != Vec3::ZERO {
                let c = (e.state >> 8) as f32 / 255.0;
                effects.glow(
                    vis.muzzle,
                    Vec4::new(0.6, 1.0, 1.0, 0.5 + 0.5 * c),
                    0.15 + 0.35 * c,
                    0.05,
                );
            }
            if e.flags & player_flags::PROTECTED != 0 && combat.rng.unit() < vdt * 8.0 {
                effects.glow(
                    e.position + Vec3::Y * 1.0 + combat.rng.dir() * 0.4,
                    Vec4::new(1.0, 0.85, 0.85, 0.4),
                    0.5,
                    0.2,
                );
            }
        }

        // Entities first (so prisms' last positions are known for Fx).
        let (own, own_time, local_owner) = own_projectiles(net);
        let camera = last_camera.position;
        let own_offset = viewmodel
            .last_draw()
            .map_or(Vec3::ZERO, |d| last_camera.rotation * d.muzzle);
        let frame = EntityFrame {
            entities,
            render_tick,
            tick_rate: rate,
            tuning: t,
            own: &own,
            own_time,
            local_owner,
            own_offset,
            camera,
        };
        combat
            .entities
            .update(&frame, vdt, effects, &mut common.audio, &mut combat.rng);

        // Fx due at the render tick.
        let due = combat.queue.due(render_tick, local);
        if !due.is_empty() {
            let view = &combat.entities;
            let prism_position = |id: NetId| view.prism_position(id);
            let mut ctx = FxCtx {
                audio: &mut common.audio,
                effects,
                feedback,
                beams: &mut combat.beams,
                remote,
                world: net.world().map(|w| &w.collision.world),
                entities,
                local,
                listener,
                local_chest,
                prism_position: &prism_position,
                rng: &mut combat.rng,
            };
            for ev in &due {
                fx::play(&mut ctx, ev);
            }
        }
        combat.beams.update(vdt);

        // Viewmodel: Overcharge glow + charge sound, Riveter spin.
        let charge = combat.predictor.charge(t, rate);
        viewmodel.set_charge(charge);
        let charging = combat.predictor.state().charge > 0;
        if charging && !combat.charging {
            common.audio.play_2d(Sfx::LancetCharge);
        } else if !charging && combat.charging {
            common.audio.stop(Sfx::LancetCharge);
        }
        combat.charging = charging;
        let last_buttons = net
            .prediction()
            .and_then(|p| p.unacked().last().map(|i| i.buttons))
            .unwrap_or(Buttons::NONE);
        viewmodel.spinning =
            combat.predictor.state().weapon == 2 && last_buttons.contains(Buttons::FIRE);

        // Music: nearby alert enemies + recent combat.
        let near = combat.entities.alert_enemies_near(entities, listener, 40.0) as f32;
        combat.heat = (combat.heat - vdt.max(0.0) * 0.12).max(0.0);
        let target = (0.2 + 0.12 * near + combat.heat).clamp(0.2, 1.0);
        let rate_k = if target > combat.music { 1.5 } else { 0.25 };
        combat.music += (target - combat.music) * (1.0 - (-rate_k * vdt.max(0.0)).exp());
        common.audio.set_music_intensity(combat.music);
    }

    /// Combat entities and tracers.
    pub(super) fn combat_draw(&self, common: &Common, out: &mut FrameBuffers) {
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        if !net.is_in_game() {
            return;
        }
        let (own, own_time, local_owner) = own_projectiles(net);
        let camera = self.last_camera.position;
        let own_offset = self
            .viewmodel
            .last_draw()
            .map_or(Vec3::ZERO, |d| self.last_camera.rotation * d.muzzle);
        let frame = EntityFrame {
            entities: net.remote_entities(),
            render_tick: net.render_tick(),
            tick_rate: net.tick_rate(),
            tuning: &net.tuning().combat,
            own: &own,
            own_time,
            local_owner,
            own_offset,
            camera,
        };
        self.combat.entities.draw(
            common.models.as_ref(),
            &frame,
            &mut out.draws,
            &mut out.lights,
        );
        self.combat.beams.draw(&mut out.draws, &mut out.lights);
    }

    /// Combat HUD layer (after `Hud::draw`).
    pub(super) fn combat_hud(&mut self, common: &Common, list: &mut UiDrawList, now: f64) {
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        let t = &net.tuning().combat;
        let rate = net.tick_rate();
        let (health, max, _) = self.combat_health();
        let own = net.own_combat().copied();
        // A co-op downed player gets the mode HUD's "YOU ARE DOWN" panel instead of the death
        // view (mode_hud.rs).
        let dead = own.is_some_and(|c| c.combat.dead) && !self.local_downed();
        let respawn_in = own.map_or(0.0, |c| c.combat.respawn_in as f32 / rate as f32);
        let protected = own.is_some_and(|c| c.combat.protection > 0);
        let weapons = *self.combat.predictor.state();
        let killed_by = self.combat.killed_by.clone();
        let info = CombatHudInfo {
            camera: &self.last_camera,
            yaw: self.controls.yaw,
            health,
            max_health: max,
            dead,
            respawn_in,
            protected,
            weapons: &weapons,
            tuning: t,
            tick_rate: rate,
            killed_by: killed_by.as_deref(),
            time: now,
        };
        self.combat.hud.draw(list, &common.theme, &info);
    }
}
