//! Deterministic kinematic character controller: the whole VELOCIDE move set.
//!
//! [`step`] advances one fixed tick. It is a pure function of `(MoveState, PlayerInput,
//! MoveEnv, MovementTuning, dt)`: every timer, charge, flag and the previous tick's buttons
//! live in [`MoveState`], so the client can predict and the server can re-run any tick and get
//! bit-identical results. See `docs/movement.md` for the state machine and every rule.
//!
//! External impulses (explosion knockback, rocket jumps, enemy hits) are applied by the server
//! directly to [`MoveState::velocity`] between steps. An upward impulse while grounded lifts
//! the player off the ground on the next step; clients pick impulses up through
//! reconciliation.

mod camera;
mod collide;

pub use camera::{fov_kick, landing_dip, view_roll};
pub use collide::SKIN;

use engine::physics::CollisionWorld;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::input::{Buttons, PlayerInput};
use crate::level::{JumpPad, Volume};
use crate::tuning::MovementTuning;
use collide::{Body, Ctx, GROUND_PROBE};

/// The static world a movement step runs against. Must be identical on client and server.
#[derive(Clone, Copy)]
pub struct MoveEnv<'a> {
    pub world: &'a CollisionWorld,
    pub jump_pads: &'a [JumpPad],
}

/// What the player is doing. Exactly one at a time; see `docs/movement.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MoveMode {
    /// Running on the ground.
    Walk,
    /// Airborne (jumping, falling, launched).
    #[default]
    Air,
    /// Low-profile slide on the ground.
    Slide,
    /// Dash burst (on the ground or in the air; `on_ground` says which).
    Dash,
    /// Ground pound: falling at `slam_speed`.
    Slam,
}

/// Everything that affects future movement. Predicted on the client, authoritative on the
/// server, and sent in full to the owning client for reconciliation. There is no hidden state
/// anywhere else: `step(state, input)` is all there is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoveState {
    /// Feet position (bottom of the capsule).
    pub position: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// Buttons held last tick (edge detection).
    pub prev_buttons: Buttons,
    pub mode: MoveMode,
    /// Normal of the ground under the player (valid while `on_ground`).
    pub ground_normal: Vec3,
    /// Using the low (slide) capsule: sliding, or stuck under something too low to stand.
    pub crouched: bool,
    /// Dash charges spent and not yet regenerated (0 = full). Stored as a deficit so
    /// `MoveState::default()` / [`MoveState::spawn`] start with full stamina.
    pub stamina_used: f32,
    /// Remaining dash time (> 0 while dashing).
    pub dash_time: f32,
    pub dash_dir: Vec3,
    /// Horizontal speed the current dash ends with.
    pub dash_exit_speed: f32,
    /// Remaining invulnerability time (dash i-frames).
    pub invuln_time: f32,
    /// Slide direction (used when the slide is blocked to a stop).
    pub slide_dir: Vec3,
    /// Height the current slam started at (for its power).
    pub slam_start_y: f32,
    /// Remaining time in which a jump is a slam bounce.
    pub slam_bounce_time: f32,
    /// Power (0..=1) of the slam that opened the bounce window.
    pub slam_bounce_power: f32,
    /// Remaining coyote time (a ground jump is still allowed after leaving the ground).
    pub coyote_time: f32,
    /// Remaining jump-buffer time (a jump press waiting for a chance to jump).
    pub jump_buffer: f32,
    /// Wall jumps used since last touching the ground.
    pub wall_jumps_used: u32,
    /// Horizontal normal of a wall within reach while airborne, `Vec3::ZERO` if none. The
    /// client uses it for view tilt.
    pub wall_normal: Vec3,
    /// Pressing into a wall while falling: fall speed is capped.
    pub wall_clinging: bool,
    /// Index of the jump pad trigger the player is currently inside (pads fire on entry).
    pub jump_pad: Option<u16>,
    /// Distance walked since the last footstep.
    pub footstep_accum: f32,
    /// Time since leaving the ground (0 while grounded).
    pub air_time: f32,
}

