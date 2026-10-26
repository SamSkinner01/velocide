//! Scripted end-to-end playthrough of the player-facing flows on the real screens (headless
//! GPU, real in-process / listen servers over UDP, the app's screen transitions):
//!
//! * **Single player**: main menu → SINGLE PLAYER → Crucible of Veins; pause (the server
//!   stops); arena 1 (doors lock, waves, enemies hit back, cleared → doors open); checkpoint;
//!   death in arena 2 → death view → back at the checkpoint with the encounter reset; instant
//!   restart; the exit → graded results → MAIN MENU.
//! * **Co-op** (host + a guest behind `--netsim 60,10,1`): lobby, start, rally pull, downed (low
//!   view, no weapon) → revive with USE, team wipe → checkpoint, extraction → co-op results →
//!   CONTINUE TO LOBBY → a second match.
//! * **Deathmatch** (host + guest + 2 server bots, Cistern Nine): frags, respawns, scoreboard
//!   with pings, chat, name tags, frag limit → results → lobby.
//! * **Robustness**: the host vanishes mid-match, a guest quits mid-match, a full server, a
//!   bad address, a killed dedicated server, menu key spam.
//! * **Settings**: every retro / view / HUD option changes the picture live; saved settings
//!   load back; garbage settings files give defaults.
//!
//! Prints `PASS` / `FAIL` per check (exit code 1 on any failure) and writes PNGs.
//!
//! ```text
//! cargo run -p client --release --example playthrough -- [out_dir]   # default target/playthrough
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo, Transition};
use client::ingame::InGame;
use client::menus::lobby::LobbyScreen;
use client::menus::main_menu::MainMenu;
use client::net::NetSimConfig;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::{GamepadAxis, GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use glam::{Vec2, Vec3};
use server::ServerCommand;
use server::lobby::LobbyCommand;
use shared::input::look_dir;
use shared::lobby::{GameMode, MatchSettings};
use shared::protocol::{NetId, encounter_flags, enemy_flags, kind, player_flags};
use shared::sim::PlayerId;

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

enum Screen {
    Menu(Box<MainMenu>),
    Lobby(Box<LobbyScreen>),
    Game(Box<InGame>),
    Gone,
}

struct Client {
    common: Common,
    input: InputState,
    screen: Screen,
    time: f64,
    /// UI keys / text for the next frame.
    keys: Vec<UiKey>,
    text: String,
    /// Main-menu message from the last transition to the menu.
    menu_message: Option<String>,
    transitions: Vec<String>,
    /// Camera position of the last frame.
    camera: Vec3,
}

impl Client {
    fn game(&mut self) -> Option<&mut InGame> {
        match &mut self.screen {
            Screen::Game(g) => Some(g),
            _ => None,
        }
    }

    fn game_ref(&self) -> Option<&InGame> {
        match &self.screen {
            Screen::Game(g) => Some(g),
            _ => None,
        }
    }

    fn lobby_ref(&self) -> Option<&LobbyScreen> {
        match &self.screen {
            Screen::Lobby(l) => Some(l),
            _ => None,
        }
    }

    fn playing(&self) -> bool {
        self.game_ref().is_some_and(InGame::is_playing)
    }

    fn chat(&mut self, text: &str) {
        if let Some(g) = self.game() {
            g.send_chat(text);
        }
    }

    fn me(&self) -> Option<PlayerId> {
        self.game_ref()?.net()?.local_player_id()
    }

    /// Predicted feet.
    fn feet(&self) -> Option<Vec3> {
        Some(self.game_ref()?.net()?.local_state()?.position)
    }

    fn dead(&self) -> bool {
        self.game_ref()
            .and_then(InGame::net)
            .and_then(|n| n.own_combat())
            .is_some_and(|c| c.combat.dead)
    }

    fn health(&self) -> f32 {
        self.game_ref()
            .and_then(InGame::net)
            .and_then(|n| n.own_combat())
            .map_or(0.0, |c| 100.0 - c.combat.damage)
    }

    fn pad(&mut self, button: GamepadButton, down: bool) {
        self.input
            .handle_gamepad_event(GamepadEvent::Button(button, down));
    }

    fn axis(&mut self, axis: GamepadAxis, value: f32) {
        self.input
            .handle_gamepad_event(GamepadEvent::Axis(axis, value));
    }

    fn server_stats(&self) -> Option<server::ServerStats> {
        Some(self.game_ref()?.session()?.server()?.stats())
    }

    fn command(&self, cmd: ServerCommand) {
        let session = match &self.screen {
            Screen::Game(g) => g.session(),
            Screen::Lobby(l) => l.session(),
            _ => None,
        };
        if let Some(s) = session {
            s.command(cmd);
        }
    }

    /// Living enemies the client sees.
    fn enemies(&self) -> Vec<Vec3> {
        self.game_ref()
            .and_then(InGame::net)
            .map(|n| {
                n.remote_entities()
                    .iter()
                    .filter(|e| kind::is_enemy(e.kind) && e.flags & enemy_flags::DYING == 0)
                    .map(|e| e.position)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn encounter_flags(&self, index: u32) -> u32 {
        self.game_ref()
            .and_then(InGame::net)
            .and_then(|n| n.remote_entity(NetId::encounter(index)))
            .map_or(0, |e| e.flags)
    }

    fn door_open(&self, id: u32) -> Option<bool> {
        Some(
            self.game_ref()?
                .net()?
                .world()?
                .collision
                .is_door_open(id),
        )
    }

    fn level(&self) -> Option<std::sync::Arc<shared::level::LevelDef>> {
        Some(self.game_ref()?.net()?.world()?.level.clone())
    }

    /// Points the view at `target` (eye height 1.6).
    fn aim(&mut self, target: Vec3) {
        let Some(feet) = self.feet() else { return };
        let d = target - (feet + Vec3::Y * 1.6);
        let yaw = (-d.x).atan2(-d.z);
        let pitch = (d.y / Vec2::new(d.x, d.z).length().max(0.01)).atan();
        if let Some(g) = self.game() {
            g.controls.set_view(yaw, pitch);
        }
    }
}

struct Rig {
    renderer: Renderer,
    /// Window size in pixels (the headless target).
    size: (u32, u32),
    buffers: FrameBuffers,
    out: PathBuf,
    fails: Vec<String>,
    passes: usize,
    frame_ms: Vec<f32>,
}

impl Rig {
    fn client(&mut self, name: &str, launch: LaunchOptions) -> Client {
        let path = self.out.join(format!("settings-{name}.ron"));
        let _ = std::fs::remove_file(&path);
        let mut common = Common::new(Settings::default(), path, Audio::disabled(), launch);
        common.name_override = Some(name.to_string());
        let menu = MainMenu::new(&mut self.renderer, &common);
        Client {
            common,
            input: InputState::new(),
            screen: Screen::Menu(Box::new(menu)),
            time: 0.0,
            keys: Vec::new(),
            text: String::new(),
            menu_message: None,
            transitions: Vec::new(),
            camera: Vec3::ZERO,
        }
    }

    fn clocks(c: &Client) -> String {
        format!(
            "hud {:.2} server tick {}",
            c.game_ref().map_or(0.0, InGame::level_time),
            c.server_stats().map_or(0, |s| s.tick)
        )
    }

    fn check(&mut self, ok: bool, what: &str) -> bool {
        if ok {
            self.passes += 1;
            println!("PASS  {what}");
        } else {
            println!("FAIL  {what}");
            self.fails.push(what.to_string());
        }
        ok
    }

    /// One frame of `c`, like `VelocideApp::frame` (screen, settings, transitions).
    fn frame(&mut self, c: &mut Client) {
        c.time += DT as f64;
        c.common.stats.update(DT);
        let ui = UiInput {
            mouse_pos: Vec2::splat(-100.0),
            keys: std::mem::take(&mut c.keys),
            text: std::mem::take(&mut c.text),
            time: c.time,
            ..Default::default()
        };
        let started = Instant::now();
        self.buffers
            .clear(UiDrawList::canvas_for_window(self.size.0, self.size.1));
        let out = {
            let mut io = FrameIo {
                renderer: &mut self.renderer,
                input: &c.input,
                ui_input: &ui,
                dt: DT,
                time: c.time,
                window_size: self.size,
            };
            match &mut c.screen {
                Screen::Menu(m) => m.frame(&mut c.common, &mut io, &mut self.buffers),
                Screen::Lobby(l) => l.frame(&mut c.common, &mut io, &mut self.buffers),
                Screen::Game(g) => g.frame(&mut c.common, &mut io, &mut self.buffers),
                Screen::Gone => return,
            }
        };
        c.camera = out.scene.camera.position;
        if c.common.settings_changed {
            c.common.settings_changed = false;
            self.renderer.apply_settings(&c.common.settings.retro);
            if let Screen::Game(g) = &mut c.screen {
                g.apply_settings(&c.common.settings);
            }
        }
        if c.common.settings_save {
            c.common.save_settings();
        }
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            eprintln!("render: {e:#}");
        }
        self.frame_ms.push(started.elapsed().as_secs_f32() * 1000.0);
        c.input.end_frame();
        if let Some(t) = out.transition {
            self.transition(c, t);
        }
    }

    fn leave(&mut self, c: &mut Client) {
        match std::mem::replace(&mut c.screen, Screen::Gone) {
            Screen::Menu(m) => m.free(&mut self.renderer),
            Screen::Lobby(l) => l.shutdown(&mut self.renderer),
            Screen::Game(g) => g.shutdown(&mut self.renderer, &mut c.common),
            Screen::Gone => {}
        }
    }

    fn go_to_menu(&mut self, c: &mut Client, message: Option<String>) {
        self.leave(c);
        let mut menu = MainMenu::new(&mut self.renderer, &c.common);
        menu.show_message(message.clone());
        c.menu_message = message;
        c.screen = Screen::Menu(Box::new(menu));
        c.common.ui_state.reset_interaction();
    }

    fn transition(&mut self, c: &mut Client, t: Transition) {
        c.transitions.push(format!("{t:?}"));
        match t {
            Transition::Start(mode) => {
                let screen = match &mode {
                    StartMode::Host { .. } | StartMode::Join { .. } => {
                        LobbyScreen::start(&mode, &mut c.common, &mut self.renderer)
                            .map(|l| Screen::Lobby(Box::new(l)))
                    }
                    _ => InGame::start(&mode, &mut c.common, &mut self.renderer)
                        .map(|g| Screen::Game(Box::new(g))),
                };
                match screen {
                    Ok(s) => {
                        self.leave(c);
                        c.screen = s;
                        c.common.ui_state.reset_interaction();
                    }
                    Err(e) => self.go_to_menu(c, Some(format!("{e:#}"))),
                }
            }
            Transition::MainMenu { message } => self.go_to_menu(c, message),
            Transition::EnterMatch => {
                if let Screen::Lobby(l) = std::mem::replace(&mut c.screen, Screen::Gone) {
                    match l.into_handoff(&mut self.renderer) {
                        Some(h) => {
                            let g = InGame::from_handoff(h, &mut c.common, &mut self.renderer);
                            c.screen = Screen::Game(Box::new(g));
                            c.common.ui_state.reset_interaction();
                        }
                        None => self.go_to_menu(c, Some("Lost the session".into())),
                    }
                }
            }
            Transition::ReturnToLobby => {
                if let Screen::Game(g) = std::mem::replace(&mut c.screen, Screen::Gone) {
                    match g.into_handoff(&mut self.renderer, &mut c.common) {
                        Some(h) => {
                            let l = LobbyScreen::from_handoff(h, &c.common, &mut self.renderer);
                            c.screen = Screen::Lobby(Box::new(l));
                            c.common.ui_state.reset_interaction();
                        }
                        None => self.go_to_menu(c, Some("Lost the session".into())),
                    }
                }
            }
            Transition::Quit => self.leave(c),
        }
    }

    /// `n` real-time rounds of frames for every client.
    fn run(&mut self, cs: &mut [&mut Client], n: usize) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            for c in cs.iter_mut() {
                self.frame(c);
            }
        }
    }

    /// Rounds until `done` (at most `secs` of real time); true if it happened.
    fn until(
        &mut self,
        cs: &mut [&mut Client],
        secs: f32,
        mut done: impl FnMut(&[&mut Client]) -> bool,
    ) -> bool {
        let rounds = (secs / DT) as usize;
        for _ in 0..rounds {
            if done(cs) {
                return true;
            }
            self.run(cs, 1);
        }
        done(cs)
    }

    fn shot(&mut self, c: &mut Client, name: &str) {
        self.frame(c);
        match self.renderer.read_pixels() {
            Ok(img) => {
                let path = self.out.join(format!("{name}.png"));
                if let Err(e) = engine::assets::save_png(&path, &img) {
                    eprintln!("{name}: {e:#}");
                } else {
                    println!("      wrote {}", path.display());
                }
            }
            Err(e) => eprintln!("{name}: {e:#}"),
        }
    }
}

fn wait_playing(rig: &mut Rig, cs: &mut [&mut Client], secs: f32) -> bool {
    rig.until(cs, secs, |cs| cs.iter().all(|c| c.playing()))
}

/// Teleports (dev command) a client into encounter `i`'s trigger, 6 m past its rally point.
fn walk_into_encounter(c: &mut Client, i: usize) {
    let Some(level) = c.level() else { return };
    let Some(rally) = level.encounters.get(i).and_then(|e| e.rally) else {
        return;
    };
    let p = rally.position + look_dir(rally.yaw.to_radians(), 0.0) * 6.0;
    c.chat(&format!("/tp {} {} {}", p.x, p.y + 0.1, p.z));
}

/// `/killall` until encounter `i` is cleared (or `secs` pass).
fn clear_encounter(rig: &mut Rig, c: &mut Client, i: u32, secs: f32) -> bool {
    let rounds = (secs / 0.5) as usize;
    for _ in 0..rounds {
        if c.encounter_flags(i) & encounter_flags::CLEARED != 0 {
            return true;
        }
        c.chat("/killall");
        rig.run(&mut [&mut *c], 30);
    }
    c.encounter_flags(i) & encounter_flags::CLEARED != 0
}

// ---- single player -------------------------------------------------------------------------------

fn single_player(rig: &mut Rig) {
    println!("\n== single player: Crucible of Veins ==");
    let mut c = rig.client("Solo", LaunchOptions::default());
    rig.run(&mut [&mut c], 20);
    rig.shot(&mut c, "sp_01_menu");
    c.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut c], 10);
    rig.shot(&mut c, "sp_02_levels");
    // The selected (focused) level is the campaign: Enter plays it.
    c.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut c], 2);
    let ok = wait_playing(rig, &mut [&mut c], 20.0);
    rig.check(ok, "SP: menu -> SINGLE PLAYER -> Enter starts a game");
    println!("      clocks: {}", Rig::clocks(&c));
    let level = c.level().map(|l| l.name.clone()).unwrap_or_default();
    rig.check(
        level.to_lowercase().contains("crucible"),
        &format!("SP: the default level is the campaign ({level})"),
    );
    rig.run(&mut [&mut c], 90);
    rig.shot(&mut c, "sp_03_spawn");
    let eye = c.camera.y - c.feet().map_or(0.0, |f| f.y);
    rig.check((eye - 1.6).abs() < 0.2, &format!("SP: standing eye height {eye:.2}"));

    // Pause stops the in-process server.
    c.pad(GamepadButton::Start, true);
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::Start, false);
    rig.run(&mut [&mut c], 10);
    let paused = c.game_ref().is_some_and(|g| g.is_paused() && g.is_sim_paused());
    let tick0 = c.server_stats().map_or(0, |s| s.tick);
    let time0 = c.game_ref().map_or(0.0, InGame::level_time);
    rig.run(&mut [&mut c], 60);
    let tick1 = c.server_stats().map_or(0, |s| s.tick);
    let time1 = c.game_ref().map_or(0.0, InGame::level_time);
    rig.shot(&mut c, "sp_04_paused");
    rig.check(
        paused && tick0 == tick1 && time0 == time1,
        &format!("SP: pause freezes server ({tick0} -> {tick1}) and timer"),
    );
    c.pad(GamepadButton::Start, true);
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::Start, false);
    rig.run(&mut [&mut c], 30);
    let tick2 = c.server_stats().map_or(0, |s| s.tick);
    rig.check(
        !c.game_ref().is_some_and(InGame::is_paused) && tick2 > tick1,
        "SP: gamepad Start resumes",
    );

    // Arena 1: walk in, doors lock, waves come.
    walk_into_encounter(&mut c, 0);
    let locked = rig.until(&mut [&mut c], 3.0, |cs| cs[0].door_open(1) == Some(false));
    rig.run(&mut [&mut c], 20);
    rig.shot(&mut c, "sp_05_arena_locked");
    rig.check(locked, "SP: entering arena 1 locks door 1");
    println!("      clocks: {}", Rig::clocks(&c));
    let spawned = rig.until(&mut [&mut c], 5.0, |cs| cs[0].enemies().len() >= 3);
    rig.check(spawned, &format!("SP: wave 1 spawned ({} enemies)", c.enemies().len()));
    // Stand still: they come and hit.
    let hit = rig.until(&mut [&mut c], 12.0, |cs| cs[0].health() < 100.0);
    if let Some(e) = c.enemies().first().copied() {
        c.aim(e + Vec3::Y);
    }
    rig.shot(&mut c, "sp_06_enemies_attack");
    rig.check(hit, &format!("SP: enemies hit back (health {:.0})", c.health()));
    println!("      clocks: {}", Rig::clocks(&c));
    c.chat("/give");
    // Fight: aim at the nearest enemy and fire for a while.
    let stats0 = c.server_stats().map_or(0, |s| s.enemies);
    c.axis(GamepadAxis::RightTrigger, 1.0);
    for i in 0..240 {
        let me = c.feet().unwrap_or_default();
        if let Some(e) = c
            .enemies()
            .into_iter()
            .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
        {
            c.aim(e + Vec3::Y * 1.0);
        }
        rig.run(&mut [&mut c], 1);
        if i == 60 {
            rig.shot(&mut c, "sp_07_fight");
        }
        if c.health() < 40.0 {
            c.chat("/give");
        }
    }
    c.axis(GamepadAxis::RightTrigger, 0.0);
    let stats1 = c.server_stats().map_or(0, |s| s.enemies);
    println!("      enemies on the server: {stats0} -> {stats1}");
    let cleared = clear_encounter(rig, &mut c, 0, 20.0);
    rig.run(&mut [&mut c], 30);
    rig.shot(&mut c, "sp_08_cleared");
    rig.check(
        cleared && c.door_open(2) == Some(true) && c.door_open(1) == Some(true),
        "SP: arena 1 cleared -> doors 1 and 2 open",
    );

    // Checkpoint 2.
    c.chat("/goto checkpoint 2");
    let cp = rig.until(&mut [&mut c], 3.0, |cs| {
        cs[0].game_ref().and_then(|g| g.modes.checkpoint()) == Some(1)
    });
    rig.run(&mut [&mut c], 10);
    rig.shot(&mut c, "sp_09_checkpoint");
    rig.check(cp, "SP: checkpoint 2 reached");
    println!("      clocks: {}", Rig::clocks(&c));

    // Arena 2, then die: back at checkpoint 2 with the fight reset.
    walk_into_encounter(&mut c, 1);
    let started = rig.until(&mut [&mut c], 6.0, |cs| {
        cs[0].door_open(3) == Some(false) && !cs[0].enemies().is_empty()
    });
    rig.check(started, "SP: arena 2 started");
    c.chat("/kill");
    let died = rig.until(&mut [&mut c], 2.0, |cs| cs[0].dead());
    rig.run(&mut [&mut c], 20);
    rig.shot(&mut c, "sp_10_death_view");
    rig.check(died, "SP: /kill -> death view");
    let back = rig.until(&mut [&mut c], 4.0, |cs| !cs[0].dead());
    rig.run(&mut [&mut c], 30);
    let spawn = c
        .level()
        .map_or(Vec3::ZERO, |l| l.checkpoints[1].spawn.position);
    let at = c.feet().unwrap_or_default();
    rig.shot(&mut c, "sp_11_back_at_checkpoint");
    rig.check(
        back && at.distance(spawn) < 2.0,
        &format!("SP: respawned at checkpoint 2 ({at} vs {spawn})"),
    );
    let enc = c.encounter_flags(1);
    let enemies = c.server_stats().map_or(99, |s| s.enemies);
    rig.check(
        enc & encounter_flags::ACTIVE == 0
            && enemies == 0
            && c.door_open(3) == Some(true)
            && c.door_open(4) == Some(false),
        &format!("SP: arena 2 reset (flags {enc}, {enemies} enemies, doors)"),
    );
    walk_into_encounter(&mut c, 1);
    let again = rig.until(&mut [&mut c], 6.0, |cs| !cs[0].enemies().is_empty());
    rig.check(again, "SP: arena 2 can be fought again");
    let cleared = clear_encounter(rig, &mut c, 1, 25.0);
    rig.check(cleared, "SP: arena 2 cleared");

    // Instant restart: start position, timer reset, encounters idle.
    let t_before = c.game_ref().map_or(0.0, InGame::level_time);
    if let Some(g) = c.game() {
        g.restart();
    }
    let start = c.level().map_or(Vec3::ZERO, |l| l.spawns[0].position);
    let restarted = rig.until(&mut [&mut c], 6.0, |cs| {
        cs[0].feet().is_some_and(|f| f.distance(start) < 2.0)
            && cs[0].game_ref().is_some_and(|g| g.level_time() < 1.0)
    });
    rig.run(&mut [&mut c], 20);
    rig.shot(&mut c, "sp_12_restarted");
    let flags = c.encounter_flags(0);
    let door2 = c.door_open(2);
    rig.check(
        restarted
            && flags & (encounter_flags::ACTIVE | encounter_flags::CLEARED) == 0
            && door2 == Some(false),
        &format!(
            "SP: restart -> start ({restarted}, at {:?}, timer {t_before:.1} -> {:.1}), level reset (arena 1 flags {flags}, door 2 open {door2:?})",
            c.feet(),
            c.game_ref().map_or(-1.0, InGame::level_time)
        ),
    );

    // The exit: results with grades, then MAIN MENU.
    c.chat("/goto exit");
    let results = rig.until(&mut [&mut c], 4.0, |cs| {
        cs[0].game_ref().is_some_and(InGame::results_open)
    });
    rig.run(&mut [&mut c], 60);
    rig.shot(&mut c, "sp_13_results_reveal");
    rig.check(results, "SP: reaching the exit opens the results");
    rig.run(&mut [&mut c], 420);
    rig.shot(&mut c, "sp_14_results");
    c.keys = vec![UiKey::Right];
    rig.run(&mut [&mut c], 2);
    c.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut c], 30);
    rig.shot(&mut c, "sp_15_back_in_menu");
    rig.check(
        matches!(c.screen, Screen::Menu(_)),
        &format!("SP: MAIN MENU leaves the results ({:?})", c.transitions.last()),
    );
    rig.leave(&mut c);
}

