//! Simulates a scripted input sequence on `assets/levels/movement_test.ron` and prints a
//! per-tick trace (position, velocity, speed, mode, flags, events) to eyeball movement.
//!
//! ```text
//! cargo run -p shared --example movement_trace -- [scenario] [--every N]
//! cargo run -p shared --example movement_trace -- list
//! ```

use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::LevelDef;
use shared::movement::{self, MoveEnv, MoveEvents, MoveState};
use shared::tuning::{MovementTuning, Tuning};

const DT: f32 = 1.0 / 60.0;

/// One segment of scripted input held for `ticks` ticks.
#[derive(Clone, Copy)]
struct Seg {
    ticks: u32,
    forward: f32,
    right: f32,
    /// Degrees; 0 looks down -Z, positive turns left.
    yaw: f32,
    buttons: Buttons,
}

fn seg(ticks: u32, forward: f32, right: f32, yaw: f32, buttons: Buttons) -> Seg {
    Seg {
        ticks,
        forward,
        right,
        yaw,
        buttons,
    }
}

const NONE: Buttons = Buttons::NONE;
const J: Buttons = Buttons::JUMP;
const C: Buttons = Buttons::CROUCH;
const D: Buttons = Buttons::DASH;

struct Scenario {
    name: &'static str,
    about: &'static str,
    start: Vec3,
    script: Vec<Seg>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "run",
            about: "accelerate, run, release (friction stop)",
            start: Vec3::new(0.0, 0.0, 45.0),
            script: vec![
                seg(10, 0.0, 0.0, 0.0, NONE),
                seg(60, 1.0, 0.0, 0.0, NONE),
                seg(30, 0.0, 0.0, 0.0, NONE),
            ],
        },
        Scenario {
            name: "jump",
            about: "run and jump twice (second jump buffered before landing)",
            start: Vec3::new(0.0, 0.0, 45.0),
            script: vec![
                seg(20, 1.0, 0.0, 0.0, NONE),
                seg(1, 1.0, 0.0, 0.0, J),
                seg(40, 1.0, 0.0, 0.0, NONE),
                seg(1, 1.0, 0.0, 0.0, J),
                seg(60, 1.0, 0.0, 0.0, NONE),
            ],
        },
        Scenario {
            name: "dash",
            about: "dash four times (fourth fails), then dash-jump",
            start: Vec3::new(0.0, 0.0, 45.0),
            script: vec![
                seg(5, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, 0.0, 0.0, D),
                seg(20, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, 1.0, 0.0, D),
                seg(20, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, -1.0, 0.0, D),
                seg(20, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, 0.0, 0.0, D),
                seg(80, 0.0, 0.0, 0.0, NONE),
                seg(1, 1.0, 0.0, 0.0, D),
                seg(4, 1.0, 0.0, 0.0, NONE),
                seg(1, 1.0, 0.0, 0.0, J),
                seg(60, 1.0, 0.0, 0.0, NONE),
            ],
        },
        Scenario {
            name: "slide",
            about: "slide down the long corridor for 6 s, then slide-jump",
            start: Vec3::new(-93.0, 0.0, 45.0),
            script: vec![
                seg(10, 1.0, 0.0, 0.0, NONE),
                seg(360, 1.0, 0.0, 0.0, C),
                seg(1, 1.0, 0.0, 0.0, C | J),
                seg(60, 1.0, 0.0, 0.0, NONE),
            ],
        },
        Scenario {
            name: "slam",
            about: "jump, slam at the apex, bounce out of the landing",
            start: Vec3::new(0.0, 0.0, 45.0),
            script: vec![
                seg(5, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, 0.0, 0.0, J),
                seg(20, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, 0.0, 0.0, C),
                seg(6, 0.0, 0.0, 0.0, NONE),
                seg(1, 0.0, 0.0, 0.0, J),
                seg(90, 0.0, 0.0, 0.0, NONE),
            ],
        },
        Scenario {
            name: "walljump",
            about: "climb the narrow shaft with wall jumps",
            start: Vec3::new(0.0, 0.0, -112.0),
            script: {
                let mut s = vec![
                    seg(5, 0.0, 0.0, 0.0, NONE),
                    seg(1, 0.0, 1.0, 0.0, J),
                    seg(12, 0.0, 1.0, 0.0, NONE),
                ];
                for i in 0..4 {
                    let side = if i % 2 == 0 { -1.0 } else { 1.0 };
                    s.push(seg(1, 0.0, side, 0.0, J));
                    s.push(seg(13, 0.0, side, 0.0, NONE));
                }
                s.push(seg(90, 0.0, 0.0, 0.0, NONE));
                s
            },
        },
        Scenario {
            name: "stairs",
            about: "run up and down the stairs",
            start: Vec3::new(-53.0, 0.0, 16.0),
            script: vec![seg(120, 1.0, 0.0, 0.0, NONE)],
        },
        Scenario {
            name: "ramps",
            about: "run up the 30 degree ramp onto its platform",
            start: Vec3::new(40.0, 0.0, 14.0),
            script: vec![seg(60, 1.0, 0.0, 0.0, NONE), seg(1, 0.0, 0.0, 0.0, NONE)],
        },
        Scenario {
            name: "steep",
            about: "try to run up the 60 degree ramp",
            start: Vec3::new(64.0, 0.0, 14.0),
            script: vec![seg(90, 1.0, 0.0, 0.0, NONE)],
        },
        Scenario {
            name: "tunnel",
            about: "slide into the low tunnel, release crouch inside",
            start: Vec3::new(-28.0, 0.0, 12.0),
            script: vec![
                seg(10, 1.0, 0.0, 0.0, NONE),
                seg(30, 1.0, 0.0, 0.0, C),
                seg(60, 1.0, 0.0, 0.0, NONE),
            ],
        },
        Scenario {
            name: "pad",
            about: "walk onto the jump pad",
            start: Vec3::new(-30.0, 0.0, 46.0),
            script: vec![seg(150, 1.0, 0.0, 0.0, NONE)],
        },
        Scenario {
            name: "gap",
            about: "dash-jump across the pit from the south platform",
            start: Vec3::new(0.0, 2.0, -21.0),
            script: vec![
                seg(10, 1.0, 0.0, 0.0, NONE),
                seg(1, 1.0, 0.0, 0.0, D),
                seg(8, 1.0, 0.0, 0.0, NONE),
                seg(1, 1.0, 0.0, 0.0, J),
                seg(90, 1.0, 0.0, 0.0, NONE),
            ],
        },
    ]
}

