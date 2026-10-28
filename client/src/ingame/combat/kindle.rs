//! The Kindle Flask, drunk (docs/rework.md §3, area **client-b**): what you see and hear of
//! the knack now that it is drunk instead of thrown.
//!
//! * **The drink** ([`KindleView::drink`]): the left hand brings the flask to the mouth, pulls
//!   the stopper and tips it back in first person (`models::weapons::drink`), with
//!   `Sfx::KindleDrink`; another player's drink (`Fx::Knack`) is that sound at them and a flare
//!   of their chest wick ([`remote_drink`]).
//! * **Haste** for its few seconds. Yours ([`KindleView::own_frame`]): a small FOV kick
//!   ([`haste_fov`]), embers streaming past the view from your wick as you run, ember streaks
//!   flowing back along the screen's edges and a warm glow at its foot ([`haste_hud`]); the Q
//!   slot's bar drains with it; `Sfx::HasteFade` as it runs out. Others' (the `HASTE` player
//!   flag, [`haste_trail`]): embers streaming off their chest wick and a speed trail at chest
//!   and knee height behind them, longer the faster they go.
//! * **Heal ticks** (`Fx::Heal` from a hasted player's own body, `HealSource::KindleFlask`,
//!   [`is_draught_heal`]): kindling motes curling up round the body into the chest lantern and
//!   a soft chime ([`heal_tick`]); a faint warm flash for you.
//!
//! [`KindlePreview`] / `InGame::preview_kindle` stage any of it client-side (tools:
//! `combat_shots`' `kindle_*.png`), including a hasted Shrike running past.

use std::collections::HashMap;

use engine::audio::{Audio, PlayParams, Sfx};
use engine::gfx::{DrawItem, UiDrawList};
use engine::ui::draw::fade;
use engine::ui::{Rect, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};
use shared::protocol::{EntityState, NetId};
use shared::sim::PlayerId;

use super::super::InGame;
use super::super::effects::{Effects, FxTex, Particle};
use super::super::viewmodel::Viewmodel;
use super::Rng;
use super::fx::{Beams, FxCtx};
use crate::app::Common;
use crate::models::shrike::{LANTERN_HEIGHT, REMOTE_SCALE};
use crate::models::{ModelLibrary, PlayerAnim, Tint};
use crate::net::Session;

/// How long the haste lasts when the tuning doesn't say (docs/rework.md: 5 s).
pub const HASTE_TIME: f32 = 5.0;
/// The FOV kick at full haste (degrees).
pub const HASTE_FOV: f32 = 5.0;
/// Below this speed (m/s) the trail and the streaks fade out: standing still under haste
/// shows only the wick's embers.
const TRAIL_SPEED: f32 = 5.0;

/// The haste's embers: the palette's heal colour (the kindling), pushed hot.
pub fn ember_color() -> Vec4 {
    crate::palette::palette().heal
}

fn hot(c: Vec4, k: f32, a: f32) -> Vec4 {
    (c.truncate() * k).extend(a)
}

/// The chest wick of a replicated Shrike (drawn [`REMOTE_SCALE`] larger, facing its yaw).
pub fn wick_position(e: &EntityState) -> Vec3 {
    let fwd = shared::input::look_dir(e.yaw, 0.0);
    e.position + Vec3::Y * LANTERN_HEIGHT * REMOTE_SCALE + fwd * 0.2 * REMOTE_SCALE
}

