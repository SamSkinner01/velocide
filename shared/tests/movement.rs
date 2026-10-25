//! Headless tests of the movement controller: every move, the character-controller quality
//! guarantees, and determinism (the property client prediction + server reconciliation rely
//! on).

use std::path::Path;

use engine::physics::MASK_MOVEMENT;
use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::{Brush, JumpPad, LevelCollision, LevelDef, Volume};
use shared::movement::{self, MoveEnv, MoveEvents, MoveMode, MoveState};
use shared::tuning::{MovementTuning, Tuning};

const DT: f32 = 1.0 / 60.0;
const NONE: Buttons = Buttons::NONE;
const JUMP: Buttons = Buttons::JUMP;
const CROUCH: Buttons = Buttons::CROUCH;
const DASH: Buttons = Buttons::DASH;

// ------------------------------------------------------------------------------------------
// Helpers

fn brush(min: [f32; 3], max: [f32; 3]) -> Brush {
    Brush {
        min: Vec3::from(min),
        max: Vec3::from(max),
        rotation: Vec3::ZERO,
        material: "concrete".into(),
        tint: Vec3::ONE,
        clip_only: false,
        no_collide: false,
    }
}

/// Big flat floor with its top at y = 0.
fn floor() -> Brush {
    brush([-200.0, -1.0, -200.0], [200.0, 0.0, 200.0])
}

/// A ramp starting at floor level at `z_start` and rising towards -Z to `height`.
/// Returns the brush and the z of its top edge.
fn ramp(x0: f32, x1: f32, z_start: f32, height: f32, degrees: f32) -> (Brush, f32) {
    let th = degrees.to_radians();
    let (s, c) = th.sin_cos();
    let length = height / s + 1.0;
    let z_hi = z_start - height / th.tan();
    let top_z = z_hi + length / 2.0 * c;
    let top_y = height - length / 2.0 * s;
    let (cz, cy) = (top_z - 0.5 * s, top_y - 0.5 * c);
    let (cx, hx) = ((x0 + x1) / 2.0, (x1 - x0) / 2.0);
    let mut b = brush(
        [cx - hx, cy - 0.5, cz - length / 2.0],
        [cx + hx, cy + 0.5, cz + length / 2.0],
    );
    b.rotation = Vec3::new(degrees, 0.0, 0.0);
    (b, z_hi)
}

/// Feet position resting on a ramp made by [`ramp`] at horizontal distance `d` up the slope.
fn on_ramp(z_start: f32, degrees: f32, d: f32, r: f32) -> Vec3 {
    let th = degrees.to_radians();
    let surface = Vec3::new(0.0, d * th.tan(), z_start - d);
    let n = Vec3::new(0.0, th.cos(), th.sin());
    surface + n * (r + 0.005) - Vec3::Y * r
}

struct Level {
    def: LevelDef,
    col: LevelCollision,
}

impl Level {
    fn new(brushes: Vec<Brush>) -> Self {
        Self::with_pads(brushes, Vec::new())
    }

    fn with_pads(brushes: Vec<Brush>, jump_pads: Vec<JumpPad>) -> Self {
        let def = LevelDef {
            brushes,
            jump_pads,
            ..Default::default()
        };
        let col = def.build_collision();
        Self { def, col }
    }

    fn flat() -> Self {
        Self::new(vec![floor()])
    }

    fn test_level() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/levels/movement_test.ron");
        let def = LevelDef::load(&path).expect("movement_test.ron loads");
        let col = def.build_collision();
        Self { def, col }
    }

    fn env(&self) -> MoveEnv<'_> {
        MoveEnv {
            world: &self.col.world,
            jump_pads: &self.def.jump_pads,
        }
    }
}

fn input(forward: f32, right: f32, buttons: Buttons) -> PlayerInput {
    PlayerInput {
        forward: PlayerInput::quantize_axis(forward),
        right: PlayerInput::quantize_axis(right),
        buttons,
        ..Default::default()
    }
}

fn idle() -> PlayerInput {
    input(0.0, 0.0, NONE)
}

struct Sim<'a> {
    s: MoveState,
    t: MovementTuning,
    env: MoveEnv<'a>,
    seq: u32,
}

impl<'a> Sim<'a> {
    /// Spawns without settling (use for airborne starts).
    fn at(level: &'a Level, pos: Vec3) -> Self {
        Self {
            s: MoveState::spawn(pos),
            t: MovementTuning::default(),
            env: level.env(),
            seq: 0,
        }
    }

    /// Spawns and lets the player settle on the ground.
    fn grounded(level: &'a Level, pos: Vec3) -> Self {
        let mut sim = Self::at(level, pos);
        sim.hold(idle(), 5);
        assert!(
            sim.s.on_ground,
            "should be grounded after settling at {pos}"
        );
        sim
    }

    fn tick(&mut self, mut input: PlayerInput) -> MoveEvents {
        self.seq += 1;
        input.seq = self.seq;
        movement::step(&mut self.s, &input, &self.env, &self.t, DT)
    }

    fn hold(&mut self, input: PlayerInput, ticks: u32) -> Vec<MoveEvents> {
        (0..ticks).map(|_| self.tick(input)).collect()
    }

    /// Presses `buttons` for one tick (on top of the held axes).
    fn press(&mut self, forward: f32, right: f32, buttons: Buttons) -> MoveEvents {
        self.tick(input(forward, right, buttons))
    }

    fn hspeed(&self) -> f32 {
        self.s.horizontal_speed()
    }

    fn penetrating(&self) -> bool {
        let (c, hh, r) = self.s.capsule(&self.t);
        self.env
            .world
            .intersects_capsule(c, hh, r - 0.002, MASK_MOVEMENT)
    }

    /// Runs until the player lands (or `max` ticks); returns the max height reached.
    fn until_landed(&mut self, input: PlayerInput, max: u32) -> f32 {
        let mut top = self.s.position.y;
        for _ in 0..max {
            let ev = self.tick(input);
            top = top.max(self.s.position.y);
            if ev.landed.is_some() {
                break;
            }
        }
        top
    }
}

fn bytes(s: &MoveState) -> Vec<u8> {
    bincode::serde::encode_to_vec(s, bincode::config::standard()).expect("encode")
}

