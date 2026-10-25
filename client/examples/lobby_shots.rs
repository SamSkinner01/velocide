//! Renders the lobby flow headless (GPU, no window) with a real listen server, several fake
//! players (real clients with simulated latency, a server bot) and chat, and saves PNGs:
//! host view, guest view, countdown, the match it starts, the scoreboard with pings, the host
//! pause menu, the results screen and the lobby after returning.
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
use client::net::{ClientConfig, NetClient, NetSimConfig};
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::{GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use glam::Vec2;
use server::{LobbyCommand, ServerCommand};
use shared::lobby::GameMode;

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

enum Screen<'a> {
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
                Screen::Lobby(l) => l.frame(common, &mut io, &mut self.buffers),
                Screen::Game(g) => g.frame(common, &mut io, &mut self.buffers),
            }
        };
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
}

fn common_for(name: &str, bot: bool) -> Common {
    let settings = Settings {
        player_name: name.into(),
        ..Default::default()
    };
    let launch = LaunchOptions {
        bot,
        mode: Some(GameMode::Deathmatch),
        max_players: Some(8),
        ..Default::default()
    };
    let scratch = std::env::temp_dir().join(format!("velocide-lobby-shots-{name}.ron"));
    Common::new(settings, scratch, Audio::disabled(), launch)
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
    let mut host_common = common_for("GRAVEL_SHRIKE", true);
    let mut host = LobbyScreen::start(
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
    rig.lobby(&mut host_common, &mut host, 3.0, LobbyScreen::is_ready);

    // Fake players with different pings, one server bot, some chat.
    for (name, latency) in [
        ("Mx. Ossuary", 12.0),
        ("rivet_witch", 31.0),
        ("HOLLOWPOINT", 70.0),
    ] {
        let config = ClientConfig::new(name)
            .with_netsim(Some(NetSimConfig::new(latency, 2.0, 0.0).with_seed(7)));
        rig.extras.push(NetClient::connect(addr, config)?);
    }
    if let Some(s) = host.session() {
        s.command(ServerCommand::AddBot {
            name: "BOT HUSKER".into(),
        });
    }
    rig.lobby(&mut host_common, &mut host, 2.0, |_| false);
    rig.extras[0].send_chat("arena or campaign? I want to try the cistern");
    rig.extras[1].send_chat("cistern. loser buys the prisms");
    rig.extras[0].set_ready(true);
    rig.extras[2].set_ready(true);
    rig.lobby(&mut host_common, &mut host, 0.5, |_| false);
    rig.extras[2].send_chat("gl hf");
    // Pings refresh once per second.
    rig.lobby(&mut host_common, &mut host, 2.5, |_| false);
    rig.shot("lobby_host");

    // ---- guest view (joins the same server) ---------------------------------------------------
    let mut guest_common = common_for("tin saint", false);
    let mut guest = LobbyScreen::start(
        &StartMode::Join {
            address: format!("127.0.0.1:{port}"),
        },
        &mut guest_common,
        &mut rig.renderer,
    )?;
    rig.lobby(&mut guest_common, &mut guest, 3.0, |l| {
        l.net()
            .and_then(|n| n.lobby())
            .is_some_and(|l| l.players.len() == 6)
    });
    rig.lobby(&mut guest_common, &mut guest, 1.5, |_| false);
    rig.shot("lobby_guest");
    guest.shutdown(&mut rig.renderer);

    // ---- countdown -----------------------------------------------------------------------------
    rig.extras[1].set_ready(true);
    // The host (a --bot launch) readies itself; force start from the host's own client.
    rig.lobby(&mut host_common, &mut host, 1.0, |_| false);
    if let Some(s) = host.session() {
        s.command(ServerCommand::Lobby(LobbyCommand::StartMatch {
            force: true,
        }));
    }
    rig.lobby(&mut host_common, &mut host, 1.3, |_| false);
    rig.shot("lobby_countdown");

    // ---- the match ---------------------------------------------------------------------------
    let t = rig.lobby(&mut host_common, &mut host, 6.0, |_| false);
    println!("lobby transition: {t:?}");
    let handoff = host.into_handoff(&mut rig.renderer).expect("session");
    let mut game = InGame::from_handoff(handoff, &mut host_common, &mut rig.renderer);
    rig.game(&mut host_common, &mut game, 150, &[]);
    println!("in game: {}", game.is_playing());
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
    rig.game(&mut host_common, &mut game, 60, &[]);
    println!("results open: {}", game.results_open());
    rig.shot("match_results");

    // CONTINUE TO LOBBY (focused by default) -> everyone back in the lobby.
    let mut transition = rig.game(&mut host_common, &mut game, 1, &[UiKey::Enter]);
    for _ in 0..120 {
        if transition.is_some() {
            break;
        }
        transition = rig.game(&mut host_common, &mut game, 1, &[]);
    }
    println!("game transition: {transition:?}");
    let handoff = game
        .into_handoff(&mut rig.renderer, &mut host_common)
        .expect("session");
    let mut host = LobbyScreen::from_handoff(handoff, &host_common, &mut rig.renderer);
    rig.lobby(&mut host_common, &mut host, 1.5, |_| false);
    rig.shot("lobby_after_match");

    // Esc asks before leaving; Up + Enter picks LEAVE.
    rig.frame(&mut host_common, Screen::Lobby(&mut host), &[UiKey::Escape]);
    for _ in 0..3 {
        rig.frame(&mut host_common, Screen::Lobby(&mut host), &[]);
    }
    rig.shot("lobby_leave_confirm");
    rig.frame(&mut host_common, Screen::Lobby(&mut host), &[UiKey::Up]);
    let out = rig.frame(&mut host_common, Screen::Lobby(&mut host), &[UiKey::Enter]);
    println!("leave transition: {:?}", out.transition);
    host.shutdown(&mut rig.renderer);
    Ok(())
}
