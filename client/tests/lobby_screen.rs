//! The real lobby and game screens, headless (GPU, no window): host a listen server, land in
//! the warm-up (the lobby screen hands over as soon as the hub arrives), autostart a match in
//! the same game screen, join it from a second lobby screen, return everyone to the warm-up
//! from the host's pause menu and start again; the lobby overlay, the Ready key and the
//! customise sub-screen (the look reaches the server); and the old lobby screen round trip on
//! a server without a warm-up world. Skips without a GPU adapter.

use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo, ScreenOutput, Transition};
use client::ingame::InGame;
use client::menus::lobby::LobbyScreen;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::{GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use server::{LobbyCommand, LobbyConfig, ServerCommand, ServerConfig, ServerHandle};
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
        if common.launch.bot {
            assert!(!out.grab_cursor, "bots never grab the cursor");
        }
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

    /// Game frames until `done` (`None`) or a transition (returned), at most `max` frames.
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

    /// A gamepad button tap (two frames down, one up).
    fn tap(&mut self, common: &mut Common, game: &mut InGame, button: GamepadButton) {
        self.input
            .handle_gamepad_event(GamepadEvent::Button(button, true));
        self.frame(common, Screen::Game(game), &[]);
        self.input
            .handle_gamepad_event(GamepadEvent::Button(button, false));
        self.frame(common, Screen::Game(game), &[]);
    }
}

fn common(name: &str, bot: bool, autostart: bool) -> Common {
    let launch = LaunchOptions {
        bot,
        ready: bot,
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

/// The game is in the warm-up hub and plays.
fn warming_up(g: &InGame) -> bool {
    g.is_playing() && g.in_warmup()
}

/// The game plays a match (not the hub).
fn in_match(g: &InGame) -> bool {
    g.is_playing()
        && !g.in_warmup()
        && g.net().and_then(|n| n.lobby_phase()) == Some(LobbyPhase::InMatch)
        && g.net()
            .and_then(|n| n.world())
            .is_some_and(|w| w.level.kind != shared::level::LevelKind::Hub)
}

#[test]
fn warmup_match_and_back_through_the_real_screens() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let mut host_common = common("Host", true, true);
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

    // The hub arrives with the first messages: straight on to the game screen.
    let t = rig.lobby_until(&mut host_common, &mut lobby, 300);
    assert_eq!(t, Some(Transition::EnterMatch), "the warm-up is a world");
    let handoff = lobby.into_handoff(&mut rig.renderer).unwrap();
    // Like the app: a new screen starts with fresh UI focus.
    host_common.ui_state.reset_interaction();
    let mut game = InGame::from_handoff(handoff, &mut host_common, &mut rig.renderer);
    let t = rig.game_until(&mut host_common, &mut game, 300, warming_up);
    assert_eq!(t, None);
    assert!(warming_up(&game), "the host warms up");
    assert!(
        game.net().unwrap().is_host(),
        "the listen server's player hosts"
    );
    assert!(!game.lobby_overlay_open(), "bots don't get the overlay");

    // Autostart: the host readies itself, the countdown runs, the match loads in place.
    let t = rig.game_until(&mut host_common, &mut game, 900, in_match);
    assert_eq!(t, None, "the match starts on the same game screen");
    assert!(in_match(&game), "the host plays the match");

    // A second player joins the running match through their own lobby screen.
    let mut guest_common = common("Guest", true, false);
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
        if in_match(&guest_game) {
            break;
        }
    }
    assert!(in_match(&guest_game), "the guest plays");
    let (_, rows) = game.score_rows();
    assert_eq!(rows.len(), 2, "scoreboard lists both");
    assert!(rows.iter().all(|r| r.ping_ms.is_some()), "pings known");
    assert!(rows.iter().any(|r| r.is_host));

    // Host pause menu: RESUME, RETURN TO LOBBY... -> everyone back in the warm-up, on the
    // same game screens. (A loaded machine can drop the key presses' frame timing: press again
    // a couple of times, like a player would.)
    let mut back = (false, false);
    for _attempt in 0..3 {
        if !in_match(&game) {
            break;
        }
        game.open_pause();
        rig.frame(&mut host_common, Screen::Game(&mut game), &[]);
        rig.frame(&mut host_common, Screen::Game(&mut game), &[UiKey::Down]);
        rig.frame(&mut host_common, Screen::Game(&mut game), &[UiKey::Enter]);
        game.resume();
        for _ in 0..300 {
            let h = rig
                .frame(&mut host_common, Screen::Game(&mut game), &[])
                .transition;
            let g = rig
                .frame(&mut guest_common, Screen::Game(&mut guest_game), &[])
                .transition;
            assert_eq!(
                (h, g),
                (None, None),
                "no screen change: the warm-up is the lobby"
            );
            // (Each on its own: autostart may already be counting down the next match.)
            back.0 |= warming_up(&game);
            back.1 |= warming_up(&guest_game);
            if back == (true, true) {
                break;
            }
        }
        if back == (true, true) {
            break;
        }
    }
    assert_eq!(
        back,
        (true, true),
        "everyone back in the warm-up: {:?} {:?} {} {:?}",
        game.net().and_then(|n| n.lobby_phase()),
        game.net().map(|n| n.state()),
        game.in_warmup(),
        game.net()
            .and_then(|n| n.world())
            .map(|w| w.level_name.clone()),
    );

    // The guest leaves; autostart brings the host into a second match.
    guest_game.shutdown(&mut rig.renderer, &mut guest_common);
    let t = rig.game_until(&mut host_common, &mut game, 900, in_match);
    assert_eq!(t, None);
    assert!(in_match(&game), "second match");
    game.shutdown(&mut rig.renderer, &mut host_common);
}