/// Tiny deterministic RNG for scripted "random" input.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A long input script that exercises every move (random but reproducible).
fn chaotic_inputs(seed: u64, ticks: usize) -> Vec<PlayerInput> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(ticks);
    let mut yaw = 0.0f32;
    while out.len() < ticks {
        let len = 5 + rng.below(40) as usize;
        let forward = rng.below(3) as f32 - 1.0;
        let right = rng.below(3) as f32 - 1.0;
        let turn = (rng.below(7) as f32 - 3.0) * 0.02;
        let mut held = NONE;
        if rng.below(3) == 0 {
            held.set(CROUCH, true);
        }
        for i in 0..len {
            yaw += turn;
            let mut buttons = held;
            if i == 0 || rng.below(12) == 0 {
                match rng.below(4) {
                    0 => buttons.set(JUMP, true),
                    1 => buttons.set(DASH, true),
                    2 => buttons.set(CROUCH, true),
                    _ => {}
                }
            }
            out.push(PlayerInput {
                seq: out.len() as u32 + 1,
                forward: PlayerInput::quantize_axis(forward),
                right: PlayerInput::quantize_axis(right),
                yaw,
                buttons,
                ..Default::default()
            });
        }
    }
    out.truncate(ticks);
    out
}

/// Runs a script; applies a scripted "explosion" impulse every 150 ticks like the server would.
fn run_script(
    level: &Level,
    start: MoveState,
    inputs: &[PlayerInput],
    first_tick: usize,
) -> Vec<MoveState> {
    let env = level.env();
    let t = MovementTuning::default();
    let mut s = start;
    let mut states = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let tick = first_tick + i;
        if tick % 150 == 149 {
            s.velocity += Vec3::new(3.0, 12.0, -5.0);
        }
        movement::step(&mut s, input, &env, &t, DT);
        states.push(s);
    }
    states
}

// ------------------------------------------------------------------------------------------
// Determinism

#[test]
fn determinism_same_inputs_give_bit_identical_states() {
    let level = Level::test_level();
    for (i, spawn) in level.def.spawns.iter().enumerate().take(4) {
        let inputs = chaotic_inputs(0x9E37_79B9 + i as u64, 1500);
        let a = run_script(&level, MoveState::spawn(spawn.position), &inputs, 0);
        let b = run_script(&level, MoveState::spawn(spawn.position), &inputs, 0);
        for (tick, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(bytes(x), bytes(y), "spawn {i}: diverged at tick {tick}");
        }
    }
}

#[test]
fn replaying_from_a_snapshot_reproduces_the_same_states() {
    // Exactly what reconciliation does: take the authoritative state at some tick (sent over
    // the network), then re-run the inputs after it.
    let level = Level::test_level();
    let inputs = chaotic_inputs(1234, 1500);
    let spawn = level.def.spawns[0].position;
    let full = run_script(&level, MoveState::spawn(spawn), &inputs, 0);
    for cut in [1usize, 100, 377, 750, 1200] {
        let wire = bytes(&full[cut - 1]);
        let (snapshot, _): (MoveState, usize) =
            bincode::serde::decode_from_slice(&wire, bincode::config::standard()).expect("decode");
        assert_eq!(snapshot, full[cut - 1]);
        let replay = run_script(&level, snapshot, &inputs[cut..], cut);
        for (k, s) in replay.iter().enumerate() {
            assert_eq!(
                bytes(s),
                bytes(&full[cut + k]),
                "replay from {cut} diverged at {}",
                cut + k
            );
        }
    }
}

#[test]
fn chaotic_play_on_test_level_never_penetrates_or_explodes() {
    let level = Level::test_level();
    let t = MovementTuning::default();
    let mut seen = std::collections::HashSet::new();
    let mut wall_jumps = 0;
    for (i, spawn) in level.def.spawns.iter().enumerate() {
        let inputs = chaotic_inputs(77 + i as u64, 1200);
        let mut sim = Sim::at(&level, spawn.position);
        for (tick, input) in inputs.iter().enumerate() {
            wall_jumps += sim.tick(*input).wall_jumped as u32;
            let s = &sim.s;
            seen.insert(s.mode);
            assert!(
                s.position.is_finite() && s.velocity.is_finite(),
                "spawn {i} tick {tick}"
            );
            assert!(s.velocity.length() <= t.max_speed + 1e-3);
            assert!(
                s.position.y > -31.0,
                "spawn {i} tick {tick}: fell out of the world"
            );
            assert!(
                !sim.penetrating(),
                "spawn {i} tick {tick}: inside geometry at {}",
                s.position
            );
        }
    }
    // The script really exercised the whole move set.
    for mode in [
        MoveMode::Walk,
        MoveMode::Air,
        MoveMode::Slide,
        MoveMode::Dash,
        MoveMode::Slam,
    ] {
        assert!(seen.contains(&mode), "never saw {mode:?}");
    }
    assert!(wall_jumps > 0, "never wall jumped");
}

// ------------------------------------------------------------------------------------------
// Ground and air

#[test]
fn ground_speed_is_reached_almost_instantly() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let ground_speed = sim.t.ground_speed;
    sim.hold(input(1.0, 0.0, NONE), 7);
    assert!(
        sim.hspeed() >= 0.99 * ground_speed,
        "{} after 0.12 s",
        sim.hspeed()
    );
    for _ in 0..60 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!((sim.hspeed() - ground_speed).abs() < 1e-3);
        assert!(sim.s.on_ground);
    }
}

#[test]
fn friction_stops_quickly_without_input() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.hold(input(1.0, 0.0, NONE), 30);
    sim.hold(idle(), 18);
    assert_eq!(sim.hspeed(), 0.0, "should stop within 0.3 s");
}

#[test]
fn air_strafing_never_loses_or_gains_speed() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 150.0, 0.0));
    sim.s.velocity = Vec3::new(0.0, 0.0, -30.0);
    let mut yaw = 0.0f32;
    for i in 0..90 {
        yaw += 0.03; // turning the mouse while strafing
        let right = if (i / 15) % 2 == 0 { 1.0 } else { -1.0 };
        let forward = if i % 3 == 0 { 1.0 } else { 0.0 };
        let mut inp = input(forward, right, NONE);
        inp.yaw = yaw;
        sim.tick(inp);
        let speed = sim.hspeed();
        assert!(speed >= 30.0 - 1e-3, "tick {i}: lost speed ({speed})");
        assert!(speed <= 30.0 + 1e-3, "tick {i}: gained speed ({speed})");
    }
    // ...but the velocity did turn.
    assert!(sim.s.velocity.x.abs() > 1.0);
}

