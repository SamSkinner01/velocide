//! The expansion arsenal's presentation (`docs/expansion.md`, area **arsenal-client**): which
//! build the local player is holding, and the visuals and sounds of the new projectiles,
//! blasts, hazards and status effects.
//!
//! Everything is keyed on small enums so the replicated kinds and flags map onto them in one
//! place: [`ArsenalProjectile`] (arrows, rivets, cinder rockets, flak shells, limpets, and the
//! rework's kites, bolas and arrows, drawn with the retired abilities' models for now),
//! [`Blast`] (what a new explosion looks like) and [`Status`] (Burn, Pinned). [`ArsenalView`] keeps the per-entity bookkeeping (trails,
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
use shared::loadout::{Ability, Loadout, WeaponKind};
use shared::protocol::{EntityState, NetId, kind};
use shared::sim::{FireIntent, WeaponState};
use shared::tuning::CombatTuning;

use super::super::InGame;
use super::super::effects::{DecalKind, Effects, FxTex, OnHit, Particle};
use super::super::hud::HudWeapons;
use super::super::viewmodel::Viewmodel;
use super::Rng;
use super::entities::enemy_size;
use super::fx::{Beams, FxCtx, shake_at};
use crate::models::{ModelId, ModelLibrary, Tint, ability_color};

// ---------------------------------------------------------------------------------------------
// The held build

impl InGame {
    /// The held weapon and its selected ability (the predicted weapon state, which the server's
    /// `OwnState` re-bases).
    pub(crate) fn held_weapon(&self) -> (WeaponKind, Ability) {
        let state = self.combat.predictor.state();
        (state.current(), state.ability())
    }

    /// The HUD's weapon block: the carried loadout, the selection, each slot's ability cooldown
    /// (as a fraction of the cooldown of the ability that slot has selected), the armed bow
    /// ability and, with a Limpet in hand, the live limpets.
    pub(crate) fn hud_weapons(&self) -> HudWeapons {
        let Some(net) = self.net() else {
            return HudWeapons::default();
        };
        let s = self.combat.predictor.state();
        let t = &net.tuning().combat;
        let rate = net.tick_rate() as f32;
        let cooldown = std::array::from_fn(|i| {
            let Some(b) = s.loadout.slots[i] else {
                return 0.0;
            };
            let mode = if i == s.slot as usize {
                s.mode
            } else {
                self.controls.weapons.modes[i]
            };
            let full = (b.ability(mode).cooldown(t) * rate).max(1.0);
            (s.alt_cooldown[i] as f32 / full).clamp(0.0, 1.0)
        });
        let build = s.build();
        let limpets = build.abilities.contains(&Ability::Limpet).then(|| {
            let (own, _, _) = super::own_projectiles(net);
            let live = own.iter().filter(|e| e.kind == kind::LIMPET).count() as u32;
            (live, t.limpet.max_live)
        });
        HudWeapons {
            loadout: if s.is_unarmed() {
                Loadout::unarmed()
            } else {
                s.loadout
            },
            slot: s.slot,
            mode: s.mode,
            cooldown,
            armed: s.armed_ability(),
            limpets,
        }
    }

    /// Stages an arsenal visual client-side (tools and screenshots; see [`ArsenalPreview`]).
    #[doc(hidden)]
    pub fn preview_arsenal(&mut self, preview: ArsenalPreview) {
        self.combat.arsenal.preview(preview);
    }
}

// ---------------------------------------------------------------------------------------------
// The local player's weapon feel

/// What the local weapon feel remembers between frames.
#[derive(Clone, Copy, Debug, Default)]
struct LocalFeel {
    /// The charge whine playing (Discharge).
    charge_sfx: Option<Sfx>,
    drawing: bool,
    full_draw: bool,
    /// When the Kiln last sprayed / the last Cinderbox salvo started (view time).
    last_kiln: f64,
    last_salvo: f64,
}