// ---- co-op -----------------------------------------------------------------------------------------

fn lobby_ready(c: &Client, players: usize) -> bool {
    c.lobby_ref()
        .and_then(LobbyScreen::net)
        .and_then(|n| n.lobby())
        .is_some_and(|l| l.players.len() == players && l.host.is_some())
}

fn host_and_join(
    rig: &mut Rig,
    level: &str,
    mode: GameMode,
    bots: u32,
    max_players: Option<u32>,
) -> (Client, Client) {
    let mut host = rig.client(
        "Host",
        LaunchOptions {
            ready: true,
            mode: Some(mode),
            add_bots: bots,
            max_players,
            ..Default::default()
        },
    );
    rig.transition(
        &mut host,
        Transition::Start(StartMode::Host {
            port: Some(0),
            level: Some(level.into()),
        }),
    );
    rig.run(&mut [&mut host], 30);
    let port = host
        .lobby_ref()
        .and_then(LobbyScreen::session)
        .and_then(|s| s.hosted_port())
        .unwrap_or(0);
    let mut guest = rig.client(
        "Guest",
        LaunchOptions {
            ready: true,
            netsim: Some(NetSimConfig::new(60.0, 10.0, 1.0)),
            ..Default::default()
        },
    );
    rig.transition(
        &mut guest,
        Transition::Start(StartMode::Join {
            address: format!("127.0.0.1:{port}"),
        }),
    );
    (host, guest)
}

