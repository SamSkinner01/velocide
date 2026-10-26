//! Renders the lobby flow headless (GPU, no window) with a real listen server, several fake
//! players (real clients with simulated latency, a server bot) and chat, and saves PNGs:
//!
//! - the **warm-up** (docs/lobby.md): the host arriving in the Vestry with the lobby overlay
//!   open (`warmup_overlay_host`), the hub with the HUD card and the ready board
//!   (`warmup_board`), the customise sub-screen over the world (`warmup_customize`), a guest's
//!   overlay (`warmup_overlay_guest`), a practice pack summoned at the console
//!   (`warmup_practice`), the countdown banner (`warmup_countdown`);
//! - the match it starts (`match_started`), the scoreboard with pings, the host pause menu,
//!   the results (`match_results`) and the warm-up after it (`warmup_after_match`);
//! - the menu-style lobby screen of a server without a warm-up world (`lobby_no_world`) and
//!   its leave dialog (`lobby_leave_confirm`);
//! - the main menu's CUSTOMISE page (`menu_customize`).
//!
//! ```text
//! cargo run -p client --release --example lobby_shots -- [out_dir]   # default target/lobby_shots
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo, ScreenOutput, Transition};
use client::ingame::InGame;
use client::menus::lobby::LobbyScreen;
use client::menus::main_menu::MainMenu;
use client::net::{ClientConfig, NetClient, NetSimConfig};
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::{GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use glam::Vec2;
use server::{LobbyCommand, LobbyConfig, ServerCommand, ServerConfig, ServerHandle};
use shared::cosmetics::Cosmetics;
use shared::lobby::GameMode;

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

enum Screen<'a> {
    Menu(&'a mut MainMenu),
    Lobby(&'a mut LobbyScreen),
    Game(&'a mut InGame),
}

struct Rig {
    renderer: Renderer,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
    out: PathBuf,
    /// Fake players: real clients pumped every frame.
    extras: Vec<NetClient>,
}

impl Rig {
    fn frame(&mut self, common: &mut Common, screen: Screen, keys: &[UiKey]) -> ScreenOutput {
        std::thread::sleep(Duration::from_secs_f32(DT));
        self.time += DT as f64;
        common.stats.update(DT);
        for c in &mut self.extras {
            c.update(Duration::from_secs_f32(DT));
            c.drain_events();
            // Keep their bodies in the warm-up (inputs keep the connection's clock honest).
            if c.is_in_game() {
                c.tick(shared::input::PlayerInput {
                    yaw: 3.4,
                    ..Default::default()
                });
            }
        }
        let ui = UiInput {
            mouse_pos: Vec2::splat(-100.0),
            keys: keys.to_vec(),
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
            match screen {
                Screen::Menu(m) => m.frame(common, &mut io, &mut self.buffers),
                Screen::Lobby(l) => l.frame(common, &mut io, &mut self.buffers),
                Screen::Game(g) => g.frame(common, &mut io, &mut self.buffers),
            }
        };
        if common.settings_changed {
            common.settings_changed = false;
            common.apply_display_settings(&mut self.renderer);
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

    /// Runs lobby frames until `done` or `secs`; returns the transition if one happened.
    fn lobby(
        &mut self,
        common: &mut Common,
        lobby: &mut LobbyScreen,
        secs: f32,
        mut done: impl FnMut(&LobbyScreen) -> bool,
    ) -> Option<Transition> {
        let start = Instant::now();
        while start.elapsed().as_secs_f32() < secs {
            let out = self.frame(common, Screen::Lobby(lobby), &[]);
            if out.transition.is_some() {
                return out.transition;
            }
            if done(lobby) {
                break;
            }
        }
        None
    }

    fn game(
        &mut self,
        common: &mut Common,
        game: &mut InGame,
        frames: usize,
        keys: &[UiKey],
    ) -> Option<Transition> {
        let mut t = None;
        for i in 0..frames {
            let k = if i == 0 { keys } else { &[][..] };
            let out = self.frame(common, Screen::Game(game), k);
            if out.transition.is_some() {
                t = out.transition;
            }
        }
        t
    }

    /// Game frames until `done` (at most `secs`).
    fn game_until(
        &mut self,
        common: &mut Common,
        game: &mut InGame,
        secs: f32,
        mut done: impl FnMut(&InGame) -> bool,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed().as_secs_f32() < secs {
            self.frame(common, Screen::Game(game), &[]);
            if done(game) {
                return true;
            }
        }
        false
    }

    /// A gamepad button tap over two frames.
    fn tap(&mut self, common: &mut Common, game: &mut InGame, button: GamepadButton) {
        self.input
            .handle_gamepad_event(GamepadEvent::Button(button, true));
        self.game(common, game, 2, &[]);
        self.input
            .handle_gamepad_event(GamepadEvent::Button(button, false));
        self.game(common, game, 1, &[]);
    }
}

fn common_for(name: &str, look: Cosmetics) -> Common {
    let settings = Settings {
        player_name: name.into(),
        cosmetics: look,
        ..Default::default()
    };
    let launch = LaunchOptions {
        mode: Some(GameMode::Deathmatch),
        max_players: Some(8),
        ..Default::default()
    };
    let scratch = std::env::temp_dir().join(format!("velocide-lobby-shots-{name}.ron"));
    Common::new(settings, scratch, Audio::disabled(), launch)
}

/// Lobby screen frames until the session goes to the game screen, then the game screen.
fn enter_game(rig: &mut Rig, common: &mut Common, mut lobby: LobbyScreen) -> InGame {
    let t = rig.lobby(common, &mut lobby, 6.0, |_| false);
    println!("lobby transition: {t:?}");
    let handoff = lobby.into_handoff(&mut rig.renderer).expect("session");
    let mut game = InGame::from_handoff(handoff, common, &mut rig.renderer);
    rig.game_until(common, &mut game, 6.0, |g| g.is_playing() && g.in_warmup());
    game
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/lobby_shots"));
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let mut rig = Rig {
        renderer,
        buffers: FrameBuffers::default(),
        input: InputState::new(),
        time: 10.0,
        out,
        extras: Vec::new(),
    };

    // ---- host: listen server on an ephemeral port, deathmatch on Cistern Nine ----------------
    let host_look = Cosmetics {
        mask: 1,
        crest: 2,
        cloak: 2,
        emblem: 6,
        primary: 5,
        secondary: 14,
        flame: 6,
    };
    let mut host_common = common_for("GRAVEL_SHRIKE", host_look);
    let host = LobbyScreen::start(
        &StartMode::Host {
            port: Some(0),
            level: Some("cistern_nine".into()),
        },
        &mut host_common,
        &mut rig.renderer,
    )?;
    let port = host
        .session()
        .and_then(|s| s.hosted_port())
        .expect("hosting");
    let addr = format!("127.0.0.1:{port}").parse()?;
    let mut game = enter_game(&mut rig, &mut host_common, host);
    println!(
        "host in the warm-up: {} (overlay open: {})",
        game.in_warmup(),
        game.lobby_overlay_open()
    );

    // Fake players with different pings and looks, one server bot, some chat.
    for (i, (name, latency)) in [
        ("Mx. Ossuary", 12.0),
        ("rivet_witch", 31.0),
        ("HOLLOWPOINT", 70.0),
    ]
    .into_iter()
    .enumerate()
    {
        let config = ClientConfig::new(name)
            .with_netsim(Some(NetSimConfig::new(latency, 2.0, 0.0).with_seed(7)))
            .with_cosmetics(Cosmetics::from_seed(11 + i as u32 * 5));
        rig.extras.push(NetClient::connect(addr, config)?);
    }
    if let Some(s) = game.session() {
        s.command(ServerCommand::AddBot {
            name: "BOT HUSKER".into(),
        });
    }
    rig.game(&mut host_common, &mut game, 120, &[]);
    rig.extras[0].send_chat("warm-up first, then the cistern?");
    rig.extras[1].send_chat("loser buys the thorns");
    rig.extras[0].set_ready(true);
    rig.extras[2].set_ready(true);
    rig.game(&mut host_common, &mut game, 30, &[]);
    rig.extras[2].send_chat("gl hf");
    // Pings refresh once per second.
    rig.game(&mut host_common, &mut game, 150, &[]);
    game.set_lobby_overlay(true);
    rig.game(&mut host_common, &mut game, 20, &[]);
    rig.shot("warmup_overlay_host");

    // TAB closes it: the hub, the HUD card and the ready board in front of the spawn.
    game.set_lobby_overlay(false);
    rig.game(&mut host_common, &mut game, 40, &[]);
    rig.shot("warmup_board");

    // CUSTOMISE over the world (the preview turns by itself).
    game.open_customize();
    rig.game(&mut host_common, &mut game, 90, &[]);
    rig.shot("warmup_customize");
    rig.game(&mut host_common, &mut game, 1, &[UiKey::Escape]);
    game.set_lobby_overlay(false);
    rig.game(&mut host_common, &mut game, 10, &[]);

    // LOADOUT from the overlay (applies at once in the warm-up).
    game.open_loadout();
    rig.game(&mut host_common, &mut game, 20, &[]);
    rig.shot("warmup_loadout");
    rig.game(&mut host_common, &mut game, 1, &[UiKey::Escape]);
    game.set_lobby_overlay(false);
    rig.game(&mut host_common, &mut game, 10, &[]);

    // The firing range: the counter, the Effigies down the lanes (west).
    game.send_chat("/tp -11 0.05 -15.6");
    rig.game(&mut host_common, &mut game, 20, &[]);
    let sensitivity = host_common.settings.mouse_sensitivity as f64;
    let degrees = |d: f64| d / (sensitivity * 0.022);
    rig.input
        .handle_mouse_motion((-degrees(90.0), degrees(4.0)));
    rig.game(&mut host_common, &mut game, 30, &[]);
    rig.shot("warmup_range");
    rig.input
        .handle_mouse_motion((degrees(90.0), -degrees(4.0)));
    rig.game(&mut host_common, &mut game, 2, &[]);

    // ---- a guest joins: through the lobby screen straight into the warm-up -----------------
    let mut guest_common = common_for("tin saint", Cosmetics::from_seed(99));
    let guest = LobbyScreen::start(
        &StartMode::Join {
            address: format!("127.0.0.1:{port}"),
        },
        &mut guest_common,
        &mut rig.renderer,
    )?;
    let mut guest_game = enter_game(&mut rig, &mut guest_common, guest);
    rig.game(&mut guest_common, &mut guest_game, 120, &[]);
    println!(
        "guest in the warm-up: {} (overlay opened by itself: {})",
        guest_game.in_warmup(),
        guest_game.lobby_overlay_open()
    );
    rig.shot("warmup_overlay_guest");
    guest_game.shutdown(&mut rig.renderer, &mut guest_common);

    // ---- the practice console: stand at it, press USE; watch the pack from the kerb.
    game.send_chat("/tp 23.5 0.05 3.5");
    rig.game(&mut host_common, &mut game, 30, &[]);
    rig.tap(&mut host_common, &mut game, GamepadButton::West);
    rig.game(&mut host_common, &mut game, 20, &[]);
    game.send_chat("/tp 21 0.35 5.55");
    rig.game(&mut host_common, &mut game, 20, &[]);
    // From the kerb: face south over the pit and look down at the pack below.
    let degrees = |d: f64| d / (host_common.settings.mouse_sensitivity as f64 * 0.022);
    rig.input
        .handle_mouse_motion((-degrees(180.0), degrees(55.0)));
    rig.game(&mut host_common, &mut game, 70, &[]);
    rig.shot("warmup_practice");

    // ---- countdown -------------------------------------------------------------------------------
    rig.extras[1].set_ready(true);
    rig.tap(&mut host_common, &mut game, GamepadButton::DPadDown);
    rig.game(&mut host_common, &mut game, 30, &[]);
    if let Some(s) = game.session() {
        s.command(ServerCommand::Lobby(LobbyCommand::StartMatch {
            force: true,
        }));
    }
    rig.game(&mut host_common, &mut game, 75, &[]);
    rig.shot("warmup_countdown");

    // ---- the match ---------------------------------------------------------------------------
    rig.game_until(&mut host_common, &mut game, 8.0, |g| {
        g.is_playing() && !g.in_warmup() && g.world.name() != "The Vestry"
    });
    rig.game(&mut host_common, &mut game, 120, &[]);
    println!("in the match: {}", game.world.name());
    rig.shot("match_started");
    rig.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, true));
    rig.game(&mut host_common, &mut game, 20, &[]);
    rig.shot("match_scoreboard");
    rig.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, false));
    game.open_pause();
    rig.game(&mut host_common, &mut game, 10, &[]);
    rig.shot("match_pause_host");
    game.resume();

    // ---- results -------------------------------------------------------------------------------
    if let Some(s) = game.session() {
        s.command(ServerCommand::Lobby(LobbyCommand::EndMatch));
    }
    rig.game(&mut host_common, &mut game, 400, &[]);
    println!("results open: {}", game.results_open());
    rig.shot("match_results");

    // CONTINUE TO LOBBY -> everyone back in a fresh warm-up (the game screen stays).
    if let Some(s) = game.session() {
        s.command(ServerCommand::Lobby(LobbyCommand::ReturnToLobby));
    }
    let back = rig.game_until(&mut host_common, &mut game, 6.0, |g| {
        g.in_warmup() && g.is_playing()
    });
    rig.game(&mut host_common, &mut game, 90, &[]);
    println!("back in the warm-up: {back}");
    rig.shot("warmup_after_match");
    game.shutdown(&mut rig.renderer, &mut host_common);
    rig.extras.clear();

    // ---- a server whose lobby has no world: the menu-style lobby screen -------------------
    let mut config = ServerConfig::host(0, "cistern_nine", 8);
    config.bind = "127.0.0.1:0".parse()?;
    config.lobby = LobbyConfig {
        warmup: false,
        ..LobbyConfig::enabled()
    };
    let server = ServerHandle::spawn(config)?;
    let mut common = common_for("tin saint", Cosmetics::from_seed(3));
    let mut lobby = LobbyScreen::start(
        &StartMode::Join {
            address: server.connect_addr().to_string(),
        },
        &mut common,
        &mut rig.renderer,
    )?;
    let t = rig.lobby(&mut common, &mut lobby, 3.0, |l| {
        l.net().is_some_and(|n| n.is_host())
    });
    println!("no-world lobby transition: {t:?}");
    rig.lobby(&mut common, &mut lobby, 1.5, |_| false);
    rig.shot("lobby_no_world");
    // Esc asks before leaving.
    rig.frame(&mut common, Screen::Lobby(&mut lobby), &[UiKey::Escape]);
    for _ in 0..3 {
        rig.frame(&mut common, Screen::Lobby(&mut lobby), &[]);
    }
    rig.shot("lobby_leave_confirm");
    lobby.shutdown(&mut rig.renderer);
    server.stop();

    // ---- the main menu's CUSTOMISE page ----------------------------------------------------
    let mut common = common_for("tin saint", Cosmetics::from_seed(21));
    common.load_models(&mut rig.renderer);
    let mut menu = MainMenu::new(&mut rig.renderer, &common);
    rig.frame(&mut common, Screen::Menu(&mut menu), &[]);
    for key in [UiKey::Down, UiKey::Down, UiKey::Down, UiKey::Enter] {
        rig.frame(&mut common, Screen::Menu(&mut menu), &[key]);
    }
    for _ in 0..80 {
        rig.frame(&mut common, Screen::Menu(&mut menu), &[]);
    }
    rig.shot("menu_customize");
    menu.free(&mut rig.renderer);
    Ok(())
}
