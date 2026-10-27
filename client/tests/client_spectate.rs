//! Spectating a teammate while dead / downed / out in PvE (`client::ingame::spectate`,
//! docs/style_modes.md "Spectating"): who is watched first, cycling, moving on when the
//! watched one goes down, the fallback to the death view when nobody is up, stopping the
//! moment you are back; then for real, two clients in campaign co-op on a real server: one
//! goes down and watches the other from their eyes, JUMP swaps to the chase camera, a team
//! wipe falls back to the death view and the respawn brings the own camera back. The real
//! session skips without a GPU adapter.

use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::ingame::spectate::{
    SPECTATE_DELAY, Spectate, SpectateInput, cycle, first_target, living_teammates,
};
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{Camera, RetroSettings, UiDrawList};
use engine::input::{GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::UiInput;
use glam::{Vec2, Vec3};
use server::{ServerConfig, ServerHandle};
use shared::protocol::{EntityState, NetId, kind, player_flags};
use shared::sim::PlayerId;

fn player(id: u8, position: Vec3, flags: u32) -> EntityState {
    EntityState {
        id: NetId::player(PlayerId(id)),
        kind: kind::PLAYER,
        position,
        flags,
        ..Default::default()
    }
}

const UP: u32 = player_flags::ON_GROUND;
const DOWNED: u32 = player_flags::DOWNED;
const DEAD: u32 = player_flags::DEAD;

const NONE: SpectateInput = SpectateInput {
    next: false,
    prev: false,
    toggle: false,
};
const NEXT: SpectateInput = SpectateInput { next: true, ..NONE };
const PREV: SpectateInput = SpectateInput { prev: true, ..NONE };
const TOGGLE: SpectateInput = SpectateInput {
    toggle: true,
    ..NONE
};

#[test]
fn the_nearest_teammate_who_is_up_is_watched_first() {
    let me = Vec3::ZERO;
    let team = [
        player(1, Vec3::new(30.0, 0.0, 0.0), UP),
        player(2, Vec3::new(5.0, 0.0, 0.0), DOWNED),
        player(3, Vec3::new(12.0, 0.0, 0.0), UP),
        player(4, Vec3::new(2.0, 0.0, 0.0), DEAD),
    ];
    assert_eq!(living_teammates(&team), vec![PlayerId(1), PlayerId(3)]);
    assert_eq!(first_target(&team, Some(me)), Some(PlayerId(3)));
    // Without a position: the first by id; ties go to the lower id.
    assert_eq!(first_target(&team, None), Some(PlayerId(1)));
    let tie = [
        player(5, Vec3::X * 4.0, UP),
        player(2, Vec3::NEG_X * 4.0, UP),
    ];
    assert_eq!(first_target(&tie, Some(me)), Some(PlayerId(2)));
    // Nobody up: nobody to watch.
    let wiped = [player(1, Vec3::ZERO, DOWNED), player(2, Vec3::ZERO, DEAD)];
    assert_eq!(first_target(&wiped, Some(me)), None);
    assert_eq!(cycle(&wiped, Some(PlayerId(1)), 1), None);
}

#[test]
fn cycling_wraps_round_the_teammates_who_are_up() {
    let team = [
        player(3, Vec3::ZERO, UP),
        player(1, Vec3::ZERO, UP),
        player(2, Vec3::ZERO, DOWNED),
        player(6, Vec3::ZERO, UP),
    ];
    let p = |n| Some(PlayerId(n));
    assert_eq!(cycle(&team, p(1), 1), p(3), "skips the downed one");
    assert_eq!(cycle(&team, p(3), 1), p(6));
    assert_eq!(cycle(&team, p(6), 1), p(1), "wraps");
    assert_eq!(cycle(&team, p(1), -1), p(6), "wraps backwards");
    assert_eq!(cycle(&team, p(6), -1), p(3));
    // From someone who went down (or left), their place in the order still counts.
    assert_eq!(cycle(&team, p(2), 1), p(3));
    assert_eq!(cycle(&team, p(2), -1), p(1));
    assert_eq!(cycle(&team, p(9), 1), p(1));
    assert_eq!(cycle(&team, None, 1), p(1));
    assert_eq!(cycle(&team, None, -1), p(6));
}

#[test]
fn spectating_starts_after_the_death_view_and_stops_when_you_are_back() {
    let mut s = Spectate::new();
    let team = [
        player(1, Vec3::new(20.0, 0.0, 0.0), UP),
        player(2, Vec3::new(4.0, 0.0, 0.0), UP),
    ];
    let me = Some(Vec3::ZERO);
    // Alive (or not PvE): never.
    assert!(!s.step(false, &team, me, 0.0, NONE));
    assert_eq!(s.target(), None);
    // Down at t = 10: the death view first ...
    s.step(true, &team, me, 10.0, NONE);
    s.step(true, &team, me, 10.0 + SPECTATE_DELAY - 0.1, NONE);
    assert_eq!(s.target(), None, "still the death view");
    // ... then the nearest teammate, from their eyes.
    assert!(s.step(true, &team, me, 10.0 + SPECTATE_DELAY, NONE));
    assert_eq!(s.target(), Some(PlayerId(2)));
    assert!(!s.chase());
    assert_eq!(
        s.hidden_player(),
        Some(PlayerId(2)),
        "their model is hidden"
    );
    // FIRE / ALT_FIRE cycle; JUMP swaps to the chase camera (their model drawn again once
    // the camera has backed off; it starts right behind their head).
    s.step(true, &team, me, 12.0, NEXT);
    assert_eq!(s.target(), Some(PlayerId(1)));
    s.step(true, &team, me, 12.1, PREV);
    assert_eq!(s.target(), Some(PlayerId(2)));
    s.step(true, &team, me, 12.2, TOGGLE);
    assert!(s.chase());
    // Revived / respawned: the own view at once (the chase choice is kept for next time).
    assert!(s.step(false, &team, me, 12.3, NONE));
    assert_eq!(s.target(), None);
    assert!(!s.is_active() && s.chase());
    // Down again: the delay starts over; a press skips it.
    s.step(true, &team, me, 20.0, NONE);
    assert_eq!(s.target(), None);
    s.step(true, &team, me, 20.2, NEXT);
    assert_eq!(s.target(), Some(PlayerId(2)), "FIRE skips the wait");
    // JUMP in the death view does nothing.
    let mut t = Spectate::new();
    t.step(true, &team, me, 0.0, TOGGLE);
    assert!(!t.chase());
}

#[test]
fn the_view_moves_on_when_the_watched_teammate_goes_down() {
    let mut s = Spectate::new();
    let me = Some(Vec3::ZERO);
    let mut team = [
        player(1, Vec3::new(3.0, 0.0, 0.0), UP),
        player(2, Vec3::new(9.0, 0.0, 0.0), UP),
        player(3, Vec3::new(30.0, 0.0, 0.0), UP),
    ];
    s.step(true, &team, me, 0.0, NONE);
    s.step(true, &team, me, SPECTATE_DELAY, NONE);
    assert_eq!(s.target(), Some(PlayerId(1)));
    // The one we watch goes down: the next one up.
    team[0].flags = DOWNED;
    assert!(s.step(true, &team, me, 3.0, NONE));
    assert_eq!(s.target(), Some(PlayerId(2)));
    // ... and leaves the game: the next one after them.
    let rest = [team[0], team[2]];
    assert!(s.step(true, &rest, me, 3.1, NONE));
    assert_eq!(s.target(), Some(PlayerId(3)));
    // Nobody left up (a team wipe): the death view.
    let wiped = [player(1, Vec3::ZERO, DEAD), player(3, Vec3::ZERO, DOWNED)];
    assert!(s.step(true, &wiped, me, 3.2, NONE));
    assert_eq!(s.target(), None);
    // Someone is up again (revived): back to watching them.
    let revived = [player(1, Vec3::ZERO, DEAD), player(3, Vec3::ZERO, UP)];
    s.step(true, &revived, me, 3.3, NONE);
    assert_eq!(s.target(), Some(PlayerId(3)));
    // Solo: nobody to watch, ever.
    let mut solo = Spectate::new();
    solo.step(true, &[], me, 0.0, NONE);
    solo.step(true, &[], me, 60.0, NEXT);
    assert_eq!(solo.target(), None);
}

// ---- for real: two clients, campaign co-op ---------------------------------------------------

const W: u32 = 320;
const H: u32 = 180;
const DT: f32 = 1.0 / 60.0;

struct Client {
    common: Common,
    input: InputState,
    game: InGame,
    time: f64,
    camera: Camera,
}

impl Client {
    fn id(&self) -> PlayerId {
        self.game
            .net()
            .and_then(|n| n.local_player_id())
            .expect("connected")
    }

    fn down(&self) -> bool {
        self.game
            .net()
            .and_then(|n| n.own_combat())
            .is_some_and(|c| c.combat.dead)
    }

    /// Where this client's player stands (feet), from its own prediction.
    fn feet(&self) -> Vec3 {
        self.game
            .net()
            .and_then(|n| n.local_state())
            .map_or(Vec3::ZERO, |s| s.position)
    }
}

struct Session {
    renderer: Renderer,
    buffers: FrameBuffers,
}

impl Session {
    fn frame(&mut self, c: &mut Client) {
        c.time += DT as f64;
        let ui = UiInput {
            mouse_pos: Vec2::splat(-100.0),
            time: c.time,
            ..Default::default()
        };
        self.buffers.clear(UiDrawList::canvas_for_window(W, H));
        let out = {
            let mut io = FrameIo {
                renderer: &mut self.renderer,
                input: &c.input,
                ui_input: &ui,
                dt: DT,
                time: c.time,
                window_size: (W, H),
            };
            c.game.frame(&mut c.common, &mut io, &mut self.buffers)
        };
        c.camera = out.scene.camera;
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            panic!("render: {e:#}");
        }
        c.input.end_frame();
    }

    /// `n` real-time frames of both clients (the server runs on its own thread).
    fn run(&mut self, a: &mut Client, b: &mut Client, n: usize) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            self.frame(a);
            self.frame(b);
        }
    }

    /// Runs until `done` (at most `max` frames); returns whether it happened.
    fn until(
        &mut self,
        a: &mut Client,
        b: &mut Client,
        max: usize,
        done: impl Fn(&Client, &Client) -> bool,
    ) -> bool {
        for _ in 0..max {
            self.run(a, b, 1);
            if done(a, b) {
                return true;
            }
        }
        false
    }
}

