//! The real lobby and game screens, headless (GPU, no window): host a listen server in the
//! lobby, autostart, hand the session to the game, join a match in progress from a second
//! lobby screen, return to the lobby from the host's pause menu and start again. Skips
//! without a GPU adapter.

use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo, ScreenOutput, Transition};
use client::ingame::InGame;
use client::menus::lobby::LobbyScreen;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::InputState;
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use shared::lobby::LobbyPhase;

const W: u32 = 320;
const H: u32 = 180;
const DT: f32 = 1.0 / 60.0;

struct Rig {
    renderer: Renderer,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
}

enum Screen<'a> {
    Lobby(&'a mut LobbyScreen),
    Game(&'a mut InGame),
}

impl Rig {
    fn new() -> Option<Rig> {
        match Renderer::new_headless(W, H, &RetroSettings::default()) {
            Ok(renderer) => Some(Rig {
                renderer,
                buffers: FrameBuffers::default(),
                input: InputState::new(),
                time: 0.0,
            }),
            Err(e) => {
                eprintln!("skipping: no GPU adapter ({e:#})");
                None
            }
        }
    }

    fn frame(&mut self, common: &mut Common, screen: Screen, keys: &[UiKey]) -> ScreenOutput {
        std::thread::sleep(Duration::from_secs_f32(DT));
        self.time += DT as f64;
        let ui = UiInput {
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
        assert!(!out.grab_cursor, "lobby / bot never grab the cursor");
        self.renderer
            .render(&self.buffers.desc(&out.scene), None)
            .unwrap();
        self.input.end_frame();
        out
    }

    /// Lobby frames until a transition (or `max` frames).
    fn lobby_until(
        &mut self,
        common: &mut Common,
        lobby: &mut LobbyScreen,
        max: usize,
    ) -> Option<Transition> {
        (0..max).find_map(|_| self.frame(common, Screen::Lobby(lobby), &[]).transition)
    }

    fn game_until(
        &mut self,
        common: &mut Common,
        game: &mut InGame,
        max: usize,
        mut done: impl FnMut(&InGame) -> bool,
    ) -> Option<Transition> {
        for _ in 0..max {
            let t = self.frame(common, Screen::Game(game), &[]).transition;
            if t.is_some() {
                return t;
            }
            if done(game) {
                return None;
            }
        }
        None
    }
}

fn common(name: &str, autostart: bool) -> Common {
    let launch = LaunchOptions {
        bot: true,
        ready: true,
        autostart,
        ..Default::default()
    };
    let settings = Settings {
        player_name: name.into(),
        ..Default::default()
    };
    let path = std::env::temp_dir().join(format!(
        "velocide-lobby-screen-{name}-{}.ron",
        std::process::id()
    ));
    Common::new(settings, path, Audio::disabled(), launch)
}

#[test]
fn lobby_match_and_back_through_the_real_screens() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let mut host_common = common("Host", true);
    let mut lobby = LobbyScreen::start(
        &StartMode::Host {
            port: Some(0),
            level: Some(shared::sim::BUILTIN_LEVEL.into()),
        },
        &mut host_common,
        &mut rig.renderer,
    )
    .unwrap();
    let port = lobby.session().unwrap().hosted_port().unwrap();

    // Autostart: the host readies itself, the countdown runs, the match starts.
    for _ in 0..120 {
        rig.frame(&mut host_common, Screen::Lobby(&mut lobby), &[]);
        // The host claim takes one round trip after the first lobby state.
        if lobby.is_ready() && lobby.net().is_some_and(|n| n.is_host()) {
            break;
        }
    }
    assert!(lobby.is_ready(), "lobby state arrived");
    let net = lobby.net().unwrap();
    assert!(net.is_host(), "the listen server's player hosts");
    assert!(net.in_lobby());
    assert!(!rig.buffers.ui.quads.is_empty(), "lobby UI drawn");
    let t = rig.lobby_until(&mut host_common, &mut lobby, 600);
    assert_eq!(t, Some(Transition::EnterMatch));
    let handoff = lobby.into_handoff(&mut rig.renderer).unwrap();
    // Like the app: a new screen starts with fresh UI focus.
    host_common.ui_state.reset_interaction();
    let mut game = InGame::from_handoff(handoff, &mut host_common, &mut rig.renderer);
    let t = rig.game_until(&mut host_common, &mut game, 300, InGame::is_playing);
    assert_eq!(t, None);
    assert!(game.is_playing(), "the host plays");

    // A second player joins the running match through their own lobby screen.
    let mut guest_common = common("Guest", false);
    let mut guest = LobbyScreen::start(
        &StartMode::Join {
            address: format!("127.0.0.1:{port}"),
        },
        &mut guest_common,
        &mut rig.renderer,
    )
    .unwrap();
    let t = rig.lobby_until(&mut guest_common, &mut guest, 300);
    assert_eq!(t, Some(Transition::EnterMatch), "join-in-progress");
    let handoff = guest.into_handoff(&mut rig.renderer).unwrap();
    let mut guest_game = InGame::from_handoff(handoff, &mut guest_common, &mut rig.renderer);
    for _ in 0..300 {
        rig.frame(&mut guest_common, Screen::Game(&mut guest_game), &[]);
        rig.frame(&mut host_common, Screen::Game(&mut game), &[]);
        if guest_game.is_playing() {
            break;
        }
    }
    assert!(guest_game.is_playing(), "the guest plays");
    let (_, rows) = game.score_rows();
    assert_eq!(rows.len(), 2, "scoreboard lists both");
    assert!(rows.iter().all(|r| r.ping_ms.is_some()), "pings known");
    assert!(rows.iter().any(|r| r.is_host));

    // Host pause menu: RESUME, RETURN TO LOBBY... -> everyone back in the lobby.
    game.open_pause();
    rig.frame(&mut host_common, Screen::Game(&mut game), &[]);
    rig.frame(&mut host_common, Screen::Game(&mut game), &[UiKey::Down]);
    rig.frame(&mut host_common, Screen::Game(&mut game), &[UiKey::Enter]);
    let mut host_t = None;
    let mut guest_t = None;
    for _ in 0..300 {
        if host_t.is_none() {
            host_t = rig
                .frame(&mut host_common, Screen::Game(&mut game), &[])
                .transition;
        }
        if guest_t.is_none() {
            guest_t = rig
                .frame(&mut guest_common, Screen::Game(&mut guest_game), &[])
                .transition;
        }
        if host_t.is_some() && guest_t.is_some() {
            break;
        }
    }
    assert_eq!(host_t, Some(Transition::ReturnToLobby));
    assert_eq!(guest_t, Some(Transition::ReturnToLobby));
    let handoff = guest_game
        .into_handoff(&mut rig.renderer, &mut guest_common)
        .unwrap();
    let guest = LobbyScreen::from_handoff(handoff, &guest_common, &mut rig.renderer);
    let handoff = game
        .into_handoff(&mut rig.renderer, &mut host_common)
        .unwrap();
    let mut lobby = LobbyScreen::from_handoff(handoff, &host_common, &mut rig.renderer);
    rig.frame(&mut host_common, Screen::Lobby(&mut lobby), &[]);
    let net = lobby.net().unwrap();
    assert_eq!(net.lobby_phase(), Some(LobbyPhase::Lobby));
    assert!(net.world().is_none() && !net.is_in_game());

    // The guest leaves; autostart brings the host into a second match.
    guest.shutdown(&mut rig.renderer);
    let t = rig.lobby_until(&mut host_common, &mut lobby, 600);
    assert_eq!(t, Some(Transition::EnterMatch), "second match");
    let handoff = lobby.into_handoff(&mut rig.renderer).unwrap();
    let mut game = InGame::from_handoff(handoff, &mut host_common, &mut rig.renderer);
    rig.game_until(&mut host_common, &mut game, 300, InGame::is_playing);
    assert!(game.is_playing());
    game.shutdown(&mut rig.renderer, &mut host_common);
}