#[test]
fn air_control_accelerates_up_to_air_max_speed() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 150.0, 0.0));
    let max = sim.t.air_max_speed;
    for _ in 0..60 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!(sim.hspeed() <= max + 1e-3);
    }
    assert!((sim.hspeed() - max).abs() < 1e-3);
}

#[test]
fn jump_height_is_about_two_and_a_half_metres() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let ev = sim.press(0.0, 0.0, JUMP);
    assert!(ev.jumped);
    let top = sim.until_landed(idle(), 200);
    assert!((2.3..2.7).contains(&top), "jump apex {top}");
    assert!(sim.s.on_ground);
}

#[test]
fn ceiling_hit_kills_upward_velocity() {
    let level = Level::new(vec![floor(), brush([-5.0, 2.5, -5.0], [5.0, 3.0, 5.0])]);
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.press(0.0, 0.0, JUMP);
    let mut bonked = false;
    for _ in 0..60 {
        let ev = sim.tick(idle());
        assert!(sim.s.position.y + sim.t.standing_height <= 2.5 + 1e-3);
        if ev.hit_ceiling {
            bonked = true;
            assert!(sim.s.velocity.y <= 0.0);
        }
    }
    assert!(bonked);
    assert!(sim.s.on_ground);
}

// ------------------------------------------------------------------------------------------
// Dash

#[test]
fn dash_distance_duration_iframes_and_exit_speed() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let t = sim.t.clone();
    let start = sim.s.position;
    let ev = sim.press(0.0, 0.0, DASH);
    assert!(ev.dashed);
    assert_eq!(sim.s.stamina(&t), t.dash_charges - 1.0);
    let mut dash_ticks = 1;
    while sim.s.mode == MoveMode::Dash {
        assert!(sim.s.is_invulnerable());
        assert!((sim.hspeed() - t.dash_speed).abs() < 1e-3);
        sim.tick(idle());
        dash_ticks += 1;
        assert!(dash_ticks < 30);
    }
    let duration = dash_ticks as f32 * DT;
    assert!(
        (t.dash_duration - DT..=t.dash_duration + DT).contains(&duration),
        "dash lasted {duration}"
    );
    let dist = (sim.s.position - start).length();
    let expected = t.dash_speed * duration;
    assert!(
        (dist - expected).abs() < 0.2,
        "dash went {dist}, expected ~{expected}"
    );
    // Dashed forward (no input = facing direction, -Z), stayed on the floor.
    assert!(sim.s.position.z < -7.0 && sim.s.position.x.abs() < 1e-3);
    assert!(sim.s.position.y.abs() < 0.02);
    // Momentum is carried out sensibly: not 48 m/s forever, not an abrupt stop.
    assert!(
        (sim.hspeed() - t.dash_end_speed).abs() < 1.0,
        "exit speed {}",
        sim.hspeed()
    );
    sim.hold(idle(), 3);
    assert!(!sim.s.is_invulnerable());
}

#[test]
fn dash_uses_input_direction() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.press(0.0, 1.0, DASH);
    sim.hold(input(0.0, 1.0, NONE), 5);
    assert!(sim.s.position.x > 3.0 && sim.s.position.z.abs() < 1e-3);
}

#[test]
fn dash_charges_regenerate_and_fail_when_empty() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let t = sim.t.clone();
    for i in 0..3 {
        let ev = sim.press(0.0, 0.0, DASH);
        assert!(ev.dashed, "dash {i}");
        sim.hold(idle(), 12);
    }
    assert!(sim.s.stamina(&t) < 1.0);
    let ev = sim.press(0.0, 0.0, DASH);
    assert!(ev.dash_failed && !ev.dashed);
    assert_ne!(sim.s.mode, MoveMode::Dash);

    // One charge comes back after 1 / regen_rate seconds.
    let mut recharged = false;
    let ticks = (1.0 / t.dash_regen_rate / DT).ceil() as u32 + 2;
    for _ in 0..ticks {
        recharged |= sim.tick(idle()).stamina_recharged;
    }
    assert!(recharged);
    assert!(sim.s.stamina(&t) >= 1.0);
    assert!(sim.press(0.0, 0.0, DASH).dashed);
}

#[test]
fn air_dash_ignores_gravity() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 10.0, 0.0));
    sim.hold(idle(), 10); // falling
    let ev = sim.press(1.0, 0.0, DASH);
    assert!(ev.dashed);
    let y = sim.s.position.y;
    while sim.s.mode == MoveMode::Dash {
        assert_eq!(sim.s.position.y, y);
        sim.tick(input(1.0, 0.0, NONE));
    }
    assert!(sim.s.velocity.y <= 0.0);
}

#[test]
fn dash_jump_keeps_a_fraction_of_dash_speed() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let t = sim.t.clone();
    sim.press(1.0, 0.0, DASH);
    sim.hold(input(1.0, 0.0, NONE), 4);
    let ev = sim.press(1.0, 0.0, JUMP);
    assert!(ev.jumped && ev.dash_jumped);
    let expected = t.dash_speed * t.dash_jump_keep;
    assert!((sim.hspeed() - expected).abs() < 0.5, "{}", sim.hspeed());
    sim.until_landed(input(1.0, 0.0, NONE), 200);
    assert!(sim.hspeed() > expected - 0.5, "speed kept through the air");
}

// ------------------------------------------------------------------------------------------
// Slide

#[test]
fn slide_keeps_speed_on_flat_ground_for_five_seconds() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 150.0));
    let t = sim.t.clone();
    sim.hold(input(1.0, 0.0, NONE), 10);
    let ev = sim.press(1.0, 0.0, CROUCH);
    assert!(ev.slide_started);
    assert!(sim.s.crouched);
    let z0 = sim.s.position.z;
    for i in 0..330 {
        sim.tick(input(1.0, 0.0, CROUCH));
        assert_eq!(sim.s.mode, MoveMode::Slide, "tick {i}");
        assert!(
            (sim.hspeed() - t.slide_min_speed).abs() < 0.01,
            "tick {i}: {}",
            sim.hspeed()
        );
        assert_eq!(sim.s.height(&t), t.slide_height);
    }
    let travelled = z0 - sim.s.position.z;
    assert!(travelled > 5.0 * t.slide_min_speed * 0.99, "{travelled}");
    assert!(sim.s.position.x.abs() < 0.01, "slid straight");
}

