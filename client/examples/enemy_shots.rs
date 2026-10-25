//! Headless screenshots of the enemies in action (docs/enemies.md): a real single-player
//! session (in-process server) driven by dev commands — Spitters winding up and spitting,
//! Gnats hovering and firing, a Bastion's slam wind-up and parry window — then an arena
//! encounter on the campaign level (doors locked, a wave spawning, the HUD readout).
//!
//! ```text
//! cargo run -p client --release --example enemy_shots -- [out_dir]   # default target/enemy_shots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::InputState;
use engine::render::Renderer;
use engine::ui::UiInput;
use glam::{Vec2, Vec3};
use shared::input::Buttons;
use shared::protocol::{EntityState, enemy_flags, kind};
use shared::sim::AiState;

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

struct Harness {
    renderer: Renderer,
    common: Common,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
    out: PathBuf,
}

impl Harness {
    fn frame(&mut self, game: &mut InGame) {
        std::thread::sleep(Duration::from_secs_f32(DT));
        self.time += DT as f64;
        self.common.stats.update(DT);
        let ui = UiInput {
            mouse_pos: Vec2::splat(-100.0),
            time: self.time,
            ..Default::default()
        };
        self.buffers.clear(UiDrawList::canvas_for_window(W, H));
        let out = {
            let mut io = FrameIo {
                renderer: &mut self.renderer,
                input: &self.input,
                ui_input: &ui,
                dt: DT,
                time: self.time,
                window_size: (W, H),
            };
            game.frame(&mut self.common, &mut io, &mut self.buffers)
        };
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            eprintln!("render: {e:#}");
        }
        self.input.end_frame();
    }

    fn frames(&mut self, game: &mut InGame, n: usize) {
        for _ in 0..n {
            hold(game);
            self.frame(game);
        }
    }

    fn shot(&mut self, name: &str) {
        match self.renderer.read_pixels() {
            Ok(img) => {
                let path = self.out.join(format!("{name}.png"));
                match engine::assets::save_png(&path, &img) {
                    Ok(()) => println!("wrote {}", path.display()),
                    Err(e) => eprintln!("{name}: {e:#}"),
                }
            }
            Err(e) => eprintln!("{name}: {e:#}"),
        }
    }

    /// Runs frames (aiming at the nearest `kind` enemy) until `pred` holds; returns whether it
    /// did within `max` frames.
    fn until(
        &mut self,
        game: &mut InGame,
        kind: u16,
        max: usize,
        pred: impl Fn(&EntityState) -> bool,
    ) -> bool {
        for _ in 0..max {
            if let Some(t) = nearest(game, kind) {
                aim_at(game, chest(&t));
            }
            hold(game);
            self.frame(game);
            let hit = game.net().is_some_and(|n| {
                n.remote_entities()
                    .iter()
                    .any(|e| e.kind == kind && e.flags & enemy_flags::DYING == 0 && pred(e))
            });
            if hit {
                return true;
            }
        }
        false
    }
}

fn eye(game: &InGame) -> Option<Vec3> {
    let net = game.net()?;
    Some(net.local_state()?.eye_position(&net.tuning().movement))
}

/// Where to look at an enemy (its body centre; Gnats hover 0.45 m above their feet).
fn chest(e: &EntityState) -> Vec3 {
    e.position
        + Vec3::Y
            * match e.kind {
                kind::GNAT => 0.45,
                kind::BASTION => 1.7,
                kind::SPITTER => 1.0,
                _ => 1.0,
            }
}

fn nearest(game: &InGame, k: u16) -> Option<EntityState> {
    let net = game.net()?;
    let me = eye(game)?;
    net.remote_entities()
        .iter()
        .filter(|e| e.kind == k && e.flags & enemy_flags::DYING == 0)
        .min_by(|a, b| a.position.distance(me).total_cmp(&b.position.distance(me)))
        .copied()
}

fn aim_at(game: &mut InGame, target: Vec3) {
    let Some(from) = eye(game) else {
        return;
    };
    let d = (target - from).normalize();
    game.controls.yaw = (-d.x).atan2(-d.z);
    game.controls.pitch = d.y.clamp(-1.0, 1.0).asin();
}

fn hold(game: &mut InGame) {
    game.controls.override_move = Some((0.0, 0.0, Buttons::NONE));
}

fn windup_progress(e: &EntityState) -> u32 {
    if e.state == AiState::Windup as u16 {
        e.aux & 0xFF
    } else {
        0
    }
}

