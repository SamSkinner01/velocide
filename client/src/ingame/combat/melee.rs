//! Melee presentation (docs/refit.md, area **arsenal-client**): what you see and hear of the
//! Trimmer and the Tamper and their abilities, beyond the models themselves
//! (`models::weapons::melee`: the viewmodel and third-person swings).
//!
//! * **Slash trails** ([`MeleeView::swing`]): a crescent of light swept in front of whoever
//!   swings, growing with the blade through the strike — the Trimmer's alternate right to left
//!   and left to right, the Tamper's comes down from overhead to the floor, the Upswing rises.
//!   Local swings start from the camera, other players' from their eyes.
//! * **Hits** ([`melee_hit`]): the Trimmer slices (sparks and wax along the cut), the Tamper
//!   lands (lumps, dust, a shock ring, a jolt).
//! * **Abilities**: the Lunge's streak ([`MeleeView::lunge`], following the body), the Whirl's
//!   ring ([`whirl`]), the Hurl's spinning blade / head ([`draw_hurled`], with
//!   [`hurled_trail`]), the Upswing's burst under what it launches ([`upswing_burst`]).
//!
//! [`MeleePreview`] / `InGame::preview_melee` stage any of it client-side for tools
//! (`combat_shots`' `melee_*.png`).

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, PointLight};
use glam::{Quat, Vec2, Vec3, Vec4};
use shared::loadout::{Ability, WeaponKind};
use shared::protocol::{EntityState, NetId, kind, player_weapon, projectile_flags};
use shared::sim::{Attack, FireIntent, PlayerId};
use shared::tuning::CombatTuning;

use super::super::InGame;
use super::super::effects::{Effects, FxTex, OnHit, Particle};
use super::super::feedback::Feedback;
use super::super::viewmodel::Viewmodel;
use super::Rng;
use super::entities::EntityFrame;
use super::fx::{Beams, FxCtx};
use crate::models::Tint;
use crate::models::weapons::melee::{self as model, duration, impact};
pub use crate::models::weapons::melee::{MeleeWeapon, Swing, SwingKind};
use crate::models::{ModelLibrary, WeaponLook as _, ability_color as weapon_ability_color};

/// The colour of a weapon's primary swing (trails, sparks): `WeaponLook::primary_color`, the
/// Trimmer's honed steel (a warm white), the Tamper's hot iron.
pub fn primary_color(weapon: MeleeWeapon) -> Vec4 {
    model::weapon_kind(weapon).primary_color()
}

/// The colour of a melee move (its trails, rings, streaks): a slash in the weapon's primary
/// colour, an ability in its ability colour (`models::ability_color`: Lunge cyan, Whirl amber,
/// Hurl violet, Riposte lime). (The retired Upswing's rising strike stands in for the
/// Riposte's counter-swing until client-a gives it its own move.)
pub fn ability_color(kind: SwingKind) -> Vec4 {
    match kind {
        SwingKind::Slash => primary_color(MeleeWeapon::Trimmer),
        SwingKind::Lunge => weapon_ability_color(Ability::Lunge),
        SwingKind::Whirl => weapon_ability_color(Ability::Whirl),
        SwingKind::Hurl => weapon_ability_color(Ability::Hurl),
        SwingKind::Upswing | SwingKind::Guard | SwingKind::Riposte => {
            weapon_ability_color(Ability::Riposte)
        }
    }
}

/// The colour of a move made with `weapon`.
pub fn move_color(weapon: MeleeWeapon, kind: SwingKind) -> Vec4 {
    match kind {
        SwingKind::Slash => primary_color(weapon),
        _ => ability_color(kind),
    }
}

/// The melee move an attack is (`None`: not a melee attack).
pub fn swing_kind(attack: Attack) -> Option<SwingKind> {
    Some(match attack {
        Attack::Trimmer | Attack::Tamper => SwingKind::Slash,
        Attack::Lunge => SwingKind::Lunge,
        Attack::Whirl => SwingKind::Whirl,
        Attack::Hurl => SwingKind::Hurl,
        Attack::Riposte => SwingKind::Riposte,
        _ => return None,
    })
}

/// How hard a move shakes its swinger's view.
fn swing_shake(weapon: MeleeWeapon, kind: SwingKind) -> f32 {
    match (weapon, kind) {
        (MeleeWeapon::Trimmer, SwingKind::Slash) => 0.03,
        (MeleeWeapon::Tamper, SwingKind::Slash) => 0.1,
        (_, SwingKind::Hurl) => 0.05,
        _ => 0.09,
    }
}

/// `c` boosted for unlit drawing (beams), alpha `a`.
fn hot(c: Vec4, k: f32, a: f32) -> Vec4 {
    (c.truncate() * k).extend(a)
}

/// The sound a move makes when it starts.
pub fn swing_sfx(weapon: MeleeWeapon, kind: SwingKind) -> Sfx {
    match (weapon, kind) {
        (MeleeWeapon::Trimmer, SwingKind::Slash) => Sfx::TrimmerSlash,
        (MeleeWeapon::Tamper, SwingKind::Slash) => Sfx::TamperSwing,
        (_, SwingKind::Lunge) => Sfx::Lunge,
        (_, SwingKind::Whirl) => Sfx::Whirl,
        (_, SwingKind::Hurl) => Sfx::HurlThrow,
        (_, SwingKind::Upswing) => Sfx::Upswing,
        (_, SwingKind::Guard) => Sfx::RiposteGuard,
        (_, SwingKind::Riposte) => Sfx::RiposteCounter,
    }
}

// ---------------------------------------------------------------------------------------------
// Slash trails

/// Where a trail's arc runs, in the swinger's view frame (x right, y up, looking down -Z): the
/// direction `start` turned about `axis` through `sweep` radians, `radius` from `center`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Arc {
    /// The axis the arc turns about (view frame).
    axis: Vec3,
    /// The arc's start direction (view frame).
    start: Vec3,
    /// Total sweep (radians, about `axis`, right-handed).
    sweep: f32,
    radius: f32,
    /// Offset of the arc's centre from the eye (view frame).
    center: Vec3,
}