/// The FOV kick (degrees) for haste `strength` (0..1 of its time left): in at once, held, out
/// over its last eighth.
pub fn haste_fov(strength: f32) -> f32 {
    if strength.is_finite() && strength > 0.0 {
        HASTE_FOV * (strength * 8.0).min(1.0)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------------
// Effects

/// Others' haste (the `HASTE` flag) this frame: embers streaming off the chest wick behind
/// them and, moving fast, a speed trail at chest and knee height.
#[allow(clippy::too_many_arguments)]
pub fn haste_trail(
    effects: &mut Effects,
    beams: &mut Beams,
    rng: &mut Rng,
    wick: Vec3,
    feet: Vec3,
    velocity: Vec3,
    dt: f32,
) {
    let c = ember_color();
    let flat = Vec3::new(velocity.x, 0.0, velocity.z);
    let speed = flat.length();
    let back = if speed > 0.1 {
        -flat / speed
    } else {
        Vec3::ZERO
    };
    // The wick burns bright under it.
    effects.billboard(wick, 0.55, hot(c, 1.3, 0.7), FxTex::Glow, 0.0);
    // Embers off the wick: a steady stream, blown back by the run.
    let rate = 40.0 + 2.5 * speed;
    let mut n = (rate * dt) as u32;
    if rng.unit() < rate * dt - n as f32 {
        n += 1;
    }
    for _ in 0..n {
        let jitter = rng.dir() * 0.12;
        effects.spawn(Particle {
            position: wick + jitter,
            velocity: back * (0.35 * speed + 1.0) + Vec3::Y * rng.range(0.6, 1.6) + jitter * 4.0,
            gravity: -0.8,
            drag: 1.5,
            max_life: rng.range(0.4, 0.7),
            size: Vec2::splat(rng.range(0.11, 0.18)),
            end_scale: 0.35,
            color: hot(c, 1.6, 1.0),
            texture: FxTex::Spark,
            additive: true,
            rotation: rng.angle(),
            spin: rng.range(-6.0, 6.0),
            ..Default::default()
        });
    }
    // The speed trail: short-lived streaks laid behind the body every frame.
    let k = ((speed - TRAIL_SPEED) / 10.0).clamp(0.0, 1.0);
    if k > 0.0 {
        let len = (0.18 + 0.12 * k) * speed.min(30.0) * 0.22;
        for (h, w, a) in [(0.55, 0.07, 0.45), (LANTERN_HEIGHT, 0.1, 0.6)] {
            let from = feet + Vec3::Y * h * REMOTE_SCALE;
            beams.beam(from, from + back * len, w, hot(c, 1.4, a * k), 0.06);
        }
    }
}

/// Your haste this frame: embers drifting past the view (spawned ahead and to the sides,
/// streaming back as you run through them) and one off your wick below the view now and then.
fn own_embers(
    effects: &mut Effects,
    rng: &mut Rng,
    eye: Vec3,
    view: Vec3,
    velocity: Vec3,
    strength: f32,
    dt: f32,
) {
    let c = ember_color();
    let speed = velocity.length();
    let fwd = Vec3::new(view.x, 0.0, view.z).normalize_or(Vec3::NEG_Z);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let rate = (24.0 + 3.0 * speed) * strength.sqrt();
    let mut n = (rate * dt) as u32;
    if rng.unit() < rate * dt - n as f32 {
        n += 1;
    }
    for _ in 0..n {
        // Off to the sides and low, never across the crosshair.
        let side = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        let at = eye
            + fwd * rng.range(1.0, 2.0 + 0.3 * speed)
            + right * side * rng.range(0.6, 1.6)
            + Vec3::Y * rng.range(-1.0, 0.3);
        effects.spawn(Particle {
            position: at,
            // The embers hang back from the wick (you outrun them) and rise.
            velocity: velocity * 0.35 + Vec3::Y * rng.range(0.4, 1.2) + rng.dir() * 0.3,
            gravity: -0.6,
            drag: 0.8,
            max_life: rng.range(0.35, 0.6),
            size: Vec2::splat(rng.range(0.045, 0.08)),
            end_scale: 0.4,
            color: hot(c, 1.7, 0.9),
            texture: FxTex::Spark,
            additive: true,
            rotation: rng.angle(),
            ..Default::default()
        });
    }
}

/// Your haste on the HUD (`strength` 0..1 of its time left, `rush` 0..1 how fast you are
/// going): ember streaks flowing back along the screen's sides and foot, faster and longer
/// with your speed, and a warm glow along the bottom edge (your wick's light).
pub fn haste_hud(list: &mut UiDrawList, c: Rect, strength: f32, rush: f32, now: f64) {
    if strength.is_nan() || strength <= 0.0 {
        return;
    }
    let color = ember_color();
    let k = strength.clamp(0.0, 1.0).sqrt();
    let rush = rush.clamp(0.0, 1.0);
    // The wick's glow at the foot of the screen.
    for i in 0..6 {
        let h = 4.0 + i as f32 * 5.0;
        let a = 0.16 * k * (1.0 - i as f32 / 6.0);
        list.fill(Rect::new(vec2(c.min.x, c.max.y - h), c.max), fade(color, a));
    }
    // Streaks: along the left and right edges and up from the bottom corners, flowing out
    // towards the edges (the world rushing past).
    let center = c.center();
    let streaks = 22;
    for i in 0..streaks {
        let seed = i as f64 * 2.399 + 0.7;
        let side = if i % 2 == 0 { -1.0 } else { 1.0 };
        let y = c.min.y + c.height() * (0.25 + 0.7 * ((seed * 0.613).fract() as f32));
        let speed = 0.9 + 1.4 * rush;
        let phase = ((now * speed as f64 + seed).fract()) as f32;
        let x_edge = if side < 0.0 { c.min.x } else { c.max.x };
        let inner = center.x + side * c.width() * (0.3 + 0.15 * (seed * 0.37).fract() as f32);
        let from = vec2(
            inner + (x_edge - inner) * phase,
            y + (y - center.y) * 0.35 * phase,
        );
        let dir = (from - center).normalize_or(vec2(side, 0.0));
        let len = (14.0 + 46.0 * rush) * (0.5 + phase);
        // Unbroken: a square every couple of units, the head bigger.
        let steps = ((len / 2.0).ceil() as usize).clamp(6, 40);
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let p = from - dir * len * t;
            list.fill(
                Rect::from_center_size(p, Vec2::splat(1.5 + (1.0 - t) * 2.0)),
                fade(
                    color,
                    0.7 * k * (0.35 + 0.65 * rush) * (1.0 - t) * phase.sqrt(),
                ),
            );
        }
    }
}

/// Another player drank (`Fx::Knack`): their Shrike's drink, the swig at them and their wick
/// flaring.
pub fn remote_drink(ctx: &mut FxCtx, e: &EntityState) {
    ctx.remote.visual_mut(e.id).drinking = 1e-3;
    let wick = wick_position(e);
    ctx.audio.play(Sfx::KindleDrink, PlayParams::at(wick));
    ctx.effects
        .glow(wick, hot(ember_color(), 1.6, 0.9), 0.9, 0.35);
    for _ in 0..10 {
        let d = ctx.rng.dir();
        ctx.effects.spawn(Particle {
            position: wick + d * 0.1,
            velocity: d * 2.0 + Vec3::Y * 1.5,
            gravity: -0.5,
            drag: 2.0,
            max_life: 0.45,
            size: Vec2::splat(0.08),
            end_scale: 0.4,
            color: hot(ember_color(), 1.8, 1.0),
            texture: FxTex::Spark,
            additive: true,
            ..Default::default()
        });
    }
}

/// A heal from a draught (`Fx::Heal` whose source is the healed player's own body)? Ember
/// bursts heal from where the burst lies (`burst_there`: an ember burst entity at `from`); the
/// Kindle heals from you.
pub fn is_draught_heal(body: Option<Vec3>, burst_there: bool, from: Vec3) -> bool {
    !burst_there && body.is_some_and(|b| Vec2::new(b.x - from.x, b.z - from.z).length() < 1.2)
}

/// Plays `Fx::Heal` as a Kindle heal tick if it is one ([`is_draught_heal`]); `false`: it is
/// another heal (an ember burst's stream).
pub fn play_heal(ctx: &mut FxCtx, player: PlayerId, from: Vec3, amount: f32) -> bool {
    let local = Some(player) == ctx.local;
    let (to, feet) = if local {
        match ctx.local_chest {
            Some(c) => (c, c - Vec3::Y * 1.1),
            None => return false,
        }
    } else {
        match ctx.entities.iter().find(|e| e.id == NetId::player(player)) {
            Some(e) => (
                e.position + Vec3::Y * LANTERN_HEIGHT * REMOTE_SCALE,
                e.position,
            ),
            None => return false,
        }
    };
    let burst = ctx
        .entities
        .iter()
        .any(|e| e.kind == shared::protocol::kind::EMBER_BURST && e.position.distance(from) < 0.6);
    if !is_draught_heal(Some(feet), burst, from) {
        return false;
    }
    heal_tick(ctx, player, to, feet, amount);
    true
}

/// A Kindle heal tick on `player` (`to` = their chest lantern, `feet` their feet): kindling
/// motes curling up round the body into the lantern, a soft chime, a faint warm flash for you.
pub fn heal_tick(ctx: &mut FxCtx, player: PlayerId, to: Vec3, feet: Vec3, amount: f32) {
    let local = Some(player) == ctx.local;
    let c = ember_color();
    for i in 0..8 {
        let a = ctx.rng.angle();
        let r = 0.45 + 0.1 * (i % 3) as f32;
        let start = Vec3::new(feet.x + a.cos() * r, feet.y + 0.3, feet.z + a.sin() * r);
        let life = 0.45 + 0.05 * (i % 4) as f32;
        // Up and in, with a swirl round the body.
        let swirl = Vec3::new(-a.sin(), 0.0, a.cos()) * 1.2;
        ctx.effects.spawn(Particle {
            position: start,
            velocity: (to - start) / life + swirl,
            drag: 0.6,
            max_life: life,
            size: Vec2::splat(0.07),
            end_scale: 0.5,
            color: hot(c, 1.5, 1.0),
            end_alpha: 0.6,
            texture: FxTex::Spark,
            additive: true,
            rotation: a,
            ..Default::default()
        });
    }
    if local {
        ctx.feedback.flash(
            crate::palette::palette().flash_heal,
            (amount / 80.0).min(0.05),
        );
        ctx.audio.play(
            Sfx::KindleTick,
            PlayParams::default().with_volume((0.5 + amount / 40.0).min(0.8)),
        );
    } else {
        ctx.effects.glow(to, hot(c, 1.2, 0.7), 0.4, 0.2);
        ctx.audio
            .play(Sfx::KindleTick, PlayParams::at(to).with_volume(0.6));
    }
}

// ---------------------------------------------------------------------------------------------
// Bookkeeping and previews

/// Something staged client-side by `InGame::preview_kindle` (tools: look at the visuals without
/// the simulation making them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KindlePreview {
    /// You drink: the drink in first person, then haste for `seconds`.
    Drink { seconds: f32 },
    /// A hasted Shrike running from `from` at `velocity` for `seconds` (what others see).
    Runner {
        from: Vec3,
        velocity: Vec3,
        seconds: f32,
    },
    /// A heal tick on you.
    Tick { amount: f32 },
}