fn join(renderer: &mut Renderer, address: &str, name: &str) -> Client {
    let path = std::env::temp_dir().join(format!(
        "velocide-spectate-test-{name}-{}.ron",
        std::process::id()
    ));
    let mut common = Common::new(
        Settings::default(),
        path,
        Audio::disabled(),
        LaunchOptions::default(),
    );
    common.name_override = Some(name.to_string());
    let mode = StartMode::Join {
        address: address.to_string(),
    };
    let game = InGame::start(&mode, &mut common, renderer).expect("joins");
    Client {
        common,
        input: InputState::new(),
        game,
        time: 10.0,
        camera: Camera::default(),
    }
}

/// A one-frame gamepad button tap.
fn tap(s: &mut Session, a: &mut Client, b: &mut Client, button: GamepadButton) {
    b.input
        .handle_gamepad_event(GamepadEvent::Button(button, true));
    s.run(a, b, 1);
    b.input
        .handle_gamepad_event(GamepadEvent::Button(button, false));
    s.run(a, b, 1);
}

#[test]
fn a_downed_player_watches_their_teammate() {
    let renderer = match Renderer::new_headless(W, H, &RetroSettings::default()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e:#})");
            return;
        }
    };
    let server = ServerHandle::spawn(ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: "crucible_of_veins".into(),
        max_players: 4,
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some("/nonexistent/velocide-test/tuning.ron".into()),
        watch_tuning: false,
        dev_commands: true, // `/kill`
        ..Default::default()
    })
    .expect("server starts");
    let address = server.connect_addr().to_string();
    let mut s = Session {
        renderer,
        buffers: FrameBuffers::default(),
    };
    let mut a = join(&mut s.renderer, &address, "Alpha");
    let mut b = join(&mut s.renderer, &address, "Bravo");
    assert!(
        s.until(&mut a, &mut b, 900, |a, b| a.game.is_playing()
            && b.game.is_playing()),
        "both in game"
    );
    s.run(&mut a, &mut b, 60);
    let alpha = a.id();
    assert!(!b.game.spectate.is_active(), "alive: your own view");

    // Bravo goes down: the death view first, then Alpha from their eyes.
    b.game.send_chat("/kill");
    assert!(
        s.until(&mut a, &mut b, 120, |_, b| b.down()),
        "Bravo is down"
    );
    s.run(&mut a, &mut b, 30);
    assert!(!b.game.spectate.is_active(), "the death view first");
    assert!(
        s.until(&mut a, &mut b, 150, |_, b| b.game.spectate.is_active()),
        "watching a teammate after {SPECTATE_DELAY} s"
    );
    assert_eq!(b.game.spectate.target(), Some(alpha));
    assert!(!b.game.spectate.chase());
    s.run(&mut a, &mut b, 10);
    let eye = a.feet() + Vec3::Y * 1.6;
    assert!(
        b.camera.position.distance(eye) < 0.6,
        "Bravo sees from Alpha's eyes: {} vs {eye}",
        b.camera.position
    );

    // JUMP (gamepad A): the chase camera, behind and above Alpha's head, off the walls.
    tap(&mut s, &mut a, &mut b, GamepadButton::South);
    assert!(b.game.spectate.chase());
    s.run(&mut a, &mut b, 60);
    let head = a.feet() + Vec3::Y * 1.55;
    let d = b.camera.position.distance(head);
    assert!(
        d > 1.0 && d < 3.7,
        "the chase camera sits behind Alpha: {d} m"
    );
    assert!(b.camera.position.y > head.y, "looking down at them");
    tap(&mut s, &mut a, &mut b, GamepadButton::South);
    assert!(!b.game.spectate.chase(), "JUMP again: their eyes");
    // FIRE (the right trigger) with nobody else up: still Alpha.
    b.input.handle_gamepad_event(GamepadEvent::Axis(
        engine::input::GamepadAxis::RightTrigger,
        1.0,
    ));
    s.run(&mut a, &mut b, 1);
    b.input.handle_gamepad_event(GamepadEvent::Axis(
        engine::input::GamepadAxis::RightTrigger,
        0.0,
    ));
    s.run(&mut a, &mut b, 1);
    assert_eq!(b.game.spectate.target(), Some(alpha));

    // Alpha goes down too: a team wipe, nobody to watch, the death view.
    a.game.send_chat("/kill");
    assert!(
        s.until(&mut a, &mut b, 120, |a, _| a.down()),
        "Alpha is down"
    );
    assert!(
        s.until(&mut a, &mut b, 30, |_, b| !b.game.spectate.is_active()),
        "nobody up: back to the death view"
    );
    assert!(b.down());
    // Everyone respawns together: Bravo's own camera again, at once.
    assert!(
        s.until(&mut a, &mut b, 400, |a, b| !a.down() && !b.down()),
        "the wipe respawns everyone"
    );
    s.run(&mut a, &mut b, 2);
    assert!(!b.game.spectate.is_active());
    let own = b.feet();
    assert!(
        b.camera.position.distance(own) < 2.0,
        "Bravo's own view: {} vs feet {own}",
        b.camera.position
    );
    a.game.shutdown(&mut s.renderer, &mut a.common);
    b.game.shutdown(&mut s.renderer, &mut b.common);
    server.stop();
}