/// The arc a move sweeps (the sim's reach is 3.0 m for the Trimmer, 3.4 m for the Tamper; the
/// trail runs a little inside it, where the blade would be).
fn arc(weapon: MeleeWeapon, kind: SwingKind, left: bool) -> Option<Arc> {
    let side = if left { -1.0 } else { 1.0 };
    let flat = |sweep: f32, tilt: f32, radius: f32| Arc {
        // A cut across the front from the right (or the left), its plane tilted so it starts
        // high and finishes low.
        axis: Vec3::new(-tilt * side, 1.0, 0.0).normalize() * side,
        start: Vec3::new(side * 0.8, 0.12, -0.6).normalize(),
        sweep,
        radius,
        center: Vec3::new(0.0, -0.25, 0.0),
    };
    // Overhead: from up over the right shoulder, through the front, down to the floor left of
    // centre. The plane leans 25 degrees and turns about the right shoulder, not the eye, so it
    // is seen as a crescent (a plane through the eye would be seen edge on, as a line).
    let (lean_s, lean_c) = 0.44f32.sin_cos();
    let overhead_axis = Vec3::new(-lean_c, lean_s, 0.0);
    let overhead_start = Quat::from_axis_angle(Vec3::Z, -0.44) * Vec3::new(0.0, 0.95, 0.3);
    let shoulder = Vec3::new(0.45, -0.25, 0.1);
    let vertical = |down: bool, radius: f32| {
        let sweep = 2.6;
        let start = overhead_start.normalize();
        if down {
            Arc {
                axis: overhead_axis,
                start,
                sweep,
                radius,
                center: shoulder,
            }
        } else {
            // The same path upwards: from where the overhead one ends.
            Arc {
                axis: -overhead_axis,
                start: Quat::from_axis_angle(overhead_axis, sweep) * start,
                sweep,
                radius,
                center: shoulder,
            }
        }
    };
    Some(match (weapon, kind) {
        (MeleeWeapon::Trimmer, SwingKind::Slash) => flat(1.75, 0.35, 1.7),
        // The counter: a wide flat cut across the front.
        (_, SwingKind::Riposte) => flat(2.1, 0.2, 2.0),
        (MeleeWeapon::Tamper, SwingKind::Slash) => vertical(true, 2.0),
        (_, SwingKind::Upswing) => vertical(false, 1.8),
        (_, SwingKind::Whirl) => Arc {
            axis: Vec3::Y,
            start: Vec3::new(1.0, 0.0, -0.2).normalize(),
            sweep: std::f32::consts::TAU,
            radius: 2.2,
            center: Vec3::new(0.0, -0.75, 0.0),
        },
        _ => return None,
    })
}

/// When a move's trail sweeps (seconds after the swing starts): from the wind-up's end to
/// the blow.
fn trail_window(weapon: MeleeWeapon, kind: SwingKind) -> (f32, f32) {
    let d = duration(weapon, kind);
    let hit = impact(weapon, kind) * d;
    match kind {
        SwingKind::Whirl => (0.12 * d, hit),
        _ => (hit * 0.3, hit),
    }
}

/// A slash trail being swept.
#[derive(Clone, Copy, Debug)]
struct Trail {
    eye: Vec3,
    /// View rotation at the swing.
    frame: Quat,
    arc: Arc,
    window: (f32, f32),
    /// Fraction of the arc drawn so far.
    drawn: f32,
    age: f32,
    color: Vec4,
    width: f32,
}

impl Trail {
    fn point(&self, f: f32) -> Vec3 {
        let a = self.arc;
        let dir = Quat::from_axis_angle(a.axis, a.sweep * f) * a.start;
        self.eye + self.frame * (a.center + dir * a.radius)
    }
}

/// A Lunge's streak: the body's path while it lunges.
#[derive(Clone, Copy, Debug)]
struct Streak {
    /// Whose body to follow (`None`: a preview that moves itself).
    player: Option<u16>,
    last: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    color: Vec4,
}

/// A hurled piece staged by a preview.
#[derive(Clone, Copy, Debug)]
struct Flying {
    weapon: MeleeWeapon,
    position: Vec3,
    velocity: Vec3,
    age: f32,
}

/// Something staged client-side by `InGame::preview_melee` (tools).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeleePreview {
    /// A move's trail (and sound) from `eye` looking along (`yaw`, `pitch`).
    Swing {
        weapon: MeleeWeapon,
        kind: SwingKind,
        left: bool,
        eye: Vec3,
        yaw: f32,
        pitch: f32,
    },
    /// A Lunge streak from `from` at `velocity` for `seconds`.
    Lunge {
        from: Vec3,
        velocity: Vec3,
        seconds: f32,
    },
    /// The Whirl's ring round `feet`.
    Whirl { feet: Vec3, radius: f32 },
    /// A hurled blade / head flying from `from`.
    Hurl {
        weapon: MeleeWeapon,
        from: Vec3,
        velocity: Vec3,
    },
    /// The Upswing launching something standing at `feet`.
    Upswing { feet: Vec3 },
    /// A melee hit at `at`, the blow travelling along `dir`.
    Hit {
        weapon: MeleeWeapon,
        at: Vec3,
        dir: Vec3,
    },
}

/// Melee visuals that live across frames.
#[derive(Default)]
pub struct MeleeView {
    trails: Vec<Trail>,
    streaks: Vec<Streak>,
    flying: Vec<Flying>,
    /// Previews waiting for the next update (they need the effects).
    staged: Vec<MeleePreview>,
    /// The Trimmer's next local slash is the left blade's.
    pub next_left: bool,
    /// Your Lunge's rush (seconds left, total): the HUD's speed lines.
    rush: (f32, f32),
    /// Replicated hurled pieces drawn last frame.
    hurled: Vec<Hurled>,
    /// Catches shown lately (thrower's owner code, when): a catch shows once, whichever of
    /// `Fx::HurlCaught` and the piece vanishing at the hand comes first.
    catches: Vec<(u16, f64)>,
    /// Your own hurled piece came back to your hand (`take_caught`).
    caught: bool,
    time: f64,
}

/// Most trails alive at once (oldest dropped).
const MAX_TRAILS: usize = 16;

/// A replicated hurled piece as drawn last frame.
#[derive(Clone, Copy, Debug)]
struct Hurled {
    id: NetId,
    weapon: MeleeWeapon,
    at: Vec3,
    velocity: Vec3,
    /// Its thrower's owner code.
    owner: u16,
    /// On its way back (`projectile_flags::RETURNING`).
    returning: bool,
}

/// How long a shown catch stands for the other half of it (the Fx or the vanishing).
const CATCH_MEMORY: f64 = 0.6;