/// A slam hit the ground: the server applies a shockwave here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlamLanded {
    /// Feet position at impact.
    pub position: Vec3,
    /// 0..=1, from the height fallen while slamming (1 at `slam_full_power_height`).
    pub power: f32,
}

/// Things that happened during one movement step (for sounds, effects and gameplay hooks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoveEvents {
    /// Any jump off the ground or within coyote time (including slide-jumps, dash-jumps and
    /// slam bounces; those also set their own flag). Wall jumps set `wall_jumped` instead.
    pub jumped: bool,
    pub slide_jumped: bool,
    pub dash_jumped: bool,
    /// The jump was a slam bounce.
    pub slam_bounced: bool,
    pub wall_jumped: bool,
    pub dashed: bool,
    /// Dash pressed without a full charge ("no stamina" sound).
    pub dash_failed: bool,
    /// A dash charge finished regenerating.
    pub stamina_recharged: bool,
    pub slide_started: bool,
    pub slide_ended: bool,
    pub slam_started: bool,
    pub slam_landed: Option<SlamLanded>,
    /// Landed this tick with the given downward speed (m/s). Drives the client landing dip.
    pub landed: Option<f32>,
    /// Walked/slid off an edge or was knocked off the ground (no jump).
    pub left_ground: bool,
    /// Entered this jump pad (index into `MoveEnv::jump_pads`).
    pub jump_pad: Option<u16>,
    /// Footstep cadence trigger (walking on the ground).
    pub footstep: bool,
    pub hit_ceiling: bool,
    /// Hit a wall (or any blocking surface) hard: speed lost into it, if above 8 m/s.
    pub wall_impact: Option<f32>,
    /// Climbed a step this tick by this height (client may smooth the camera).
    pub stepped_up: Option<f32>,
}

impl MoveState {
    pub fn spawn(position: Vec3) -> Self {
        Self {
            position,
            ..Default::default()
        }
    }

    /// Current capsule height (standing or crouched/sliding).
    pub fn height(&self, t: &MovementTuning) -> f32 {
        if self.crouched {
            t.slide_height
        } else {
            t.standing_height
        }
    }

    /// Capsule centre, inner half-height and radius for the current stance.
    pub fn capsule(&self, t: &MovementTuning) -> (Vec3, f32, f32) {
        let body = self.body(t);
        (body.center(self.position), body.half_height(), body.radius)
    }

    pub fn eye_position(&self, t: &MovementTuning) -> Vec3 {
        let eye = if self.crouched {
            t.slide_eye_height
        } else {
            t.eye_height
        };
        self.position + Vec3::Y * eye
    }

    /// Dash invulnerability frames.
    pub fn is_invulnerable(&self) -> bool {
        self.invuln_time > 0.0
    }

    /// Available dash charges (fractional while regenerating).
    pub fn stamina(&self, t: &MovementTuning) -> f32 {
        (t.dash_charges - self.stamina_used).max(0.0)
    }

    pub fn is_sliding(&self) -> bool {
        self.mode == MoveMode::Slide
    }

    pub fn is_dashing(&self) -> bool {
        self.mode == MoveMode::Dash
    }

    pub fn is_slamming(&self) -> bool {
        self.mode == MoveMode::Slam
    }

    pub fn horizontal_speed(&self) -> f32 {
        horizontal(self.velocity).length()
    }

