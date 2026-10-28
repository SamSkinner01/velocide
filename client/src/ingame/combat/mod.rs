//! Combat presentation (the "client presentation contract" of `docs/combat.md`).
//!
//! * [`weapon_feel`]: local weapon prediction with the shared `step_weapons` → instant muzzle
//!   flash, viewmodel kick, punch / throw animations, sounds and tracers for our own shots.
//! * [`fx`]: tracer beams, temporary lights, and playback of the server's cosmetic `Fx` events
//!   (remote shots, explosions, ricochets, heals, enemy telegraphs...) at their render tick.
//! * [`entities`]: enemies, projectiles, ember bursts, thorns and brand sigils.
//! * [`hud`]: hit markers, damage numbers, damage direction, vignette, death screen, thorns,
//!   cooldowns, the Discharge gauge, BRANDED.
//! * [`arsenal`]: the expansion arsenal's projectiles, blasts, hazards and status effects, and
//!   which build the local player holds.
//! * [`stand_in`]: own projectiles drawn from the muzzle on the frame the shot is predicted,
//!   handed over to the server's when they arrive (`docs/feel.md`).
//! * [`melee`] and [`knacks`] (docs/refit.md): slash trails, melee hits and abilities; the Q
//!   slot's knacks (HUD slot, flask and pool, snare, soot cloud, Tallow Skin, Surge).
//!
//! The `impl InGame` block at the bottom is the glue the in-game screen calls each frame:
//! `combat_game_event` (reliable `GameEvent`s), `combat_after_ticks` (predicted shots),
//! `combat_update` (Fx playback, entity bookkeeping, music), `combat_draw`, `combat_hud`.

pub mod arsenal;
/// The rework's Scatter / Rapid / Explosive abilities and the Thorn relays (docs/rework.md).
pub mod bolas;
pub mod bulwark;
pub mod enemy_fx;
pub mod entities;
pub mod fx;
pub mod gear;
pub mod hud;
/// The Kindle Flask drunk: the drink, the haste, its heal ticks (docs/rework.md).
pub mod kindle;
pub mod kite;
pub mod knacks;
pub mod lantern;
pub mod melee;
pub mod relay;
pub mod stand_in;
pub mod stoke;
pub mod weapon_feel;
pub mod wisp;

use engine::audio::{PlayParams, Sfx};
use engine::gfx::UiDrawList;
use engine::physics::MASK_SHOTS;
use engine::render::Renderer;
use glam::{Vec3, Vec4};
use shared::input::look_dir;
use shared::protocol::{EntityState, GameEvent, NetId, kind, player_flags, player_weapon};
use shared::sim::{Attack, Blocked, DamageFlags, FireIntent, HitTarget, KillInfo, Owner};

use crate::models::{Ability, WeaponKind, WeaponLook as _, ability_color};

use super::InGame;
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
    /// Drawn as a jagged electric arc instead of a straight tracer.
    pub jagged: bool,
}

/// `c` boosted for unlit / additive drawing, with alpha `a`.
fn glow(c: Vec4, k: f32, a: f32) -> Vec4 {
    (c.truncate() * k).extend(a)
}

