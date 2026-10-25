//! Headless captures of the style meter and game-mode screens on the real client (GPU, no
//! window): the HUD style meter at several ranks with the style feed and STALE indicator, the
//! rank-up banner, the checkpoint banner, the deathmatch HUD and Tab scoreboard, co-op downed /
//! revive (two real clients on a real server: one goes down via `/kill`, the other walks up and
//! holds USE), and the campaign (solo + co-op) and deathmatch results screens mid-reveal and
//! finished.
//!
//! ```text
//! cargo run -p client --release --example style_shots -- [out_dir]   # default target/style_shots
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::{GamepadAxis, GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use glam::{Vec2, Vec3};
use server::lobby::LobbyCommand;
use server::{ServerCommand, ServerConfig, ServerHandle};
use shared::sim::{
    EndReason, Grade, Grades, LifeState, MatchPhase, MatchResults, ModeEvent, ModeRules, PlayerId,
    PlayerResult, PlayerStats, ScoreEntry, Scoreboard, StyleBonus, StyleEvent, StyleKey, StyleView,
};

/// A HUD capture: rank, meter fill, stale keys, style feed lines.
type RankShot = (u8, f32, &'static [StyleKey], &'static [(StyleBonus, u8)]);

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

/// One client: its app state, input and game screen.
struct Client {
    common: Common,
    input: InputState,
    game: InGame,
    time: f64,
}

struct Shots {
    renderer: Renderer,
    buffers: FrameBuffers,
    out: PathBuf,
}

impl Shots {
    /// One frame of `c` with scripted UI keys / text; renders it.
    fn frame(&mut self, c: &mut Client, keys: &[UiKey], text: &str) {
        c.time += DT as f64;
        c.common.stats.update(DT);
        let ui = UiInput {
            mouse_pos: Vec2::splat(-100.0),
            keys: keys.to_vec(),
            text: text.to_string(),
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
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            eprintln!("render: {e:#}");
        }
        c.input.end_frame();
    }

    /// `n` real-time frames (the in-process server runs on its own thread).
    fn run(&mut self, c: &mut Client, n: usize) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            self.frame(c, &[], "");
        }
    }

    /// Frames of two clients side by side.
    fn run2(&mut self, a: &mut Client, b: &mut Client, n: usize) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            self.frame(a, &[], "");
            self.frame(b, &[], "");
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

    /// Renders one more frame of `c` and saves it.
    fn capture(&mut self, c: &mut Client, name: &str) {
        self.frame(c, &[], "");
        self.shot(name);
    }
}

fn client(renderer: &mut Renderer, mode: StartMode, name: &str) -> anyhow::Result<Client> {
    let scratch = std::env::temp_dir().join(format!("velocide-style-shots-{name}.ron"));
    let mut common = Common::new(
        Settings::default(),
        scratch,
        Audio::disabled(),
        LaunchOptions::default(),
    );
    common.name_override = Some(name.to_string());
    let game = InGame::start(&mode, &mut common, renderer)?;
    Ok(Client {
        common,
        input: InputState::new(),
        game,
        time: 10.0,
    })
}

fn wait_in_game(s: &mut Shots, c: &mut Client) {
    for _ in 0..600 {
        s.run(c, 1);
        if c.game.is_playing() {
            break;
        }
    }
    s.run(c, 90);
}

fn view(rank: u8, meter: f32, stale: &[StyleKey]) -> StyleView {
    let mut v = StyleView {
        rank,
        meter: (meter * 255.0) as u8,
        ..Default::default()
    };
    for k in StyleKey::ALL {
        v.freshness[k.index()] = 200;
    }
    for k in stale {
        v.freshness[k.index()] = 25;
    }
    v
}

fn bonus(bonus: StyleBonus, count: u8) -> StyleEvent {
    StyleEvent::Bonus {
        player: PlayerId(0),
        bonus,
        points: 0.0,
        count,
    }
}

fn feed(c: &mut Client, bonuses: &[(StyleBonus, u8)]) {
    let now = c.game.hud_clock();
    for (b, n) in bonuses {
        c.game
            .style
            .on_event(&bonus(*b, *n), &mut c.common.audio, now);
    }
}