#[test]
fn slide_keeps_entry_speed_above_the_minimum() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 150.0));
    sim.s.velocity = Vec3::new(0.0, 0.0, -35.0);
    sim.press(0.0, 0.0, CROUCH);
    sim.hold(input(0.0, 0.0, CROUCH), 120);
    assert!((sim.hspeed() - 35.0).abs() < 0.01, "{}", sim.hspeed());
}

#[test]
fn slide_steers_slightly() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 150.0));
    sim.press(1.0, 0.0, CROUCH);
    sim.hold(input(0.0, 1.0, CROUCH), 60);
    let dir = sim.s.velocity.normalize();
    let angle = dir.dot(Vec3::NEG_Z).clamp(-1.0, 1.0).acos().to_degrees();
    assert!(dir.x > 0.0, "steered right");
    assert!(
        (40.0..100.0).contains(&angle),
        "turned {angle} degrees in 1 s"
    );
}

#[test]
fn slide_accelerates_downhill() {
    // Long 12 degree hill descending towards -Z (built rising towards +Z by sliding up it is
    // awkward, so start on top of a ramp that rises towards -Z and slide towards +Z).
    let (hill, z_hi) = ramp(-5.0, 5.0, 0.0, 8.0, 12.0);
    let top = brush([-5.0, 0.0, z_hi - 10.0], [5.0, 8.0, z_hi]);
    let level = Level::new(vec![floor(), hill, top]);
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 8.0, z_hi - 2.0));
    let mut inp = input(1.0, 0.0, CROUCH);
    inp.yaw = std::f32::consts::PI; // facing +Z (downhill)
    sim.tick(inp);
    assert_eq!(sim.s.mode, MoveMode::Slide);
    let mut max_speed: f32 = 0.0;
    for _ in 0..150 {
        sim.tick(inp);
        max_speed = max_speed.max(sim.hspeed());
    }
    assert!(max_speed > sim.t.slide_min_speed + 3.0, "{max_speed}");
}

#[test]
fn slide_under_low_ceiling_stays_crouched_until_clear() {
    // Tunnel with a 1.2 m ceiling from z = -5 to z = -15.
    let level = Level::new(vec![floor(), brush([-3.0, 1.2, -15.0], [3.0, 3.0, -5.0])]);
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 5.0));
    let t = sim.t.clone();
    sim.hold(input(1.0, 0.0, NONE), 5);
    sim.hold(input(1.0, 0.0, CROUCH), 10);
    assert_eq!(sim.s.mode, MoveMode::Slide);
    // Slide into the tunnel holding crouch, then release once fully inside.
    while sim.s.position.z > -7.0 {
        sim.tick(input(1.0, 0.0, CROUCH));
    }
    let mut stood_at = None;
    for _ in 0..60 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!(!sim.penetrating());
        let z = sim.s.position.z;
        if z > -15.0 - t.radius {
            assert!(sim.s.crouched, "stood up under the ceiling at z = {z}");
            assert_eq!(sim.s.mode, MoveMode::Slide, "kept sliding while stuck low");
        } else if stood_at.is_none() && !sim.s.crouched {
            stood_at = Some(z);
        }
    }
    let z = stood_at.expect("stood up after leaving the tunnel");
    assert!(z > -17.0, "stood up promptly ({z})");
    assert_eq!(sim.s.mode, MoveMode::Walk);
}

#[test]
fn slide_jump_carries_slide_speed() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 150.0));
    let t = sim.t.clone();
    sim.press(1.0, 0.0, CROUCH);
    sim.hold(input(1.0, 0.0, CROUCH), 20);
    let ev = sim.press(1.0, 0.0, CROUCH | JUMP);
    assert!(ev.jumped && ev.slide_jumped && ev.slide_ended);
    assert!(sim.hspeed() >= t.slide_min_speed * t.slide_jump_boost - 1e-3);
    // Holding crouch through the jump doesn't slam (crouch wasn't *pressed* in the air)...
    sim.until_landed(input(1.0, 0.0, CROUCH), 200);
    assert!(
        sim.hspeed() >= t.slide_min_speed - 1e-3,
        "kept slide speed in the air"
    );
    // ...and landing with crouch held goes straight back into a slide.
    sim.tick(input(1.0, 0.0, CROUCH));
    assert_eq!(sim.s.mode, MoveMode::Slide);
}

// ------------------------------------------------------------------------------------------
// Slam

#[test]
fn slam_reaches_slam_speed_and_reports_landing() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 12.0, 0.0));
    let t = sim.t.clone();
    sim.s.velocity = Vec3::new(10.0, 0.0, 0.0);
    let ev = sim.press(0.0, 0.0, CROUCH);
    assert!(ev.slam_started);
    assert_eq!(sim.s.velocity.y, -t.slam_speed);
    assert!(sim.hspeed() <= 10.0 * t.slam_horizontal_keep + 1e-3);
    let mut landed = None;
    for _ in 0..30 {
        let ev = sim.tick(input(0.0, 0.0, CROUCH));
        if let Some(s) = ev.slam_landed {
            landed = Some(s);
            assert!(ev.landed.is_some());
            break;
        }
        assert_eq!(sim.s.mode, MoveMode::Slam);
    }
    let slam = landed.expect("slam landed");
    assert!(slam.position.y.abs() < 0.02);
    let expected_power = 12.0 / t.slam_full_power_height;
    assert!(
        (slam.power - expected_power).abs() < 0.05,
        "power {}",
        slam.power
    );
    assert!(sim.s.on_ground);
}

#[test]
fn slam_bounce_is_much_higher_than_a_jump() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.press(0.0, 0.0, JUMP);
    let jump_top = sim.until_landed(idle(), 200);

    let mut sim = Sim::at(&level, Vec3::new(0.0, 6.0, 0.0));
    sim.press(0.0, 0.0, CROUCH);
    sim.until_landed(idle(), 60);
    sim.tick(idle());
    let ev = sim.press(0.0, 0.0, JUMP);
    assert!(ev.jumped && ev.slam_bounced);
    let bounce_top = sim.until_landed(idle(), 300);
    assert!(
        bounce_top > 2.0 * jump_top,
        "bounce {bounce_top} vs jump {jump_top}"
    );
}