pub fn attack_style(a: Attack) -> AttackStyle {
    let s = |sfx, kick, w, c: Vec4, life, shake| AttackStyle {
        sfx,
        kick,
        beam_width: w,
        beam_color: c,
        beam_life: life,
        shake,
        flash: 0.35 + 0.45 * kick,
        jagged: false,
    };
    let quill = WeaponKind::Quill;
    let bellows = WeaponKind::Bellows;
    match a {
        Attack::Quill => s(
            Sfx::QuillFire,
            0.5,
            0.055,
            glow(quill.primary_color(), 1.8, 0.95),
            0.2,
            0.04,
        ),
        Attack::Brand => s(
            Sfx::BrandFire,
            0.6,
            0.075,
            glow(quill.glow_color(0), 2.0, 0.95),
            0.42,
            0.07,
        ),
        Attack::Splinter => s(
            Sfx::SplinterFire,
            0.75,
            0.08,
            glow(quill.glow_color(1), 1.9, 0.95),
            0.3,
            0.12,
        ),
        Attack::Blast => s(
            Sfx::BellowsBlast,
            1.0,
            0.036,
            glow(bellows.primary_color(), 1.7, 0.8),
            0.13,
            0.22,
        ),
        Attack::Gust => s(Sfx::BellowsGust, 0.8, 0.0, Vec4::ONE, 0.0, 0.16),
        Attack::BothBarrels => s(
            Sfx::BothBarrels,
            1.0,
            0.04,
            glow(bellows.glow_color(1), 1.8, 0.85),
            0.16,
            0.42,
        ),
        Attack::Arc => AttackStyle {
            jagged: true,
            flash: 0.0,
            ..s(
                Sfx::ArcZap,
                0.12,
                0.042,
                crate::ingame::combat::fx::ARC_COLOR,
                0.11,
                0.015,
            )
        },
        Attack::Wisp => s(Sfx::WispLaunch, 0.6, 0.0, Vec4::ONE, 0.0, 0.08),
        Attack::Discharge => s(Sfx::DischargeRelease, 1.0, 0.0, Vec4::ONE, 0.0, 0.45),
        Attack::Tumble => s(Sfx::BombLaunch, 0.9, 0.0, Vec4::ONE, 0.0, 0.14),
        Attack::Breach => s(Sfx::BreachBlast, 1.0, 0.0, Vec4::ONE, 0.0, 0.5),
        Attack::Gravewell => s(Sfx::GravewellLaunch, 1.0, 0.0, Vec4::ONE, 0.0, 0.18),
        Attack::Fist => s(Sfx::Punch, 0.0, 0.0, Vec4::ONE, 0.0, 0.0),
        // The expansion arsenal (arsenal-client): primaries, then abilities.
        Attack::Lancet => s(
            Sfx::LancetFire,
            0.35,
            0.04,
            glow(WeaponKind::Lancet.primary_color(), 1.8, 0.9),
            0.13,
            0.03,
        ),
        Attack::Thresher => s(
            Sfx::ThresherFire,
            0.9,
            0.034,
            glow(WeaponKind::Thresher.primary_color(), 1.7, 0.8),
            0.12,
            0.18,
        ),
        Attack::Riveter => s(Sfx::RiveterFire, 0.3, 0.0, Vec4::ONE, 0.0, 0.02),
        Attack::Cinderbox => s(Sfx::CinderboxFire, 0.6, 0.0, Vec4::ONE, 0.0, 0.1),
        Attack::Longwick => s(Sfx::BowRelease, 1.0, 0.0, Vec4::ONE, 0.0, 0.05),
        Attack::Kiln => AttackStyle {
            flash: 0.0,
            ..s(Sfx::KilnRoar, 0.25, 0.0, Vec4::ONE, 0.0, 0.01)
        },
        Attack::Skewer => s(
            Sfx::SkewerFire,
            0.9,
            0.09,
            glow(ability_color(Ability::Skewer), 2.0, 0.95),
            0.45,
            0.15,
        ),
        Attack::Flurry => s(
            Sfx::FlurryFire,
            0.4,
            0.045,
            glow(ability_color(Ability::Flurry), 1.8, 0.9),
            0.15,
            0.04,
        ),
        Attack::Slug => s(
            Sfx::SlugFire,
            1.0,
            0.1,
            glow(ability_color(Ability::Slug), 1.9, 0.95),
            0.36,
            0.35,
        ),
        Attack::Flak => s(Sfx::FlakFire, 0.8, 0.0, Vec4::ONE, 0.0, 0.15),
        Attack::Volley => s(
            Sfx::VolleyFire,
            1.0,
            0.038,
            glow(ability_color(Ability::Volley), 1.7, 0.85),
            0.14,
            0.3,
        ),
        Attack::Flare => s(Sfx::FlareLaunch, 0.5, 0.0, Vec4::ONE, 0.0, 0.06),
        Attack::Limpet => s(Sfx::LimpetThrow, 0.3, 0.0, Vec4::ONE, 0.0, 0.0),
        Attack::Mortar => s(Sfx::MortarLaunch, 1.0, 0.0, Vec4::ONE, 0.0, 0.3),
        Attack::TripleNock => s(Sfx::TripleNock, 1.0, 0.0, Vec4::ONE, 0.0, 0.06),
        Attack::PinningArrow => s(Sfx::PinningRelease, 1.0, 0.0, Vec4::ONE, 0.0, 0.06),
        Attack::EmberArrow => s(Sfx::BowRelease, 1.0, 0.0, Vec4::ONE, 0.0, 0.06),
        // Melee (melee.rs draws the swings): no flash, no tracer.
        Attack::Trimmer
        | Attack::Tamper
        | Attack::Lunge
        | Attack::Whirl
        | Attack::Hurl
        | Attack::Upswing => {
            let (weapon, kind) = match attack_melee(a) {
                Some(m) => m,
                None => (melee::MeleeWeapon::Trimmer, melee::SwingKind::Slash),
            };
            AttackStyle {
                flash: 0.0,
                ..s(
                    melee::swing_sfx(weapon, kind),
                    0.0,
                    0.0,
                    Vec4::ONE,
                    0.0,
                    0.05,
                )
            }
        }
        _ => s(Sfx::QuillFire, 0.5, 0.0, Vec4::ONE, 0.0, 0.0),
    }
}

/// The melee weapon and move of a melee attack.
fn attack_melee(a: Attack) -> Option<(melee::MeleeWeapon, melee::SwingKind)> {
    melee::weapon_of_attack(a, None).zip(melee::swing_kind(a))
}

/// Range of a hitscan attack's rays (client tracers).
pub(crate) fn hitscan_range(attack: Attack, t: &shared::tuning::CombatTuning) -> f32 {
    match attack {
        Attack::Blast | Attack::BothBarrels => t.bellows.pellet_range,
        Attack::Thresher => t.thresher.pellet_range,
        Attack::Arc => t.arclash.zap_range,
        _ => t.hitscan_range,
    }
}