/// The Kiln's ember spray from `muzzle` along `aim`: tongues of flame filling the cone and a
/// few sparks falling out of it (local shots and other players').
pub fn kiln_spray(
    effects: &mut Effects,
    rng: &mut Rng,
    muzzle: Vec3,
    aim: Vec3,
    range: f32,
    cone_degrees: f32,
) {
    let aim = aim.normalize_or(Vec3::NEG_Z);
    let (right, up) = shared::sim::weapons::aim_basis(aim);
    let half = (cone_degrees * 0.5).to_radians().tan();
    for i in 0..7 {
        let r = rng.unit().sqrt() * half;
        let a = rng.angle();
        let dir = (aim + right * (r * a.cos()) + up * (r * a.sin())).normalize_or(aim);
        let life = rng.range(0.28, 0.4);
        let spark = i >= 5;
        effects.spawn(Particle {
            position: muzzle + aim * 0.08,
            velocity: dir * range / life * if spark { 0.9 } else { 1.1 },
            gravity: if spark { 6.0 } else { -1.0 },
            drag: 1.2,
            max_life: life,
            size: Vec2::splat(if spark { 0.07 } else { 0.18 }),
            end_scale: if spark { 1.0 } else { 6.0 },
            color: if spark {
                Vec4::new(1.0, 0.75, 0.3, 1.0)
            } else {
                Vec4::new(1.0, rng.range(0.38, 0.62), 0.12, 0.9)
            },
            texture: if spark {
                FxTex::Spark
            } else {
                FxTex::MuzzleFlash
            },
            additive: true,
            rotation: rng.angle(),
            spin: rng.range(-4.0, 4.0),
            collide: spark.then_some(OnHit::Die),
            ..Default::default()
        });
    }
}

impl ArsenalView {
    /// A predicted shot of the local player: the weapon-specific feel on top of the shared
    /// shot handling (sound, kick): the Thresher's drum turns, a Cinderbox salvo starts its
    /// tubes, the Kiln catches (ignite on the first tick), sprays and roars.
    #[allow(clippy::too_many_arguments)]
    pub fn local_shot(
        &mut self,
        intent: &FireIntent,
        viewmodel: &mut Viewmodel,
        effects: &mut Effects,
        rng: &mut Rng,
        audio: &mut Audio,
        muzzle: Vec3,
        aim: Vec3,
        t: &CombatTuning,
    ) {
        let weapon = match intent {
            FireIntent::Primary { weapon, .. } => weapon,
            // A Brand loosed from the bow: the string twangs under the branding hiss.
            FireIntent::Ability {
                weapon: WeaponKind::Longwick,
                ability: Ability::Brand,
                ..
            } => {
                audio.play_2d(Sfx::BowRelease);
                return;
            }
            _ => return,
        };
        match weapon {
            WeaponKind::Thresher => viewmodel.turn_drum(),
            WeaponKind::Cinderbox => {
                if self.time - self.local.last_salvo > 0.3 {
                    viewmodel.salvo();
                    self.local.last_salvo = self.time;
                }
            }
            WeaponKind::Kiln => {
                if self.time - self.local.last_kiln > 0.3 {
                    audio.play_2d(Sfx::KilnIgnite);
                }
                self.local.last_kiln = self.time;
                viewmodel.spray();
                kiln_spray(effects, rng, muzzle, aim, t.kiln.range, t.kiln.cone_degrees);
            }
            _ => {}
        }
    }

    /// Per frame, from the predicted weapon state: the charge glow and whine (Discharge;
    /// a held Tally / Bulwark / Riposte counts its hold in `charge` too, but neither glows nor
    /// whines), the bow's draw (creak while drawing, a ring and a bright glint at full draw,
    /// silence on release). Returns the draw (0..1: the bow's, or the Lamplighter's focus,
    /// which rides the same field and has its own sounds, `tricks::lamplighter`) and whether
    /// the bow reached full draw this frame (the caller zooms and flashes, `docs/feel.md`).
    pub fn local_feel(
        &mut self,
        s: &WeaponState,
        t: &CombatTuning,
        tick_rate: u32,
        viewmodel: &mut Viewmodel,
        audio: &mut Audio,
    ) -> (f32, bool) {
        // Nothing carried (gun game's Fist tier): fists only.
        viewmodel.set_unarmed(s.is_unarmed());
        let charging = s.ability().style() == shared::sim::weapons::AbilityStyle::Charge;
        viewmodel.set_charge(if charging {
            s.charge_fraction(t, tick_rate)
        } else {
            0.0
        });
        let whine = (charging && s.charge > 0).then_some(Sfx::DischargeCharge);
        if whine != self.local.charge_sfx {
            if let Some(old) = self.local.charge_sfx {
                audio.stop(old);
            }
            if let Some(new) = whine {
                audio.play_2d(new);
            }
            self.local.charge_sfx = whine;
        }
        let draw = if s.draw > 0 {
            s.draw_fraction(t, tick_rate)
        } else {
            0.0
        };
        viewmodel.set_draw(draw);
        let drawing = s.draw > 0 && s.current() == WeaponKind::Longwick;
        if drawing && !self.local.drawing {
            audio.play_2d(Sfx::BowDraw);
        } else if !drawing && self.local.drawing {
            audio.stop(Sfx::BowDraw);
            self.local.full_draw = false;
        }
        let mut full_now = false;
        if drawing && draw >= 1.0 && !self.local.full_draw {
            audio.play_2d(Sfx::BowFullDraw);
            audio.play_2d(Sfx::FullDrawGlint);
            self.local.full_draw = true;
            full_now = true;
        }
        self.local.drawing = drawing;
        (draw, full_now)
    }
}