fn session(h: &mut Harness, level: &str) -> anyhow::Result<InGame> {
    let mut game = InGame::start(
        &StartMode::SinglePlayer {
            level: Some(level.into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.frames(&mut game, 200);
    anyhow::ensure!(game.is_playing(), "not in game");
    game.send_chat("/god");
    h.frames(&mut game, 10);
    Ok(game)
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/enemy_shots"));
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let scratch = std::env::temp_dir().join("velocide-enemy-shots-settings.ron");
    let mut h = Harness {
        renderer,
        common: Common::new(
            Settings::default(),
            scratch,
            Audio::disabled(),
            LaunchOptions::default(),
        ),
        buffers: FrameBuffers::default(),
        input: InputState::new(),
        time: 10.0,
        out,
    };

    // ---- Proving ground: one type at a time. ----
    let mut game = session(&mut h, shared::sim::BUILTIN_LEVEL)?;

    // Spitters: alert, strafe, wind up (sac glows), spit.
    game.send_chat("/spawn spitter 2");
    h.frames(&mut game, 20);
    h.shot("spitter_spawn");
    let ok = h.until(&mut game, kind::SPITTER, 600, |e| windup_progress(e) > 170);
    println!("spitter wind-up: {ok}");
    h.shot("spitter_windup");
    let ok = h.until(&mut game, kind::SPITTER, 120, |e| e.state == AiState::Strike as u16);
    println!("spitter strike: {ok}");
    h.frames(&mut game, 8);
    h.shot("spitter_orb");
    game.send_chat("/killall");
    h.frames(&mut game, 60);

    // Gnats: hover at 4-8 m, circle, burst of bolts.
    game.send_chat("/spawn gnat 3");
    h.frames(&mut game, 150);
    if let Some(g) = nearest(&game, kind::GNAT) {
        aim_at(&mut game, chest(&g));
    }
    h.frames(&mut game, 2);
    h.shot("gnats_hovering");
    let ok = h.until(&mut game, kind::GNAT, 600, |e| windup_progress(e) > 150);
    println!("gnat wind-up: {ok}");
    h.shot("gnat_windup");
    let ok = h.until(&mut game, kind::GNAT, 60, |e| e.state == AiState::Strike as u16);
    println!("gnat strike: {ok}");
    h.frames(&mut game, 6);
    h.shot("gnat_bolts");
    game.send_chat("/killall");
    h.frames(&mut game, 60);

    // Bastion: slow approach, slam wind-up, parry window (yellow), slam.
    game.send_chat("/spawn bastion 1");
    h.frames(&mut game, 30);
    h.shot("bastion_spawn");
    let ok = h.until(&mut game, kind::BASTION, 900, |e| windup_progress(e) > 60);
    println!("bastion wind-up: {ok}");
    h.shot("bastion_windup");
    let ok = h.until(&mut game, kind::BASTION, 60, |e| {
        e.flags & enemy_flags::PARRYABLE != 0
    });
    println!("bastion parry window: {ok}");
    h.frames(&mut game, 6);
    h.shot("bastion_parryable");
    let ok = h.until(&mut game, kind::BASTION, 60, |e| e.state == AiState::Strike as u16);
    println!("bastion slam: {ok}");
    h.frames(&mut game, 3);
    h.shot("bastion_slam");
    // A mixed crowd.
    game.send_chat("/spawn husk 4");
    game.send_chat("/spawn spitter 2");
    game.send_chat("/spawn gnat 2");
    h.frames(&mut game, 180);
    if let Some(b) = nearest(&game, kind::BASTION) {
        aim_at(&mut game, chest(&b));
    }
    h.frames(&mut game, 2);
    h.shot("mixed_crowd");
    game.shutdown(&mut h.renderer, &mut h.common);

    // ---- Campaign: an arena encounter (Crucible Heart: the bastion on the island). ----
    let mut game = session(&mut h, "crucible_of_veins")?;
    game.send_chat("/encounter 4");
    h.frames(&mut game, 20);
    h.shot("arena4_locked");
    h.frames(&mut game, 60);
    h.shot("arena4_wave1");
    let ok = h.until(&mut game, kind::BASTION, 600, |e| windup_progress(e) > 100);
    println!("arena bastion wind-up: {ok}");
    h.shot("arena4_bastion");
    game.send_chat("/killall");
    h.frames(&mut game, 150);
    h.shot("arena4_wave2");
    game.send_chat("/killall");
    h.frames(&mut game, 30);
    game.send_chat("/encounters");
    h.frames(&mut game, 60);
    h.shot("arena4_cleared");
    game.shutdown(&mut h.renderer, &mut h.common);
    Ok(())
}