/// Everything combat keeps on the client.
pub struct CombatClient {
    pub predictor: WeaponPredictor,
    pub beams: Beams,
    pub queue: FxQueue,
    pub entities: EntityView,
    pub hud: CombatHud,
    /// The expansion arsenal's visuals (`arsenal.rs`).
    pub arsenal: arsenal::ArsenalView,
    /// Own projectiles before the server's arrive (`stand_in.rs`).
    pub stand_ins: stand_in::StandIns,
    /// Melee trails, streaks and hurled pieces; the knacks' things (`melee.rs`, `knacks.rs`).
    pub melee: melee::MeleeView,
    pub knacks: knacks::KnackView,
    /// The rework's abilities and the Thorn relays (`gear.rs`).
    pub gear: gear::GearView,
    rng: Rng,
    /// Predicted shots of this frame's ticks (played after the tick loop).
    shots: Vec<PredictedShot>,
    /// Press-frame shots: the next tick's, played on the frame the button changed
    /// (`WeaponPredictor::lookahead`).
    early: Vec<PredictedShot>,
    /// When the local player last parried on their own screen (`InGame::combat_predict_parry`):
    /// the server's `Parried` then only confirms it.
    parry_predicted_at: Option<f64>,
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
            arsenal: arsenal::ArsenalView::new(),
            stand_ins: stand_in::StandIns::new(),
            melee: melee::MeleeView::new(),
            knacks: knacks::KnackView::new(),
            gear: gear::GearView::new(),
            rng: Rng::default(),
            shots: Vec::new(),
            early: Vec::new(),
            parry_predicted_at: None,
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
        self.arsenal.clear();
        self.stand_ins.clear();
        self.early.clear();
        self.melee.clear();
        self.knacks.clear();
        self.gear.clear();
        self.predictor.reset();
        self.killed_by = None;
        self.parry_predicted_at = None;
        self.heat = 0.0;
    }

    /// Records the input `NetClient::tick` just predicted (call right after it).
    pub fn after_tick(&mut self, net: &NetClient) {
        if let Some(shot) = self.predictor.after_tick(net) {
            self.shots.push(shot);
        }
    }

    /// No tick ran this frame: if `next` (the input the next tick will send, built now) fires
    /// something, play it now (press-frame feedback, `WeaponPredictor::lookahead`). Only when
    /// a fire / punch / throw button changed since the last tick.
    pub fn lookahead(&mut self, net: &NetClient, next: &shared::input::PlayerInput) {
        let alive = !net.own_combat().is_some_and(|c| c.combat.dead);
        let t = &net.tuning().combat;
        if let Some(mut shot) = self.predictor.lookahead(next, alive, t, net.tick_rate()) {
            shot.eye = net
                .local_state()
                .map(|s| s.eye_position(&net.tuning().movement));
            self.early.push(shot);
        }
    }

    pub fn music_intensity(&self) -> f32 {
        self.music
    }

    /// Predicted shots of this frame's ticks not played yet (the play of the game records
    /// them: `replay.rs`).
    pub fn pending_shots(&self) -> &[PredictedShot] {
        &self.shots
    }
}

/// Name of whoever caused something, for the kill feed.
/// A player's wick flame colour (their cosmetics) for glows.
fn wick_color(net: &NetClient, player: shared::sim::PlayerId) -> Vec4 {
    let rgb = net
        .players()
        .get(&player)
        .map(|p| p.cosmetics)
        .unwrap_or_default()
        .flame_rgb();
    Vec4::new(rgb[0], rgb[1], rgb[2], 0.9)
}

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

/// "QUILL", or the enemy's own attack ("SWIPE", "SLAM") instead of a generic "ENEMY".
fn attack_name(net: &NetClient, attack: Attack) -> String {
    if let Attack::Enemy {
        kind,
        attack: index,
    } = attack
        && let Some(name) = net
            .world()
            .and_then(|w| w.enemy_defs.by_kind(kind))
            .and_then(|d| d.attacks.get(index as usize).map(|a| a.name.to_uppercase()))
    {
        return name;
    }
    attack.name().to_uppercase()
}

/// "YOU [QUILL] GNAWER 38m HEADSHOT" style kill feed line ("GNAWER [BITE] YOU 3m"), names
/// from the session.
pub(crate) fn kill_line(net: &NetClient, k: &KillInfo) -> String {
    let attack = match k.weapon {
        Some(weapon) => weapon_attack_label(weapon, k.attack),
        None => attack_name(net, k.attack),
    };
    kill_line_text(
        &owner_name(net, k.killer, k.attack),
        &attack,
        &victim_name(net, k),
        k,
    )
}

/// The bracket of a weapon's kill: its name for the primary ("RIVETER"), weapon and ability
/// for an ability (several weapons share one: "RIVETER VOLLEY", "LONGWICK BRAND").
pub fn weapon_attack_label(weapon: shared::loadout::WeaponKind, attack: Attack) -> String {
    if Attack::primary(weapon) == attack {
        weapon.name().to_uppercase()
    } else {
        format!("{} {}", weapon.name(), attack.name()).to_uppercase()
    }
}

/// Whole metres from the killer to the victim for the kill feed, when the killer has a body
/// (a player or a Mould, not the world) and it wasn't a suicide.
pub fn kill_distance(k: &KillInfo) -> Option<u32> {
    let body = matches!(k.killer, Owner::Player(_) | Owner::Enemy(_));
    let suicide = k.killer.as_target() == Some(k.victim);
    (body && !suicide && k.distance.is_finite() && k.distance > 0.0)
        .then(|| (k.distance.round() as u32).max(1))
}

/// The kill feed line: killer, [weapon], victim, the distance, then what made it special.
pub fn kill_line_text(killer: &str, attack: &str, victim: &str, k: &KillInfo) -> String {
    let mut s = format!("{killer} [{attack}] {victim}");
    if let Some(m) = kill_distance(k) {
        s.push_str(&format!(" {m}m"));
    }
    if k.flags.contains(DamageFlags::HEADSHOT) {
        s.push_str(" HEADSHOT");
    }
    if k.parried_projectile {
        s.push_str(" RETURNED");
    }
    if k.ricochets > 0 {
        s.push_str(&format!(" RICOCHET x{}", k.ricochets));
    }
    if k.killer_airborne {
        s.push_str(" AIR");
    }
    s
}

/// How much the bow narrows the view at full draw (tan(fov/2) × 0.9).
const BOW_ZOOM: f32 = 0.1;

/// How long after a parry seen on the punch's frame the server's `Parried` counts as its
/// confirmation (a round trip and the parry delay, generously).
const PARRY_CONFIRM_WINDOW: f64 = 0.9;