// ---------------------------------------------------------------------------------------------
// Projectiles

/// The arsenal projectile a replicated entity kind is.
pub fn projectile_kind(k: u16) -> Option<ArsenalProjectile> {
    Some(match k {
        kind::ARROW => ArsenalProjectile::Arrow,
        kind::PIN_ARROW => ArsenalProjectile::PinningArrow,
        kind::BLINK_ARROW => ArsenalProjectile::BlinkArrow,
        kind::BEACON_ARROW => ArsenalProjectile::BeaconArrow,
        kind::BRAND_ARROW => ArsenalProjectile::BrandArrow,
        kind::RIVET => ArsenalProjectile::Rivet,
        kind::CINDER_ROCKET => ArsenalProjectile::CinderRocket,
        kind::FLAK_SHELL => ArsenalProjectile::FlakShell,
        kind::LIMPET => ArsenalProjectile::Limpet,
        // The Bolas, the Kite and the tossed Lantern are drawn by `gear.rs` (client-b).
        _ => return None,
    })
}

/// A replicated arsenal projectile as drawn this frame (`position` / `velocity` from its
/// trajectory at the time it is drawn at, `age` from the client's bookkeeping).
pub fn projectile_view(
    e: &EntityState,
    position: Vec3,
    velocity: Vec3,
    age: f32,
    world: Option<&CollisionWorld>,
) -> Option<ProjectileView> {
    let kind = projectile_kind(e.kind)?;
    let stuck = e.flags & shared::protocol::projectile_flags::STUCK != 0;
    let flight = if e.velocity.length_squared() > 1e-6 {
        e.velocity
    } else {
        Vec3::NEG_Y
    };
    // A stuck limpet grips the nearest surface (it may ride on a wall, a ceiling or a body);
    // anything else that stuck keeps its flight direction.
    let facing = if stuck && kind == ArsenalProjectile::Limpet {
        surface_facing(world, position).unwrap_or(flight)
    } else {
        flight
    };
    Some(ProjectileView {
        kind,
        position,
        velocity: if stuck { Vec3::ZERO } else { velocity },
        stuck,
        facing,
        age,
        seed: e.id.0,
    })
}

/// Direction from `at` to the nearest level surface within half a metre (probing the six
/// axes), if there is one.
pub fn surface_facing(world: Option<&CollisionWorld>, at: Vec3) -> Option<Vec3> {
    let w = world?;
    [
        Vec3::NEG_Y,
        Vec3::Y,
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Z,
        Vec3::NEG_Z,
    ]
    .into_iter()
    .filter_map(|d| w.cast_ray(at, d, 0.5, MASK_SHOTS).map(|h| (h.distance, d)))
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, d)| d)
}

/// A projectile of the expansion arsenal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArsenalProjectile {
    /// Longwick taper arrow.
    Arrow,
    PinningArrow,
    /// The Blink Arrow (docs/rework.md): a cold mint flame in a glass bead; it pulses while
    /// you can step to it.
    BlinkArrow,
    /// The Beacon Arrow: a little brass lantern behind its spike, lit where it sticks
    /// (`tricks::beacon`).
    BeaconArrow,
    /// A Brand loosed from the Longwick (the taper arrow, glowing the brand's crimson).
    BrandArrow,
    /// Riveter hot rivet.
    Rivet,
    /// Cinderbox salvo rocket.
    CinderRocket,
    /// Flak shell (airbursts into a ring of pellets).
    FlakShell,
    /// Limpet sticky charge.
    Limpet,
}