fn coop(rig: &mut Rig) {
    println!("\n== co-op: host + guest (netsim 60,10,1) in Crucible of Veins ==");
    let (mut host, mut guest) =
        host_and_join(rig, "crucible_of_veins", GameMode::Campaign, 0, None);
    let ok = rig.until(&mut [&mut host, &mut guest], 8.0, |cs| {
        cs.iter().all(|c| lobby_ready(c, 2))
    });
    rig.check(ok, "co-op: both in the lobby");
    // Both ready up automatically (--ready, 1 s).
    let ready = rig.until(&mut [&mut host, &mut guest], 5.0, |cs| {
        cs[0]
            .lobby_ref()
            .and_then(LobbyScreen::net)
            .and_then(|n| n.lobby())
            .is_some_and(|l| l.all_ready())
    });
    rig.shot(&mut host, "coop_01_lobby_host");
    rig.shot(&mut guest, "coop_02_lobby_guest");
    rig.check(ready, "co-op: everyone ready");
    // Host: READY is focused; START MATCH is below it.
    host.keys = vec![UiKey::Down];
    rig.run(&mut [&mut host, &mut guest], 2);
    host.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut host, &mut guest], 2);
    let ok = wait_playing(rig, &mut [&mut host, &mut guest], 12.0);
    rig.check(ok, "co-op: START MATCH -> countdown -> both in the level");
    rig.run(&mut [&mut host, &mut guest], 60);
    rig.shot(&mut guest, "coop_03_guest_spawn");

    // Rally: the host walks into arena 1; the guest (at the start) is pulled in.
    host.chat("/god");
    host.chat("/notarget");
    rig.run(&mut [&mut host, &mut guest], 10);
    walk_into_encounter(&mut host, 0);
    let rally = host
        .level()
        .and_then(|l| l.encounters[0].rally)
        .map_or(Vec3::ZERO, |r| r.position);
    let pulled = rig.until(&mut [&mut host, &mut guest], 4.0, |cs| {
        cs[1].feet().is_some_and(|f| f.distance(rally) < 2.0)
    });
    rig.run(&mut [&mut host, &mut guest], 20);
    rig.shot(&mut guest, "coop_04_guest_rallied");
    rig.check(pulled, "co-op: encounter lock pulls the teammate to the rally point");

    // The guest gets downed (the host is god + notarget, so everything goes for the guest).
    let guest_id = guest.me().unwrap_or(PlayerId(0));
    let downed = rig.until(&mut [&mut host, &mut guest], 30.0, |cs| {
        cs[1].game_ref().is_some_and(|g| g.modes.is_downed(guest_id))
    });
    rig.check(downed, "co-op: guest downed by enemies");
    rig.run(&mut [&mut host, &mut guest], 60);
    let eye = guest.camera.y - guest.feet().map_or(0.0, |f| f.y);
    rig.shot(&mut guest, "coop_05_guest_downed");
    rig.shot(&mut host, "coop_06_host_sees_downed");
    rig.check(
        (eye - 0.6).abs() < 0.15,
        &format!("co-op: downed view is low ({eye:.2} m)"),
    );
    // Revive: the host goes to the body and holds USE.
    let body = host
        .game_ref()
        .and_then(InGame::net)
        .and_then(|n| n.remote_entity(NetId::player(guest_id)))
        .map(|e| (e.position, e.flags));
    if let Some((p, flags)) = body {
        rig.check(
            flags & player_flags::DOWNED != 0,
            "co-op: host sees the DOWNED flag",
        );
        host.chat(&format!("/tp {} {} {}", p.x + 1.0, p.y + 0.1, p.z));
    }
    rig.run(&mut [&mut host, &mut guest], 30);
    host.pad(GamepadButton::West, true);
    let revived = rig.until(&mut [&mut host, &mut guest], 6.0, |cs| {
        !cs[1].dead() && cs[1].game_ref().is_some_and(|g| !g.modes.is_downed(guest_id))
    });
    rig.run(&mut [&mut host, &mut guest], 10);
    rig.shot(&mut host, "coop_07_reviving_done");
    host.pad(GamepadButton::West, false);
    rig.run(&mut [&mut host, &mut guest], 60);
    let eye = guest.camera.y - guest.feet().map_or(0.0, |f| f.y);
    rig.shot(&mut guest, "coop_08_guest_revived");
    rig.check(revived, "co-op: holding USE revives");
    rig.check(
        (eye - 1.6).abs() < 0.15,
        &format!("co-op: view back up after revive ({eye:.2} m)"),
    );

    // Team wipe: the host stops being a ghost, the guest goes down, the host dies too.
    host.chat("/notarget");
    host.chat("/god");
    let downed = rig.until(&mut [&mut host, &mut guest], 30.0, |cs| {
        cs[1].game_ref().is_some_and(|g| g.modes.is_downed(guest_id))
    });
    rig.check(downed, "co-op: guest downed again");
    host.chat("/kill");
    let wiped = rig.until(&mut [&mut host, &mut guest], 2.0, |cs| {
        cs[0].dead() && cs[1].dead()
    });
    rig.run(&mut [&mut host, &mut guest], 30);
    rig.shot(&mut host, "coop_09_team_wipe");
    let back = rig.until(&mut [&mut host, &mut guest], 6.0, |cs| {
        !cs[0].dead() && !cs[1].dead()
    });
    rig.run(&mut [&mut host, &mut guest], 30);
    rig.shot(&mut guest, "coop_10_after_wipe");
    let enemies = host.server_stats().map_or(99, |s| s.enemies);
    rig.check(
        wiped && back && enemies == 0 && host.door_open(1) == Some(true),
        &format!("co-op: team wipe -> both back, arena reset ({enemies} enemies)"),
    );

    // Extraction: the host reaches the exit; the guest stays behind.
    host.chat("/god");
    host.chat("/goto exit");
    let results = rig.until(&mut [&mut host, &mut guest], 16.0, |cs| {
        cs.iter()
            .all(|c| c.game_ref().is_some_and(InGame::results_open))
    });
    rig.shot(&mut guest, "coop_11_results_reveal_guest");
    rig.check(results, "co-op: extraction countdown ends the level for both");
    rig.run(&mut [&mut host, &mut guest], 300);
    rig.shot(&mut host, "coop_12_results_host");
    rig.shot(&mut guest, "coop_13_results_guest");
    // CONTINUE TO LOBBY (focused).
    host.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut host, &mut guest], 2);
    let lobby = rig.until(&mut [&mut host, &mut guest], 5.0, |cs| {
        cs.iter().all(|c| matches!(c.screen, Screen::Lobby(_)))
    });
    rig.run(&mut [&mut host, &mut guest], 30);
    rig.shot(&mut host, "coop_14_back_in_lobby");
    rig.check(lobby, "co-op: CONTINUE TO LOBBY brings everyone back");
    // Second match.
    let ready = rig.until(&mut [&mut host, &mut guest], 5.0, |cs| {
        cs[0]
            .lobby_ref()
            .and_then(LobbyScreen::net)
            .and_then(|n| n.lobby())
            .is_some_and(|l| l.all_ready())
    });
    rig.check(ready, "co-op: --ready readies again after a match");
    host.command(ServerCommand::Lobby(LobbyCommand::StartMatch { force: true }));
    let ok = wait_playing(rig, &mut [&mut host, &mut guest], 12.0);
    rig.run(&mut [&mut host, &mut guest], 60);
    let at_start = host
        .level()
        .is_some_and(|l| {
            l.spawns
                .iter()
                .any(|s| guest.feet().is_some_and(|f| f.distance(s.position) < 3.0))
        });
    rig.shot(&mut guest, "coop_15_second_match");
    rig.check(ok && at_start, "co-op: second match starts fresh");

    // Robustness: the guest quits mid-match, the host keeps playing.
    rig.leave(&mut guest);
    let left = rig.until(&mut [&mut host], 3.0, |cs| {
        cs[0].game_ref().and_then(InGame::net).is_some_and(|n| n.players().len() == 1)
    });
    rig.run(&mut [&mut host], 30);
    rig.shot(&mut host, "coop_16_guest_left");
    rig.check(left && host.playing(), "robustness: a guest quitting doesn't stop the host");
    rig.leave(&mut host);
}