impl MeleeView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self {
            time: self.time,
            ..Self::default()
        };
    }

    /// A swing starts: its trail sweeps from `eye` along the look (`yaw`, `pitch`) in the
    /// move's colour (`color`: the ability's for abilities, the weapon's for slashes).
    #[allow(clippy::too_many_arguments)]
    pub fn swing(
        &mut self,
        weapon: MeleeWeapon,
        kind: SwingKind,
        left: bool,
        eye: Vec3,
        yaw: f32,
        pitch: f32,
        color: Vec4,
    ) {
        let Some(arc) = arc(weapon, kind, left) else {
            return;
        };
        if !(eye.is_finite() && yaw.is_finite() && pitch.is_finite()) {
            return;
        }
        if self.trails.len() >= MAX_TRAILS {
            self.trails.remove(0);
        }
        // The Whirl turns flat round the body whatever the look's pitch.
        let pitch = if kind == SwingKind::Whirl {
            0.0
        } else {
            pitch.clamp(-1.2, 1.2)
        };
        let frame = engine::gfx::Camera::rotation_from_angles(yaw, pitch, 0.0);
        self.trails.push(Trail {
            eye,
            frame,
            arc,
            window: trail_window(weapon, kind),
            drawn: 0.0,
            age: 0.0,
            color,
            width: match (weapon, kind) {
                (MeleeWeapon::Tamper, _) => 0.11,
                (_, SwingKind::Whirl) => 0.1,
                _ => 0.075,
            },
        });
    }

    /// A streak following `player`'s body (the Lunge; the Surge knack), or flying on at
    /// `velocity` (`None`: a preview).
    pub fn streak(&mut self, player: Option<u16>, from: Vec3, velocity: Vec3, color: Vec4) {
        if !from.is_finite() {
            return;
        }
        self.streaks.push(Streak {
            player,
            last: from,
            velocity,
            age: 0.0,
            life: 0.4,
            color,
        });
    }

    fn preview(&mut self, p: MeleePreview, ctx: &mut UpdateCtx) {
        match p {
            MeleePreview::Swing {
                weapon,
                kind,
                left,
                eye,
                yaw,
                pitch,
            } => {
                let color = if kind == SwingKind::Slash {
                    primary_color(weapon)
                } else {
                    ability_color(kind)
                };
                self.swing(weapon, kind, left, eye, yaw, pitch, color);
                ctx.audio.play(swing_sfx(weapon, kind), PlayParams::at(eye));
                if kind == SwingKind::Whirl {
                    whirl(ctx.effects, ctx.beams, ctx.rng, eye - Vec3::Y * 1.5, 3.5);
                }
            }
            MeleePreview::Lunge {
                from,
                velocity,
                seconds,
            } => {
                self.streak(None, from, velocity, ability_color(SwingKind::Lunge));
                if let Some(s) = self.streaks.last_mut() {
                    s.life = seconds.max(0.1);
                }
                ctx.audio.play(Sfx::Lunge, PlayParams::at(from));
            }
            MeleePreview::Whirl { feet, radius } => {
                whirl(ctx.effects, ctx.beams, ctx.rng, feet, radius);
                ctx.audio.play(Sfx::Whirl, PlayParams::at(feet));
            }
            MeleePreview::Hurl {
                weapon,
                from,
                velocity,
            } => {
                self.flying.push(Flying {
                    weapon,
                    position: from,
                    velocity,
                    age: 0.0,
                });
                ctx.audio.play(Sfx::HurlThrow, PlayParams::at(from));
            }
            MeleePreview::Upswing { feet } => {
                upswing_burst(ctx.effects, ctx.beams, ctx.rng, feet);
                ctx.audio.play(Sfx::Upswing, PlayParams::at(feet));
            }
            MeleePreview::Hit { weapon, at, dir } => {
                melee_hit(ctx.effects, ctx.beams, ctx.rng, ctx.audio, weapon, at, dir);
            }
        }
    }

    /// Advances trails (sweeping their newest segments), streaks and previews.
    pub fn update(&mut self, ctx: &mut UpdateCtx, bodies: &dyn Fn(u16) -> Option<Vec3>) {
        let dt = ctx.dt.max(0.0);
        self.time += dt as f64;
        self.rush.0 = (self.rush.0 - dt).max(0.0);
        for p in std::mem::take(&mut self.staged) {
            self.preview(p, ctx);
        }
        for t in &mut self.trails {
            t.age += dt;
            let (a, b) = t.window;
            let target = ((t.age - a) / (b - a).max(1e-3)).clamp(0.0, 1.0);
            // Segments of ~1/12 of the arc; the newest one leads with a white-hot core.
            let steps = ((target - t.drawn) * 12.0).ceil().max(0.0) as usize;
            for i in 0..steps {
                let f0 = t.drawn + (target - t.drawn) * i as f32 / steps as f32;
                let f1 = t.drawn + (target - t.drawn) * (i + 1) as f32 / steps as f32;
                let (p0, p1) = (t.point(f0), t.point(f1));
                // A crescent: thickest through the middle of the arc.
                let mid = 1.0 - (2.0 * (f0 + f1) * 0.5 - 1.0).abs();
                let w = t.width * (0.35 + 0.65 * mid);
                ctx.beams.beam(p0, p1, w, hot(t.color, 1.7, 0.8), 0.16);
                ctx.beams
                    .beam(p0, p1, w * 0.35, Vec4::new(2.4, 2.3, 2.1, 0.9), 0.09);
            }
            if steps > 0 && t.drawn < 1.0 && target >= 1.0 {
                // The blow: a spark burst at the end of the arc.
                let at = t.point(1.0);
                for _ in 0..6 {
                    ctx.effects.spawn(Particle {
                        position: at,
                        velocity: ctx.rng.dir() * ctx.rng.range(1.0, 3.0),
                        gravity: 6.0,
                        max_life: ctx.rng.range(0.15, 0.3),
                        size: Vec2::splat(0.05),
                        color: hot(t.color, 1.4, 1.0),
                        texture: FxTex::Spark,
                        additive: true,
                        ..Default::default()
                    });
                }
            }
            t.drawn = target;
        }
        self.trails.retain(|t| t.age < t.window.1 + 0.3);

        for s in &mut self.streaks {
            s.age += dt;
            let now = match s.player.and_then(bodies) {
                Some(p) => p,
                None => s.last + s.velocity * dt,
            };
            if s.age < s.life && now.distance(s.last) > 0.02 {
                let fade = 1.0 - s.age / s.life;
                for (h, w) in [(0.5, 0.09), (1.1, 0.12), (1.6, 0.07)] {
                    let up = Vec3::Y * h;
                    ctx.beams
                        .beam(s.last + up, now + up, w * fade, hot(s.color, 1.6, 0.7), 0.3);
                }
                let d = (now - s.last).normalize_or_zero();
                ctx.effects.spawn(Particle {
                    position: now + Vec3::Y * ctx.rng.range(0.3, 1.5),
                    velocity: -d * ctx.rng.range(2.0, 5.0),
                    drag: 3.0,
                    max_life: 0.3,
                    size: Vec2::splat(0.2),
                    end_scale: 2.5,
                    color: s.color.truncate().extend(0.6),
                    texture: FxTex::Glow,
                    additive: true,
                    ..Default::default()
                });
            }
            s.last = now;
        }
        self.streaks.retain(|s| s.age < s.life);

        let mut landed = Vec::new();
        for f in &mut self.flying {
            f.age += dt;
            let step = f.velocity * dt;
            let hit = ctx.world.and_then(|w| {
                let len = step.length();
                (len > 1e-5)
                    .then(|| w.cast_ray(f.position, step / len, len, engine::physics::MASK_SHOTS))
                    .flatten()
            });
            if let Some(h) = hit {
                landed.push((f.weapon, h.point, step.normalize_or(Vec3::NEG_Z)));
                f.age = f32::INFINITY;
                continue;
            }
            f.position += step;
            f.velocity.y -= HURL_GRAVITY * dt;
            if f.age > 0.06 {
                hurled_trail(
                    ctx.effects,
                    ctx.beams,
                    ctx.rng,
                    f.weapon,
                    f.position,
                    f.velocity,
                    dt,
                );
            }
        }
        for (weapon, at, dir) in landed {
            hurl_impact(ctx.effects, ctx.audio, ctx.rng, weapon, at, dir);
        }
        self.flying.retain(|f| f.age < 3.0);
    }

    /// Draws staged hurled pieces.
    pub fn draw(
        &self,
        lib: Option<&ModelLibrary>,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        for f in &self.flying {
            if let Some(lib) = lib {
                draw_hurled(lib, f.weapon, f.position, f.velocity, f.age, draws);
            }
            if lights.len() < 48 {
                lights.push(hurled_light(f.weapon, f.position));
            }
        }
    }

    pub fn trail_count(&self) -> usize {
        self.trails.len()
    }

    /// Your Lunge's rush (1 = just started → 0), for the HUD's speed lines.
    pub fn rush(&self) -> f32 {
        if self.rush.0 > 0.0 {
            self.rush.0 / self.rush.1.max(0.01)
        } else {
            0.0
        }
    }

    /// A predicted intent of the local player. A melee one (a slash, a melee ability) starts
    /// the viewmodel's swing (the Trimmer alternating hands), its sound and its trail from the
    /// eye; returns whether it was one. The Hurl's piece is the server's projectile.
    #[allow(clippy::too_many_arguments)]
    pub fn local_intent(
        &mut self,
        intent: &FireIntent,
        viewmodel: &mut Viewmodel,
        audio: &mut Audio,
        feedback: &mut Feedback,
        beams: &mut Beams,
        effects: &mut Effects,
        rng: &mut Rng,
        eye: Vec3,
        feet: Option<Vec3>,
        yaw: f32,
        pitch: f32,
        t: &CombatTuning,
    ) -> bool {
        let Some(weapon) = intent.weapon().and_then(model::of) else {
            return false;
        };
        let Some(kind) = intent.attack().and_then(swing_kind) else {
            return false;
        };
        let left = weapon == MeleeWeapon::Trimmer && kind == SwingKind::Slash && self.next_left;
        if weapon == MeleeWeapon::Trimmer && kind == SwingKind::Slash {
            self.next_left = !self.next_left;
        }
        viewmodel.swing(kind, left);
        audio.play_2d(swing_sfx(weapon, kind));
        feedback.shake(swing_shake(weapon, kind));
        let color = move_color(weapon, kind);
        self.swing(weapon, kind, left, eye, yaw, pitch, color);
        let feet = feet.unwrap_or(eye - Vec3::Y * 1.6);
        match kind {
            // Your own lunge is felt, not watched: speed lines at the screen's edges (others
            // see its streak).
            SwingKind::Lunge => self.rush = (0.4, 0.4),
            SwingKind::Whirl => whirl(effects, beams, rng, feet, t.whirl.radius),
            _ => {}
        }
        true
    }
}