impl ArsenalProjectile {
    pub const ALL: [ArsenalProjectile; 9] = [
        ArsenalProjectile::Arrow,
        ArsenalProjectile::PinningArrow,
        ArsenalProjectile::BlinkArrow,
        ArsenalProjectile::BeaconArrow,
        ArsenalProjectile::BrandArrow,
        ArsenalProjectile::Rivet,
        ArsenalProjectile::CinderRocket,
        ArsenalProjectile::FlakShell,
        ArsenalProjectile::Limpet,
    ];

    pub fn model(self) -> ModelId {
        match self {
            ArsenalProjectile::Arrow | ArsenalProjectile::BrandArrow => ModelId::Arrow,
            ArsenalProjectile::PinningArrow => ModelId::PinningArrow,
            ArsenalProjectile::BlinkArrow => ModelId::BlinkArrow,
            ArsenalProjectile::BeaconArrow => ModelId::BeaconArrow,
            ArsenalProjectile::Rivet => ModelId::Rivet,
            ArsenalProjectile::CinderRocket => ModelId::CinderRocket,
            ArsenalProjectile::FlakShell => ModelId::FlakShell,
            ArsenalProjectile::Limpet => ModelId::Limpet,
        }
    }

    /// Drawn scale in flight: at least twice life size, more for the small fast ones, so they
    /// read at 640×360 (`docs/feel.md`, the same as `entities::projectile_scale`).
    pub fn scale(self) -> f32 {
        match self {
            ArsenalProjectile::Rivet => 3.2,
            ArsenalProjectile::Limpet | ArsenalProjectile::FlakShell => 2.4,
            ArsenalProjectile::Arrow
            | ArsenalProjectile::PinningArrow
            | ArsenalProjectile::BlinkArrow
            | ArsenalProjectile::BeaconArrow
            | ArsenalProjectile::BrandArrow => 2.2,
            _ => 2.0,
        }
    }

    /// Drawn scale once stuck (arrows in a wall, a limpet on a body): a little over life size.
    pub fn stuck_scale(self) -> f32 {
        match self {
            ArsenalProjectile::Limpet => 1.8,
            ArsenalProjectile::Rivet => 1.5,
            _ => 1.3,
        }
    }

    /// The light it carries (colour, radius).
    pub fn light(self) -> (Vec3, f32) {
        match self {
            ArsenalProjectile::Arrow => (Vec3::new(1.2, 0.8, 0.4), 2.5),
            ArsenalProjectile::PinningArrow => (Vec3::new(0.6, 0.85, 1.4), 3.0),
            ArsenalProjectile::BrandArrow => (Vec3::new(1.6, 0.35, 0.7), 3.0),
            ArsenalProjectile::BlinkArrow => (Vec3::new(0.7, 1.8, 1.4), 3.5),
            ArsenalProjectile::BeaconArrow => (Vec3::new(1.8, 1.4, 0.5), 4.0),
            ArsenalProjectile::Rivet => (Vec3::new(1.4, 0.5, 0.2), 2.0),
            ArsenalProjectile::CinderRocket => (Vec3::new(2.0, 0.9, 0.3), 4.5),
            ArsenalProjectile::FlakShell => (Vec3::new(1.4, 0.6, 0.9), 3.0),
            ArsenalProjectile::Limpet => (Vec3::new(1.4, 1.3, 0.4), 2.5),
        }
    }

    /// How far a stuck projectile's front sinks into the surface it hit (arrows, rivets).
    fn embed(self) -> f32 {
        match self {
            ArsenalProjectile::Arrow
            | ArsenalProjectile::PinningArrow
            | ArsenalProjectile::BlinkArrow
            | ArsenalProjectile::BeaconArrow
            | ArsenalProjectile::BrandArrow => 0.24,
            ArsenalProjectile::Rivet => 0.06,
            _ => 0.0,
        }
    }