#[test]
fn the_overlay_ready_key_and_customise() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let mut c = common("Player", false, false);
    let mut lobby = LobbyScreen::start(
        &StartMode::Host {
            port: Some(0),
            level: Some(shared::sim::BUILTIN_LEVEL.into()),
        },
        &mut c,
        &mut rig.renderer,
    )
    .unwrap();
    let t = rig.lobby_until(&mut c, &mut lobby, 300);
    assert_eq!(t, Some(Transition::EnterMatch));
    let handoff = lobby.into_handoff(&mut rig.renderer).unwrap();
    c.ui_state.reset_interaction();
    let mut game = InGame::from_handoff(handoff, &mut c, &mut rig.renderer);
    rig.game_until(&mut c, &mut game, 300, |g| {
        warming_up(g) && g.net().is_some_and(|n| n.lobby().is_some())
    });
    rig.game_until(&mut c, &mut game, 10, |_| false);
    // First arrival: the overlay opens by itself, the cursor is free.
    assert!(game.lobby_overlay_open(), "the overlay opens on arrival");
    let out = rig.frame(&mut c, Screen::Game(&mut game), &[]);
    assert!(!out.grab_cursor);
    let me = game.net().unwrap().local_player_id().unwrap();
    let ready = |g: &InGame| {
        g.net()
            .and_then(|n| n.lobby())
            .and_then(|l| l.player(me))
            .is_some_and(|p| p.ready)
    };

    // TAB / Select closes it (the cursor is grabbed again) and opens it again.
    rig.tap(&mut c, &mut game, GamepadButton::Select);
    assert!(!game.lobby_overlay_open());
    let out = rig.frame(&mut c, Screen::Game(&mut game), &[]);
    assert!(out.grab_cursor, "playing again");
    rig.tap(&mut c, &mut game, GamepadButton::Select);
    assert!(game.lobby_overlay_open());
    // Esc closes it too.
    rig.frame(&mut c, Screen::Game(&mut game), &[UiKey::Escape]);
    assert!(!game.lobby_overlay_open());

    // The Ready key toggles ready without opening anything.
    rig.tap(&mut c, &mut game, GamepadButton::DPadDown);
    rig.game_until(&mut c, &mut game, 60, ready);
    assert!(ready(&game) && !game.lobby_overlay_open());
    rig.tap(&mut c, &mut game, GamepadButton::DPadDown);
    rig.game_until(&mut c, &mut game, 60, |g| !ready(g));
    assert!(!ready(&game));

    // CUSTOMISE: the first row (MASK) is focused; Right picks the next mask, the server
    // shares it (debounced), DONE (Esc) closes it and saves.
    game.open_customize();
    rig.frame(&mut c, Screen::Game(&mut game), &[]);
    let before = c.settings.cosmetics;
    rig.frame(&mut c, Screen::Game(&mut game), &[UiKey::Right]);
    assert_eq!(c.settings.cosmetics.mask, (before.mask + 1) % 8);
    rig.game_until(&mut c, &mut game, 120, |g| {
        g.net()
            .and_then(|n| n.players().get(&me))
            .is_some_and(|p| p.cosmetics.mask == (before.mask + 1) % 8)
    });
    let shared_look = game.net().unwrap().players()[&me].cosmetics;
    assert_eq!(shared_look, c.settings.cosmetics, "the server has our look");
    rig.frame(&mut c, Screen::Game(&mut game), &[UiKey::Escape]);
    assert!(c.settings_save, "closing customise saves the settings");
    game.shutdown(&mut rig.renderer, &mut c);
}