/// Another player's swing (`Fx::Swing`): their Shrike swings (the Trimmer alternating hands:
/// the client alternates per swing it shows), the whoosh at them, the trail swept in front of
/// their eyes (drawn whole: they are seen from outside), the Whirl's ring, the Lunge's streak.
pub fn remote_swing(ctx: &mut FxCtx, player: PlayerId, attack: Attack, hits: u8) {
    let Some(kind) = swing_kind(attack) else {
        return;
    };
    let id = NetId::player(player);
    let Some(e) = ctx.entities.iter().find(|e| e.id == id).copied() else {
        return;
    };
    let weapon = model::of(player_weapon(e.state).0).unwrap_or(match attack {
        Attack::Tamper => MeleeWeapon::Tamper,
        _ => MeleeWeapon::Trimmer,
    });
    let vis = ctx.remote.visual_mut(id);
    let left = weapon == MeleeWeapon::Trimmer && kind == SwingKind::Slash && !vis.swing.left;
    vis.swing = Swing {
        kind,
        progress: 1e-3,
        left,
    };
    let eye = e.position + Vec3::Y * 1.6;
    ctx.audio.play(
        swing_sfx(weapon, kind),
        PlayParams::at(eye).with_volume(0.9),
    );
    let color = move_color(weapon, kind);
    if let Some(end) = whole_trail(ctx.beams, weapon, kind, left, eye, e.yaw, e.pitch, color)
        && hits > 0
    {
        ctx.effects.sparks(end, Vec3::Y, 6 + 2 * hits as usize);
    }
    match kind {
        SwingKind::Whirl => whirl(ctx.effects, ctx.beams, ctx.rng, e.position, 3.5),
        SwingKind::Lunge => {
            // Where the lunge carries them (the replicated body follows a moment later).
            let dir = shared::input::look_dir(e.yaw, 0.0);
            lunge_streak(ctx.beams, e.position, dir * 5.6, color);
        }
        _ => {}
    }
}

/// A move's trail drawn whole at once (others' swings, replays: no sweep to follow), from
/// `eye` looking along (`yaw`, `pitch`). Returns where the blow ends.
#[allow(clippy::too_many_arguments)]
pub fn whole_trail(
    beams: &mut Beams,
    weapon: MeleeWeapon,
    kind: SwingKind,
    left: bool,
    eye: Vec3,
    yaw: f32,
    pitch: f32,
    color: Vec4,
) -> Option<Vec3> {
    let arc = arc(weapon, kind, left)?;
    let pitch = if kind == SwingKind::Whirl {
        0.0
    } else {
        pitch.clamp(-1.2, 1.2)
    };
    let trail = Trail {
        eye,
        frame: engine::gfx::Camera::rotation_from_angles(yaw, pitch, 0.0),
        arc,
        window: (0.0, 1.0),
        drawn: 0.0,
        age: 0.0,
        color,
        width: 0.09,
    };
    for i in 0..12 {
        let (f0, f1) = (i as f32 / 12.0, (i + 1) as f32 / 12.0);
        let mid = 1.0 - (f0 + f1 - 1.0).abs();
        beams.beam(
            trail.point(f0),
            trail.point(f1),
            trail.width * (0.35 + 0.65 * mid),
            hot(color, 1.7, 0.8),
            0.14 + 0.01 * i as f32,
        );
    }
    Some(trail.point(1.0))
}