fn mode_event(c: &mut Client, e: &ModeEvent) {
    let now = c.game.hud_clock();
    let players = BTreeMap::new();
    let local = c.game.net().and_then(|n| n.local_player_id());
    c.game.modes.on_event(
        e,
        &c.common.theme,
        &mut c.game.hud,
        &mut c.common.audio,
        &players,
        local,
        now,
    );
}

fn end_match(s: &mut Shots, c: &mut Client) {
    if let Some(server) = c.game.session().and_then(|s| s.server()) {
        server.send(ServerCommand::Lobby(LobbyCommand::EndMatch));
    }
    for _ in 0..300 {
        s.run(c, 1);
        if c.game.results_open() {
            break;
        }
    }
}

fn result(
    id: u8,
    name: &str,
    stats: PlayerStats,
    style: u32,
    best: u8,
    grades: Option<Grades>,
) -> PlayerResult {
    PlayerResult {
        player: PlayerId(id),
        name: name.into(),
        stats,
        style_points: style,
        best_rank: best,
        extracted: true,
        grades,
    }
}

fn stats(kills: u32, deaths: u32, frags: i32, damage: f32, revives: u32) -> PlayerStats {
    PlayerStats {
        kills,
        deaths,
        frags,
        damage_taken: damage,
        revives,
    }
}