#[derive(Clone, Copy, Debug)]
struct Runner {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    seconds: f32,
    cycle: f32,
    /// Seconds to its next heal tick.
    tick: f32,
}

/// The Kindle Flask's visuals that live across frames.
#[derive(Default)]
pub struct KindleView {
    /// Your haste (predicted from the drink): seconds left and total.
    haste: (f32, f32),
    /// The fade sound was played for the haste running now (yours).
    faded: bool,
    /// Your haste was staged by a preview (no server to confirm it).
    staged_haste: bool,
    /// Others hasted last frame: whether their fade was heard.
    others: HashMap<NetId, bool>,
    runners: Vec<Runner>,
    staged: Vec<KindlePreview>,
    /// Staged heal ticks waiting for an `FxCtx`.
    ticks: Vec<f32>,
    /// Your haste (0..1) and how fast you went this frame (0..1), for the HUD.
    shown: (f32, f32),
    time: f64,
}

impl KindleView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self {
            time: self.time,
            ..Self::default()
        };
    }

    /// You drank (a predicted `FireIntent::Knack(KindleFlask)`): the drink in first person, its
    /// sound, and your haste for `seconds`.
    pub fn drink(&mut self, viewmodel: &mut Viewmodel, audio: &mut Audio, seconds: f32) {
        viewmodel.drink();
        audio.play_2d(Sfx::KindleDrink);
        let d = if seconds.is_finite() && seconds > 0.1 {
            seconds
        } else {
            HASTE_TIME
        };
        self.haste = (d, d);
        self.faded = false;
        self.staged_haste = false;
    }

    /// Your haste 0..1 of its time left: the server's prediction when the movement state carries
    /// it (`predicted`: seconds left, total), else the timer started by the drink, held while
    /// the `HASTE` flag (`flag`) says so; a timer the server never confirmed is dropped after
    /// its first moments.
    pub fn strength(&self, flag: bool, predicted: Option<(f32, f32)>) -> f32 {
        if let Some((left, total)) = predicted {
            return if left > 0.0 && total > 0.0 {
                (left / total).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
        let own = if self.haste.0 > 0.0 {
            self.haste.0 / self.haste.1.max(0.01)
        } else {
            0.0
        };
        if flag || self.staged_haste {
            own.max(if flag { 0.15 } else { 0.0 })
        } else if own > 0.85 {
            own
        } else {
            0.0
        }
    }

    /// Your haste this frame (`strength` from [`KindleView::strength`]): embers past the view,
    /// the fade sound as it runs out. Returns the FOV kick (degrees).
    #[allow(clippy::too_many_arguments)]
    pub fn own_frame(
        &mut self,
        effects: &mut Effects,
        audio: &mut Audio,
        rng: &mut Rng,
        eye: Vec3,
        view: Vec3,
        velocity: Vec3,
        strength: f32,
        dt: f32,
    ) -> f32 {
        let run = (Vec3::new(velocity.x, 0.0, velocity.z).length() - TRAIL_SPEED) / 15.0;
        self.shown = (strength, run.clamp(0.0, 1.0));
        if strength > 0.0 {
            own_embers(effects, rng, eye, view, velocity, strength, dt);
            if strength < 0.1 && !self.faded {
                self.faded = true;
                audio.play_2d(Sfx::HasteFade);
            }
        }
        haste_fov(strength)
    }

    fn preview(&mut self, p: KindlePreview, viewmodel: &mut Viewmodel, audio: &mut Audio) {
        match p {
            KindlePreview::Drink { seconds } => {
                self.drink(viewmodel, audio, seconds);
                self.staged_haste = true;
            }
            KindlePreview::Runner {
                from,
                velocity,
                seconds,
            } => self.runners.push(Runner {
                position: from,
                velocity,
                age: 0.0,
                seconds,
                cycle: 0.0,
                tick: 0.3,
            }),
            KindlePreview::Tick { amount } => self.ticks.push(amount),
        }
    }

    /// Advances your haste timer and the staged runners (their trail and embers).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
        viewmodel: &mut Viewmodel,
        dt: f32,
    ) {
        let dt = dt.max(0.0);
        self.time += dt as f64;
        for p in std::mem::take(&mut self.staged) {
            self.preview(p, viewmodel, audio);
        }
        self.haste.0 = (self.haste.0 - dt).max(0.0);
        for r in &mut self.runners {
            r.age += dt;
            r.position += r.velocity * dt;
            r.cycle = PlayerAnim::advance_cycle(r.cycle, r.velocity, dt);
            let e = EntityState {
                position: r.position,
                velocity: r.velocity,
                yaw: (-r.velocity.x).atan2(-r.velocity.z),
                ..Default::default()
            };
            haste_trail(
                effects,
                beams,
                rng,
                wick_position(&e),
                r.position,
                r.velocity,
                dt,
            );
            r.tick -= dt;
        }
        self.runners.retain(|r| r.age < r.seconds);
    }

    /// Staged heal ticks (yours) and runners' ticks, played through the Fx context.
    pub fn play_staged(&mut self, ctx: &mut FxCtx, local: Option<PlayerId>) {
        for amount in std::mem::take(&mut self.ticks) {
            if let (Some(p), Some(chest)) = (local.or(ctx.local), ctx.local_chest) {
                heal_tick(ctx, p, chest, chest - Vec3::Y * 1.1, amount);
            }
        }
        for r in &mut self.runners {
            if r.tick <= 0.0 {
                r.tick += 1.0;
                let feet = r.position;
                let chest = feet + Vec3::Y * LANTERN_HEIGHT * REMOTE_SCALE;
                heal_tick(ctx, PlayerId(u8::MAX), chest, feet, 8.0);
            }
        }
    }

    /// Whether anything staged waits for [`KindleView::play_staged`].
    pub fn has_staged(&self) -> bool {
        !self.ticks.is_empty() || self.runners.iter().any(|r| r.tick <= 0.0)
    }

    /// Others' haste this frame from their `HASTE` flags (`hasted`: id, wick, feet, velocity):
    /// the trail and embers; the fade sound as one runs out.
    #[allow(clippy::too_many_arguments)]
    pub fn update_others(
        &mut self,
        effects: &mut Effects,
        beams: &mut Beams,
        audio: &mut Audio,
        rng: &mut Rng,
        players: &[EntityState],
        hasted: &dyn Fn(&EntityState) -> bool,
        local: Option<NetId>,
        dt: f32,
    ) {
        let mut now = HashMap::new();
        for e in players {
            if Some(e.id) == local {
                continue;
            }
            if hasted(e) {
                haste_trail(
                    effects,
                    beams,
                    rng,
                    wick_position(e),
                    e.position,
                    e.velocity,
                    dt,
                );
                now.insert(e.id, false);
            } else if self.others.get(&e.id) == Some(&false) {
                audio.play(
                    Sfx::HasteFade,
                    PlayParams::at(wick_position(e)).with_volume(0.6),
                );
            }
        }
        self.others = now;
    }

    /// Your haste on the HUD this frame ([`haste_hud`]).
    pub fn hud(&self, list: &mut UiDrawList, c: Rect, now: f64) {
        haste_hud(list, c, self.shown.0, self.shown.1, now);
    }

    /// Your haste 0..1 as shown this frame (the Q slot's draining bar).
    pub fn shown(&self) -> f32 {
        self.shown.0
    }

    /// Draws the staged runners (hasted Shrikes for the tools).
    pub fn draw(&self, lib: Option<&ModelLibrary>, draws: &mut Vec<DrawItem>) {
        let Some(lib) = lib else {
            return;
        };
        for r in &self.runners {
            let anim = PlayerAnim {
                velocity: r.velocity,
                yaw: (-r.velocity.x).atan2(-r.velocity.z),
                on_ground: true,
                cycle: r.cycle,
                time: self.time,
                weapon: 1,
                ..Default::default()
            };
            lib.draw_player_scaled(
                &anim,
                r.position,
                REMOTE_SCALE,
                &Tint::team(crate::palette::palette().team(1)),
                draws,
            );
        }
    }
}