/// The melee weapon a melee attack was made with: the primaries' own; an ability's carrier
/// (`held`, when it is a melee weapon), else the weapon whose default pair has it.
pub fn weapon_of_attack(attack: Attack, held: Option<WeaponKind>) -> Option<MeleeWeapon> {
    swing_kind(attack)?;
    Some(match attack {
        Attack::Trimmer => MeleeWeapon::Trimmer,
        Attack::Tamper => MeleeWeapon::Tamper,
        _ => match held.and_then(model::of) {
            Some(m) => m,
            None if matches!(attack, Attack::Whirl | Attack::Riposte) => MeleeWeapon::Tamper,
            None => MeleeWeapon::Trimmer,
        },
    })
}

/// The viewpoint's own swing in a replay (first person): the viewmodel swings (the Trimmer
/// alternating), the sound, the trail drawn whole from the eye.
pub fn first_person_swing(
    vm: &mut Viewmodel,
    audio: &mut Audio,
    beams: &mut Beams,
    feedback: &mut Feedback,
    cam: &engine::gfx::Camera,
    attack: Attack,
) {
    let Some(kind) = swing_kind(attack) else {
        return;
    };
    let Some(weapon) = weapon_of_attack(attack, Some(vm.weapon())) else {
        return;
    };
    let left = weapon == MeleeWeapon::Trimmer && kind == SwingKind::Slash && !vm.swing_left();
    vm.swing(kind, left);
    audio.play_2d(swing_sfx(weapon, kind));
    feedback.shake(swing_shake(weapon, kind));
    let fwd = cam.rotation * Vec3::NEG_Z;
    let yaw = (-fwd.x).atan2(-fwd.z);
    let pitch = fwd.y.clamp(-1.0, 1.0).asin();
    whole_trail(
        beams,
        weapon,
        kind,
        left,
        cam.position,
        yaw,
        pitch,
        move_color(weapon, kind),
    );
}

/// A Lunge streak drawn whole from `from` along `travel` (another player's, seen at a
/// distance): three fading lines at knee, chest and head height.
pub fn lunge_streak(beams: &mut Beams, from: Vec3, travel: Vec3, color: Vec4) {
    for (h, w) in [(0.5, 0.08), (1.1, 0.11), (1.6, 0.06)] {
        let up = Vec3::Y * h;
        beams.beam(from + up, from + travel + up, w, hot(color, 1.6, 0.7), 0.35);
    }
}

/// A Lunge reached its target (`Fx::LungeStruck`): a cyan burst and the bite of a hit.
pub fn lunge_struck(ctx: &mut FxCtx, player: PlayerId, at: Vec3) {
    let c = ability_color(SwingKind::Lunge);
    ctx.effects.glow(at, hot(c, 1.2, 0.9), 1.6, 0.18);
    ctx.effects.sparks(at, Vec3::Y, 14);
    ctx.beams
        .ring(at, Vec3::Y, 0.2, 1.4, 0.05, hot(c, 1.6, 0.8), 0.2);
    let weapon = ctx
        .entities
        .iter()
        .find(|e| e.id == NetId::player(player))
        .and_then(|e| model::of(player_weapon(e.state).0))
        .unwrap_or(MeleeWeapon::Trimmer);
    let sfx = match weapon {
        MeleeWeapon::Trimmer => Sfx::TrimmerHit,
        MeleeWeapon::Tamper => Sfx::TamperHit,
    };
    ctx.audio.play(sfx, PlayParams::at(at));
    if Some(player) == ctx.local {
        ctx.feedback.hitstop(0.05);
        ctx.feedback.shake(0.15);
    }
}

/// The melee weapon whose blow an attack is (for hit effects): its own for the primaries,
/// the carrier's (`held`) for the Lunge, Whirl and Upswing. The Hurl's piece makes its own
/// impact ([`hurl_impact`]).
pub fn hit_weapon(attack: Attack, held: Option<WeaponKind>) -> Option<MeleeWeapon> {
    match attack {
        Attack::Trimmer => Some(MeleeWeapon::Trimmer),
        Attack::Tamper => Some(MeleeWeapon::Tamper),
        Attack::Lunge | Attack::Whirl | Attack::Riposte => {
            Some(held.and_then(model::of).unwrap_or(MeleeWeapon::Trimmer))
        }
        _ => None,
    }
}

/// The hurled piece a replicated `HURL` projectile carries (`state` = the carrier's weapon id).
/// `held`: the local melee weapon, for a local stand-in (`stand_in.rs`) that has no `state`.
pub fn hurled_weapon(e: &EntityState, held: Option<MeleeWeapon>) -> MeleeWeapon {
    model::of_id(e.state as u8)
        .or(held)
        .unwrap_or(MeleeWeapon::Trimmer)
}

/// The melee weapon a weapon state holds (`None`: a gun, or nothing).
pub fn held(s: &shared::sim::WeaponState) -> Option<MeleeWeapon> {
    if s.is_unarmed() {
        None
    } else {
        model::of(s.current())
    }
}

/// Whether a replicated entity is a hurled piece.
pub fn is_hurled(e: &EntityState) -> bool {
    e.kind == kind::HURL
}

/// Another player hurled their blade / head (the `Fx::Shot` of a Hurl): the throw.
pub fn remote_hurl(ctx: &mut FxCtx, player: PlayerId, at: Vec3) {
    let vis = ctx.remote.visual_mut(NetId::player(player));
    vis.swing = Swing {
        kind: SwingKind::Hurl,
        progress: 1e-3,
        left: false,
    };
    ctx.audio.play(Sfx::HurlThrow, PlayParams::at(at));
}