#[test]
fn the_lobby_screen_without_a_world() {
    let Some(mut rig) = Rig::new() else {
        return;
    };
    let mut config = ServerConfig::host(0, shared::sim::BUILTIN_LEVEL, 4);
    config.bind = "127.0.0.1:0".parse().unwrap();
    config.lobby = LobbyConfig {
        warmup: false,
        autostart: true,
        ..LobbyConfig::enabled()
    };
    let server = ServerHandle::spawn(config).unwrap();
    let mut c = common("Guest", true, false);
    let mut lobby = LobbyScreen::start(
        &StartMode::Join {
            address: server.connect_addr().to_string(),
        },
        &mut c,
        &mut rig.renderer,
    )
    .unwrap();
    for _ in 0..60 {
        rig.frame(&mut c, Screen::Lobby(&mut lobby), &[]);
        if lobby.is_ready() {
            break;
        }
    }
    assert!(lobby.is_ready(), "lobby state arrived");
    assert!(!rig.buffers.ui.quads.is_empty(), "lobby UI drawn");
    assert!(
        lobby.net().unwrap().world().is_none(),
        "no world in this lobby"
    );
    // --ready + autostart: the match starts and the session moves to the game screen.
    let t = rig.lobby_until(&mut c, &mut lobby, 600);
    assert_eq!(t, Some(Transition::EnterMatch));
    let handoff = lobby.into_handoff(&mut rig.renderer).unwrap();
    let mut game = InGame::from_handoff(handoff, &mut c, &mut rig.renderer);
    rig.game_until(&mut c, &mut game, 300, in_match);
    assert!(in_match(&game));
    // Back to a lobby without a world: back to the lobby screen.
    server.send(ServerCommand::Lobby(LobbyCommand::ReturnToLobby));
    let t = rig.game_until(&mut c, &mut game, 300, |_| false);
    assert_eq!(t, Some(Transition::ReturnToLobby));
    let handoff = game.into_handoff(&mut rig.renderer, &mut c).unwrap();
    let mut lobby = LobbyScreen::from_handoff(handoff, &c, &mut rig.renderer);
    rig.frame(&mut c, Screen::Lobby(&mut lobby), &[]);
    let net = lobby.net().unwrap();
    assert_eq!(net.lobby_phase(), Some(LobbyPhase::Lobby));
    assert!(net.world().is_none() && !net.is_in_game());
    lobby.shutdown(&mut rig.renderer);
    server.stop();
}