#[test]
fn slam_bounce_window_expires() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 6.0, 0.0));
    sim.press(0.0, 0.0, CROUCH);
    sim.until_landed(idle(), 60);
    sim.hold(idle(), (sim.t.slam_bounce_window / DT) as u32 + 2);
    let ev = sim.press(0.0, 0.0, JUMP);
    assert!(ev.jumped && !ev.slam_bounced);
}

#[test]
fn buffered_jump_fires_slam_bounce_on_landing() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 8.0, 0.0));
    sim.press(0.0, 0.0, CROUCH);
    // Jump pressed a few ticks before the ground (slam falls 1.25 m per tick).
    while sim.s.position.y > 3.0 {
        sim.tick(idle());
    }
    sim.press(0.0, 0.0, JUMP);
    let mut bounced = false;
    for _ in 0..10 {
        let ev = sim.tick(idle());
        if ev.slam_bounced {
            bounced = true;
            break;
        }
    }
    assert!(bounced);
}

#[test]
fn crouch_close_to_the_ground_slides_instead_of_slamming() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 0.5, 0.0));
    sim.s.velocity = Vec3::new(0.0, -2.0, -20.0);
    let ev = sim.press(1.0, 0.0, CROUCH);
    assert!(!ev.slam_started);
    for _ in 0..10 {
        sim.tick(input(1.0, 0.0, CROUCH));
    }
    assert_eq!(sim.s.mode, MoveMode::Slide);
    assert!(sim.hspeed() >= sim.t.slide_min_speed - 1e-3);
}

#[test]
fn dash_cancels_a_slam() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 20.0, 0.0));
    sim.press(0.0, 0.0, CROUCH);
    sim.tick(idle());
    assert_eq!(sim.s.mode, MoveMode::Slam);
    let ev = sim.press(1.0, 0.0, DASH);
    assert!(ev.dashed);
    assert_eq!(sim.s.mode, MoveMode::Dash);
    assert!((sim.hspeed() - sim.t.dash_speed).abs() < 1e-3);
}

// ------------------------------------------------------------------------------------------
// Walls

#[test]
fn wall_jumps_are_limited_and_reset_on_landing() {
    // A 3 m wide shaft between two tall walls.
    let level = Level::new(vec![
        floor(),
        brush([-3.5, 0.0, -10.0], [-1.5, 40.0, 10.0]),
        brush([1.5, 0.0, -10.0], [3.5, 40.0, 10.0]),
    ]);
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let t = sim.t.clone();

    // Bounce between the walls, jumping whenever the wall we're heading to is in reach.
    let climb = |sim: &mut Sim| -> (u32, u32) {
        let mut dir = 1.0f32;
        assert!(sim.press(0.0, dir, JUMP).jumped);
        let (mut jumps, mut attempts) = (0, 0);
        let mut pressed = false;
        for _ in 0..600 {
            let at_wall = sim.s.wall_normal.x * dir < 0.0;
            let buttons = if at_wall && !pressed { JUMP } else { NONE };
            pressed = buttons == JUMP;
            let ev = sim.tick(input(0.0, dir, buttons));
            if pressed {
                attempts += 1;
            }
            if ev.wall_jumped {
                jumps += 1;
                dir = -dir;
            }
            if ev.landed.is_some() {
                break;
            }
        }
        (jumps, attempts)
    };
    let (jumps, attempts) = climb(&mut sim);
    assert_eq!(jumps, t.wall_jumps);
    assert!(attempts > jumps, "tried at least once more than allowed");
    assert!(sim.s.on_ground);
    assert_eq!(sim.s.wall_jumps_used, 0);
    let (jumps, _) = climb(&mut sim);
    assert_eq!(jumps, t.wall_jumps, "reset after landing");
}

#[test]
fn wall_jump_pushes_out_and_up() {
    let level = Level::new(vec![floor(), brush([-10.0, 0.0, -3.0], [10.0, 40.0, -1.0])]);
    let mut sim = Sim::at(&level, Vec3::new(0.0, 10.0, -0.5));
    sim.s.velocity = Vec3::new(5.0, -3.0, 0.0);
    sim.tick(input(1.0, 0.0, NONE));
    assert!(
        (sim.s.wall_normal - Vec3::Z).length() < 1e-4,
        "{}",
        sim.s.wall_normal
    );
    let ev = sim.press(1.0, 0.0, JUMP);
    assert!(ev.wall_jumped && !ev.jumped);
    assert!(sim.s.velocity.y > 10.0);
    assert!(sim.s.velocity.z > 8.0, "pushed away from the wall");
    assert!(sim.s.velocity.x > 4.0, "kept speed along the wall");
}

#[test]
fn wall_cling_caps_fall_speed() {
    let level = Level::new(vec![floor(), brush([-10.0, 0.0, -3.0], [10.0, 60.0, -1.0])]);
    let mut sim = Sim::at(&level, Vec3::new(0.0, 40.0, -0.5));
    let t = sim.t.clone();
    sim.s.velocity = Vec3::new(0.0, -20.0, 0.0);
    // Pressing forward = into the wall (yaw 0 looks down -Z).
    for _ in 0..60 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!(sim.s.wall_clinging);
        assert!(sim.s.velocity.y >= -t.wall_cling_max_fall_speed - 1e-4);
    }
    // Looking along the wall (wall on the right at yaw 90 degrees): the view tilts away.
    let roll = movement::view_roll(&sim.s, std::f32::consts::FRAC_PI_2, &t);
    assert!(roll > 0.0, "client tilt while clinging ({roll})");
    // Not pressing into the wall: normal fall.
    sim.hold(idle(), 30);
    assert!(!sim.s.wall_clinging);
    assert!(sim.s.velocity.y < -t.wall_cling_max_fall_speed - 5.0);
}

// ------------------------------------------------------------------------------------------
// Coyote time and jump buffering

/// Floor ends at z = -10 with a 10 m drop.
fn ledge_level() -> Level {
    Level::new(vec![
        brush([-20.0, -1.0, -10.0], [20.0, 0.0, 20.0]),
        brush([-20.0, -11.0, -80.0], [20.0, -10.0, -10.0]),
    ])
}