impl MeleeView {
    /// The replicated hurled pieces' wakes this frame, and where one vanished (it struck).
    /// (`held`: the local melee weapon, for stand-ins.) The halo, core and streak are the
    /// entity drawer's (`entities::projectile_look`); these are the sparks off its spin.
    pub fn update_replicated(
        &mut self,
        f: &EntityFrame,
        held: Option<MeleeWeapon>,
        ctx: &mut UpdateCtx,
    ) {
        let dt = ctx.dt.max(0.0);
        let mut seen = Vec::new();
        for (e, time, offset) in f.projectiles() {
            if !is_hurled(e) {
                continue;
            }
            let at = shared::sim::projectiles::replicated_position(e, time, f.tick_rate, f.tuning)
                + offset;
            let weapon = hurled_weapon(e, held);
            // Not right in front of the eye (an own throw starts at the camera).
            let age = ((time - e.aux as f64) / f.tick_rate.max(1) as f64) as f32;
            if age > 0.06 {
                hurled_sparks(ctx.effects, ctx.rng, weapon, at, dt);
            }
            // It turned (the far end of its flight, the level, the Tamper's first victim):
            // coming back now.
            let returning = e.flags & projectile_flags::RETURNING != 0;
            let turned = self
                .hurled
                .iter()
                .find(|h| h.id == e.id)
                .is_some_and(|h| returning && !h.returning);
            if turned {
                ctx.audio.play(Sfx::HurlReturn, PlayParams::at(at));
            }
            seen.push(Hurled {
                id: e.id,
                weapon,
                at,
                velocity: e.velocity,
                owner: e.owner,
                returning,
            });
        }
        // Gone since last frame: on its way back near its thrower it was caught (docs/rework.md;
        // unless `Fx::HurlCaught` showed that already); otherwise it struck something where it
        // was last drawn (a returning piece the level stopped).
        let now = self.time;
        self.catches.retain(|c| now - c.1 < CATCH_MEMORY);
        for h in &self.hurled {
            if seen.iter().any(|s| s.id == h.id) {
                continue;
            }
            let own = f.local_owner == Some(h.owner);
            let chest = if own {
                Some(f.camera - Vec3::Y * 0.5)
            } else {
                thrower_chest(f.entities, h.owner)
            };
            let at_hand = h.returning && chest.is_some_and(|c| c.distance(h.at) < CATCH_REACH);
            if at_hand {
                if !self.catches.iter().any(|c| c.0 == h.owner) {
                    hurl_catch(ctx.effects, ctx.audio, ctx.rng, h.at, own);
                    self.caught |= own;
                    self.catches.push((h.owner, now));
                }
            } else {
                hurl_impact(
                    ctx.effects,
                    ctx.audio,
                    ctx.rng,
                    h.weapon,
                    h.at,
                    h.velocity.normalize_or(Vec3::NEG_Z),
                );
            }
        }
        self.hurled = seen;
    }

    /// `Fx::HurlCaught`: `owner`'s Hurl came back to their hand (`own`: yours, at `chest`).
    /// Shown here unless its piece already vanished at the hand (`update_replicated`).
    #[allow(clippy::too_many_arguments)]
    pub fn hurl_caught(
        &mut self,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
        entities: &[EntityState],
        owner: u16,
        own: bool,
        chest: Option<Vec3>,
    ) {
        let now = self.time;
        if self
            .catches
            .iter()
            .any(|c| c.0 == owner && now - c.1 < CATCH_MEMORY)
        {
            return;
        }
        let at = if own {
            chest
        } else {
            thrower_chest(entities, owner)
        };
        let Some(at) = at else {
            return;
        };
        hurl_catch(effects, audio, rng, at, own);
        self.caught |= own;
        self.catches.push((owner, now));
        // Its piece (if still drawn this frame) is in hand now, not striking anything.
        self.hurled.retain(|h| h.owner != owner || !h.returning);
    }

    /// Whether your own hurled piece came back to your hand since the last call (the
    /// viewmodel's catch).
    pub fn take_caught(&mut self) -> bool {
        std::mem::take(&mut self.caught)
    }

    /// Draws the replicated hurled pieces at the time the entity frame shows them.
    pub fn draw_replicated(
        &self,
        lib: Option<&ModelLibrary>,
        f: &EntityFrame,
        held: Option<MeleeWeapon>,
        draws: &mut Vec<DrawItem>,
        lights: &mut Vec<PointLight>,
    ) {
        for (e, time, offset) in f.projectiles() {
            if !is_hurled(e) {
                continue;
            }
            let at = shared::sim::projectiles::replicated_position(e, time, f.tick_rate, f.tuning)
                + offset;
            if at.distance(f.camera) < 0.6 {
                continue;
            }
            let age = ((time - e.aux as f64) / f.tick_rate.max(1) as f64) as f32;
            let weapon = hurled_weapon(e, held);
            if let Some(lib) = lib {
                draw_hurled(lib, weapon, at, e.velocity, age.max(0.0), draws);
            }
            if lights.len() < 48 {
                lights.push(hurled_light(weapon, at));
            }
        }
    }
}

/// What [`MeleeView::update`] needs.
pub struct UpdateCtx<'a> {
    pub effects: &'a mut Effects,
    pub beams: &'a mut Beams,
    pub audio: &'a mut Audio,
    pub rng: &'a mut Rng,
    pub world: Option<&'a engine::physics::CollisionWorld>,
    pub dt: f32,
}

impl InGame {
    /// The Trimmer's deflect as you see it (`docs/feel.md`'s parry path): on the frame of a
    /// slash, a parryable hostile projectile drawn inside the slash's arc and reach in front of
    /// the eye is turned back *now*, with the parry's flash, hitstop and sound; the server's
    /// `Parried` (the same test on the swing's tick) then only confirms it.
    pub(super) fn combat_predict_deflect(&mut self, common: &mut crate::app::Common) {
        let now = self.time;
        if self
            .combat
            .parry_predicted_at
            .is_some_and(|at| now - at < super::PARRY_CONFIRM_WINDOW)
        {
            return;
        }
        let Some(net) = self.session.as_ref().map(crate::net::Session::client) else {
            return;
        };
        let t = &net.tuning().combat;
        let eye = self.last_camera.position;
        let view = self.last_camera.rotation * Vec3::NEG_Z;
        let local = net.local_player_id().map(|p| p.0 as u16);
        let render_tick = net.render_tick();
        let half_arc = (t.trimmer.arc_degrees * 0.5).to_radians().cos();
        let hit = net.remote_entities().iter().find_map(|e| {
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
            let p =
                shared::sim::projectiles::replicated_position(e, render_tick, net.tick_rate(), t);
            let to = p - eye;
            (to.length() <= t.trimmer.reach && to.normalize_or_zero().dot(view) >= half_arc)
                .then_some(p)
        });
        let Some(at) = hit else {
            return;
        };
        self.combat.parry_predicted_at = Some(now);
        let blade = self.viewmodel.last_draw().map_or(eye + view * 0.6, |d| {
            eye + self.last_camera.rotation * d.muzzle
        });
        super::parry_feedback(
            &mut self.feedback,
            &mut common.audio,
            &mut self.effects,
            &mut self.combat.beams,
            blade,
            view,
        );
        self.effects.sparks(at, -view, 18);
        self.hud.banner("DEFLECT", common.theme.bone, now);
    }