fn grades(t: Grade, k: Grade, s: Grade, d: Grade, o: Grade) -> Option<Grades> {
    Some(Grades {
        time: Some(t),
        kills: Some(k),
        style: s,
        damage: d,
        overall: o,
    })
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/style_shots"));
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let mut s = Shots {
        renderer,
        buffers: FrameBuffers::default(),
        out,
    };

    // ---- solo campaign: the style meter at several ranks --------------------------------------
    let mut c = client(
        &mut s.renderer,
        StartMode::SinglePlayer {
            level: Some("crucible_of_veins".into()),
        },
        "Solo",
    )?;
    wait_in_game(&mut s, &mut c);
    use StyleBonus::*;
    let ranks: [RankShot; 5] = [
        (0, 0.35, &[], &[(Kill, 0)]),
        (2, 0.6, &[], &[(Kill, 0), (Headshot, 0), (Airkill, 0)]),
        (
            4,
            0.8,
            &[StyleKey::Lancet],
            &[(Kill, 0), (Stale, 0), (Parry, 0)],
        ),
        (
            6,
            0.5,
            &[],
            &[
                (Kill, 0),
                (Exploded, 0),
                (Kill, 0),
                (Exploded, 0),
                (Multikill, 2),
                (Kill, 0),
                (Exploded, 0),
                (Multikill, 3),
            ],
        ),
        (
            7,
            0.95,
            &[],
            &[
                (ReturnToSender, 0),
                (Kill, 0),
                (Ricochet, 3),
                (CloseCall, 0),
            ],
        ),
    ];
    for (rank, meter, stale, events) in ranks {
        c.game.style.override_view = Some(view(rank, meter, stale));
        c.game.style.feed.entries.clear();
        s.run(&mut c, 2);
        feed(&mut c, events);
        s.run(&mut c, 12);
        let name = shared::sim::style::rank_name(rank).to_lowercase();
        s.capture(&mut c, &format!("hud_rank_{rank}_{name}"));
    }
    // Rank-up banner (just ranked up to GRISLY).
    c.game.style.override_view = Some(view(5, 0.05, &[]));
    let now = c.game.hud_clock();
    c.game.style.on_event(
        &StyleEvent::Rank {
            player: PlayerId(0),
            rank: 5,
            up: true,
        },
        &mut c.common.audio,
        now,
    );
    feed(&mut c, &[(Kill, 0), (SlamKill, 0), (Fisted, 0)]);
    s.run(&mut c, 9);
    s.capture(&mut c, "hud_rank_up_banner");
    // Checkpoint banner.
    c.game.style.override_view = Some(view(3, 0.4, &[]));
    mode_event(
        &mut c,
        &ModeEvent::Checkpoint {
            index: 0,
            player: PlayerId(0),
        },
    );
    s.run(&mut c, 20);
    s.capture(&mut c, "hud_checkpoint");

    // ---- campaign results (solo), mid reveal and finished ----------------------------------------
    end_match(&mut s, &mut c);
    c.game.show_results(MatchResults {
        rules: ModeRules::campaign(),
        reason: EndReason::Exit,
        level: "Crucible of Veins".into(),
        duration_secs: 342.6,
        par_secs: 330.0,
        total_enemies: 86,
        team_kills: 84,
        players: vec![result(
            0,
            "Solo",
            stats(84, 1, 0, 212.0, 0),
            9420,
            6,
            grades(Grade::A, Grade::S, Grade::A, Grade::B, Grade::A),
        )],
        team_grade: Some(Grade::A),
        winner: None,
    });
    s.run(&mut c, 80);
    s.capture(&mut c, "results_campaign_reveal");
    s.run(&mut c, 200);
    s.capture(&mut c, "results_campaign");
    // A perfect run.
    c.game.show_results(MatchResults {
        rules: ModeRules::campaign(),
        reason: EndReason::Exit,
        level: "Crucible of Veins".into(),
        duration_secs: 301.2,
        par_secs: 330.0,
        total_enemies: 86,
        team_kills: 86,
        players: vec![result(
            0,
            "Solo",
            stats(86, 0, 0, 61.0, 0),
            13710,
            7,
            grades(Grade::S, Grade::S, Grade::S, Grade::S, Grade::S),
        )],
        team_grade: Some(Grade::S),
        winner: None,
    });
    s.run(&mut c, 260);
    s.capture(&mut c, "results_campaign_s");

    // ---- co-op results -------------------------------------------------------------------------
    c.game.show_results(MatchResults {
        rules: ModeRules::campaign(),
        reason: EndReason::Exit,
        level: "Crucible of Veins".into(),
        duration_secs: 401.9,
        par_secs: 330.0,
        total_enemies: 172,
        team_kills: 170,
        players: vec![
            result(
                0,
                "Solo",
                stats(71, 1, 0, 288.0, 2),
                8120,
                6,
                grades(Grade::B, Grade::S, Grade::A, Grade::B, Grade::A),
            ),
            result(
                1,
                "Vex",
                stats(62, 3, 0, 512.0, 1),
                6930,
                5,
                grades(Grade::B, Grade::A, Grade::A, Grade::C, Grade::B),
            ),
            {
                let mut r = result(
                    2,
                    "Grinder",
                    stats(37, 5, 0, 801.0, 0),
                    3010,
                    3,
                    grades(Grade::B, Grade::C, Grade::C, Grade::C, Grade::C),
                );
                r.extracted = false;
                r
            },
        ],
        team_grade: Some(Grade::B),
        winner: None,
    });
    s.run(&mut c, 40);
    s.capture(&mut c, "results_coop_reveal");
    s.run(&mut c, 240);
    s.capture(&mut c, "results_coop");
    c.game.shutdown(&mut s.renderer, &mut c.common);

    // ---- deathmatch HUD, scoreboard and results ------------------------------------------------
    let mut c = client(
        &mut s.renderer,
        StartMode::SinglePlayer {
            level: Some("cistern_nine".into()),
        },
        "Solo",
    )?;
    wait_in_game(&mut s, &mut c);
    let start = c
        .game
        .net()
        .and_then(|n| n.server_tick_estimate())
        .unwrap_or(0.0) as u32;
    let me = c
        .game
        .net()
        .and_then(|n| n.local_player_id())
        .unwrap_or_default();
    mode_event(
        &mut c,
        &ModeEvent::Scoreboard(Scoreboard {
            rules: ModeRules::deathmatch(20, 600.0),
            phase: MatchPhase::Playing,
            start_tick: start.saturating_sub(60 * 252),
            checkpoint: None,
            rows: vec![ScoreEntry {
                player: me,
                kills: 13,
                deaths: 4,
                frags: 12,
                rank: 5,
                best_rank: 7,
                style: 6420,
                life: LifeState::Alive,
            }],
        }),
    );
    c.game.style.override_view = Some(view(5, 0.7, &[StyleKey::Thresher]));
    feed(&mut c, &[(Frag, 0), (Airkill, 0), (Headshot, 0)]);
    s.run(&mut c, 15);
    s.capture(&mut c, "hud_deathmatch");
    c.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, true));
    s.run(&mut c, 10);
    s.capture(&mut c, "scoreboard_deathmatch");
    c.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, false));
    s.run(&mut c, 2);
    end_match(&mut s, &mut c);
    let dm = |id: u8, name: &str, frags: i32, deaths: u32, best: u8, style: u32| {
        result(
            id,
            name,
            stats(frags.max(0) as u32, deaths, frags, 0.0, 0),
            style,
            best,
            None,
        )
    };
    c.game.show_results(MatchResults {
        rules: ModeRules::deathmatch(20, 600.0),
        reason: EndReason::FragLimit,
        level: "Cistern Nine".into(),
        duration_secs: 512.0,
        par_secs: 0.0,
        total_enemies: 0,
        team_kills: 0,
        players: vec![
            dm(me.0, "Solo", 20, 7, 7, 14210),
            dm(3, "Vex", 17, 9, 6, 11030),
            dm(1, "Grinder", 12, 14, 5, 7720),
            dm(5, "Halcyon", 9, 12, 4, 6100),
            dm(2, "Molt", 6, 15, 3, 3990),
            dm(6, "Sorrow", -1, 18, 1, 820),
        ],
        team_grade: None,
        winner: Some(me),
    });
    s.run(&mut c, 50);
    s.capture(&mut c, "results_deathmatch_reveal");
    s.run(&mut c, 120);
    s.capture(&mut c, "results_deathmatch");
    c.game.shutdown(&mut s.renderer, &mut c.common);

    // ---- co-op downed / revive: two real clients on a real server ----------------------------------
    let server = ServerHandle::spawn(ServerConfig {
        bind: "127.0.0.1:0".parse()?,
        level: "crucible_of_veins".into(),
        max_players: 4,
        watch_tuning: false,
        ..Default::default()
    })?;
    let address = server.connect_addr().to_string();
    let mut a = client(
        &mut s.renderer,
        StartMode::Join {
            address: address.clone(),
        },
        "Alpha",
    )?;
    wait_in_game(&mut s, &mut a);
    let mut b = client(&mut s.renderer, StartMode::Join { address }, "Bravo")?;
    for _ in 0..600 {
        s.run2(&mut a, &mut b, 1);
        if b.game.is_playing() {
            break;
        }
    }
    s.run2(&mut a, &mut b, 60);
    // Bravo goes down (dev command).
    b.game.chat.open();
    s.frame(&mut b, &[], "");
    s.frame(&mut b, &[], "/kill");
    s.frame(&mut b, &[UiKey::Enter], "");
    s.run2(&mut a, &mut b, 40);
    // Alpha walks up to the body (strafing right) until it is within reach.
    let bravo = b
        .game
        .net()
        .and_then(|n| n.local_player_id())
        .unwrap_or_default();
    let body = |a: &Client| {
        a.game
            .net()
            .and_then(|n| n.remote_entity(shared::protocol::NetId::player(bravo)))
            .map(|e| e.position)
    };
    let me = |a: &Client| {
        a.game
            .net()
            .and_then(|n| n.local_state())
            .map(|s| s.position)
    };
    a.input
        .handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickX, 0.35));
    for _ in 0..120 {
        s.run2(&mut a, &mut b, 1);
        if let (Some(p), Some(m)) = (body(&a), me(&a))
            && p.distance(m) < 2.1
        {
            break;
        }
    }
    a.input
        .handle_gamepad_event(GamepadEvent::Axis(GamepadAxis::LeftStickX, 0.0));
    s.run2(&mut a, &mut b, 30);
    // Look at the body.
    if let (Some(p), Some(m)) = (body(&a), me(&a)) {
        let eye = m + Vec3::Y * 1.6;
        let d = (p + Vec3::Y * 0.3 - eye).normalize();
        a.game
            .controls
            .set_view((-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin() + 0.15);
    }
    s.run2(&mut a, &mut b, 10);
    s.capture(&mut a, "coop_downed_teammate");
    s.capture(&mut b, "coop_downed_self");
    // Alpha holds USE.
    a.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::West, true));
    s.run2(&mut a, &mut b, 45);
    s.capture(&mut a, "coop_reviving");
    s.capture(&mut b, "coop_being_revived");
    s.run2(&mut a, &mut b, 70);
    a.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::West, false));
    s.run2(&mut a, &mut b, 10);
    s.capture(&mut b, "coop_revived");
    a.game.shutdown(&mut s.renderer, &mut a.common);
    b.game.shutdown(&mut s.renderer, &mut b.common);
    server.stop();
    Ok(())
}
