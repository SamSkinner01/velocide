//! Play of the game screenshots (docs/potg.md): a scripted deathmatch on Cistern Nine against
//! server bots — the local player lands a triple kill (the last one from beyond 40 m), which
//! ends the match at the frag limit — and PNGs of the title card, the first-person replay
//! (lead-in, a kill, the slow-motion final kill) and the results after it. A second match does
//! the same from a rival's eyes: another (headless) client gets the kills, so the replay is of
//! a remote player.
//!
//! The server runs in this thread on the same virtual clock as the client (no sleeping).
//!
//! ```text
//! cargo run -p client --release --example potg_shots -- [out_dir]   # default target/potg_shots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::ingame::replay::{CardInfo, draw_card};
use client::net::{ClientConfig, NetClient};
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::InputState;
use engine::physics::{MASK_MOVEMENT, MASK_SHOTS};
use engine::render::Renderer;
use engine::ui::{Rect, UiInput};
use glam::{Vec2, Vec3};
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput, look_dir};
use shared::movement::MoveState;
use shared::protocol::kind;
use shared::sim::{ModeRules, PlayerId};

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;
const LEVEL: &str = "cistern_nine";
/// Head height of a standing Shrike (aim point).
const HEAD: f32 = 1.55;

struct Rig {
    renderer: Renderer,
    common: Common,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
    out: PathBuf,
    server: Server,
    /// A rival: a real client driven by the script (second match).
    rival: Option<NetClient>,
    rival_input: PlayerInput,
    shots: Vec<String>,
}

impl Rig {
    fn frame(&mut self, game: &mut InGame) {
        self.time += DT as f64;
        self.common.stats.update(DT);
        let step = Duration::from_secs_f32(DT);
        self.server.update(step);
        if let Some(rival) = &mut self.rival {
            rival.update(step);
            rival.drain_events();
            if rival.is_in_game() {
                rival.tick(self.rival_input);
            }
            self.rival_input.buttons = Buttons::NONE;
        }
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
            self.frame(game);
        }
    }

    /// Runs frames until `done` (at most `max`); false on timeout.
    fn until(&mut self, game: &mut InGame, max: usize, done: impl Fn(&InGame) -> bool) -> bool {
        for _ in 0..max {
            if done(game) {
                return true;
            }
            self.frame(game);
        }
        done(game)
    }

    fn shot(&mut self, name: &str) {
        match self.renderer.read_pixels() {
            Ok(img) => {
                let path = self.out.join(format!("{name}.png"));
                match engine::assets::save_png(&path, &img) {
                    Ok(()) => {
                        println!("wrote {}", path.display());
                        self.shots.push(name.to_string());
                    }
                    Err(e) => eprintln!("{name}: {e:#}"),
                }
            }
            Err(e) => eprintln!("{name}: {e:#}"),
        }
    }

    /// The level's floor under (x, z), if any.
    fn ground(&self, x: f32, z: f32) -> Option<Vec3> {
        let world = &self.server.world().collision.world;
        world
            .cast_ray(Vec3::new(x, 30.0, z), Vec3::NEG_Y, 60.0, MASK_MOVEMENT)
            .map(|h| h.point + Vec3::Y * 0.05)
    }

    fn visible(&self, from: Vec3, to: Vec3) -> bool {
        !self
            .server
            .world()
            .collision
            .world
            .line_blocked(from, to, MASK_SHOTS)
    }

    /// A standing spot about `distance` m from `eye` near `yaw` (radians) with a clear line of
    /// sight to its head.
    fn spot(&self, eye: Vec3, yaw: f32, distance: f32) -> Option<Vec3> {
        for step in 0..24 {
            let turn = (step as f32 * 0.5).ceil() * if step % 2 == 0 { 1.0 } else { -1.0 } * 0.06;
            for dd in [0.0, -3.0, 3.0, -6.0, 6.0] {
                let dir = look_dir(yaw + turn, 0.0);
                let p = eye + dir * (distance + dd);
                let Some(g) = self.ground(p.x, p.z) else {
                    continue;
                };
                if (g.y - (eye.y - 1.6)).abs() > 4.0 {
                    continue;
                }
                if self.visible(eye, g + Vec3::Y * HEAD) && self.visible(eye, g + Vec3::Y * 0.9) {
                    return Some(g);
                }
            }
        }
        None
    }

    fn place(&mut self, p: PlayerId, at: Vec3) {
        if let Some(pl) = self.server.world_mut().players.get_mut(&p) {
            pl.movement = MoveState::spawn(at);
            pl.combat.protection = 0;
        }
    }

    /// One shot from a bot's body kills it.
    fn soften(&mut self, p: PlayerId) {
        let max = self.server.tuning().combat.max_health;
        if let Some(pl) = self.server.world_mut().players.get_mut(&p) {
            pl.combat.damage = max - 6.0;
            pl.combat.protection = 0;
        }
    }
}