impl InGame {
    /// Stages a Kindle Flask visual client-side (tools and screenshots; see [`KindlePreview`]).
    #[doc(hidden)]
    pub fn preview_kindle(&mut self, preview: KindlePreview) {
        self.combat.knacks.kindle.staged.push(preview);
    }

    /// Per frame (visual time, after the Fx playback): your haste (embers past the view, the
    /// FOV kick), staged previews and heal ticks.
    pub(super) fn combat_kindle(&mut self, common: &mut Common, vdt: f32) {
        let InGame {
            session,
            combat,
            effects,
            viewmodel,
            feedback,
            remote,
            camera,
            last_camera,
            ..
        } = self;
        let Some(net) = session.as_ref().map(Session::client) else {
            return;
        };
        let k = &mut combat.knacks.kindle;
        k.update(
            effects,
            &mut combat.beams,
            &mut common.audio,
            &mut combat.rng,
            viewmodel,
            vdt,
        );
        let velocity = net.local_state().map_or(Vec3::ZERO, |s| s.velocity);
        let alive = !net.own_combat().is_some_and(|c| c.combat.dead);
        let strength = if alive { k.strength(false, None) } else { 0.0 };
        let fov = k.own_frame(
            effects,
            &mut common.audio,
            &mut combat.rng,
            last_camera.position,
            last_camera.rotation * Vec3::NEG_Z,
            velocity,
            strength,
            vdt,
        );
        camera.set_fov_boost(fov);
        if k.has_staged() {
            let view = &combat.entities;
            let thorn_position = |id: NetId| view.thorn_position(id);
            let mut ctx = FxCtx {
                audio: &mut common.audio,
                effects,
                feedback,
                beams: &mut combat.beams,
                remote,
                world: net.world().map(|w| &w.collision.world),
                entities: net.remote_entities(),
                local: net.local_player_id(),
                listener: last_camera.position,
                local_chest: net.local_state().map(|s| s.position + Vec3::Y * 1.1),
                thorn_position: &thorn_position,
                rng: &mut combat.rng,
            };
            combat
                .knacks
                .kindle
                .play_staged(&mut ctx, net.local_player_id());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haste_holds_while_the_server_says_so_and_kicks_the_view() {
        let mut k = KindleView::new();
        assert_eq!(k.strength(false, None), 0.0);
        k.haste = (5.0, 5.0);
        assert!((k.strength(false, None) - 1.0).abs() < 1e-6, "just drunk");
        k.haste = (2.5, 5.0);
        assert_eq!(k.strength(false, None), 0.0, "never confirmed");
        assert!((k.strength(true, None) - 0.5).abs() < 1e-6);
        // The movement state's prediction wins when there is one.
        assert!((k.strength(false, Some((1.0, 4.0))) - 0.25).abs() < 1e-6);
        assert_eq!(k.strength(true, Some((0.0, 4.0))), 0.0);
        assert_eq!(haste_fov(0.0), 0.0);
        assert_eq!(haste_fov(f32::NAN), 0.0);
        assert!((haste_fov(0.6) - HASTE_FOV).abs() < 1e-6);
        assert!(haste_fov(0.05) < HASTE_FOV * 0.5, "eases out at the end");
    }

    #[test]
    fn a_draught_heals_from_the_drinker_not_from_a_burst() {
        let body = Some(Vec3::new(4.0, 0.0, 2.0));
        assert!(is_draught_heal(body, false, Vec3::new(4.2, 1.3, 2.1)));
        assert!(!is_draught_heal(body, true, Vec3::new(4.2, 1.3, 2.1)));
        assert!(!is_draught_heal(body, false, Vec3::new(9.0, 0.0, 2.0)));
        assert!(!is_draught_heal(None, false, Vec3::ZERO));
    }
}