    fn body(&self, t: &MovementTuning) -> Body {
        Body {
            height: self.height(t),
            radius: t.radius,
        }
    }
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Moves `from` towards `to` by at most `max_delta`.
fn move_towards(from: Vec3, to: Vec3, max_delta: f32) -> Vec3 {
    let d = to - from;
    let len = d.length();
    if len <= max_delta || len < 1e-6 {
        to
    } else {
        from + d * (max_delta / len)
    }
}

fn volume_overlaps(v: &Volume, lo: Vec3, hi: Vec3) -> bool {
    lo.cmple(v.max).all() && hi.cmpge(v.min).all()
}

/// Advances one fixed tick.
pub fn step(
    state: &mut MoveState,
    input: &PlayerInput,
    env: &MoveEnv,
    t: &MovementTuning,
    dt: f32,
) -> MoveEvents {
    let mut ev = MoveEvents::default();
    let held = input.buttons;
    let prev = std::mem::replace(&mut state.prev_buttons, held);
    if !(dt > 0.0 && dt.is_finite()) {
        return ev;
    }
    if !state.velocity.is_finite() {
        state.velocity = Vec3::ZERO;
    }

    let ctx = Ctx {
        world: env.world,
        min_walk_y: t.max_ground_slope_degrees.to_radians().cos(),
    };
    let jump_pressed = held.pressed(prev, Buttons::JUMP);
    let crouch_pressed = held.pressed(prev, Buttons::CROUCH);
    let dash_pressed = held.pressed(prev, Buttons::DASH);
    let crouch_held = held.contains(Buttons::CROUCH);
    let wish = input.wish_dir();
    let facing = {
        let (s, c) = input.yaw.sin_cos();
        Vec3::new(-s, 0.0, -c)
    };

    // 1. Safety: get out of anything we're inside.
    collide::depenetrate(&ctx, state.body(t), &mut state.position);

    // 2. An external upward impulse while grounded lifts off (never re-snapped this tick).
    if state.on_ground && state.velocity.y > 0.0 {
        leave_ground(state, &mut ev, t);
    }

    // 3. Timers.
    state.invuln_time = (state.invuln_time - dt).max(0.0);
    state.coyote_time = (state.coyote_time - dt).max(0.0);
    state.slam_bounce_time = (state.slam_bounce_time - dt).max(0.0);
    state.jump_buffer = (state.jump_buffer - dt).max(0.0);
    if jump_pressed {
        state.jump_buffer = t.jump_buffer_time.max(1e-6);
    }
    if state.mode != MoveMode::Dash && state.stamina_used > 0.0 {
        let before = state.stamina(t).floor();
        state.stamina_used = (state.stamina_used - t.dash_regen_rate * dt).max(0.0);
        if state.stamina(t).floor() > before {
            ev.stamina_recharged = true;
        }
    }
    if state.on_ground {
        state.wall_jumps_used = 0;
    }

    // 4. Walls within reach and steep slopes being touched (airborne only).
    let nearby = if state.on_ground {
        collide::Nearby::default()
    } else {
        collide::probe_nearby(&ctx, state.body(t), state.position, t.wall_detect_distance)
    };
    let wall = nearby.wall;
    state.wall_normal = wall.unwrap_or(Vec3::ZERO);
    state.wall_clinging = false;

    // 5. Actions: dash, slam, jump, slide.
    if dash_pressed {
        if state.stamina(t) >= 1.0 {
            start_dash(state, wish, facing, t, &mut ev);
        } else {
            ev.dash_failed = true;
        }
    }

    // (Dash and crouch pressed on the same tick: the dash wins.)
    if crouch_pressed
        && !ev.dashed
        && !state.on_ground
        && state.mode != MoveMode::Slam
        && !collide::ground_within(&ctx, state.body(t), state.position, t.slam_min_height)
    {
        start_slam(state, t, &mut ev);
    }

    if state.jump_buffer > 0.0 && try_jump(state, wish, wall, t, &mut ev) {
        state.jump_buffer = 0.0;
    }

    if state.on_ground && state.mode != MoveMode::Dash {
        if crouch_held {
            if state.mode != MoveMode::Slide {
                start_slide(state, wish, facing, t, &mut ev);
            }
        } else if state.mode == MoveMode::Slide
            && collide::fits(&ctx, state.position, t.standing_height, t.radius)
        {
            state.mode = MoveMode::Walk;
            state.crouched = false;
            ev.slide_ended = true;
        }
    }
    if state.crouched
        && state.mode != MoveMode::Slide
        && collide::fits(&ctx, state.position, t.standing_height, t.radius)
    {
        state.crouched = false;
    }

    // 6. Velocity.
    let tick_start_speed = state.velocity.length();
    match state.mode {
        MoveMode::Walk => {
            let vh = ground_velocity(horizontal(state.velocity), wish, t, dt);
            state.velocity = vh;
        }
        MoveMode::Slide => slide_velocity(state, wish, t, dt),
        MoveMode::Air => {
            state.velocity.y = (state.velocity.y - t.gravity * dt).max(-t.max_fall_speed);
            // Air control can't push into a steep slope (or you could climb it): you slide off.
            let mut air_wish = wish;
            if let Some(n) = nearby.steep {
                let nh = horizontal(n).normalize_or_zero();
                air_wish -= nh * air_wish.dot(nh).min(0.0);
            }
            let vh = air_accelerate(horizontal(state.velocity), air_wish, t, dt);
            state.velocity = vh + Vec3::Y * state.velocity.y;
            if let Some(n) = wall
                && state.velocity.y < 0.0
                && wish.dot(n) < -0.3
            {
                state.wall_clinging = true;
                state.velocity.y = state.velocity.y.max(-t.wall_cling_max_fall_speed);
            }
        }
        MoveMode::Dash => {
            state.velocity = state.dash_dir * t.dash_speed + Vec3::Y * state.velocity.y;
            state.dash_time -= dt;
        }
        MoveMode::Slam => {
            state.velocity.y = -t.slam_speed;
        }
    }
    let speed = state.velocity.length();
    if speed > t.max_speed {
        state.velocity *= t.max_speed / speed;
    }

    // 7. Move.
    let start = state.position;
    let was_grounded = state.on_ground;
    let fall_speed = -state.velocity.y;
    let body = state.body(t);
    let motion = state.velocity * dt;
    let res = if was_grounded {
        let m = collide::along_ground(motion, state.ground_normal);
        collide::step_slide_move(
            &ctx,
            body,
            &mut state.position,
            &mut state.velocity,
            m,
            t.step_height,
        )
    } else {
        collide::slide_move(
            &ctx,
            body,
            &mut state.position,
            &mut state.velocity,
            motion,
            false,
        )
    };
    ev.hit_ceiling = res.hit_ceiling;
    // Bounded by the speed we had coming into the tick, so pushing against a wall we're
    // already stopped at (head-on slide, held dash) doesn't re-trigger every tick.
    let impact = res.impact_speed.min(tick_start_speed);
    if impact > 8.0 {
        ev.wall_impact = Some(impact);
    }
    if res.stepped > 0.01 {
        ev.stepped_up = Some(res.stepped);
    }

    // 8. Ground: snap down (stairs, ramps) while grounded, detect landings.
    let ground = if state.velocity.y <= 0.0 {
        let reach = if was_grounded {
            t.step_height + GROUND_PROBE
        } else {
            GROUND_PROBE
        };
        collide::snap_to_ground(&ctx, body, &mut state.position, reach)
    } else {
        None
    };
    match ground {
        Some(n) => {
            state.ground_normal = n;
            state.velocity.y = 0.0;
            if !was_grounded {
                land(state, fall_speed, crouch_held, wish, facing, t, &mut ev);
            }
            state.on_ground = true;
        }
        None => {
            state.ground_normal = Vec3::ZERO;
            if was_grounded {
                leave_ground(state, &mut ev, t);
            }
        }
    }

    // Safety net: never end a tick inside geometry (a bad sweep, external impulses).
    collide::depenetrate(&ctx, body, &mut state.position);

    // 9. Dash ends: keep a sensible part of its momentum.
    if state.mode == MoveMode::Dash && state.dash_time <= 0.0 {
        let scale = if t.dash_speed > 0.0 {
            (state.dash_exit_speed / t.dash_speed).min(1.0)
        } else {
            0.0
        };
        state.velocity = horizontal(state.velocity) * scale + Vec3::Y * state.velocity.y;
        state.dash_time = 0.0;
        state.mode = MoveMode::Air;
        if state.on_ground {
            state.mode = MoveMode::Walk;
            if crouch_held {
                start_slide(state, wish, facing, t, &mut ev);
            }
        }
    }

    // 10. Jump pads fire when entered.
    let lo = state.position - Vec3::new(t.radius, 0.0, t.radius);
    let hi = state.position + Vec3::new(t.radius, state.height(t), t.radius);
    let pad = env
        .jump_pads
        .iter()
        .position(|p| volume_overlaps(&p.trigger, lo, hi))
        .map(|i| i.min(u16::MAX as usize) as u16);
    if let Some(i) = pad
        && state.jump_pad != Some(i)
    {
        launch(state, env.jump_pads[i as usize].velocity, &mut ev);
        ev.jump_pad = Some(i);
    }
    state.jump_pad = pad;

    // 11. Footsteps and air time.
    if state.on_ground && state.mode == MoveMode::Walk && t.footstep_stride > 0.0 {
        state.footstep_accum += horizontal(state.position - start).length();
        if state.footstep_accum >= t.footstep_stride {
            state.footstep_accum = (state.footstep_accum - t.footstep_stride) % t.footstep_stride;
            ev.footstep = true;
        }
    } else if !state.on_ground {
        state.footstep_accum = 0.0;
    }
    state.air_time = if state.on_ground {
        0.0
    } else {
        state.air_time + dt
    };
    ev
}

/// Ground → air without jumping (walked off an edge, knocked up).
fn leave_ground(state: &mut MoveState, ev: &mut MoveEvents, t: &MovementTuning) {
    state.on_ground = false;
    state.ground_normal = Vec3::ZERO;
    state.coyote_time = t.coyote_time;
    ev.left_ground = true;
    match state.mode {
        MoveMode::Walk => state.mode = MoveMode::Air,
        MoveMode::Slide => {
            state.mode = MoveMode::Air;
            ev.slide_ended = true;
        }
        _ => {}
    }
}

/// Air → ground.
fn land(
    state: &mut MoveState,
    fall_speed: f32,
    crouch_held: bool,
    wish: Vec3,
    facing: Vec3,
    t: &MovementTuning,
    ev: &mut MoveEvents,
) {
    ev.landed = Some(fall_speed.max(0.0));
    state.wall_jumps_used = 0;
    state.wall_normal = Vec3::ZERO;
    state.wall_clinging = false;
    state.coyote_time = 0.0;
    state.footstep_accum = 0.0;
    if state.mode == MoveMode::Slam {
        let fallen = (state.slam_start_y - state.position.y).max(0.0);
        let power = if t.slam_full_power_height > 0.0 {
            (fallen / t.slam_full_power_height).clamp(0.0, 1.0)
        } else {
            1.0
        };
        ev.slam_landed = Some(SlamLanded {
            position: state.position,
            power,
        });
        state.slam_bounce_time = t.slam_bounce_window;
        state.slam_bounce_power = power;
    }
    if state.mode != MoveMode::Dash {
        state.mode = MoveMode::Walk;
        if crouch_held {
            start_slide(state, wish, facing, t, ev);
        }
    }
}

fn start_dash(
    state: &mut MoveState,
    wish: Vec3,
    facing: Vec3,
    t: &MovementTuning,
    ev: &mut MoveEvents,
) {
    let dir = if wish.length_squared() > 0.01 {
        wish.normalize()
    } else {
        facing
    };
    if state.mode == MoveMode::Slide {
        ev.slide_ended = true;
    }
    let entry_speed = state.horizontal_speed();
    state.mode = MoveMode::Dash;
    state.dash_time = t.dash_duration;
    state.dash_dir = dir;
    state.dash_exit_speed = t.dash_end_speed.max(entry_speed).min(t.dash_speed);
    state.invuln_time = state.invuln_time.max(t.dash_invuln_time);
    state.stamina_used += 1.0;
    state.velocity = dir * t.dash_speed;
    ev.dashed = true;
}

fn start_slam(state: &mut MoveState, t: &MovementTuning, ev: &mut MoveEvents) {
    state.mode = MoveMode::Slam;
    state.dash_time = 0.0;
    state.coyote_time = 0.0;
    state.slam_start_y = state.position.y;
    state.velocity =
        horizontal(state.velocity) * t.slam_horizontal_keep + Vec3::NEG_Y * t.slam_speed;
    ev.slam_started = true;
}

fn start_slide(
    state: &mut MoveState,
    wish: Vec3,
    facing: Vec3,
    t: &MovementTuning,
    ev: &mut MoveEvents,
) {
    let vh = horizontal(state.velocity);
    let speed = vh.length();
    let dir = if speed > 1.0 {
        vh / speed
    } else if wish.length_squared() > 0.01 {
        wish.normalize()
    } else {
        facing
    };
    state.mode = MoveMode::Slide;
    state.crouched = true;
    state.slide_dir = dir;
    state.velocity = dir * speed.max(t.slide_min_speed);
    ev.slide_started = true;
}

/// Uses a buffered jump if any jump is possible right now. Returns true if it jumped.
fn try_jump(
    state: &mut MoveState,
    wish: Vec3,
    wall: Option<Vec3>,
    t: &MovementTuning,
    ev: &mut MoveEvents,
) -> bool {
    let dash_carry = |state: &mut MoveState, ev: &mut MoveEvents| {
        if state.mode == MoveMode::Dash {
            state.dash_time = 0.0;
            ev.dash_jumped = true;
            state.dash_dir * (t.dash_speed * t.dash_jump_keep).max(state.dash_exit_speed)
        } else {
            horizontal(state.velocity)
        }
    };

    if (state.on_ground || state.coyote_time > 0.0) && state.mode != MoveMode::Slam {
        let mut vh = dash_carry(state, ev);
        if state.mode == MoveMode::Slide {
            vh *= t.slide_jump_boost;
            ev.slide_jumped = true;
            ev.slide_ended = true;
        }
        let vy = if state.slam_bounce_time > 0.0 {
            ev.slam_bounced = true;
            t.slam_bounce_speed + t.slam_bounce_power_bonus * state.slam_bounce_power
        } else {
            state.velocity.y.max(0.0) + t.jump_speed
        };
        state.velocity = vh + Vec3::Y * vy;
        state.on_ground = false;
        state.ground_normal = Vec3::ZERO;
        state.coyote_time = 0.0;
        state.slam_bounce_time = 0.0;
        state.mode = MoveMode::Air;
        ev.jumped = true;
        return true;
    }

    if let Some(n) = wall
        && state.wall_jumps_used < t.wall_jumps
    {
        let mut vh = dash_carry(state, ev);
        let into = vh.dot(n);
        if into < 0.0 {
            vh -= n * into;
        }
        let wish_away = wish - n * wish.dot(n).min(0.0);
        vh += n * t.wall_jump_out_speed + wish_away * t.wall_jump_input_speed;
        state.velocity = vh + Vec3::Y * state.velocity.y.max(t.wall_jump_up_speed);
        state.mode = MoveMode::Air;
        state.wall_jumps_used += 1;
        state.wall_clinging = false;
        ev.wall_jumped = true;
        return true;
    }
    false
}

/// Ground running: near-instant acceleration, slow bleed of overspeed while input is held,
/// strong friction only without input.
fn ground_velocity(vh: Vec3, wish: Vec3, t: &MovementTuning, dt: f32) -> Vec3 {
    let speed = vh.length();
    let wish_len = wish.length();
    if wish_len > 1e-3 {
        let wish_dir = wish / wish_len;
        let target_speed = t.ground_speed * wish_len.min(1.0);
        if speed > target_speed && vh.dot(wish_dir) > 0.0 {
            // Faster than running speed (landed from a dash-jump, slide...): steer like normal
            // but only bleed the extra speed off gradually.
            let new_speed = (speed - t.ground_overspeed_decel * dt).max(target_speed);
            let steered = move_towards(vh, wish_dir * speed, t.ground_accel * dt);
            steered.normalize_or(wish_dir) * new_speed
        } else {
            move_towards(vh, wish_dir * target_speed, t.ground_accel * dt)
        }
    } else if speed < 1e-4 {
        Vec3::ZERO
    } else {
        let control = speed.max(t.ground_stop_speed);
        let new_speed = (speed - control * t.ground_friction * dt).max(0.0);
        vh * (new_speed / speed)
    }
}

/// Quake-style air acceleration. Below `air_max_speed` it accelerates (and can brake); above
/// it, it only *redirects*: speed is never added (no strafe-jump speed gain) and never taken
/// away, so momentum from slides, dashes and explosions survives any air input.
fn air_accelerate(vh: Vec3, wish: Vec3, t: &MovementTuning, dt: f32) -> Vec3 {
    let wish_len = wish.length();
    if wish_len < 1e-3 {
        return vh;
    }
    let dir = wish / wish_len;
    let wish_speed = t.air_max_speed * wish_len.min(1.0);
    let add = wish_speed - vh.dot(dir);
    if add <= 0.0 {
        return vh;
    }
    let new = vh + dir * (t.air_accel * dt).min(add);
    let speed = vh.length();
    let len = new.length();
    if speed > t.air_max_speed {
        if len > 1e-6 { new * (speed / len) } else { vh }
    } else if len > t.air_max_speed {
        new * (t.air_max_speed / len)
    } else {
        new
    }
}

/// Sliding: no friction, slight steering, gravity along slopes, never below `slide_min_speed`.
fn slide_velocity(state: &mut MoveState, wish: Vec3, t: &MovementTuning, dt: f32) {
    let vh = horizontal(state.velocity);
    let speed = vh.length();
    let mut dir = if speed > 0.5 {
        vh / speed
    } else {
        state.slide_dir
    };
    let right = Vec3::new(-dir.z, 0.0, dir.x);
    let steer = wish.dot(right);
    if steer.abs() > 1e-3 {
        dir = (dir + right * (t.slide_steer_degrees.to_radians() * dt * steer)).normalize_or(dir);
    }
    let n = state.ground_normal;
    let downhill = Vec3::new(n.x, 0.0, n.z) * (t.gravity * t.slide_slope_accel);
    let v = dir * speed + downhill * dt;
    let cap = t.slide_max_speed.max(speed);
    let new_speed = v
        .length()
        .clamp(t.slide_min_speed, cap.max(t.slide_min_speed));
    dir = v.normalize_or(dir);
    state.slide_dir = dir;
    state.velocity = dir * new_speed;
}

/// Jump pad / scripted launch.
fn launch(state: &mut MoveState, velocity: Vec3, ev: &mut MoveEvents) {
    if state.mode == MoveMode::Slide {
        ev.slide_ended = true;
    }
    state.velocity = velocity;
    state.mode = MoveMode::Air;
    state.dash_time = 0.0;
    state.slam_bounce_time = 0.0;
    state.coyote_time = 0.0;
    if velocity.y > 0.0 {
        state.on_ground = false;
        state.ground_normal = Vec3::ZERO;
    } else if state.on_ground {
        state.mode = MoveMode::Walk;
    }
}