// ---- deathmatch --------------------------------------------------------------------------------------

fn deathmatch(rig: &mut Rig) {
    println!("\n== deathmatch: host + guest + 2 bots in Cistern Nine ==");
    let (mut host, mut guest) = host_and_join(rig, "cistern_nine", GameMode::Deathmatch, 2, None);
    let ok = rig.until(&mut [&mut host, &mut guest], 8.0, |cs| {
        cs.iter().all(|c| lobby_ready(c, 4))
    });
    rig.check(ok, "DM: host, guest and 2 bots in the lobby");
    let mut settings = MatchSettings::new(GameMode::Deathmatch, "cistern_nine");
    settings.frag_limit = 3;
    settings.max_players = 8;
    host.command(ServerCommand::Lobby(LobbyCommand::SetSettings(settings)));
    rig.run(&mut [&mut host, &mut guest], 30);
    rig.shot(&mut guest, "dm_01_lobby_guest");
    host.command(ServerCommand::Lobby(LobbyCommand::StartMatch { force: true }));
    let ok = wait_playing(rig, &mut [&mut host, &mut guest], 12.0);
    rig.check(ok, "DM: both in Cistern Nine");
    rig.run(&mut [&mut host, &mut guest], 60);

    // Chat.
    if let Some(g) = guest.game() {
        g.chat.open();
    }
    rig.run(&mut [&mut guest], 1);
    guest.text = "gl hf".into();
    rig.run(&mut [&mut guest], 1);
    guest.keys = vec![UiKey::Enter];
    let got = rig.until(&mut [&mut host, &mut guest], 3.0, |cs| {
        cs[0]
            .game_ref()
            .is_some_and(|g| g.chat.lines.iter().any(|l| l.text.contains("gl hf")))
    });
    rig.check(got, "DM: chat reaches the host");

    // Name tag: the host looks at the guest.
    let guest_id = guest.me().unwrap_or(PlayerId(0));
    let gpos = host
        .game_ref()
        .and_then(InGame::net)
        .and_then(|n| n.remote_entity(NetId::player(guest_id)))
        .map(|e| e.position);
    if let Some(p) = gpos {
        host.aim(p + Vec3::Y);
    }
    rig.run(&mut [&mut host, &mut guest], 5);
    rig.shot(&mut host, "dm_02_name_tag");
    // Scoreboard with pings.
    host.pad(GamepadButton::Select, true);
    rig.run(&mut [&mut host, &mut guest], 10);
    rig.shot(&mut host, "dm_03_scoreboard");
    host.pad(GamepadButton::Select, false);
    let rows = host.game_ref().map(|g| g.score_rows().1).unwrap_or_default();
    rig.check(
        rows.len() == 4
            && rows
                .iter()
                .filter(|r| r.status != "BOT")
                .all(|r| r.ping_ms.is_some()),
        &format!("DM: scoreboard lists 4 with pings ({} rows)", rows.len()),
    );

    // The host hunts bots: each kill is a frag, bots respawn far away; 3 frags end it.
    let host_id = host.me().unwrap_or(PlayerId(0));
    let mut respawn_checked = false;
    c_fire(&mut host, true);
    let mut ended = false;
    // Bot respawns: (distance from the host who killed it, spawn protection on).
    let mut dead_bots = std::collections::BTreeSet::new();
    let mut respawns: Vec<(f32, bool)> = Vec::new();
    for _ in 0..(60 * 40) {
        let me = host.feet().unwrap_or_default();
        let all: Vec<(PlayerId, Vec3, u32)> = host
            .game_ref()
            .and_then(InGame::net)
            .map(|n| {
                n.remote_players()
                    .map(|e| (PlayerId(e.id.0 as u8), e.position, e.flags))
                    .filter(|(id, _, _)| *id != host_id && *id != guest_id)
                    .collect()
            })
            .unwrap_or_default();
        for (id, p, flags) in &all {
            if flags & player_flags::DEAD != 0 {
                dead_bots.insert(*id);
            } else if dead_bots.remove(id) {
                respawns.push((p.distance(me), flags & player_flags::PROTECTED != 0));
            }
        }
        let bots: Vec<(PlayerId, Vec3)> = host
            .game_ref()
            .and_then(InGame::net)
            .map(|n| {
                n.remote_players()
                    .filter(|e| e.flags & (player_flags::DEAD | player_flags::DOWNED) == 0)
                    .filter_map(|e| {
                        let id = PlayerId(e.id.0 as u8);
                        (id != host_id && id != guest_id).then_some((id, e.position))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some((_, p)) = bots
            .iter()
            .min_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me)))
        {
            // Stand next to it (dev teleport) and shoot.
            if p.distance(me) > 6.0 {
                host.chat(&format!("/tp {} {} {}", p.x + 3.0, p.y + 0.1, p.z));
                rig.run(&mut [&mut host, &mut guest], 10);
            }
            host.aim(*p + Vec3::Y * 1.2);
        }
        rig.run(&mut [&mut host, &mut guest], 1);
        if !respawn_checked
            && host
                .game_ref()
                .is_some_and(|g| g.score_rows().1.iter().any(|r| r.cells.first().is_some_and(|c| c == "1")))
        {
            respawn_checked = true;
            rig.shot(&mut host, "dm_04_first_frag");
        }
        if host.game_ref().is_some_and(InGame::results_open) {
            ended = true;
            break;
        }
    }
    c_fire(&mut host, false);
    rig.check(ended, "DM: frag limit 3 ends the match");
    rig.check(
        !respawns.is_empty() && respawns.iter().all(|(d, protected)| *d > 25.0 && *protected),
        &format!("DM: bots respawn far from their killer, protected ({respawns:?})"),
    );
    rig.until(&mut [&mut host, &mut guest], 3.0, |cs| {
        cs[1].game_ref().is_some_and(InGame::results_open)
    });
    rig.run(&mut [&mut host, &mut guest], 300);
    rig.shot(&mut host, "dm_05_results_host");
    rig.shot(&mut guest, "dm_06_results_guest");
    host.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut host, &mut guest], 2);
    let lobby = rig.until(&mut [&mut host, &mut guest], 5.0, |cs| {
        cs.iter().all(|c| matches!(c.screen, Screen::Lobby(_)))
    });
    rig.run(&mut [&mut host, &mut guest], 20);
    rig.shot(&mut guest, "dm_07_back_in_lobby");
    rig.check(lobby, "DM: results -> CONTINUE TO LOBBY");

    // Time limit: 8 s, no frag limit.
    let mut settings = MatchSettings::new(GameMode::Deathmatch, "cistern_nine");
    settings.frag_limit = 0;
    settings.time_limit_secs = 8;
    settings.max_players = 8;
    host.command(ServerCommand::Lobby(LobbyCommand::SetSettings(settings)));
    rig.run(&mut [&mut host, &mut guest], 20);
    host.command(ServerCommand::Lobby(LobbyCommand::StartMatch { force: true }));
    let ok = wait_playing(rig, &mut [&mut host, &mut guest], 12.0);
    let started = Instant::now();
    let ended = rig.until(&mut [&mut host, &mut guest], 15.0, |cs| {
        cs.iter().all(|c| c.game_ref().is_some_and(InGame::results_open))
    });
    let secs = started.elapsed().as_secs_f32();
    rig.run(&mut [&mut host, &mut guest], 200);
    rig.shot(&mut guest, "dm_08_time_limit_results");
    rig.check(
        ok && ended && (6.0..11.0).contains(&secs),
        &format!("DM: an 8 s time limit ends the match after {secs:.1} s"),
    );
    host.keys = vec![UiKey::Enter];
    rig.run(&mut [&mut host, &mut guest], 2);
    rig.until(&mut [&mut host, &mut guest], 5.0, |cs| {
        cs.iter().all(|c| matches!(c.screen, Screen::Lobby(_)))
    });

    // Robustness: the host's game vanishes mid-match (process gone, no goodbye).
    host.command(ServerCommand::Lobby(LobbyCommand::StartMatch { force: true }));
    wait_playing(rig, &mut [&mut host, &mut guest], 12.0);
    rig.run(&mut [&mut host, &mut guest], 30);
    let started = Instant::now();
    rig.leave(&mut host);
    let menu = rig.until(&mut [&mut guest], 20.0, |cs| matches!(cs[0].screen, Screen::Menu(_)));
    rig.run(&mut [&mut guest], 10);
    rig.shot(&mut guest, "robust_01_host_gone");
    rig.check(
        menu && guest.menu_message.is_some(),
        &format!(
            "robustness: host gone -> guest at the menu after {:.1} s with {:?}",
            started.elapsed().as_secs_f32(),
            guest.menu_message
        ),
    );
    rig.leave(&mut guest);
}

