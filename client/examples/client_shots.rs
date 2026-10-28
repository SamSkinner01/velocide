//! Renders the real client screens headless (GPU, no window) with scripted input and saves
//! PNGs: every main-menu page (the LOADOUT page also with its ability, weapon and knack
//! pickers and the HUNTER preset loaded), each settings tab, a notice, the update prompt (a new
//! release, its download, a failure), single-player gameplay (bot driven),
//! the pause menu (+ its loadout and settings pages), chat, the scoreboard and the connecting
//! screen. Used to check layout and look without clicking through the game.
//!
//! ```text
//! cargo run -p client --release --example client_shots -- [out_dir]   # default target/client_shots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo, ScreenOutput};
use client::ingame::InGame;
use client::menus::main_menu::MainMenu;
use client::settings::Settings;
use client::update::{Latest, Release, State, UpdateConfig, Updater};
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::{GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use glam::Vec2;

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

enum Screen<'a> {
    Menu(&'a mut MainMenu),
    Game(&'a mut InGame),
}

impl Harness {
    fn ui(&self, keys: &[UiKey], text: &str) -> UiInput {
        UiInput {
            mouse_pos: Vec2::splat(-100.0),
            keys: keys.to_vec(),
            text: text.to_string(),
            time: self.time,
            ..Default::default()
        }
    }

    /// One frame of a screen with scripted UI keys; renders it.
    fn frame(
        &mut self,
        screen: Screen,
        keys: &[UiKey],
        text: &str,
        realtime: bool,
    ) -> ScreenOutput {
        if realtime {
            std::thread::sleep(Duration::from_secs_f32(DT));
        }
        self.time += DT as f64;
        self.common.stats.update(DT);
        let ui = self.ui(keys, text);
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
            match screen {
                Screen::Menu(m) => m.frame(&mut self.common, &mut io, &mut self.buffers),
                Screen::Game(g) => g.frame(&mut self.common, &mut io, &mut self.buffers),
            }
        };
        if self.common.settings_changed {
            self.common.settings_changed = false;
            self.common.apply_display_settings(&mut self.renderer);
        }
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            eprintln!("render: {e:#}");
        }
        self.input.end_frame();
        out
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

    fn menu(&mut self, menu: &mut MainMenu, script: &[&[UiKey]]) {
        for keys in script {
            self.frame(Screen::Menu(menu), keys, "", false);
        }
        for _ in 0..3 {
            self.frame(Screen::Menu(menu), &[], "", false);
        }
    }

    fn game(&mut self, game: &mut InGame, frames: usize, keys: &[UiKey], text: &str) {
        for i in 0..frames {
            let (k, t) = if i == 0 { (keys, text) } else { (&[][..], "") };
            self.frame(Screen::Game(game), k, t, true);
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/client_shots"));
    std::fs::create_dir_all(&out)?;
    let settings = Settings::default();
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let launch = LaunchOptions {
        bot: true,
        ..Default::default()
    };
    let scratch = std::env::temp_dir().join("velocide-client-shots-settings.ron");
    let mut h = Harness {
        renderer,
        common: Common::new(settings, scratch, Audio::disabled(), launch),
        buffers: FrameBuffers::default(),
        input: InputState::new(),
        time: 10.0,
        out,
    };

    // ---- main menu ----------------------------------------------------------------------------
    use UiKey::*;
    // (The CUSTOMISE and LOADOUT pages turn models.)
    h.common.load_models(&mut h.renderer);
    let mut menu = MainMenu::new(&mut h.renderer, &h.common);
    h.menu(&mut menu, &[&[], &[]]);
    h.shot("menu_root");
    h.menu(&mut menu, &[&[Enter]]);
    h.shot("menu_single_player");
    h.menu(&mut menu, &[&[Escape], &[], &[Down], &[Enter]]);
    h.shot("menu_host");
    h.menu(&mut menu, &[&[Escape], &[], &[Down], &[Enter]]);
    h.shot("menu_join");
    h.menu(&mut menu, &[&[Escape], &[], &[Down], &[Enter]]);
    h.shot("menu_customize");
    h.menu(&mut menu, &[&[Escape], &[], &[Down], &[Enter]]);
    h.shot("menu_loadout");
    // Slot card → (Right) CHANGE WEAPON → (Down) ability box A → its picker.
    h.menu(&mut menu, &[&[Right], &[Down], &[Enter]]);
    h.shot("menu_loadout_ability");
    // Back to the showcase, then the weapon picker with the Trimmer (melee, the last column)
    // under the focus.
    h.menu(
        &mut menu,
        &[
            &[Escape],
            &[Up],
            &[Enter],
            &[],
            &[Right],
            &[Right],
            &[Right],
            &[Right],
            &[Right],
        ],
    );
    h.shot("menu_loadout_weapon");
    // Cancel; the knack card (Up from CHANGE WEAPON) opens the knack picker on the Thorn
    // (PvE only: Surge in PvP), then the Snare.
    h.menu(&mut menu, &[&[Escape], &[Up], &[Enter]]);
    h.shot("menu_loadout_knack_thorn");
    h.menu(&mut menu, &[&[Down], &[Down]]);
    h.shot("menu_loadout_knack");
    // Cancel; load the HUNTER preset (PRESET choice → next, LOAD) and look at its bow.
    h.menu(
        &mut menu,
        &[&[Escape], &[Down; 4], &[Right], &[Down], &[Enter], &[]],
    );
    h.shot("menu_loadout_hunter");
    h.menu(&mut menu, &[&[Escape], &[], &[Down], &[Enter]]);
    h.shot("settings_video");
    for (i, name) in [
        "settings_audio",
        "settings_controls",
        "settings_gameplay",
        "settings_accessibility",
    ]
    .iter()
    .enumerate()
    {
        let _ = i;
        h.menu(&mut menu, &[&[Right]]);
        h.shot(name);
    }
    // Scroll the controls page down to the bindings.
    h.menu(&mut menu, &[&[Left], &[Left], &[Down; 6]]);
    h.shot("settings_controls_bindings");
    h.menu(&mut menu, &[&[Escape]]);
    menu.show_message(Some("Disconnected: connection timed out".into()));
    h.menu(&mut menu, &[]);
    h.shot("menu_notice");
    // The update prompt (after the notice is closed): a newer release, its download, a
    // failure. Fixed states: nothing goes online.
    h.menu(&mut menu, &[&[Escape]]);
    let latest = Latest {
        release: Release {
            tag_name: "build-43".into(),
            target_commitish: "9f3c2e1d0b4a5968776655443322110fedcba987".into(),
            html_url: "https://github.com/SamSkinner01/velocide/releases/tag/build-43".into(),
            assets: Vec::new(),
        },
        commit: "9f3c2e1d0b4a5968776655443322110fedcba987".into(),
    };
    let config = UpdateConfig {
        api: "http://127.0.0.1:9".into(),
        repo: "SamSkinner01/velocide".into(),
        version: "build-42".into(),
        commit: "4c7d21aa93be0f6e1d2c3b4a5968778899aabbcc".into(),
        exe: std::env::temp_dir().join("velocide-client-shots-exe"),
    };
    for (name, state) in [
        ("menu_update", State::Available(latest.clone())),
        (
            "menu_update_download",
            State::Downloading {
                latest: latest.clone(),
                done: 23_900_000,
                total: 51_400_000,
            },
        ),
        (
            "menu_update_failed",
            State::Failed {
                latest: latest.clone(),
                reason: "can't write C:\\Games\\velocide: Access is denied. (os error 5)".into(),
            },
        ),
    ] {
        h.common.update = Updater::showing(Some(config.clone()), state);
        h.menu(&mut menu, &[]);
        h.shot(name);
    }
    h.common.update = Updater::off();
    menu.free(&mut h.renderer);

    // ---- single-player ------------------------------------------------------------------------
    let mut game = InGame::start(
        &StartMode::SinglePlayer {
            level: Some("movement_test".into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.game(&mut game, 240, &[], "");
    println!(
        "in game: {}  level time {:.2}",
        game.is_playing(),
        game.level_time()
    );
    h.shot("game_sp");
    game.open_pause();
    h.game(&mut game, 30, &[], "");
    let t0 = game.level_time();
    h.game(&mut game, 30, &[], "");
    println!(
        "paused: menu {} sim {}  timer frozen {}",
        game.is_paused(),
        game.is_sim_paused(),
        game.level_time() == t0
    );
    h.shot("game_pause");
    h.game(&mut game, 3, &[Down, Down], "");
    h.game(&mut game, 3, &[Enter], "");
    h.shot("game_pause_loadout");
    h.game(&mut game, 3, &[Escape], "");
    h.game(&mut game, 3, &[Down, Down, Down], "");
    h.game(&mut game, 3, &[Enter], "");
    h.shot("game_pause_settings");
    h.game(&mut game, 3, &[Escape], "");
    game.resume();
    h.game(&mut game, 60, &[], "");
    game.chat.open();
    h.game(&mut game, 2, &[], "");
    h.game(&mut game, 1, &[], "gg, nice slide");
    h.game(&mut game, 2, &[], "");
    h.shot("game_chat");
    h.game(&mut game, 2, &[Enter], "");
    h.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, true));
    h.game(&mut game, 20, &[], "");
    h.shot("game_scoreboard");
    h.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, false));
    h.common.debug_overlay = true;
    h.game(&mut game, 30, &[], "");
    h.shot("game_debug");
    h.common.debug_overlay = false;

    // Effects + feedback API: molten wax, sparks, wax lumps, smoke, a shockwave and a damage
    // flash in front of the camera.
    if let Some(state) = game.net().and_then(|n| n.local_state()).copied() {
        let fwd = shared::input::look_dir(game.controls.yaw, 0.0);
        let p = state.position + fwd * 6.0 + glam::Vec3::Y * 1.2;
        game.effects.wax_spray(p, -fwd + glam::Vec3::Y, 3.0);
        game.effects.sparks(p + glam::Vec3::X, -fwd, 16);
        game.effects.lumps(p - glam::Vec3::X, 6);
        game.effects.smoke(p + glam::Vec3::Y, 4);
        game.effects.shockwave(state.position + fwd * 9.0, 6.0);
        game.effects.muzzle_flash(p, fwd);
        game.feedback.damage_flash(0.3);
        game.feedback.shake(0.6);
    }
    h.game(&mut game, 6, &[], "");
    h.shot("game_effects");
    h.common.debug_overlay = false;
    game.shutdown(&mut h.renderer, &mut h.common);

    // ---- connecting (nobody listens there) ----------------------------------------------------
    let mut game = InGame::start(
        &StartMode::Join {
            address: "127.0.0.1:9".into(),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.game(&mut game, 60, &[], "");
    h.shot("connecting");
    game.shutdown(&mut h.renderer, &mut h.common);
    Ok(())
}