    /// Sound when it hits / lands (blasts have their own, [`Blast::sfx`]).
    pub fn impact_sfx(self) -> Option<Sfx> {
        match self {
            ArsenalProjectile::Arrow
            | ArsenalProjectile::BrandArrow
            | ArsenalProjectile::BlinkArrow
            | ArsenalProjectile::BeaconArrow => Some(Sfx::ArrowImpact),
            ArsenalProjectile::PinningArrow => Some(Sfx::PinThunk),
            ArsenalProjectile::Rivet => Some(Sfx::RivetImpact),
            ArsenalProjectile::Limpet => Some(Sfx::LimpetStick),
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
        (ArsenalProjectile::FlakShell, false) => Quat::from_rotation_z(t * 9.0),
        (ArsenalProjectile::Limpet, false) => {
            Quat::from_rotation_x(t * 8.0) * Quat::from_rotation_y(t * 5.0)
        }
        _ => Quat::IDENTITY,
    };
    let scale = if p.stuck {
        p.kind.stuck_scale()
    } else {
        p.kind.scale()
    };
    let sink = if p.stuck { p.kind.embed() } else { 0.0 };
    // The models' fronts sit ~0.36 m (arrows) / 0.12 m (rivet) ahead of their origin (at
    // life size).
    let front = scale
        * match p.kind {
            ArsenalProjectile::Rivet => 0.12,
            ArsenalProjectile::Arrow
            | ArsenalProjectile::PinningArrow
            | ArsenalProjectile::BlinkArrow
            | ArsenalProjectile::BeaconArrow
            | ArsenalProjectile::BrandArrow => 0.36,
            _ => 0.0,
        };
    let offset = if p.stuck {
        -dir * (front - sink).max(0.0)
    } else {
        Vec3::ZERO
    };
    Mat4::from_scale_rotation_translation(Vec3::splat(scale), look * spin, p.position + offset)
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
        ArsenalProjectile::BrandArrow => {
            let b = super::fx::brand_color();
            tint.glow = (b.truncate() * (0.8 + 0.3 * flicker)).extend(1.0);
        }
        // The bead pulses: you can step to it.
        ArsenalProjectile::BlinkArrow => {
            let pulse = 0.5 + 0.5 * ((time * 7.0 + p.seed as f64).sin() as f32).abs();
            tint.glow = Vec4::splat(1.0 + 1.6 * pulse);
            light.0 *= 0.7 + 0.6 * pulse;
        }
        // Stuck, it burns (`tricks::beacon` adds the beacon's light and column).
        ArsenalProjectile::BeaconArrow if p.stuck => tint.glow = Vec4::splat(2.6 * flicker),
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
            ArsenalProjectile::BlinkArrow if chance(rng, 10.0) => {
                // Cold motes rising off the bead while it waits for you.
                effects.spawn(Particle {
                    position: p.position - p.facing.normalize_or_zero() * 0.3 + rng.dir() * 0.05,
                    velocity: Vec3::Y * 0.5 + rng.dir() * 0.25,
                    max_life: 0.5,
                    size: Vec2::splat(0.06),
                    end_scale: 0.3,
                    color: Vec4::new(0.55, 1.0, 0.85, 1.0),
                    texture: FxTex::Spark,
                    additive: true,
                    ..Default::default()
                });
            }
            ArsenalProjectile::Arrow | ArsenalProjectile::BeaconArrow if chance(rng, 6.0) => {
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
        | ArsenalProjectile::BlinkArrow
        | ArsenalProjectile::BeaconArrow
        | ArsenalProjectile::BrandArrow => {
            // A thin streak of wick smoke, sparks from the flame (the beacon's lantern and the
            // blink bead shed more).
            let ember = matches!(
                p.kind,
                ArsenalProjectile::BeaconArrow | ArsenalProjectile::BlinkArrow
            );
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
                ArsenalProjectile::BrandArrow => super::fx::brand_color(),
                ArsenalProjectile::BlinkArrow => Vec4::new(0.5, 1.0, 0.82, 1.0),
                ArsenalProjectile::BeaconArrow => Vec4::new(1.0, 0.84, 0.36, 1.0),
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
        ArsenalProjectile::Arrow
        | ArsenalProjectile::PinningArrow
        | ArsenalProjectile::BrandArrow => {
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
    /// The ember kite bursting (the biggest): a rose shock ring and scraps of burning paper.
    Kite,
}

impl Blast {
    pub fn sfx(self) -> Sfx {
        match self {
            Blast::Cinder => Sfx::CinderBlast,
            Blast::Limpet => Sfx::LimpetDetonate,
            Blast::Kite => Sfx::MortarBlast,
        }
    }

    /// Fireball colour.
    fn color(self) -> Vec4 {
        match self {
            Blast::Cinder => Vec4::new(1.0, 0.5, 0.18, 1.0),
            Blast::Limpet => Vec4::new(1.0, 0.85, 0.35, 1.0),
            Blast::Kite => Vec4::new(1.0, 0.65, 0.4, 1.0),
        }
    }
}

/// A blast of radius `radius` at `position`: flash, fireball, smoke, sparks, ring, light,
/// scorch, shake and sound. The Kite adds a rose shock ring and burning paper.
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
        Blast::Kite => 16,
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
    e.smoke(position, if kind == Blast::Kite { 12 } else { 5 });
    e.sparks(position, Vec3::Y, if kind == Blast::Kite { 26 } else { 12 });
    e.shockwave(position, r);
    match kind {
        Blast::Kite => super::kite::burst_scraps(ctx, position, r),
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
        if kind == Blast::Kite { 0.5 } else { 0.3 },
    );
    let d = position.distance(ctx.listener);
    let strength = match kind {
        Blast::Kite => 1.0,
        Blast::Cinder => 0.35,
        _ => 0.6,
    };
    ctx.feedback.shake(shake_at(d, r * 4.0, strength));
    ctx.audio.play(kind.sfx(), PlayParams::at(position));
}

/// The Flak shell's airburst for a staged / synthetic burst: a ring of `pellets` rays flung out
/// perpendicular to its flight (`dir`), `range` long (stopped by the level); see [`flak_burst`].
pub fn flak_airburst(ctx: &mut FxCtx, position: Vec3, dir: Vec3, pellets: usize, range: f32) {
    let axis = dir.normalize_or(Vec3::NEG_Z);
    let side = axis.any_orthonormal_vector();
    let up = axis.cross(side);
    let n = pellets.max(1);
    let ends: Vec<Vec3> = (0..n)
        .map(|k| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU + ctx.rng.range(-0.1, 0.1);
            let out = (side * a.cos() + up * a.sin() + axis * 0.15).normalize();
            ctx.world
                .and_then(|w| w.cast_ray(position, out, range, MASK_SHOTS))
                .map_or(position + out * range, |h| h.point)
        })
        .collect();
    flak_burst(ctx, position, &ends);
}

/// A Flak shell bursting at `origin` into pellets that ended at `ends` (`Fx::Burst`): a rose
/// flash, a shock ring in the plane of the pellets, a hot tracer to each end with sparks where
/// it struck, smoke, light, a little shake and `Sfx::FlakBurst`.
pub fn flak_burst(ctx: &mut FxCtx, origin: Vec3, ends: &[Vec3]) {
    let rose = ability_color(Ability::Flak);
    let tracer = Vec4::new(2.0, 1.4, 0.9, 0.9);
    // The ring's axis: the normal of the pellets' fan (they fly out across the shell's path).
    let dirs: Vec<Vec3> = ends
        .iter()
        .map(|e| (*e - origin).normalize_or_zero())
        .collect();
    let mut normal = Vec3::ZERO;
    for (i, a) in dirs.iter().enumerate() {
        let b = dirs[(i + 1) % dirs.len()];
        let n = a.cross(b);
        normal += if n.dot(normal) < 0.0 { -n } else { n };
    }
    let axis = normal.normalize_or(Vec3::Y);
    let mut reach = 0.0f32;
    for (end, dir) in ends.iter().zip(&dirs) {
        ctx.beams.beam(origin, *end, 0.03, tracer, 0.14);
        ctx.effects.sparks(*end - *dir * 0.05, -*dir, 3);
        reach = reach.max(end.distance(origin));
    }
    ctx.effects
        .glow(origin, (rose.truncate() * 2.0).extend(1.0), 1.6, 0.15);
    ctx.effects.smoke(origin, 4);
    ctx.beams.ring(
        origin,
        axis,
        0.2,
        (reach * 0.5).clamp(2.0, 8.0),
        0.06,
        (rose.truncate() * 1.8).extend(0.9),
        0.18,
    );
    ctx.beams.light(origin, Vec3::new(2.2, 1.0, 1.4), 6.0, 0.25);
    let d = origin.distance(ctx.listener);
    ctx.feedback.shake(shake_at(d, 10.0, 0.3));
    ctx.audio.play(Sfx::FlakBurst, PlayParams::at(origin));
}

// ---------------------------------------------------------------------------------------------
// Status effects

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
    Vec4::new(1.0 + 0.25 * f, 0.9 + 0.08 * f, 0.78, 1.0)
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

/// Arsenal visuals that need state from frame to frame: staged previews (projectiles,
/// statuses) and pending blasts.
#[derive(Default)]
pub struct ArsenalView {
    flying: Vec<Flying>,
    statuses: HashMap<(NetId, u8), (Status, f32)>,
    /// Blasts waiting for an `FxCtx` (staged, or a preview projectile that burst).
    blasts: Vec<(Blast, Vec3, f32)>,
    flak: Vec<(Vec3, Vec3)>,
    /// Seconds since the next burn sizzle per burning entity.
    sizzle: HashMap<NetId, f32>,
    /// Replicated arsenal projectiles seen last frame (impacts when one vanishes).
    tracked: HashMap<NetId, Tracked>,
    local: LocalFeel,
    time: f64,
}

/// Where a replicated projectile was last frame.
#[derive(Clone, Copy, Debug)]
struct Tracked {
    kind: Option<ArsenalProjectile>,
    position: Vec3,
    velocity: Vec3,
    seen: bool,
}

/// What `ArsenalView::update` reads from the replicated world this frame.
pub struct ArsenalFrame<'a> {
    /// Interpolated entities (render tick).
    pub entities: &'a [EntityState],
    pub world: Option<&'a CollisionWorld>,
    pub render_tick: f64,
    pub tick_rate: u32,
    pub tuning: &'a CombatTuning,
}

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