fn c_fire(c: &mut Client, on: bool) {
    c.axis(GamepadAxis::RightTrigger, if on { 1.0 } else { 0.0 });
}

// ---- robustness --------------------------------------------------------------------------------------

fn robustness(rig: &mut Rig) {
    println!("\n== robustness ==");
    // Full server: a host with one slot.
    let mut host = rig.client(
        "Host",
        LaunchOptions {
            max_players: Some(1),
            ..Default::default()
        },
    );
    rig.transition(
        &mut host,
        Transition::Start(StartMode::Host {
            port: Some(0),
            level: Some("crucible_of_veins".into()),
        }),
    );
    rig.run(&mut [&mut host], 30);
    let port = host
        .lobby_ref()
        .and_then(LobbyScreen::session)
        .and_then(|s| s.hosted_port())
        .unwrap_or(0);
    let mut late = rig.client("Late", LaunchOptions::default());
    rig.transition(
        &mut late,
        Transition::Start(StartMode::Join {
            address: format!("127.0.0.1:{port}"),
        }),
    );
    let refused = rig.until(&mut [&mut host, &mut late], 10.0, |cs| {
        matches!(cs[1].screen, Screen::Menu(_))
    });
    rig.run(&mut [&mut late], 10);
    rig.shot(&mut late, "robust_02_server_full");
    rig.check(
        refused
            && late
                .menu_message
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains("full")),
        &format!("robustness: full server -> {:?}", late.menu_message),
    );
    rig.leave(&mut host);
    rig.leave(&mut late);

    // Nobody there.
    let mut lost = rig.client("Lost", LaunchOptions::default());
    let started = Instant::now();
    rig.transition(
        &mut lost,
        Transition::Start(StartMode::Join {
            address: "127.0.0.1:9".into(),
        }),
    );
    let gave_up = rig.until(&mut [&mut lost], 20.0, |cs| {
        matches!(cs[0].screen, Screen::Menu(_))
    });
    let secs = started.elapsed().as_secs_f32();
    rig.run(&mut [&mut lost], 10);
    rig.shot(&mut lost, "robust_03_no_server");
    // netcode's unsecure connect tokens time out after 15 s (the panel has CANCEL meanwhile).
    rig.check(
        gave_up && lost.menu_message.is_some() && secs < 17.0,
        &format!(
            "robustness: no server -> message after {secs:.1} s: {:?}",
            lost.menu_message
        ),
    );
    // Garbage addresses.
    for bad in ["not an address", "999.1.1.1:5", "host:notaport", ""] {
        lost.menu_message = None;
        rig.transition(
            &mut lost,
            Transition::Start(StartMode::Join {
                address: bad.into(),
            }),
        );
        rig.run(&mut [&mut lost], 2);
        rig.check(
            matches!(lost.screen, Screen::Menu(_)) && lost.menu_message.is_some(),
            &format!("robustness: join '{bad}' -> {:?}", lost.menu_message),
        );
    }

    // A dedicated server process killed mid-match (no goodbye packet).
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|s| s.local_addr())
        .map_or(47999, |a| a.port());
    let server_bin = std::env::current_exe()
        .ok()
        .and_then(|p| Some(p.parent()?.parent()?.join("velocide-server")));
    match server_bin.map(|bin| {
        std::process::Command::new(bin)
            .args([
                "--port",
                &port.to_string(),
                "--level",
                "cistern_nine",
                "--autostart",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }) {
        Some(Ok(mut child)) => {
            std::thread::sleep(Duration::from_millis(800));
            let mut joiner = rig.client(
                "Joiner",
                LaunchOptions {
                    ready: true,
                    ..Default::default()
                },
            );
            rig.transition(
                &mut joiner,
                Transition::Start(StartMode::Join {
                    address: format!("127.0.0.1:{port}"),
                }),
            );
            let playing = wait_playing(rig, &mut [&mut joiner], 15.0);
            rig.check(playing, "robustness: dedicated server --autostart puts a joiner in");
            rig.run(&mut [&mut joiner], 60);
            let _ = child.kill();
            let _ = child.wait();
            let killed_at = Instant::now();
            rig.run(&mut [&mut joiner], 120);
            rig.shot(&mut joiner, "robust_04_server_stalled");
            let back = rig.until(&mut [&mut joiner], 25.0, |cs| {
                matches!(cs[0].screen, Screen::Menu(_))
            });
            let secs = killed_at.elapsed().as_secs_f32();
            rig.run(&mut [&mut joiner], 10);
            rig.shot(&mut joiner, "robust_05_server_killed");
            rig.check(
                back && joiner.menu_message.is_some(),
                &format!(
                    "robustness: dedicated server killed -> menu after {secs:.1} s: {:?}",
                    joiner.menu_message
                ),
            );
            rig.leave(&mut joiner);
        }
        _ => println!("SKIP  dedicated server binary not found (cargo build --release first)"),
    }

    // Menu key spam: every page, back and forth (never starts anything that needs a server
    // that isn't there for long).
    let keys = [
        UiKey::Down,
        UiKey::Up,
        UiKey::Enter,
        UiKey::Escape,
        UiKey::Left,
        UiKey::Right,
        UiKey::Down,
        UiKey::Down,
        UiKey::Enter,
        UiKey::Escape,
        UiKey::Escape,
    ];
    let mut seed = 12345u32;
    for _ in 0..900 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let k = keys[(seed >> 16) as usize % keys.len()];
        // Never QUIT (5th root item) or start a game: Escape back to the root often.
        lost.keys = vec![k];
        rig.run(&mut [&mut lost], 1);
        if !matches!(lost.screen, Screen::Menu(_)) {
            // Something started (single player / host): leave again.
            rig.run(&mut [&mut lost], 30);
            rig.go_to_menu(&mut lost, None);
        }
    }
    rig.check(true, "robustness: 900 frames of random menu keys, no panic");
    rig.leave(&mut lost);
}