fn walk_off_and_jump_after(ticks_after_leaving: u32) -> bool {
    let level = ledge_level();
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 5.0));
    let mut left = None;
    for i in 0..200 {
        if sim.tick(input(1.0, 0.0, NONE)).left_ground {
            left = Some(i);
            break;
        }
    }
    assert!(left.is_some(), "walked off");
    assert!(!sim.s.on_ground);
    sim.hold(input(1.0, 0.0, NONE), ticks_after_leaving);
    sim.press(1.0, 0.0, JUMP).jumped
}

#[test]
fn coyote_jump_just_after_walking_off_a_ledge() {
    assert!(walk_off_and_jump_after(0));
    assert!(walk_off_and_jump_after(4));
    assert!(!walk_off_and_jump_after(12), "coyote time expired");
}

#[test]
fn buffered_jump_fires_on_landing() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(0.0, 4.0, 0.0));
    let mut pressed_at = None;
    let mut landed_at = None;
    let mut jumped_at = None;
    for i in 0..120u32 {
        let buttons = if pressed_at.is_none() && sim.s.position.y < 1.2 {
            pressed_at = Some(i);
            JUMP
        } else {
            NONE
        };
        let ev = sim.tick(input(0.0, 0.0, buttons));
        assert!(
            !(ev.jumped && landed_at.is_none() && ev.landed.is_none()),
            "no air jump"
        );
        if ev.landed.is_some() && landed_at.is_none() {
            landed_at = Some(i);
        }
        if ev.jumped {
            jumped_at = Some(i);
            break;
        }
    }
    let (p, l, j) = (
        pressed_at.unwrap(),
        landed_at.expect("landed"),
        jumped_at.expect("buffered jump fired"),
    );
    assert!(p < l, "pressed before landing");
    assert!(
        j - l <= 1,
        "jumped right on landing (landed {l}, jumped {j})"
    );
}

// ------------------------------------------------------------------------------------------
// Character controller quality

#[test]
fn steps_up_a_small_step_but_not_a_high_ledge() {
    let level = Level::new(vec![
        floor(),
        brush([-3.5, 0.0, -30.0], [-0.5, 0.3, -4.0]),
        brush([0.5, 0.0, -30.0], [3.5, 0.6, -4.0]),
    ]);
    // Walking, sliding and dashing all climb the 0.3 m step.
    for (name, buttons) in [("walk", NONE), ("slide", CROUCH), ("dash", DASH)] {
        let mut sim = Sim::grounded(&level, Vec3::new(-2.0, 0.0, 0.0));
        for i in 0..40 {
            let b = if buttons == DASH && i % 12 != 0 {
                NONE
            } else {
                buttons
            };
            sim.tick(input(1.0, 0.0, b));
        }
        assert!(
            sim.s.position.z < -6.0,
            "{name}: got past the step ({})",
            sim.s.position
        );
        assert!(
            (sim.s.position.y - 0.3).abs() < 0.02,
            "{name}: on top ({})",
            sim.s.position
        );
        assert!(sim.s.on_ground);
    }
    let mut sim = Sim::grounded(&level, Vec3::new(2.0, 0.0, 0.0));
    sim.hold(input(1.0, 0.0, NONE), 60);
    assert!(
        sim.s.position.z > -4.0 + 0.3,
        "blocked by the 0.6 m ledge ({})",
        sim.s.position
    );
    assert!(sim.s.position.y < 0.05);
}

#[test]
fn running_down_stairs_stays_grounded() {
    let mut brushes = vec![floor(), brush([-3.0, 0.0, -5.0], [3.0, 3.0, 5.0])];
    for i in 0..9 {
        let h = 3.0 - 0.3 * (i + 1) as f32;
        let z1 = -5.0 - 0.5 * i as f32;
        brushes.push(brush([-3.0, 0.0, z1 - 0.5], [3.0, h, z1]));
    }
    let level = Level::new(brushes);
    for speed_input in [1.0, 0.5] {
        let mut sim = Sim::grounded(&level, Vec3::new(0.0, 3.0, 0.0));
        for i in 0..150 {
            let ev = sim.tick(input(speed_input, 0.0, NONE));
            assert!(
                sim.s.on_ground,
                "left the ground on the stairs at tick {i} ({})",
                sim.s.position
            );
            assert!(!ev.left_ground);
        }
        assert!(sim.s.position.z < -12.0 && sim.s.position.y.abs() < 0.02);
    }
}

#[test]
fn walking_slowly_up_stairs_still_climbs() {
    // Each step stops the player dead for a tick; the next (small) move must still climb it.
    let mut brushes = vec![floor()];
    for i in 0..10 {
        let z1 = -2.0 - 0.5 * i as f32;
        brushes.push(brush(
            [-3.0, 0.0, z1 - 0.5],
            [3.0, 0.3 * (i + 1) as f32, z1],
        ));
    }
    brushes.push(brush([-3.0, 0.0, -30.0], [3.0, 3.0, -7.0]));
    let level = Level::new(brushes);
    for speed in [0.15, 0.4, 1.0] {
        let mut sim = Sim::grounded(&level, Vec3::ZERO);
        sim.hold(input(speed, 0.0, NONE), (6.0 / speed) as u32 * 10);
        assert!(
            (sim.s.position.y - 3.0).abs() < 0.02 && sim.s.position.z < -7.5,
            "input {speed}: stuck on the stairs at {}",
            sim.s.position
        );
    }
}

#[test]
fn sliding_never_loses_distance_over_seams_and_through_the_tunnel() {
    // A tick's motion must never be eaten by spurious zero-distance hits (brush seams, the
    // tunnel entrance, resting inside the skin band...).
    let level = Level::test_level();
    let t = MovementTuning::default();
    for (start, ticks) in [
        (Vec3::new(-93.0, 0.0, 45.0), 400),
        (Vec3::new(-28.0, 0.0, 12.0), 60),
    ] {
        let mut sim = Sim::grounded(&level, start);
        sim.press(1.0, 0.0, CROUCH);
        for i in 0..ticks {
            let before = sim.s.position;
            sim.tick(input(1.0, 0.0, CROUCH));
            let moved = (sim.s.position - before).length();
            let expected = sim.hspeed() * DT;
            assert!(
                (moved - expected).abs() < 1e-3,
                "{start} tick {i}: moved {moved}, expected {expected}"
            );
            assert_eq!(sim.s.mode, MoveMode::Slide);
            assert!((sim.hspeed() - t.slide_min_speed).abs() < 1e-3);
        }
    }
}