    /// Advances the staged projectiles (sticking / bursting where they meet the level), spawns
    /// trails, flames and status visuals.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        frame: &ArsenalFrame,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
        dt: f32,
    ) {
        let (entities, world) = (frame.entities, frame.world);
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
                ArsenalProjectile::FlakShell => self.flak.push((at, vel)),
                _ => projectile_impact(effects, audio, rng, kind, at, normal),
            }
        }
        // Bursting kinds that landed are gone; arrows, rivets and limpets stay.
        self.flying.retain(|f| {
            let gone = matches!(
                f.view.kind,
                ArsenalProjectile::CinderRocket | ArsenalProjectile::FlakShell
            ) && f.view.stuck;
            !gone && f.view.age < 30.0
        });
        self.update_replicated(frame, effects, audio, rng);
        // Statuses: the replicated flags, and staged previews.
        let mut shown: Vec<(NetId, Status)> = entities
            .iter()
            .flat_map(|e| replicated_statuses(e).map(move |s| (e.id, s)))
            .collect();
        for ((id, _), (status, left)) in &mut self.statuses {
            *left -= dt;
            shown.push((*id, *status));
        }
        self.statuses.retain(|_, (_, left)| *left > 0.0);
        for (id, status) in &shown {
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
        self.sizzle
            .retain(|id, _| shown.iter().any(|(e, s)| e == id && *s == Status::Burning));
    }

    /// Replicated arsenal projectiles that vanished without a blast (arrows, rivets) leave
    /// their impact where they were last seen.
    fn update_replicated(
        &mut self,
        frame: &ArsenalFrame,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
    ) {
        for t in self.tracked.values_mut() {
            t.seen = false;
        }
        for e in frame.entities {
            let Some(k) = projectile_kind(e.kind) else {
                continue;
            };
            let position = shared::sim::projectiles::replicated_position(
                e,
                frame.render_tick,
                frame.tick_rate,
                frame.tuning,
            );
            let velocity = if e.flags & shared::protocol::projectile_flags::STUCK != 0 {
                Vec3::ZERO
            } else {
                e.velocity
            };
            self.tracked.insert(
                e.id,
                Tracked {
                    kind: Some(k),
                    position,
                    velocity,
                    seen: true,
                },
            );
        }
        let gone: Vec<Tracked> = self.tracked.values().filter(|t| !t.seen).copied().collect();
        self.tracked.retain(|_, t| t.seen);
        for t in gone {
            let Some(k) = t.kind else { continue };
            if !matches!(
                k,
                ArsenalProjectile::Arrow
                    | ArsenalProjectile::PinningArrow
                    | ArsenalProjectile::BrandArrow
                    | ArsenalProjectile::Rivet
            ) {
                continue; // blasts come as Fx
            }
            let dir = t.velocity.normalize_or_zero();
            let hit = frame
                .world
                .filter(|_| dir != Vec3::ZERO)
                .and_then(|w| w.cast_ray(t.position - dir * 0.5, dir, 2.0, MASK_SHOTS));
            let (at, normal) = hit.map_or((t.position, -dir), |h| (h.point, h.normal));
            projectile_impact(effects, audio, rng, k, at, normal);
        }
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
    }
}