fn flags(s: &MoveState) -> String {
    let mut f = String::new();
    f.push(if s.on_ground { 'G' } else { '-' });
    f.push(if s.crouched { 'C' } else { '-' });
    f.push(if s.is_invulnerable() { 'I' } else { '-' });
    f.push(if s.wall_clinging {
        'W'
    } else if s.wall_normal != Vec3::ZERO {
        'w'
    } else {
        '-'
    });
    f
}

fn events(e: &MoveEvents) -> String {
    let mut out = Vec::new();
    let mut add = |on: bool, name: &str| {
        if on {
            out.push(name.to_string());
        }
    };
    add(e.jumped, "jumped");
    add(e.slide_jumped, "slide_jump");
    add(e.dash_jumped, "dash_jump");
    add(e.slam_bounced, "slam_bounce");
    add(e.wall_jumped, "wall_jump");
    add(e.dashed, "dash");
    add(e.dash_failed, "dash_failed");
    add(e.stamina_recharged, "recharged");
    add(e.slide_started, "slide_start");
    add(e.slide_ended, "slide_end");
    add(e.slam_started, "slam");
    add(e.left_ground, "left_ground");
    add(e.footstep, "step");
    add(e.hit_ceiling, "ceiling");
    if let Some(s) = e.slam_landed {
        out.push(format!("slam_landed(power {:.2})", s.power));
    }
    if let Some(v) = e.landed {
        out.push(format!("landed({v:.1})"));
    }
    if let Some(i) = e.jump_pad {
        out.push(format!("jump_pad({i})"));
    }
    if let Some(v) = e.wall_impact {
        out.push(format!("impact({v:.1})"));
    }
    if let Some(h) = e.stepped_up {
        out.push(format!("step_up({h:.2})"));
    }
    out.join(" ")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().map(String::as_str).unwrap_or("all");
    let every: u32 = args
        .iter()
        .position(|a| a == "--every")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);

    let all = scenarios();
    if name == "list" {
        for s in &all {
            println!("{:10} {}", s.name, s.about);
        }
        return;
    }

    let level = match LevelDef::load_named("movement_test") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("can't load movement_test level: {e} (run from the workspace root)");
            std::process::exit(1);
        }
    };
    let collision = level.build_collision();
    let env = MoveEnv {
        world: &collision.world,
        jump_pads: &level.jump_pads,
    };
    let t: MovementTuning = Tuning::load_or_default(&Tuning::default_path()).movement;

    let mut ran = false;
    for sc in all.iter().filter(|s| name == "all" || s.name == name) {
        ran = true;
        println!("=== {}: {} ===", sc.name, sc.about);
        println!(
            "{:>5} {:>27} {:>27} {:>6} {:>5} {:>4} {:>5}  events",
            "tick", "position", "velocity", "hspeed", "mode", "flag", "stam"
        );
        let mut state = MoveState::spawn(sc.start);
        let mut tick = 0u32;
        let mut seq = 0u32;
        for s in &sc.script {
            for _ in 0..s.ticks {
                seq += 1;
                let input = PlayerInput {
                    seq,
                    forward: PlayerInput::quantize_axis(s.forward),
                    right: PlayerInput::quantize_axis(s.right),
                    yaw: s.yaw.to_radians(),
                    buttons: s.buttons,
                    ..Default::default()
                };
                let ev = movement::step(&mut state, &input, &env, &t, DT);
                let evs = events(&ev);
                if tick.is_multiple_of(every) || !evs.is_empty() {
                    let p = state.position;
                    let v = state.velocity;
                    println!(
                        "{tick:>5} ({:>7.2},{:>7.2},{:>8.2}) ({:>7.2},{:>7.2},{:>8.2}) {:>6.2} {:>5} {:>4} {:>5.2}  {evs}",
                        p.x,
                        p.y,
                        p.z,
                        v.x,
                        v.y,
                        v.z,
                        state.horizontal_speed(),
                        format!("{:?}", state.mode),
                        flags(&state),
                        state.stamina(&t),
                    );
                }
                tick += 1;
            }
        }
        println!();
    }
    if !ran {
        eprintln!("unknown scenario '{name}' (try `list`)");
        std::process::exit(2);
    }
}