// ---- settings ------------------------------------------------------------------------------------------

/// Pixels that differ noticeably between two frames.
fn differing(a: &engine::gfx::TextureData, b: &engine::gfx::TextureData) -> usize {
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(p, q)| (0..3).any(|i| p[i].abs_diff(q[i]) > 6))
        .count()
}

type Tweak = (&'static str, fn(&mut Settings));

fn settings(rig: &mut Rig) {
    println!("\n== settings: every option applies live ==");
    let mut c = rig.client("Tweaker", LaunchOptions::default());
    rig.transition(
        &mut c,
        Transition::Start(StartMode::SinglePlayer {
            level: Some("crucible_of_veins".into()),
        }),
    );
    wait_playing(rig, &mut [&mut c], 20.0);
    rig.run(&mut [&mut c], 120);
    let grab = |rig: &mut Rig, c: &mut Client| {
        rig.frame(c);
        rig.renderer.read_pixels().expect("pixels")
    };
    let base = grab(rig, &mut c);
    rig.run(&mut [&mut c], 20);
    let noise = differing(&base, &grab(rig, &mut c));
    println!("      frame-to-frame noise: {noise} pixels");
    let tweaks: [Tweak; 17] = [
        ("internal resolution 240p", |s| s.retro.internal_height = 240),
        ("internal resolution 720p", |s| s.retro.internal_height = 720),
        ("vertex snapping off", |s| s.retro.vertex_snap = false),
        ("snap strength 3x", |s| s.retro.vertex_snap_strength = 3.0),
        ("affine textures on", |s| s.retro.affine_textures = true),
        ("dithering off", |s| s.retro.dithering = false),
        ("colour depth 3 bit", |s| s.retro.color_bits = 3),
        ("colour depth 8 bit", |s| s.retro.color_bits = 8),
        ("fog off", |s| s.retro.fog = false),
        ("vertex lighting off", |s| s.retro.vertex_lighting = false),
        ("FOV 80", |s| s.fov = 80.0),
        ("weapon FOV 40", |s| s.viewmodel_fov = 40.0),
        ("weapon hidden", |s| s.show_viewmodel = false),
        ("crosshair style 2", |s| s.crosshair = 2),
        ("speedometer off", |s| s.show_speed = false),
        ("timer off", |s| s.show_timer = false),
        ("FPS counter off", |s| s.show_fps = false),
    ];
    for (i, (name, tweak)) in tweaks.into_iter().enumerate() {
        let before = c.common.settings.clone();
        rig.frame(&mut c);
        let ui_before = rig.buffers.ui.quads.len();
        tweak(&mut c.common.settings);
        c.common.settings_changed = true;
        rig.run(&mut [&mut c], 4);
        let img = grab(rig, &mut c);
        let ui_after = rig.buffers.ui.quads.len();
        let d = differing(&base, &img);
        let file = format!("settings_{}", name.replace([' ', '/'], "_"));
        rig.shot(&mut c, &file);
        // The last four are small HUD elements: their UI quads come and go.
        let ok = if i >= tweaks.len() - 4 {
            ui_after != ui_before
        } else {
            d > noise * 2 + 300
        };
        rig.check(
            ok,
            &format!("settings: {name} changes the picture ({d} px, ui {ui_before} -> {ui_after})"),
        );
        c.common.settings = before;
        c.common.settings_changed = true;
        rig.run(&mut [&mut c], 4);
    }
    // Saved and loaded back (a "restart" of the game), garbage files give defaults.
    let path = rig.out.join("settings-persist.ron");
    let mut s = Settings::default();
    s.retro.color_bits = 3;
    s.fov = 95.0;
    s.bindings.bind(
        client::controls::Action::Dash,
        engine::input::Binding::Key(engine::input::Key::KeyV),
    );
    let saved = s.save(&path).is_ok();
    rig.check(saved && Settings::load(&path) == s, "settings: survive a restart");
    for garbage in ["", "((((", "\u{0}\u{1}binary", "(fov: \"wide\")", "(retro: 5)"] {
        let _ = std::fs::write(&path, garbage);
        let loaded = Settings::load(&path);
        rig.check(
            loaded == Settings::default(),
            &format!("settings: garbage file {garbage:?} -> defaults"),
        );
    }
    rig.leave(&mut c);
}

// ---- performance -------------------------------------------------------------------------------------

fn percentile(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * p).round() as usize]
}