    /// Stages a melee visual client-side (tools and screenshots; see [`MeleePreview`]).
    #[doc(hidden)]
    pub fn preview_melee(&mut self, preview: MeleePreview) {
        self.combat.melee.staged.push(preview);
    }
}

// ---------------------------------------------------------------------------------------------
// Hits and abilities

/// A melee blow landing on a target at `at` (travelling along `dir`): the Trimmer slices
/// (sparks and molten wax along the cut, a bright nick), the Tamper lands (lumps, dust, a ring
/// on the ground, a dull flash). Plays the hit sound there.
pub fn melee_hit(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    audio: &mut Audio,
    weapon: MeleeWeapon,
    at: Vec3,
    dir: Vec3,
) {
    let dir = dir.normalize_or(Vec3::NEG_Z);
    match weapon {
        MeleeWeapon::Trimmer => {
            effects.sparks(at, -dir, 10);
            effects.wax_spray(at, dir, 0.6);
            let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);
            beams.beam(
                at - side * 0.35 + Vec3::Y * 0.12,
                at + side * 0.35 - Vec3::Y * 0.12,
                0.05,
                Vec4::new(2.4, 2.2, 1.9, 0.95),
                0.12,
            );
            audio.play(Sfx::TrimmerHit, PlayParams::at(at));
        }
        MeleeWeapon::Tamper => {
            effects.lumps(at, 8);
            effects.sparks(at, Vec3::Y, 8);
            let floor = at - Vec3::Y * at.y.rem_euclid(1.0).min(0.9);
            effects.dust(floor, 16, 2.5);
            beams.ring(
                at,
                dir,
                0.2,
                1.4,
                0.06,
                Vec4::new(1.8, 1.0, 0.45, 0.8),
                0.22,
            );
            beams.light(at, Vec3::new(2.0, 1.1, 0.5), 5.0, 0.15);
            for _ in 0..6 {
                effects.spawn(Particle {
                    position: at,
                    velocity: (rng.dir() + Vec3::Y).normalize_or(Vec3::Y) * rng.range(2.0, 5.0),
                    gravity: 14.0,
                    max_life: rng.range(0.4, 0.8),
                    size: Vec2::splat(rng.range(0.06, 0.12)),
                    color: Vec4::new(0.55, 0.5, 0.45, 1.0),
                    texture: FxTex::Lump,
                    rotation: rng.angle(),
                    spin: rng.range(-8.0, 8.0),
                    collide: Some(OnHit::Bounce(0.3)),
                    ..Default::default()
                });
            }
            audio.play(Sfx::TamperHit, PlayParams::at(at));
        }
    }
}