/// Everything a parry does for the parrier (`docs/feel.md`): a long hitstop, a white flash, a
/// hard shake, the parry clang over a deep impact, and a white shock ring bursting from the
/// fist.
fn parry_feedback(
    feedback: &mut super::feedback::Feedback,
    audio: &mut engine::audio::Audio,
    effects: &mut super::effects::Effects,
    beams: &mut fx::Beams,
    fist: Vec3,
    view: Vec3,
) {
    feedback.hitstop(0.16);
    feedback.flash(crate::palette::palette().flash_parry, 0.65);
    feedback.shake(0.28);
    audio.play_2d(Sfx::Parry);
    audio.play_2d(Sfx::ParryImpact);
    let pal = crate::palette::palette();
    let white = pal.effect(Vec4::new(2.4, 2.4, 2.2, 0.95), pal.deflected, 2.2);
    beams.ring(fist, view, 0.05, 0.7, 0.02, white, 0.2);
    beams.light(fist, Vec3::new(3.0, 3.0, 2.8), 7.0, 0.2);
    // (No glow sprite at the fist: half a metre from the eye it would white out the screen.)
    effects.sparks(fist + view * 0.3, -view, 10);
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
pub(crate) fn entity_capsule(e: &EntityState) -> Option<(Vec3, Vec3, f32)> {
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
    /// Local player's health, max and hard damage for the HUD. Dead or downed: no health and
    /// no hard damage (the hard-damage counter would read "-100").
    pub(super) fn combat_health(&self) -> (f32, f32, f32) {
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return (100.0, 100.0, 0.0);
        };
        let t = &net.tuning().combat;
        match net.own_combat() {
            Some(c) if c.combat.dead => (0.0, t.max_health, 0.0),
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
                    self.feedback.shield_flash();
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
                    MarkerKind::Head | MarkerKind::Kill => theme.crimson_bright,
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
                        if h.flags.contains(DamageFlags::HEADSHOT)
                            || h.flags.contains(DamageFlags::WEAK_POINT)
                        {
                            common.audio.play_2d(Sfx::WeakPointTick);
                        }
                        self.feedback.hitstop(0.06);
                        self.feedback.shake(0.1);
                    }
                    MarkerKind::Head => {
                        common.audio.play_2d(Sfx::Headshot);
                        common.audio.play_2d(Sfx::WeakPointTick);
                    }
                    _ => common.audio.play(
                        Sfx::Hitmarker,
                        PlayParams::default().with_pitch(if kind == MarkerKind::Armour {
                            0.7
                        } else {
                            1.0
                        }),
                    ),
                }
                // A melee blow lands: the slice / the slam where it struck (melee.rs); the
                // Tamper's lands with a jolt.
                let held = Some(self.combat.predictor.state().current());
                if let Some(weapon) = melee::hit_weapon(h.attack, held) {
                    let dir = (h.point - me).normalize_or(Vec3::NEG_Z);
                    melee::melee_hit(
                        &mut self.effects,
                        &mut self.combat.beams,
                        &mut self.combat.rng,
                        &mut common.audio,
                        weapon,
                        h.point,
                        dir,
                    );
                    if weapon == melee::MeleeWeapon::Tamper {
                        self.feedback.hitstop(0.06);
                        self.feedback.shake(0.18);
                    } else {
                        self.feedback.hitstop(0.025);
                    }
                }
                self.combat.heat = (self.combat.heat + 0.15).min(1.0);
            }
            GameEvent::Killed(k) => {
                let involves_player = matches!(k.killer, Owner::Player(_))
                    || matches!(k.victim, HitTarget::Player(_));
                let mine = k.killer.player() == local && local.is_some();
                let me_dead = k.victim.player() == local && local.is_some();
                // A Shrike's wick gutters out in its own flame colour (cosmetics).
                if let HitTarget::Player(victim) = k.victim {
                    let flame = wick_color(net, victim);
                    self.effects
                        .glow(k.position + Vec3::Y * 0.4, flame, 1.8, 0.6);
                    self.effects.smoke(k.position + Vec3::Y * 0.5, 4);
                }
                if involves_player {
                    let color = if mine || me_dead {
                        theme.crimson_bright
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
                            attack_name(net, k.attack)
                        ),
                    };
                    self.combat.killed_by = Some(by);
                    // "ARENA LOCKED" / "CHECKPOINT" would sit on top of "YOU DIED".
                    self.hud.clear_banner();
                    common.audio.play_2d(Sfx::Death);
                    self.feedback.death_flash();
                }
            }
            GameEvent::Parried(p) => {
                if Some(p.player) == local {
                    // Already shown on the punch's frame if we saw it coming (the server only
                    // confirms); otherwise now.
                    let predicted = self
                        .combat
                        .parry_predicted_at
                        .take()
                        .is_some_and(|at| now - at < PARRY_CONFIRM_WINDOW);
                    if !predicted {
                        let fist = self.parry_fist();
                        let view = self.last_camera.rotation * Vec3::NEG_Z;
                        parry_feedback(
                            &mut self.feedback,
                            &mut common.audio,
                            &mut self.effects,
                            &mut self.combat.beams,
                            fist,
                            view,
                        );
                        self.hud.banner("PARRY", theme.bone, now);
                    }
                    if p.healed > 0.5 {
                        common
                            .audio
                            .play(Sfx::Heal, PlayParams::default().with_volume(0.6));
                    }
                    self.combat.heat = (self.combat.heat + 0.3).min(1.0);
                } else {
                    common.audio.play_at(Sfx::Parry, p.position);
                }
                let pal = crate::palette::palette();
                let glow = pal.effect(Vec4::new(1.0, 1.0, 0.95, 1.0), pal.deflected, 1.0);
                // Small for the parrier: it happened an arm's length from their eye.
                let size = if Some(p.player) == local { 0.3 } else { 2.2 };
                self.effects.glow(p.position, glow, size, 0.2);
                self.effects.sparks(p.position, Vec3::Y, 16);
            }
            GameEvent::Respawned {
                player, position, ..
            } => {
                if Some(player) == local {
                    self.combat.killed_by = None;
                    self.combat.predictor.reset();
                }
                // The wick relit, in the player's flame colour (cosmetics).
                let flame = wick_color(net, player);
                self.effects.glow(position + Vec3::Y, flame, 2.5, 0.4);
            }
            // Style and game-mode events are handled in `on_net_event` (style_hud / mode_hud).
            GameEvent::Style(_) | GameEvent::Mode(_) => {}
            // Arena encounters: banners / sounds in `encounter_hud` (ingame/mod.rs).
            GameEvent::EncounterStarted { .. }
            | GameEvent::WaveStarted { .. }
            | GameEvent::EncounterCleared { .. } => {}
            // Play of the game: `replay.rs` (ingame/mod.rs).
            GameEvent::HighlightMark(_) | GameEvent::PlayOfTheGame(_) => {}
        }
    }

    /// Plays the local player's predicted shots of this frame: the press-frame shot (if the
    /// button changed between ticks) and those of this frame's ticks not played ahead. Runs
    /// after the camera update, so flashes and tracers leave this frame's muzzle.
    pub(super) fn combat_after_ticks(&mut self, common: &mut Common) {
        let InGame {
            session,
            combat,
            effects,
            viewmodel,
            feedback,
            camera,
            last_camera,
            ..
        } = self;
        let early = std::mem::take(&mut combat.early);
        let shots = std::mem::take(&mut combat.shots);
        let Some(net) = session.as_ref().map(Session::client) else {
            return;
        };
        let t = &net.tuning().combat;
        let world = net.world().map(|w| &w.collision.world);
        let cam = *last_camera;
        let vm = viewmodel.last_draw();
        let body = net.local_state().map(|s| s.position + Vec3::Y * 0.9);
        // Flashes ride along with the shooter (a 60 ms flash at 20 m/s would trail 1.2 m).
        let carry = net.local_state().map_or(Vec3::ZERO, |s| s.velocity);
        let (own, own_now, owner) = own_projectiles(net);
        let live_limpets = own.iter().filter(|e| e.kind == kind::LIMPET).count() as u32;
        let mut punched = false;
        let mut slashed = false;
        let feet = net.local_state().map(|s| s.position);
        for shot in early.iter().chain(shots.iter()).filter(|s| !s.played) {
            let input = &shot.input;
            let aim = look_dir(input.yaw, input.pitch);
            let muzzle = vm.map_or(cam.position + aim * 0.6, |d| {
                cam.position + cam.rotation * d.muzzle
            });
            let eye = shot.eye.unwrap_or(cam.position);
            for intent in &shot.intents {
                match intent {
                    FireIntent::Primary { .. } | FireIntent::Ability { .. } => {
                        let Some(attack) = intent.attack() else {
                            continue;
                        };
                        // Melee (melee.rs): the swing, its sound and trail.
                        slashed |= matches!(
                            intent,
                            FireIntent::Primary {
                                weapon: shared::loadout::WeaponKind::Trimmer,
                                ..
                            }
                        );
                        if combat.melee.local_intent(
                            intent,
                            viewmodel,
                            &mut common.audio,
                            feedback,
                            &mut combat.beams,
                            effects,
                            &mut combat.rng,
                            cam.position,
                            feet,
                            input.yaw,
                            input.pitch,
                            t,
                        ) {
                            combat.heat = (combat.heat + 0.05).min(1.0);
                            continue;
                        }
                        let style = attack_style(attack);
                        common.audio.play_2d(style.sfx);
                        // The weight under the shot (feel): a punch scaled by its kick.
                        if style.kick >= 0.3 {
                            common.audio.play(
                                Sfx::ShotPunch,
                                PlayParams::default().with_volume(0.4 + 0.6 * style.kick),
                            );
                        }
                        viewmodel.fire(style.kick);
                        camera.punch(style.kick * style.kick);
                        // A stoked weapon's shot throws embers off the muzzle (stoke.rs).
                        if combat.gear.heat().stoked.is_some()
                            && matches!(intent, FireIntent::Primary { .. })
                        {
                            stoke::stoked_shot(effects, &mut combat.rng, muzzle, aim);
                        }
                        if attack == Attack::Tumble {
                            viewmodel.turn_drum();
                        }
                        combat.arsenal.local_shot(
                            intent,
                            viewmodel,
                            effects,
                            &mut combat.rng,
                            &mut common.audio,
                            muzzle,
                            aim,
                            t,
                        );
                        // Own projectiles leave the muzzle now, not a round trip later.
                        if let Some(owner) = owner {
                            for l in stand_in::launches(
                                intent,
                                aim,
                                eye,
                                t,
                                world,
                                live_limpets,
                                &mut combat.rng,
                            ) {
                                combat.stand_ins.spawn(l, muzzle, owner, own_now, t);
                            }
                        }
                        let charge = match intent {
                            FireIntent::Ability { charge, .. } => *charge,
                            _ => 1.0,
                        };
                        feedback.shake(
                            style.shake
                                * if attack == Attack::Discharge {
                                    0.4 + 0.6 * charge
                                } else {
                                    1.0
                                },
                        );
                        combat.heat = (combat.heat + 0.05).min(1.0);
                        match attack {
                            Attack::Gust => {
                                fx::gust_puff(
                                    effects,
                                    &mut combat.beams,
                                    &mut combat.rng,
                                    muzzle,
                                    aim,
                                );
                                continue;
                            }
                            Attack::Discharge => {
                                let a = &t.arclash;
                                let r = a.discharge_min_radius
                                    + (a.discharge_max_radius - a.discharge_min_radius)
                                        * charge.clamp(0.0, 1.0);
                                let at = body.unwrap_or(cam.position - Vec3::Y * 0.7);
                                fx::discharge_nova(
                                    effects,
                                    &mut combat.beams,
                                    &mut combat.rng,
                                    at,
                                    r,
                                );
                                continue;
                            }
                            Attack::Arc => {
                                effects.glow(muzzle, fx::ARC_COLOR, 0.3, 0.06);
                            }
                            _ => fx::muzzle_flash(
                                effects,
                                &mut combat.beams,
                                &mut combat.rng,
                                muzzle,
                                aim,
                                carry,
                                style.flash,
                            ),
                        }
                        let dirs = hitscan_dirs(intent, input, t, &mut combat.rng);
                        let range = hitscan_range(attack, t);
                        for dir in dirs {
                            let world_hit = world
                                .and_then(|w| w.cast_ray(cam.position, dir, range, MASK_SHOTS));
                            let mut end_t = world_hit.map_or(range, |h| h.distance);
                            let mut hit_entity = false;
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
                            let end = cam.position + dir * end_t;
                            if style.jagged {
                                combat.beams.arc(
                                    muzzle,
                                    end,
                                    style.beam_width,
                                    style.beam_color,
                                    style.beam_life,
                                    &mut combat.rng,
                                );
                            } else {
                                fx::tracer(&mut combat.beams, muzzle, end, &style, true);
                            }
                            if hit_entity {
                                fx::body_impact(effects, end, -dir, 0.4);
                            } else if world_hit.is_some() {
                                fx::world_impact(effects, world, cam.position, end, 6);
                            }
                        }
                    }
                    FireIntent::Punch => {
                        viewmodel.punch();
                        common.audio.play_2d(Sfx::Punch);
                        punched = true;
                    }
                    // The knacks (knacks.rs): a throw, or Tallow Skin / Surge on you.
                    FireIntent::Knack(knack) => {
                        combat
                            .knacks
                            .local_knack(*knack, viewmodel, &mut common.audio, t)
                    }
                }
            }
        }
        // A punch this frame, or its parry window still open (the predicted state).
        self.combat_predict_parry(common, punched);
        // A Trimmer slash deflects what its arc meets, on the swing's frame (melee.rs).
        if slashed {
            self.combat_predict_deflect(common);
        }
    }

    /// The parry as the local player sees it (`docs/feel.md`): during the punch's parry window,
    /// a parryable projectile drawn within the parry reach in front of the camera (or touching
    /// close), or a Mould flashing its parry window within melee parry reach, is parried *now*
    /// — the server tests the same rule at the same (rewound) view time, so its `Parried` only
    /// confirms it a round trip later.
    pub(super) fn combat_predict_parry(&mut self, common: &mut Common, punched: bool) {
        let now = self.time;
        if self
            .combat
            .parry_predicted_at
            .is_some_and(|at| now - at < PARRY_CONFIRM_WINDOW)
        {
            return;
        }
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        if !punched && self.combat.predictor.state().parry == 0 {
            return;
        }
        let t = &net.tuning().combat;
        let eye = self.last_camera.position;
        let view = self.last_camera.rotation * Vec3::NEG_Z;
        let local = net.local_player_id().map(|p| p.0 as u16);
        let render_tick = net.render_tick();
        let half_cone = (t.parry_cone_degrees * 0.5).to_radians().cos();
        let hit = net.remote_entities().iter().find_map(|e| {
            if kind::is_projectile(e.kind) {
                let pk = shared::sim::ProjectileKind::from_net_kind(e.kind)?;
                let flags = e.flags;
                if !pk.parryable()
                    || Some(e.owner) == local
                    || flags
                        & (shared::protocol::projectile_flags::STUCK
                            | shared::protocol::projectile_flags::PARRIED)
                        != 0
                {
                    return None;
                }
                let p = shared::sim::projectiles::replicated_position(
                    e,
                    render_tick,
                    net.tick_rate(),
                    t,
                );
                let to = p - eye;
                let d = to.length();
                let front = to.normalize_or_zero().dot(view);
                ((d <= t.parry_range && front >= half_cone)
                    || (d <= t.parry_close_range && front > 0.0))
                    .then_some(p)
            } else if kind::is_enemy(e.kind)
                && e.flags & shared::protocol::enemy_flags::PARRYABLE != 0
            {
                let (r, h) = enemy_size(e.kind);
                let center = e.position + Vec3::Y * (h * 0.5);
                (center.distance(eye) <= t.melee_parry_range + r + h * 0.5).then_some(center)
            } else {
                None
            }
        });
        let Some(at) = hit else {
            return;
        };
        self.combat.parry_predicted_at = Some(now);
        let fist = self.parry_fist();
        parry_feedback(
            &mut self.feedback,
            &mut common.audio,
            &mut self.effects,
            &mut self.combat.beams,
            fist,
            view,
        );
        // Sparks where it was struck (no glow: right in front of the eye it would hide the
        // deflected projectile flying off, which should be what you see next).
        self.effects.sparks(at, -view, 18);
        self.hud.banner("PARRY", common.theme.bone, now);
    }

    /// Where the punching fist is drawn (world space), else just in front of the camera.
    fn parry_fist(&self) -> Vec3 {
        let cam = &self.last_camera;
        self.viewmodel.last_draw().and_then(|d| d.fist).map_or(
            cam.position + cam.rotation * Vec3::new(-0.05, -0.05, -0.45),
            |f| cam.position + cam.rotation * f,
        )
    }

    /// Per frame (visual time): Fx playback, entity bookkeeping, remote player combat
    /// visuals (Discharge charge, brands), viewmodel charge, music intensity.
    pub(super) fn combat_update(&mut self, common: &mut Common, vdt: f32, real_dt: f32) {
        let InGame {
            session,
            combat,
            effects,
            viewmodel,
            feedback,
            remote,
            camera: rig,
            last_camera,
            potg,
            ..
        } = self;
        let Some(session) = session.as_mut() else {
            return;
        };
        let fx = session.client_mut().drain_fx();
        // The play of the game keeps a copy (replay.rs).
        potg.recorder.record_fx(&fx);
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
            let (_, ability, level) = player_weapon(e.state);
            if e.flags & player_flags::CHARGING != 0
                && ability == shared::sim::Ability::Discharge
                && vis.muzzle != Vec3::ZERO
            {
                // Discharge charging: the capacitor crackles brighter.
                let c = level as f32 / 255.0;
                effects.glow(
                    vis.muzzle,
                    Vec4::new(0.55, 0.8, 1.0, 0.5 + 0.5 * c),
                    0.2 + 0.45 * c,
                    0.05,
                );
                if combat.rng.unit() < vdt * (4.0 + 20.0 * c) {
                    let to = vis.muzzle + combat.rng.dir() * (0.3 + 0.6 * c);
                    combat
                        .beams
                        .arc(vis.muzzle, to, 0.025, fx::ARC_COLOR, 0.06, &mut combat.rng);
                }
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

        // Entities first (so thorns' last positions are known for Fx).
        let (own, own_time, local_owner) = own_projectiles(net);
        let camera = last_camera.position;
        let own_offset = viewmodel
            .last_draw()
            .map_or(Vec3::ZERO, |d| last_camera.rotation * d.muzzle);
        // Own projectiles' stand-ins: fly, stop, hand over to the server's (real time: the
        // projectiles they stand in for don't stop for hitstop).
        let bodies: Vec<(Vec3, Vec3, f32)> = entities.iter().filter_map(entity_capsule).collect();
        let rtt = net.stats().rtt_ms / 1000.0;
        combat.stand_ins.update(
            &stand_in::StandInFrame {
                own: &own,
                now: own_time,
                tick_rate: rate,
                tuning: t,
                world: net.world().map(|w| &w.collision.world),
                bodies: &bodies,
                timeout: (rtt * 1.5 + 0.25).clamp(0.3, 1.2),
            },
            real_dt,
        );
        combat.entities.adopt(combat.stand_ins.claims());
        let frame = EntityFrame {
            entities,
            render_tick,
            tick_rate: rate,
            tuning: t,
            own: &own,
            own_time,
            local_owner,
            own_offset,
            stand_ins: Some(&combat.stand_ins),
            camera,
            view: last_camera.rotation * Vec3::NEG_Z,
            world: net.world().map(|w| &w.collision.world),
        };
        combat.entities.update(
            &frame,
            vdt,
            effects,
            &mut common.audio,
            &mut combat.rng,
            &mut combat.beams,
        );
        // Hurled pieces, thrown knacks, pools, clouds, snares (melee.rs, knacks.rs).
        {
            let mut ctx = melee::UpdateCtx {
                effects,
                beams: &mut combat.beams,
                audio: &mut common.audio,
                rng: &mut combat.rng,
                world: net.world().map(|w| &w.collision.world),
                dt: vdt,
            };
            let held = melee::held(combat.predictor.state());
            combat.melee.update_replicated(&frame, held, &mut ctx);
            let mut ctx = knacks::UpdateCtx {
                effects,
                beams: &mut combat.beams,
                audio: &mut common.audio,
                rng: &mut combat.rng,
                world: net.world().map(|w| &w.collision.world),
                dt: vdt,
            };
            combat.knacks.update_replicated(&frame, &mut ctx);
        }

        // Fx due at the render tick.
        let due = combat.queue.due(render_tick, local);
        if !due.is_empty() {
            let view = &combat.entities;
            let thorn_position = |id: NetId| view.thorn_position(id);
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
                thorn_position: &thorn_position,
                rng: &mut combat.rng,
            };
            for ev in &due {
                fx::play(&mut ctx, ev);
            }
            // A sprung snare's jaws stay shut there a moment (knacks.rs).
            for ev in &due {
                if let shared::protocol::Fx::SnareSprung { position } = ev.fx {
                    combat.knacks.snare_sprung(position.to_vec3());
                }
            }
        }
        combat.beams.update(vdt);
        // Brand sigils (drawn this frame only, after the update).
        combat
            .entities
            .brand_sigils(entities, local, listener, &mut combat.beams);
        // The arsenal's visuals (their per-frame shapes go in after the beams' update too).
        let world = net.world().map(|w| &w.collision.world);
        let frame = arsenal::ArsenalFrame {
            entities,
            world,
            render_tick,
            tick_rate: rate,
            tuning: t,
        };
        combat.arsenal.update(
            &frame,
            effects,
            &mut combat.beams,
            &mut common.audio,
            &mut combat.rng,
            vdt,
        );
        if combat.arsenal.has_pending() {
            let view = &combat.entities;
            let thorn_position = |id: NetId| view.thorn_position(id);
            let mut ctx = FxCtx {
                audio: &mut common.audio,
                effects,
                feedback,
                beams: &mut combat.beams,
                remote,
                world,
                entities,
                local,
                listener,
                local_chest,
                thorn_position: &thorn_position,
                rng: &mut combat.rng,
            };
            combat.arsenal.play_pending(&mut ctx);
        }

        // Melee trails and streaks, the knacks' things (melee.rs, knacks.rs).
        let local_body = net.local_state().map(|s| s.position);
        let local_code = local.map(|p| p.0 as u16);
        let bodies = |owner: u16| -> Option<Vec3> {
            if Some(owner) == local_code {
                return local_body;
            }
            entities
                .iter()
                .find(|e| {
                    e.kind == kind::PLAYER && e.id.as_player().map(|p| p.0 as u16) == Some(owner)
                })
                .map(|e| e.position)
        };
        let mut ctx = melee::UpdateCtx {
            effects,
            beams: &mut combat.beams,
            audio: &mut common.audio,
            rng: &mut combat.rng,
            world,
            dt: vdt,
        };
        combat.melee.update(&mut ctx, &bodies);
        let mut ctx = knacks::UpdateCtx {
            effects,
            beams: &mut combat.beams,
            audio: &mut common.audio,
            rng: &mut combat.rng,
            world,
            dt: vdt,
        };
        combat.knacks.update(&mut ctx);

        // Viewmodel: charge glow + whine (Discharge, Skewer), the bow's draw and its sounds;
        // the bow zooms in a little as it is drawn and flashes at full draw (docs/feel.md).
        let (draw, full_now) = combat.arsenal.local_feel(
            combat.predictor.state(),
            t,
            rate,
            viewmodel,
            &mut common.audio,
        );
        let eased = draw * draw * (3.0 - 2.0 * draw);
        rig.set_zoom(1.0 - BOW_ZOOM * eased);
        if full_now {
            feedback.flash(glam::Vec3::new(1.0, 0.86, 0.5), 0.14);
        }

        // Music: nearby alert enemies + recent combat.
        let near = combat.entities.alert_enemies_near(entities, listener, 40.0) as f32;
        combat.heat = (combat.heat - vdt.max(0.0) * 0.12).max(0.0);
        let target = (0.2 + 0.12 * near + combat.heat).clamp(0.2, 1.0);
        let rate_k = if target > combat.music { 1.5 } else { 0.25 };
        combat.music += (target - combat.music) * (1.0 - (-rate_k * vdt.max(0.0)).exp());
        common.audio.set_music_intensity(combat.music);
        // The Kindle Flask's haste, its previews and heal ticks (kindle.rs); the rework's
        // abilities (gear.rs).
        self.combat_kindle(common, vdt);
        self.combat_gear(common, vdt);
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
            stand_ins: Some(&self.combat.stand_ins),
            camera,
            view: self.last_camera.rotation * Vec3::NEG_Z,
            world: net.world().map(|w| &w.collision.world),
        };
        self.combat.entities.draw(
            common.models.as_ref(),
            &frame,
            &mut out.draws,
            &mut out.lights,
        );
        self.combat.beams.draw(&mut out.draws, &mut out.lights);
        self.combat
            .arsenal
            .draw(common.models.as_ref(), &mut out.draws, &mut out.lights);
        let lib = common.models.as_ref();
        self.combat.melee.draw(lib, &mut out.draws, &mut out.lights);
        let held = melee::held(self.combat.predictor.state());
        self.combat
            .melee
            .draw_replicated(lib, &frame, held, &mut out.draws, &mut out.lights);
        self.combat
            .knacks
            .draw_replicated(lib, &frame, &mut out.draws, &mut out.lights);
        self.combat
            .knacks
            .draw(lib, &mut out.draws, &mut out.lights);
        self.combat.gear.draw(
            lib,
            self.last_camera.position,
            self.controls.yaw,
            self.controls.pitch,
            &mut out.draws,
            &mut out.lights,
        );
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
        let own_flags = net
            .local_player_id()
            .and_then(|p| {
                net.frames()
                    .latest()
                    .and_then(|(_, f)| f.get(NetId::player(p)))
            })
            .map_or(0, |e| e.flags);
        let branded = own_flags & player_flags::BRANDED != 0;
        // Tallow Skin on you: the predicted start, held while the server's flag says so.
        let tallow = self
            .combat
            .knacks
            .tallow_strength(own_flags & player_flags::TALLOW != 0);
        // The Q slot: the loadout's knack (none when it holds no charges: a PvP Thorn).
        let knack = {
            let k = weapons.knack();
            let max = k.charges(t);
            let ready = weapons.knack_charges(t);
            (max > 0).then(|| knacks::KnackSlot {
                knack: k,
                charges: ready,
                max_charges: max,
                recharge: if ready < max {
                    weapons.knack_recharge_fraction(t, rate)
                } else {
                    0.0
                },
                active: match k {
                    knacks::Knack::TallowSkin => tallow,
                    // The haste draining (kindle.rs).
                    knacks::Knack::KindleFlask => self.combat.knacks.kindle.shown(),
                    _ => 0.0,
                },
            })
        };
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
            branded,
            burning: own_flags & player_flags::BURNING != 0,
            pinned: own_flags & player_flags::PINNED != 0,
            parry_window: weapons.parry as f32 / (t.parry_window * rate as f32).max(1.0),
            knack,
            spectating: self.spectate.is_active(),
            time: now,
        };
        self.combat.hud.draw(list, &common.theme, &info);
        // Your Tallow Skin's sheen, the Surge's speed lines (knacks.rs).
        let c = engine::ui::Rect::from_min_size(glam::Vec2::ZERO, list.canvas_size);
        if !dead {
            let rush = self.combat.melee.rush();
            self.combat.knacks.hud(list, c, tallow, rush, now);
            self.combat.gear.hud(list, &common.theme, c, now);
        }
    }
}