fn performance(rig: &mut Rig) {
    println!("\n== performance (release) ==");
    // Client: the Furnace Cathedral (arena 5) full of enemies, Cinderbox explosions.
    let mut c = rig.client("Bench", LaunchOptions::default());
    rig.transition(
        &mut c,
        Transition::Start(StartMode::SinglePlayer {
            level: Some("crucible_of_veins".into()),
        }),
    );
    wait_playing(rig, &mut [&mut c], 20.0);
    rig.run(&mut [&mut c], 30);
    c.chat("/god");
    walk_into_encounter(&mut c, 4);
    rig.run(&mut [&mut c], 150);
    c.chat("/spawn gnawer 14");
    c.chat("/spawn censer 6");
    c.chat("/spawn harrier 6");
    c.chat("/spawn heron 2");
    c.chat("/spawn gaoler 2");
    rig.run(&mut [&mut c], 120);
    let enemies = c.server_stats().map_or(0, |s| s.enemies);
    println!("      enemies alive on the server: {enemies}");
    c.pad(GamepadButton::DPadRight, true); // 2 Thresher
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::DPadRight, false);
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::DPadRight, true); // 3 Riveter
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::DPadRight, false);
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::DPadRight, true); // 4 Cinderbox
    rig.run(&mut [&mut c], 1);
    c.pad(GamepadButton::DPadRight, false);
    c_fire(&mut c, true);
    let mut cpu = Vec::new();
    let mut with_gpu = Vec::new();
    for i in 0..600 {
        let me = c.feet().unwrap_or_default();
        if let Some(e) = c
            .enemies()
            .into_iter()
            .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
        {
            c.aim(e + Vec3::Y * 0.5);
        }
        std::thread::sleep(Duration::from_secs_f32(DT));
        let t0 = Instant::now();
        rig.frame(&mut c);
        cpu.push(t0.elapsed().as_secs_f32() * 1000.0);
        // Reading the frame back waits for the GPU: an upper bound of the full frame cost.
        let _ = rig.renderer.read_pixels();
        with_gpu.push(t0.elapsed().as_secs_f32() * 1000.0);
        if i == 300 {
            rig.shot(&mut c, "perf_arena5_crowd");
            println!(
                "      mid-fight: {} enemies, {} projectiles, {} particles",
                c.server_stats().map_or(0, |s| s.enemies),
                c.server_stats().map_or(0, |s| s.projectiles),
                c.game_ref().map_or(0, |g| g.effects.particle_count())
            );
        }
    }
    c_fire(&mut c, false);
    let tick_us = c.server_stats().map_or(0.0, |s| s.tick_time_us);
    println!(
        "      client frame (CPU, 1280x720): median {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        percentile(&mut cpu, 0.5),
        percentile(&mut cpu, 0.99),
        percentile(&mut cpu, 1.0)
    );
    println!(
        "      client frame incl. GPU readback: median {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        percentile(&mut with_gpu, 0.5),
        percentile(&mut with_gpu, 0.99),
        percentile(&mut with_gpu, 1.0)
    );
    println!("      in-process server avg tick: {tick_us:.0} us");
    rig.check(
        percentile(&mut with_gpu, 0.99) < 16.0,
        "perf: ~40 enemies + explosions under 16 ms per frame (p99)",
    );
    rig.leave(&mut c);

    // Server: 8 players running, jumping and firing rockets in Cistern Nine.
    let server = match server::ServerHandle::spawn(server::ServerConfig {
        bind: "127.0.0.1:0".parse().expect("addr"),
        level: "cistern_nine".into(),
        max_players: 8,
        watch_tuning: false,
        ..Default::default()
    }) {
        Ok(s) => s,
        Err(e) => {
            rig.check(false, &format!("perf: server starts ({e:#})"));
            return;
        }
    };
    let addr = server.connect_addr();
    let mut clients: Vec<client::net::NetClient> = (0..8)
        .filter_map(|i| {
            client::net::NetClient::connect(addr, client::net::ClientConfig::new(&format!("P{i}")))
                .ok()
        })
        .collect();
    let mut ticks_us = Vec::new();
    for frame in 0..(60 * 25) {
        std::thread::sleep(Duration::from_secs_f32(DT));
        for (i, n) in clients.iter_mut().enumerate() {
            n.update(Duration::from_secs_f32(DT));
            let t = frame as f32 * DT;
            let mut buttons = shared::input::Buttons::FIRE;
            if (frame + i * 7) % 50 == 0 {
                buttons.set(shared::input::Buttons::JUMP, true);
            }
            if (frame + i * 13) % 140 == 0 {
                buttons.set(shared::input::Buttons::DASH, true);
            }
            let input = shared::input::PlayerInput {
                forward: 127,
                right: if (frame / 90 + i) % 2 == 0 { 60 } else { -60 },
                yaw: t * (0.6 + i as f32 * 0.1) + i as f32,
                pitch: -0.1,
                buttons,
                weapon: ((frame / 300 + i) % 4) as u8,
                weapon_mode: 0,
                ..Default::default()
            };
            let _ = n.tick(input);
            let _ = n.drain_events();
        }
        if frame > 300 && frame % 60 == 0 {
            ticks_us.push(server.stats().tick_time_us);
        }
    }
    let stats = server.stats();
    println!(
        "      dedicated-style server, {} players: avg tick {:.0} us (samples: median {:.0}, max {:.0}), {} projectiles live",
        stats.players,
        stats.tick_time_us,
        percentile(&mut ticks_us, 0.5),
        percentile(&mut ticks_us, 1.0),
        stats.projectiles
    );
    rig.check(
        stats.players == 8 && stats.tick_time_us < 4000.0,
        "perf: 8-player server tick well under the 16.7 ms budget",
    );
    for n in &mut clients {
        n.disconnect();
    }
    server.stop();
}