/// The Whirl: a ring of light rushing out round `feet` to `radius` (the sim's reach), a second
/// one chasing it, and a swirl of sparks and dust.
pub fn whirl(effects: &mut Effects, beams: &mut Beams, rng: &mut Rng, feet: Vec3, radius: f32) {
    let r = radius.max(1.0);
    let c = ability_color(SwingKind::Whirl);
    beams.ring(
        feet + Vec3::Y * 0.9,
        Vec3::Y,
        0.8,
        r,
        0.09,
        hot(c, 1.8, 0.9),
        0.3,
    );
    beams.ring(
        feet + Vec3::Y * 0.5,
        Vec3::Y,
        0.6,
        r * 0.85,
        0.05,
        hot(c, 1.3, 0.7),
        0.38,
    );
    beams.ring(
        feet + Vec3::Y * 0.08,
        Vec3::Y,
        0.5,
        r,
        0.05,
        hot(c, 1.0, 0.5),
        0.45,
    );
    for k in 0..16 {
        let a = k as f32 / 16.0 * std::f32::consts::TAU + rng.range(-0.1, 0.1);
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let tangent = Vec3::new(-a.sin(), 0.0, a.cos());
        effects.spawn(Particle {
            position: feet + Vec3::Y * rng.range(0.5, 1.3) + out * 0.8,
            velocity: out * r * 2.2 + tangent * 4.0,
            drag: 4.0,
            max_life: rng.range(0.25, 0.4),
            size: Vec2::splat(0.07),
            color: hot(c, 1.3, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    effects.dust(feet, 14, r * 1.6);
}

/// The Upswing launching what stands at `feet`: a column of dust and sparks thrown upward and
/// a ring kicked off the floor.
pub fn upswing_burst(effects: &mut Effects, beams: &mut Beams, rng: &mut Rng, feet: Vec3) {
    let c = ability_color(SwingKind::Upswing);
    beams.ring(
        feet + Vec3::Y * 0.06,
        Vec3::Y,
        0.3,
        1.6,
        0.06,
        hot(c, 1.5, 0.8),
        0.25,
    );
    beams.beam(feet, feet + Vec3::Y * 3.0, 0.08, hot(c, 1.6, 0.6), 0.18);
    for _ in 0..14 {
        let spread = Vec3::new(rng.range(-0.4, 0.4), 0.0, rng.range(-0.4, 0.4));
        effects.spawn(Particle {
            position: feet + spread + Vec3::Y * 0.1,
            velocity: Vec3::Y * rng.range(6.0, 11.0) + spread * 2.0,
            gravity: 10.0,
            drag: 1.0,
            max_life: rng.range(0.4, 0.7),
            size: Vec2::splat(0.06),
            color: hot(c, 1.3, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
    effects.dust(feet, 12, 1.4);
}

// ---------------------------------------------------------------------------------------------
// Hurled pieces

/// Gravity of a staged hurl (the contract's "light gravity"; the replicated ones fly by the
/// sim's numbers).
const HURL_GRAVITY: f32 = 4.0;
/// Turns per second of a hurled piece.
const HURL_SPIN: f32 = 3.2;

/// Draws a hurled blade (Trimmer) or hammer head (Tamper) `age` seconds into its flight.
pub fn draw_hurled(
    lib: &ModelLibrary,
    weapon: MeleeWeapon,
    position: Vec3,
    velocity: Vec3,
    age: f32,
    out: &mut Vec<DrawItem>,
) {
    let spin = age * HURL_SPIN * std::f32::consts::TAU;
    let glow = hot(ability_color(SwingKind::Hurl), 1.6, 1.0);
    model::draw_hurled(
        lib,
        weapon,
        position,
        velocity,
        spin,
        glow,
        &Tint::default(),
        out,
    );
}

/// The light a hurled piece carries.
pub fn hurled_light(weapon: MeleeWeapon, position: Vec3) -> PointLight {
    let c = ability_color(SwingKind::Hurl).truncate();
    PointLight {
        position,
        color: c * if weapon == MeleeWeapon::Tamper {
            1.6
        } else {
            1.1
        },
        radius: 3.5,
    }
}

/// Sparks thrown off a hurled piece's spin.
pub fn hurled_sparks(
    effects: &mut Effects,
    rng: &mut Rng,
    weapon: MeleeWeapon,
    position: Vec3,
    dt: f32,
) {
    if rng.unit() < dt * 30.0 {
        effects.spawn(Particle {
            position,
            velocity: rng.dir() * 2.0,
            gravity: 4.0,
            max_life: 0.3,
            size: Vec2::splat(if weapon == MeleeWeapon::Tamper {
                0.1
            } else {
                0.06
            }),
            color: hot(ability_color(SwingKind::Hurl), 1.3, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// A staged hurled piece's wake (previews; the replicated ones get the entity drawer's glow):
/// a violet streak and aura and the sparks off its spin.
pub fn hurled_trail(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    weapon: MeleeWeapon,
    position: Vec3,
    velocity: Vec3,
    dt: f32,
) {
    let c = ability_color(SwingKind::Hurl);
    let back = velocity * dt.clamp(0.0, 0.05) * 2.0;
    beams.beam(position - back, position, 0.06, hot(c, 1.5, 0.55), 0.12);
    // A violet aura round the spinning piece (it is dark iron and steel).
    effects.spawn(Particle {
        position,
        max_life: 0.06,
        size: Vec2::splat(if weapon == MeleeWeapon::Tamper {
            0.9
        } else {
            0.7
        }),
        color: c.truncate().extend(0.5),
        texture: FxTex::Glow,
        additive: true,
        ..Default::default()
    });
    hurled_sparks(effects, rng, weapon, position, dt);
}

/// A hurled piece striking the level or a target.
pub fn hurl_impact(
    effects: &mut Effects,
    audio: &mut Audio,
    rng: &mut Rng,
    weapon: MeleeWeapon,
    at: Vec3,
    dir: Vec3,
) {
    effects.sparks(at, -dir, 12);
    if weapon == MeleeWeapon::Tamper {
        effects.lumps(at, 5);
        effects.dust(at, 8, 1.2);
    }
    effects.glow(at, hot(ability_color(SwingKind::Hurl), 1.2, 0.8), 0.9, 0.15);
    let _ = rng;
    audio.play(Sfx::HurlHit, PlayParams::at(at));
}

/// How close to its thrower's chest a returning piece vanishes to count as caught.
const CATCH_REACH: f32 = 1.8;

/// The chest of the player with owner code `owner` among `entities`.
fn thrower_chest(entities: &[EntityState], owner: u16) -> Option<Vec3> {
    entities
        .iter()
        .find(|e| e.kind == kind::PLAYER && e.id.as_player().map(|p| p.0 as u16) == Some(owner))
        .map(|e| e.position + Vec3::Y * 1.1)
}

/// A hurled piece back in its thrower's hand (docs/rework.md: the Hurl returns): a slap and a
/// short ring, a few sparks off the catch (`own`: your hand, the sound in your head).
pub fn hurl_catch(effects: &mut Effects, audio: &mut Audio, rng: &mut Rng, at: Vec3, own: bool) {
    if !own {
        effects.sparks(at, rng.dir(), 5);
        effects.glow(at, hot(ability_color(SwingKind::Hurl), 0.9, 0.6), 0.5, 0.1);
        audio.play(Sfx::HurlCatch, PlayParams::at(at));
    } else {
        audio.play_2d(Sfx::HurlCatch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trails_sweep_their_arc_then_go() {
        let mut view = MeleeView::new();
        let mut effects = Effects::headless();
        let mut beams = Beams::headless();
        let mut audio = Audio::disabled();
        let mut rng = Rng::default();
        for (weapon, kind, left) in [
            (MeleeWeapon::Trimmer, SwingKind::Slash, false),
            (MeleeWeapon::Trimmer, SwingKind::Slash, true),
            (MeleeWeapon::Tamper, SwingKind::Slash, false),
            (MeleeWeapon::Tamper, SwingKind::Upswing, false),
            (MeleeWeapon::Trimmer, SwingKind::Whirl, false),
        ] {
            view.swing(weapon, kind, left, Vec3::Y * 1.6, 0.3, -0.1, Vec4::ONE);
        }
        // Moves without a trail (the Lunge has its streak, the Hurl its piece).
        view.swing(
            MeleeWeapon::Tamper,
            SwingKind::Hurl,
            false,
            Vec3::ZERO,
            0.0,
            0.0,
            Vec4::ONE,
        );
        assert_eq!(view.trail_count(), 5);
        let before = beams.beam_count();
        for _ in 0..90 {
            let mut ctx = UpdateCtx {
                effects: &mut effects,
                beams: &mut beams,
                audio: &mut audio,
                rng: &mut rng,
                world: None,
                dt: 1.0 / 60.0,
            };
            view.update(&mut ctx, &|_| None);
        }
        assert!(beams.beam_count() > before, "the trails drew segments");
        assert_eq!(
            view.trail_count(),
            0,
            "and are gone a moment after the blow"
        );
        for t in &view.trails {
            assert!(t.drawn <= 1.0);
        }
    }

    #[test]
    fn a_slash_trail_crosses_in_front_of_the_eye() {
        // Looking down -Z from the origin: the right-hand slash starts on the right, ends on
        // the left, and stays in front.
        let mut view = MeleeView::new();
        view.swing(
            MeleeWeapon::Trimmer,
            SwingKind::Slash,
            false,
            Vec3::ZERO,
            0.0,
            0.0,
            Vec4::ONE,
        );
        let t = view.trails[0];
        let (a, b) = (t.point(0.0), t.point(1.0));
        assert!(a.x > 0.5 && b.x < -0.5, "{a} -> {b}");
        assert!(t.point(0.5).z < -1.0, "{}", t.point(0.5));
        // The Tamper's comes down from above to below the eye.
        view.swing(
            MeleeWeapon::Tamper,
            SwingKind::Slash,
            false,
            Vec3::ZERO,
            0.0,
            0.0,
            Vec4::ONE,
        );
        let t = view.trails[1];
        assert!(t.point(0.0).y > 1.0 && t.point(1.0).y < -0.5);
        assert!(t.point(0.6).z < -1.0);
    }
}
