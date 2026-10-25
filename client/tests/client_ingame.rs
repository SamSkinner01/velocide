//! The real in-game screen, headless: a single-player session (in-process server) played by
//! the auto-pilot, rendered offscreen. Checks that it connects and plays, that the pause menu
//! freezes the single-player simulation and the level timer, that instant restart puts the
//! player back at the start, and that the main menu renders. Skips without a GPU adapter.

use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::menus::main_menu::MainMenu;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::InputState;
use engine::render::Renderer;
use engine::ui::UiInput;

const W: u32 = 320;
const H: u32 = 180;
const DT: f32 = 1.0 / 60.0;

struct Rig {
    renderer: Renderer,
    common: Common,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
}

impl Rig {
    fn new() -> Option<Rig> {
        let renderer = match Renderer::new_headless(W, H, &RetroSettings::default()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipping: no GPU adapter ({e:#})");
                return None;
            }
        };
        let path =
            std::env::temp_dir().join(format!("velocide-ingame-test-{}.ron", std::process::id()));
        let launch = LaunchOptions {
            bot: true,
            ..Default::default()
        };
        Some(Rig {
            renderer,
            common: Common::new(Settings::default(), path, Audio::disabled(), launch),
            buffers: FrameBuffers::default(),
            input: InputState::new(),
            time: 0.0,
        })
    }

    fn game_frames(&mut self, game: &mut InGame, frames: usize) {
        for _ in 0..frames {
            std::thread::sleep(Duration::from_secs_f32(DT));
            self.time += DT as f64;
            let ui = UiInput {
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
            assert!(
                out.transition.is_none(),
                "unexpected transition {:?}",
                out.transition
            );
            assert!(!out.grab_cursor, "bots never grab the cursor");
            self.renderer
                .render(&self.buffers.desc(&out.scene), None)
                .unwrap();
            self.input.end_frame();
        }
    }
}

#[test]
fn single_player_plays_pauses_and_restarts() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let mut game = InGame::start(
        &StartMode::SinglePlayer {
            level: Some(shared::sim::BUILTIN_LEVEL.into()),
        },
        &mut rig.common,
        &mut rig.renderer,
    )
    .unwrap();
    for _ in 0..300 {
        rig.game_frames(&mut game, 1);
        if game.is_playing() {
            break;
        }
    }
    assert!(game.is_playing(), "connected to the in-process server");
    let spawn = game.net().unwrap().local_state().unwrap().position;
    rig.game_frames(&mut game, 90);
    let net = game.net().unwrap();
    let moved = net.local_state().unwrap().position.distance(spawn);
    assert!(moved > 3.0, "the auto-pilot runs around ({moved:.1} m)");
    assert!(game.level_time() > 1.0);
    assert!(
        !rig.buffers.draws.is_empty(),
        "level and viewmodel are drawn"
    );
    assert!(!rig.buffers.ui.quads.is_empty(), "HUD drawn");
    let image = rig.renderer.read_pixels().unwrap();
    let first = &image.rgba[0..4];
    assert!(
        image.rgba.chunks(4).any(|p| p != first),
        "the frame isn't a flat colour"
    );

    // Pause: the in-process server stops, so do the timer and the snapshots.
    game.open_pause();
    rig.game_frames(&mut game, 20);
    assert!(game.is_sim_paused());
    let t = game.level_time();
    let snaps = game.net().unwrap().stats().snapshots_received;
    let pos = game.net().unwrap().local_state().unwrap().position;
    rig.game_frames(&mut game, 30);
    assert_eq!(game.level_time(), t, "timer frozen");
    assert_eq!(
        game.net().unwrap().stats().snapshots_received,
        snaps,
        "server paused"
    );
    assert_eq!(
        game.net().unwrap().local_state().unwrap().position,
        pos,
        "player frozen"
    );
    game.resume();
    rig.game_frames(&mut game, 30);
    assert!(
        game.net().unwrap().stats().snapshots_received > snaps,
        "server resumed"
    );
    assert!(game.level_time() > t);

    // Instant restart: back at the start with a fresh timer.
    let before = game.level_time();
    let snaps_before = game.net().unwrap().stats().correction_snaps;
    game.restart();
    for _ in 0..120 {
        rig.game_frames(&mut game, 1);
        if game.level_time() < before * 0.5 {
            break;
        }
    }
    assert!(
        game.level_time() < before * 0.5,
        "timer reset ({:.2} -> {:.2})",
        before,
        game.level_time()
    );
    // The teleport reaches prediction with the next snapshot (a snap correction).
    for _ in 0..60 {
        if game.net().unwrap().stats().correction_snaps > snaps_before {
            break;
        }
        rig.game_frames(&mut game, 1);
    }
    let spawns = &game.net().unwrap().world().unwrap().level.spawns;
    let p = game.net().unwrap().local_state().unwrap().position;
    let near = spawns
        .iter()
        .map(|s| s.position.distance(p))
        .fold(f32::MAX, f32::min);
    assert!(near < 6.0, "back near a spawn point ({near:.1} m)");

    game.shutdown(&mut rig.renderer, &mut rig.common);
}

#[test]
fn main_menu_renders() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let mut menu = MainMenu::new(&mut rig.renderer, &rig.common);
    for i in 0..5 {
        rig.buffers.clear(UiDrawList::canvas_for_window(W, H));
        let ui = UiInput {
            time: i as f64 * 0.016,
            ..Default::default()
        };
        let out = {
            let mut io = FrameIo {
                renderer: &mut rig.renderer,
                input: &rig.input,
                ui_input: &ui,
                dt: DT,
                time: i as f64 * 0.016,
                window_size: (W, H),
            };
            menu.frame(&mut rig.common, &mut io, &mut rig.buffers)
        };
        assert!(out.transition.is_none());
        assert!(!out.grab_cursor);
        rig.renderer
            .render(&rig.buffers.desc(&out.scene), None)
            .unwrap();
    }
    assert!(rig.buffers.ui.quads.len() > 50, "menu drawn");
    assert!(!rig.buffers.draws.is_empty(), "3D backdrop drawn");
    menu.free(&mut rig.renderer);
}