// ---- window sizes ------------------------------------------------------------------------------------

fn window_sizes(rig: &mut Rig) {
    println!("\n== window sizes: resize, minimise, other aspect ratios ==");
    let mut c = rig.client("Resizer", LaunchOptions::default());
    rig.transition(
        &mut c,
        Transition::Start(StartMode::SinglePlayer {
            level: Some("crucible_of_veins".into()),
        }),
    );
    wait_playing(rig, &mut [&mut c], 20.0);
    c.chat("/spawn gnawer 3");
    rig.run(&mut [&mut c], 90);
    // A long kill-feed line to see where it lands at every size.
    c.chat("/killall");
    for (w, h, name) in [
        (800, 600, "4x3"),
        (2560, 1080, "ultrawide"),
        (640, 480, "small"),
        (0, 0, "minimised"),
        (1920, 1080, "1080p"),
    ] {
        rig.size = (w, h);
        rig.renderer.resize(w, h);
        rig.run(&mut [&mut c], 30);
        if w > 0 {
            rig.shot(&mut c, &format!("window_{name}"));
        }
        rig.check(c.playing(), &format!("window: {name} ({w}x{h}) keeps the game running"));
    }
    rig.size = (W, H);
    rig.renderer.resize(W, H);
    rig.run(&mut [&mut c], 10);
    rig.leave(&mut c);
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/playthrough"));
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let mut rig = Rig {
        renderer,
        size: (W, H),
        buffers: FrameBuffers::default(),
        out,
        fails: Vec::new(),
        passes: 0,
        frame_ms: Vec::new(),
    };
    let only = std::env::var("PLAYTHROUGH").unwrap_or_default();
    let want = |s: &str| only.is_empty() || only.split(',').any(|o| o == s);
    if want("sp") {
        single_player(&mut rig);
    }
    if want("coop") {
        coop(&mut rig);
    }
    if want("dm") {
        deathmatch(&mut rig);
    }
    if want("robust") {
        robustness(&mut rig);
    }
    if want("settings") {
        settings(&mut rig);
    }
    if want("perf") {
        performance(&mut rig);
    }
    if want("window") {
        window_sizes(&mut rig);
    }
    let mut ms = rig.frame_ms.clone();
    ms.sort_by(f32::total_cmp);
    if !ms.is_empty() {
        println!(
            "\nframe + render time over {} frames: median {:.2} ms, p99 {:.2} ms",
            ms.len(),
            ms[ms.len() / 2],
            ms[ms.len() * 99 / 100]
        );
    }
    println!("\n{} passed, {} failed", rig.passes, rig.fails.len());
    for f in &rig.fails {
        println!("  FAIL {f}");
    }
    if !rig.fails.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}