#[test]
fn walkable_ramp_is_climbed_at_full_speed() {
    let (r, z_hi) = ramp(-4.0, 4.0, 0.0, 4.0, 30.0);
    let level = Level::new(vec![
        floor(),
        r,
        brush([-4.0, 0.0, z_hi - 10.0], [4.0, 4.0, z_hi]),
    ]);
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 6.0));
    sim.hold(input(1.0, 0.0, NONE), 10);
    for _ in 0..60 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!(sim.s.on_ground);
        assert!(sim.hspeed() > sim.t.ground_speed - 1e-3);
    }
    assert!(
        (sim.s.position.y - 4.0).abs() < 0.02,
        "on the platform ({})",
        sim.s.position
    );
}

#[test]
fn steep_slope_is_not_walkable() {
    let (r, _) = ramp(-4.0, 4.0, 0.0, 6.0, 60.0);
    let level = Level::new(vec![floor(), r]);
    let t = MovementTuning::default();
    let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, 5.0));
    for _ in 0..120 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!(
            sim.s.position.y < t.step_height,
            "climbed the 60 degree slope ({})",
            sim.s.position
        );
    }
    // Standing on it: slides off.
    let mut sim = Sim::at(&level, on_ramp(0.0, 60.0, 2.0, t.radius));
    let y0 = sim.s.position.y;
    sim.tick(idle());
    assert!(!sim.s.on_ground);
    sim.hold(idle(), 60);
    assert!(
        sim.s.position.y < y0 - 2.0,
        "slid down ({} -> {})",
        y0,
        sim.s.position.y
    );
}

#[test]
fn landing_on_a_ramp_keeps_horizontal_speed() {
    let (r, _) = ramp(-4.0, 4.0, 0.0, 6.0, 25.0);
    let level = Level::new(vec![floor(), r]);
    let mut sim = Sim::at(&level, on_ramp(0.0, 25.0, 4.0, 0.4) + Vec3::Y * 1.0);
    sim.s.velocity = Vec3::new(0.0, -6.0, -16.0); // running uphill
    sim.until_landed(input(1.0, 0.0, NONE), 60);
    assert!(sim.s.on_ground);
    assert!(sim.hspeed() > 15.9, "{}", sim.hspeed());
}

#[test]
fn no_tunnelling_when_dashing_into_a_thin_wall() {
    let level = Level::new(vec![floor(), brush([-5.0, 0.0, -3.2], [5.0, 4.0, -3.0])]);
    let limit = -3.0 + 0.4 - 0.02;
    for k in 0..16 {
        let z0 = -2.55 + k as f32 * 0.37;
        let mut sim = Sim::grounded(&level, Vec3::new(0.0, 0.0, z0));
        for i in 0..40 {
            let b = if i % 12 == 0 { DASH } else { NONE };
            sim.tick(input(1.0, 0.0, b));
            assert!(
                sim.s.position.z > limit,
                "start {z0}: through the wall ({})",
                sim.s.position
            );
        }
    }
    // Airborne at the absolute max speed.
    for k in 0..8 {
        let mut sim = Sim::at(&level, Vec3::new(0.0, 1.0, 10.0 + k as f32 * 0.31));
        for _ in 0..20 {
            sim.s.velocity = Vec3::new(0.0, 0.0, -sim.t.max_speed);
            sim.tick(idle());
            assert!(
                sim.s.position.z > limit,
                "through the wall at max speed ({})",
                sim.s.position
            );
        }
    }
}

#[test]
fn no_tunnelling_when_falling_onto_a_thin_floor() {
    let level = Level::new(vec![brush([-5.0, -0.2, -5.0], [5.0, 0.0, 5.0])]);
    // Free fall (reaches max fall speed), slam, and an external max-speed impulse.
    for case in 0..3 {
        let mut sim = Sim::at(&level, Vec3::new(0.0, 300.0 + case as f32 * 0.37, 0.0));
        if case == 1 {
            sim.press(0.0, 0.0, CROUCH);
        }
        let mut max_fall: f32 = 0.0;
        for _ in 0..600 {
            if case == 2 {
                sim.s.velocity.y = sim.s.velocity.y.min(-sim.t.max_speed);
            }
            sim.tick(idle());
            max_fall = max_fall.max(-sim.s.velocity.y);
            assert!(
                sim.s.position.y > -0.01,
                "case {case}: fell through ({})",
                sim.s.position
            );
            if sim.s.on_ground {
                break;
            }
        }
        assert!(sim.s.on_ground, "case {case}");
        assert!(max_fall >= 70.0, "case {case}: was fast ({max_fall})");
    }
}

#[test]
fn resting_on_the_ground_does_not_drift() {
    let (r, _) = ramp(-4.0, 4.0, 0.0, 6.0, 30.0);
    let level = Level::new(vec![floor(), r]);
    let t = MovementTuning::default();
    for start in [Vec3::new(20.0, 0.0, 0.0), on_ramp(0.0, 30.0, 3.0, t.radius)] {
        let mut sim = Sim::grounded(&level, start);
        sim.hold(idle(), 10);
        let p0 = sim.s.position;
        for i in 0..600 {
            let ev = sim.tick(idle());
            assert!(sim.s.on_ground, "tick {i}");
            assert!(ev.landed.is_none() && !ev.left_ground);
        }
        let drift = (sim.s.position - p0).length();
        assert!(drift < 1e-4, "drifted {drift} from {start}");
        assert_eq!(sim.s.velocity, Vec3::ZERO);
    }
}

#[test]
fn small_bumps_dont_launch_at_high_speed() {
    let level = Level::new(vec![floor(), brush([-3.0, 0.0, -20.0], [3.0, 0.05, -19.0])]);
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.press(1.0, 0.0, DASH);
    for _ in 0..50 {
        sim.tick(input(1.0, 0.0, NONE));
        assert!(sim.s.position.y < 0.1, "launched ({})", sim.s.position);
        assert!(sim.s.velocity.y <= 0.0);
    }
    assert!(sim.s.position.z < -19.0);
}