/// The statuses a replicated enemy or player shows (its `BURNING` / `PINNED` flags).
pub fn replicated_statuses(e: &EntityState) -> impl Iterator<Item = Status> + use<> {
    use shared::protocol::{enemy_flags, player_flags};
    // A Bolas's bind pins too: it shows as its cord (gear.rs), not the arrow's shackles.
    let (burning, pinned) = if e.kind == kind::PLAYER {
        (
            e.flags & player_flags::BURNING != 0,
            e.flags & player_flags::PINNED != 0 && e.flags & player_flags::BOUND == 0,
        )
    } else if kind::is_enemy(e.kind) {
        (
            e.flags & enemy_flags::BURNING != 0,
            e.flags & enemy_flags::PINNED != 0 && e.flags & enemy_flags::BOUND == 0,
        )
    } else {
        (false, false)
    };
    [
        burning.then_some(Status::Burning),
        pinned.then_some(Status::Pinned),
    ]
    .into_iter()
    .flatten()
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
        // Every kind has its own model, except the brand arrow (a crimson taper arrow; the
        // blink arrow too, for now).
        let mut models: Vec<ModelId> = ArsenalProjectile::ALL.iter().map(|p| p.model()).collect();
        models.sort_unstable();
        models.dedup();
        assert_eq!(models.len(), ArsenalProjectile::ALL.len() - 1);
        // Every replicated arsenal kind maps onto one.
        for k in [
            kind::ARROW,
            kind::PIN_ARROW,
            kind::BLINK_ARROW,
            kind::BEACON_ARROW,
            kind::BRAND_ARROW,
            kind::RIVET,
            kind::CINDER_ROCKET,
            kind::FLAK_SHELL,
            kind::LIMPET,
        ] {
            assert!(projectile_kind(k).is_some(), "kind {k}");
        }
        assert_eq!(projectile_kind(kind::BOMB), None);
        // The rework's Bolas, Kite and tossed Lantern are the gear's (gear.rs).
        for k in [kind::KITE, kind::BOLAS, kind::LANTERN_TOSS] {
            assert_eq!(projectile_kind(k), None, "kind {k}");
        }
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
        let m = projectile_transform(&arrow, 0.0);
        let origin = m.transform_point3(Vec3::ZERO);
        assert!(origin.z > 0.05 && origin.z < 0.3, "{origin}");
        // Its tip (0.36 m ahead of the model's origin at life size) is `embed` deep.
        let tip = m.transform_point3(Vec3::new(0.0, 0.0, -0.36));
        assert!(
            (tip.z + ArsenalProjectile::Arrow.embed()).abs() < 1e-3,
            "{tip}"
        );
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
            kind: ArsenalProjectile::CinderRocket,
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
        let tuning = CombatTuning::default();
        let frame = ArsenalFrame {
            entities: &[],
            world: None,
            render_tick: 0.0,
            tick_rate: 60,
            tuning: &tuning,
        };
        for _ in 0..40 {
            view.update(
                &frame,
                &mut effects,
                &mut beams,
                &mut audio,
                &mut rng,
                1.0 / 60.0,
            );
        }
        // The rocket timed out and is waiting to blast; the arrow (no level to stick in) is
        // still around.
        assert_eq!(view.blasts.len(), 1);
        assert_eq!(view.flying.len(), 1);
        assert!(effects.particle_count() > 20, "trails");
    }
}