fn eye(game: &InGame) -> Option<Vec3> {
    let net = game.net()?;
    Some(net.local_state()?.eye_position(&net.tuning().movement))
}

/// Where the local client draws player `p`.
fn seen(game: &InGame, p: PlayerId) -> Option<Vec3> {
    game.net()?
        .remote_entities()
        .iter()
        .find(|e| e.kind == kind::PLAYER && e.id.as_player() == Some(p))
        .map(|e| e.position)
}

fn aim(from: Vec3, at: Vec3) -> (f32, f32) {
    let d = (at - from).normalize_or(Vec3::NEG_Z);
    ((-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin())
}

fn aim_at(game: &mut InGame, target: Vec3) {
    let Some(from) = eye(game) else {
        return;
    };
    let (yaw, pitch) = aim(from, target);
    game.controls.yaw = yaw;
    game.controls.pitch = pitch;
}

fn hold(game: &mut InGame, buttons: Buttons) {
    game.controls.override_move = Some((0.0, 0.0, buttons));
}

/// Turns towards `target` over `frames` frames, then fires one shot.
fn swing_and_fire(rig: &mut Rig, game: &mut InGame, target: PlayerId, frames: usize) {
    let start = (game.controls.yaw, game.controls.pitch);
    for i in 0..frames {
        let Some(at) = seen(game, target) else {
            break;
        };
        let Some(from) = eye(game) else {
            break;
        };
        let (yaw, pitch) = aim(from, at + Vec3::Y * HEAD);
        let k = ((i + 1) as f32 / frames as f32).min(1.0);
        let s = k * k * (3.0 - 2.0 * k);
        let dyaw = (yaw - start.0 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        game.controls.yaw = start.0 + dyaw * s;
        game.controls.pitch = start.1 + (pitch - start.1) * s;
        hold(game, Buttons::NONE);
        rig.frame(game);
    }
    if let Some(at) = seen(game, target) {
        aim_at(game, at + Vec3::Y * HEAD);
    }
    hold(game, Buttons::FIRE);
    rig.frame(game);
    hold(game, Buttons::NONE);
}

/// Waits for the play of the game, then captures its card, replay moments and the results.
fn capture_potg(rig: &mut Rig, game: &mut InGame, prefix: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        rig.until(game, 240, InGame::potg_active),
        "{prefix}: no play of the game"
    );
    let potg = game.potg().map(|p| (p.name.clone(), p.title.clone()));
    println!("{prefix}: play of the game {potg:?}");
    rig.frames(game, 21);
    rig.shot(&format!("{prefix}_card_slam"));
    rig.frames(game, 50);
    rig.shot(&format!("{prefix}_card"));
    anyhow::ensure!(
        rig.until(game, 200, |g| g
            .potg()
            .is_some_and(|p| p.playback.is_some())),
        "{prefix}: no replay"
    );
    let (from, end) = game
        .potg()
        .and_then(|p| Some((p.clip.as_ref()?.from, p.highlight.end_tick)))
        .unwrap_or_default();
    println!("{prefix}: replay from tick {from}, last kill {end}");
    rig.frames(game, 30);
    rig.shot(&format!("{prefix}_replay_lead"));
    // Just after the first kill (shown a view lag after its shot), then the last one in slow
    // motion.
    let start = game.potg().map_or(0, |p| p.highlight.start_tick);
    let tick_of = |g: &InGame| {
        g.potg()
            .and_then(|p| p.playback.as_ref())
            .map_or(f64::MAX, |pb| pb.tick)
    };
    rig.until(game, 600, |g| tick_of(g) >= start as f64 + 12.0);
    rig.shot(&format!("{prefix}_replay_kill"));
    // The last shot leaves the barrel (slow motion), then the kill lands.
    rig.until(game, 900, |g| tick_of(g) >= end as f64 + 1.0);
    rig.shot(&format!("{prefix}_replay_final"));
    rig.until(game, 900, |g| tick_of(g) >= end as f64 + 14.0);
    rig.shot(&format!("{prefix}_replay_after"));
    anyhow::ensure!(
        rig.until(game, 1200, |g| !g.potg_active()),
        "{prefix}: the replay never ended"
    );
    rig.frames(game, 150);
    rig.shot(&format!("{prefix}_results"));
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/potg_shots"));
    std::fs::create_dir_all(&out)?;
    let server = Server::new(ServerConfig {
        bind: "127.0.0.1:0".parse()?,
        level: LEVEL.into(),
        max_players: 8,
        watch_tuning: false,
        dev_commands: true,
        ..Default::default()
    })?;
    let addr = server.local_addr();
    let settings = Settings {
        player_name: "Vesper".into(),
        ..Default::default()
    };
    let scratch = std::env::temp_dir().join("velocide-potg-shots-settings.ron");
    let mut rig = Rig {
        renderer: Renderer::new_headless(W, H, &RetroSettings::default())?,
        common: Common::new(
            settings,
            scratch,
            Audio::disabled(),
            LaunchOptions::default(),
        ),
        buffers: FrameBuffers::default(),
        input: InputState::new(),
        time: 10.0,
        out,
        server,
        rival: None,
        rival_input: PlayerInput::default(),
        shots: Vec::new(),
    };
    let mut game = InGame::start(
        &StartMode::Join {
            address: addr.to_string(),
        },
        &mut rig.common,
        &mut rig.renderer,
    )?;
    anyhow::ensure!(rig.until(&mut game, 600, InGame::is_playing), "not in game");
    let me = game
        .net()
        .and_then(NetClient::local_player_id)
        .ok_or_else(|| anyhow::anyhow!("no player id"))?;
    let bots: Vec<PlayerId> = ["Gravel", "Ossuary", "Tallowmere"]
        .iter()
        .filter_map(|n| rig.server.add_bot(n))
        .collect();
    anyhow::ensure!(bots.len() == 3, "bots");
    rig.frames(&mut game, 30);

    // ---- match 1: our triple kill -----------------------------------------------------------
    rig.server
        .world_mut()
        .set_mode_rules(ModeRules::deathmatch(3, 0.0));
    let feet = rig
        .ground(0.0, 30.0)
        .ok_or_else(|| anyhow::anyhow!("no floor"))?;
    rig.place(me, feet);
    game.controls.yaw = 0.0;
    game.controls.pitch = 0.0;
    rig.frames(&mut game, 20);
    let from = feet + Vec3::Y * 1.6;
    let spots = [(0.35, 13.0), (-0.3, 24.0), (0.05, 46.0)];
    for (bot, (yaw, d)) in bots.iter().zip(spots) {
        let at = rig
            .spot(from, yaw, d)
            .ok_or_else(|| anyhow::anyhow!("no clear spot at {d} m"))?;
        println!("bot {bot} at {at:.1} ({:.0} m)", at.distance(feet));
        rig.place(*bot, at);
    }
    // Long enough for the replay's lead-in to show them standing there.
    rig.frames(&mut game, 150);
    for bot in &bots {
        rig.soften(*bot);
    }
    rig.frames(&mut game, 10);
    rig.shot("potg_live_before");
    for (i, bot) in bots.iter().enumerate() {
        swing_and_fire(&mut rig, &mut game, *bot, if i == 0 { 10 } else { 16 });
        rig.frames(&mut game, 6);
        if i == 1 {
            rig.shot("potg_live_feed");
        }
    }
    capture_potg(&mut rig, &mut game, "potg_local")?;

    // ---- match 2: a rival's double kill, replayed from their eyes ---------------------------
    rig.rival = Some(NetClient::connect(addr, ClientConfig::new("Rivet Witch"))?);
    rig.frames(&mut game, 20);
    // Without a lobby, "return to lobby" restarts the level: a new match.
    rig.server.return_to_lobby();
    let ok = rig.until(&mut game, 900, |g| {
        g.is_playing() && !g.results_open() && !g.potg_active()
    });
    anyhow::ensure!(ok, "second match");
    for _ in 0..600 {
        if rig.rival.as_ref().is_some_and(NetClient::is_in_game) {
            break;
        }
        rig.frame(&mut game);
    }
    let rival = rig
        .rival
        .as_ref()
        .and_then(NetClient::local_player_id)
        .ok_or_else(|| anyhow::anyhow!("rival not connected"))?;
    rig.server
        .world_mut()
        .set_mode_rules(ModeRules::deathmatch(2, 0.0));
    let rfeet = rig
        .ground(0.0, -30.0)
        .ok_or_else(|| anyhow::anyhow!("no floor"))?;
    rig.place(rival, rfeet);
    rig.place(me, feet);
    let reye = rfeet + Vec3::Y * 1.6;
    let targets = [(bots[0], (3.3, 14.0)), (bots[1], (2.9, 20.0))];
    for (bot, (yaw, d)) in targets {
        let at = rig
            .spot(reye, yaw, d)
            .ok_or_else(|| anyhow::anyhow!("no clear spot for the rival at {d} m"))?;
        rig.place(bot, at);
    }
    // Our own view looks at the rival from across the arena.
    if let Some(e) = eye(&game) {
        let (yaw, pitch) = aim(e, reye);
        game.controls.yaw = yaw;
        game.controls.pitch = pitch;
    }
    rig.frames(&mut game, 60);
    for (bot, _) in targets {
        rig.soften(bot);
    }
    for (bot, _) in targets {
        // Aim at what the rival's client shows, turning over a few frames, then fire.
        let Some(rnet) = rig.rival.as_ref() else {
            break;
        };
        let start = (rig.rival_input.yaw, rig.rival_input.pitch);
        let target = rnet
            .remote_entities()
            .iter()
            .find(|e| e.id.as_player() == Some(bot))
            .map(|e| e.position + Vec3::Y * HEAD);
        let (Some(target), Some(state)) = (target, rnet.local_state()) else {
            continue;
        };
        let from = state.eye_position(&rnet.tuning().movement);
        let (yaw, pitch) = aim(from, target);
        for i in 0..14 {
            let k = (i + 1) as f32 / 14.0;
            rig.rival_input.yaw = start.0 + (yaw - start.0) * k;
            rig.rival_input.pitch = start.1 + (pitch - start.1) * k;
            rig.frame(&mut game);
        }
        rig.rival_input.buttons = Buttons::FIRE;
        rig.frame(&mut game);
        rig.frames(&mut game, 8);
    }
    capture_potg(&mut rig, &mut game, "potg_remote")?;

    // A client that joined after the play happened gets the card alone.
    rig.buffers.clear(UiDrawList::canvas_for_window(W, H));
    let canvas = Rect::from_min_size(Vec2::ZERO, rig.buffers.ui.canvas_size);
    let card = CardInfo {
        name: "RIVET WITCH",
        title: "LONGSHOT 87m",
        color: client::ingame::remote_players::player_color(rival),
        replay: false,
    };
    draw_card(&mut rig.buffers.ui, &rig.common.theme, canvas, &card, 2.0);
    let scene = client::frame::SceneParams {
        clear_color: Vec3::new(0.03, 0.01, 0.015),
        ..Default::default()
    };
    if let Err(e) = rig.renderer.render(&rig.buffers.desc(&scene), None) {
        eprintln!("render: {e:#}");
    }
    rig.shot("potg_card_only");

    println!("{} screenshots", rig.shots.len());
    Ok(())
}