#[test]
fn upward_impulse_while_grounded_leaves_the_ground() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.s.velocity += Vec3::new(4.0, 15.0, 0.0); // explosion, applied by the server
    let ev = sim.tick(idle());
    assert!(ev.left_ground);
    assert!(!sim.s.on_ground);
    assert!(sim.s.position.y > 0.2);
    let top = sim.until_landed(idle(), 200);
    let expected = 15.0 * 15.0 / (2.0 * sim.t.gravity);
    assert!(
        (top - expected).abs() < 0.3,
        "apex {top}, expected ~{expected}"
    );
    assert!(
        sim.s.position.x > 1.0,
        "kept horizontal knockback in the air"
    );
}

#[test]
fn jump_pad_launches_once_on_entry() {
    let pad = JumpPad {
        trigger: Volume {
            min: Vec3::new(-2.0, 0.0, -6.0),
            max: Vec3::new(2.0, 1.0, -4.0),
        },
        velocity: Vec3::new(0.0, 25.0, -5.0),
    };
    let level = Level::with_pads(vec![floor()], vec![pad]);
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    let mut fired = 0;
    for _ in 0..200 {
        let ev = sim.tick(input(1.0, 0.0, NONE));
        if let Some(i) = ev.jump_pad {
            fired += 1;
            assert_eq!(i, 0);
            assert_eq!(sim.s.velocity, Vec3::new(0.0, 25.0, -5.0));
            assert!(!sim.s.on_ground);
        }
    }
    assert_eq!(fired, 1);
}

#[test]
fn depenetrates_when_spawned_inside_geometry() {
    let level = Level::new(vec![floor(), brush([-1.0, 0.0, -1.0], [1.0, 5.0, 1.0])]);
    let mut sim = Sim::at(&level, Vec3::new(5.0, -0.3, 0.0));
    sim.tick(idle());
    assert!(
        sim.s.position.y > -0.001 && !sim.penetrating(),
        "{}",
        sim.s.position
    );
    let mut sim = Sim::at(&level, Vec3::new(0.8, 0.05, 0.0));
    sim.tick(idle());
    assert!(!sim.penetrating(), "{}", sim.s.position);
    assert!(sim.s.position.x > 1.35, "{}", sim.s.position);
}

#[test]
fn spawning_exactly_on_the_floor_is_clean() {
    let level = Level::flat();
    let mut sim = Sim::at(&level, Vec3::new(3.7, 0.0, -11.3));
    sim.hold(idle(), 30);
    assert!(sim.s.on_ground);
    let moved = sim.s.position - Vec3::new(3.7, 0.0, -11.3);
    assert!(moved.length() < 0.02, "{moved}");
    sim.hold(input(1.0, 0.0, NONE), 30);
    assert!((sim.hspeed() - sim.t.ground_speed).abs() < 1e-3);
    assert!(sim.s.velocity.x.abs() < 1e-3);
}

// ------------------------------------------------------------------------------------------
// Test level, camera helpers, serialisation

#[test]
fn test_level_spawns_are_valid() {
    let level = Level::test_level();
    assert!(level.def.spawns.len() >= 8);
    assert!(!level.def.jump_pads.is_empty());
    assert!(!level.def.kill_volumes.is_empty());
    for spawn in &level.def.spawns {
        let mut sim = Sim::at(&level, spawn.position);
        sim.hold(idle(), 20);
        assert!(sim.s.on_ground, "spawn {}", spawn.position);
        assert!(!sim.penetrating());
        let moved = sim.s.position - spawn.position;
        assert!(
            moved.length() < 0.05,
            "spawn {} moved {moved}",
            spawn.position
        );
    }
}

#[test]
fn tuning_file_has_every_movement_value_and_matches_the_defaults() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
    let text = std::fs::read_to_string(&path).expect("assets/tuning.ron exists");
    let tuning: Tuning = ron::from_str(&text).expect("assets/tuning.ron parses");
    assert_eq!(tuning.movement, MovementTuning::default());
    // Every field is spelled out in the file (not silently defaulted).
    let defaults = ron::to_string(&MovementTuning::default()).unwrap();
    let fields = defaults
        .trim_matches(|c| c == '(' || c == ')')
        .split(',')
        .filter_map(|kv| kv.split(':').next())
        .map(str::trim)
        .filter(|k| !k.is_empty());
    for field in fields {
        assert!(
            text.contains(&format!("{field}:")),
            "assets/tuning.ron is missing `{field}`"
        );
    }
}

#[test]
fn camera_helpers() {
    let t = MovementTuning::default();
    let mut s = MoveState::spawn(Vec3::ZERO);
    assert_eq!(movement::fov_kick(&s, &t), 0.0);
    s.velocity = Vec3::new(0.0, 0.0, -100.0);
    assert!((movement::fov_kick(&s, &t) - t.fov_kick_degrees).abs() < 1e-4);
    s.velocity = Vec3::new(0.0, 0.0, -30.0);
    let mid = movement::fov_kick(&s, &t);
    assert!(mid > 0.0 && mid < t.fov_kick_degrees);

    // Slide straight ahead: slight lean.
    s.mode = MoveMode::Slide;
    s.on_ground = true;
    assert!(movement::view_roll(&s, 0.0, &t) > 0.0);
    // Wall on the right (normal points left, -X at yaw 0): tilt left (positive).
    s.mode = MoveMode::Air;
    s.on_ground = false;
    s.wall_normal = Vec3::NEG_X;
    s.wall_clinging = true;
    let roll = movement::view_roll(&s, 0.0, &t);
    assert!(
        (roll - t.wall_tilt_degrees.to_radians()).abs() < 1e-5,
        "{roll}"
    );
    s.wall_normal = Vec3::X;
    assert!(movement::view_roll(&s, 0.0, &t) < 0.0);

    assert_eq!(movement::landing_dip(2.0, &t), 0.0);
    assert!((movement::landing_dip(60.0, &t) - t.landing_dip).abs() < 1e-6);
}

#[test]
fn state_round_trips_through_bincode_and_ron() {
    let level = Level::flat();
    let mut sim = Sim::grounded(&level, Vec3::ZERO);
    sim.press(1.0, 0.0, DASH);
    sim.hold(input(1.0, 0.0, CROUCH), 3);
    let s = sim.s;
    let (back, _): (MoveState, usize) =
        bincode::serde::decode_from_slice(&bytes(&s), bincode::config::standard()).unwrap();
    assert_eq!(back, s);
    let text = ron::to_string(&s).unwrap();
    assert_eq!(ron::from_str::<MoveState>(&text).unwrap(), s);
}
