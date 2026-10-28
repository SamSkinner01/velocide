//! The in-game screenshots in README.md (`docs/images/`), rendered headless on the real client
//! (GPU, no window) against real servers. The fights are played for real for 20 s (the arena
//! encounter again, up to three times, until it has given four moments) and scored
//! every few frames (Moulds in view, not behind walls, nearer ones count more; no rank-up or
//! wave banner over the middle, the camera about level); the best moments, at least 0.75 s
//! apart, are written as `<scene>_1.png` (the best) to `<scene>_4.png`, for
//! `scripts/readme_shots.sh` to pick from. The chat log (the dev commands' replies) is cleared
//! before each capture. Scenes:
//!
//! - `fight_drowned_chandlery`: single-player on BRUTAL with `/god`, the first arena
//!   encounter started with `/goto encounter 1` + `/encounter 1`; the player strafes and shoots
//!   the nearest Mould (the Bellows up close, the Quill and the Arclash in turn further out);
//! - `colour_vision`: each Drowned Chandlery moment beside the same moment in the deuteranopia
//!   colour mode, both at half size (1280x360);
//! - `coop`: two clients in campaign co-op on a real server (Crucible of Veins, encounter 1),
//!   from one player's eyes with the other ahead of it in the fight;
//! - `pvp`: two clients in a deathmatch on Furnace Ring, 6 m apart, strafing and shooting at
//!   each other (up to four moments from 0.4 s in with the other in view and the camera about
//!   level, the earliest first).
//!
//! The menus, the loadout, the warm-up and the update prompt come from `client_shots` and
//! `lobby_shots`; `scripts/readme_shots.sh` runs all three and writes the WebP files.
//!
//! ```text
//! cargo run -p client --release --example readme_shots -- [out_dir]   # default target/readme_shots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::settings::Settings;
use engine::audio::Audio;
use engine::color_vision::{ColorVision, ColorVisionMode};
use engine::gfx::{RetroSettings, TextureData, UiDrawList};
use engine::input::InputState;
use engine::physics::MASK_SHOTS;
use engine::render::Renderer;
use engine::ui::UiInput;
use glam::{Vec2, Vec3};
use server::{ServerConfig, ServerHandle};
use shared::input::{Buttons, look_dir};
use shared::lobby::Difficulty;
use shared::protocol::{EntityState, NetId, enemy_flags, kind, player_flags};

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;
/// A frame that re-renders the current moment (the server keeps its own clock).
const STILL: f32 = 1e-4;

/// Weapon slots of the default loadout (Quill, Bellows, Arclash, Tumbler).
const QUILL: u8 = 0;
const BELLOWS: u8 = 1;
const ARCLASH: u8 = 2;

/// One player: its app state, input and game screen.
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
    /// One frame of `c`, `dt` seconds on; renders it.
    fn frame(&mut self, c: &mut Client, dt: f32) {
        c.time += dt as f64;
        c.common.stats.update(DT);
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
                dt,
                time: c.time,
                window_size: (W, H),
            };
            c.game.frame(&mut c.common, &mut io, &mut self.buffers)
        };
        if c.common.settings_changed {
            c.common.settings_changed = false;
            c.common.apply_display_settings(&mut self.renderer);
        }
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            eprintln!("render: {e:#}");
        }
        c.input.end_frame();
    }

    /// `n` real-time frames (the server runs on its own thread); `each` steers before each.
    fn run(&mut self, c: &mut Client, n: usize, mut each: impl FnMut(&mut InGame)) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            each(&mut c.game);
            self.frame(c, DT);
        }
    }

    /// `n` real-time frames of two players.
    fn run2(
        &mut self,
        a: &mut Client,
        b: &mut Client,
        n: usize,
        mut each: impl FnMut(&mut InGame, &mut InGame),
    ) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            each(&mut a.game, &mut b.game);
            self.frame(a, DT);
            self.frame(b, DT);
        }
    }

    /// The current moment of `c` without the chat log, as an image.
    fn grab(&mut self, c: &mut Client) -> Option<TextureData> {
        c.game.chat.lines.clear();
        self.frame(c, STILL);
        match self.renderer.read_pixels() {
            Ok(img) => Some(img),
            Err(e) => {
                eprintln!("read pixels: {e:#}");
                None
            }
        }
    }

    /// The current moment of `c` in colour-vision `mode` (two frames: the first picks up the
    /// new palette), then back to the standard look.
    fn grab_in(&mut self, c: &mut Client, mode: ColorVisionMode) -> Option<TextureData> {
        self.set_mode(c, mode);
        c.game.chat.lines.clear();
        self.frame(c, STILL);
        let img = self.grab(c);
        self.set_mode(c, ColorVisionMode::Off);
        self.frame(c, STILL);
        img
    }

    fn set_mode(&mut self, c: &mut Client, mode: ColorVisionMode) {
        c.common.settings.accessibility.color_vision = ColorVision::new(mode, 1.0);
        c.common.apply_display_settings(&mut self.renderer);
    }

    fn save(&self, name: &str, img: &TextureData) {
        let path = self.out.join(format!("{name}.png"));
        match engine::assets::save_png(&path, img) {
            Ok(()) => println!("wrote {}", path.display()),
            Err(e) => eprintln!("{name}: {e:#}"),
        }
    }
}

fn client(renderer: &mut Renderer, mode: StartMode, name: &str) -> anyhow::Result<Client> {
    let scratch = std::env::temp_dir().join(format!("velocide-readme-shots-{name}.ron"));
    let settings = Settings {
        difficulty: Difficulty::Brutal,
        ..Default::default()
    };
    let mut common = Common::new(
        settings,
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

fn wait_in_game(s: &mut Shots, c: &mut Client) -> anyhow::Result<()> {
    for _ in 0..600 {
        s.run(c, 1, |_| {});
        if c.game.is_playing() {
            s.run(c, 60, |_| {});
            return Ok(());
        }
    }
    anyhow::bail!("never got into the game")
}

/// A chat command (dev commands; a burst of 5, then 1 per second), with time to arrive.
fn command(s: &mut Shots, c: &mut Client, text: &str) {
    c.game.send_chat(text);
    s.run(c, 20, |_| {});
}

fn eye(game: &InGame) -> Option<Vec3> {
    let net = game.net()?;
    Some(net.local_state()?.eye_position(&net.tuning().movement))
}

fn view(game: &InGame) -> Vec3 {
    look_dir(game.controls.yaw, game.controls.pitch)
}

fn aim_at(game: &mut InGame, target: Vec3) {
    let Some(from) = eye(game) else {
        return;
    };
    let d = (target - from).normalize_or_zero();
    if d != Vec3::ZERO {
        game.controls.yaw = (-d.x).atan2(-d.z);
        game.controls.pitch = d.y.clamp(-1.0, 1.0).asin();
    }
}

/// Where to look at an entity: about the middle of its body.
fn chest(e: &EntityState) -> Vec3 {
    e.position
        + Vec3::Y
            * match e.kind {
                kind::HARRIER => 0.65,
                kind::GAOLER => 2.0,
                kind::HERON => 2.4,
                kind::CENSER => 1.5,
                kind::BELLWETHER => 2.6,
                kind::SMELTER => 1.8,
                kind::SNUFFER => 0.6,
                _ => 1.0,
            }
}

fn moulds(game: &InGame) -> Vec<EntityState> {
    game.net().map_or_else(Vec::new, |n| {
        n.remote_entities()
            .iter()
            .filter(|e| {
                kind::is_enemy(e.kind)
                    && e.kind != kind::EFFIGY
                    && e.flags & enemy_flags::DYING == 0
            })
            .copied()
            .collect()
    })
}

fn nearest_mould(game: &InGame) -> Option<EntityState> {
    nearest_to(game, eye(game))
}

fn nearest_to(game: &InGame, p: Option<Vec3>) -> Option<EntityState> {
    let p = p?;
    moulds(game)
        .into_iter()
        .min_by(|a, b| a.position.distance(p).total_cmp(&b.position.distance(p)))
}

/// Whether `p` is in front of the camera (within about 37° of the view), at a distance that
/// reads well and not behind a wall.
fn in_view(game: &InGame, p: Vec3, max_dist: f32) -> bool {
    let (Some(eye), Some(world)) = (eye(game), game.net().and_then(|n| n.world())) else {
        return false;
    };
    let d = p - eye;
    let dist = d.length();
    dist > 3.0
        && dist < max_dist
        && d.normalize().dot(view(game)) > 0.8
        && world
            .collision
            .world
            .cast_ray(eye, d, dist, MASK_SHOTS)
            .is_none_or(|hit| hit.distance > dist - 0.5)
}

/// How well the Moulds in view fill the frame: one point each, up to one more the closer
/// they are (but not so close that one fills the screen).
fn moulds_in_view(game: &InGame) -> f32 {
    let Some(eye) = eye(game) else {
        return 0.0;
    };
    moulds(game)
        .iter()
        .filter(|e| e.position.distance(eye) > 4.0 && in_view(game, chest(e), 24.0))
        .map(|e| 2.0 - e.position.distance(eye) / 24.0)
        .sum()
}

/// Keeps track of the big centred banners (a rank-up, a wave starting, the arena locking or
/// clearing): they cover the top of the screen for up to 1.6 s.
#[derive(Default)]
struct Banners {
    rank: u8,
    wave: Option<String>,
    /// Frames since the last banner.
    quiet: usize,
}

impl Banners {
    fn update(&mut self, game: &InGame) {
        let rank = game.style.rank();
        // "WAVE 2/3 - 5 LEFT" -> "WAVE 2/3" (the banner comes when "WAVE 2/3 INCOMING" ends).
        let wave = game
            .net()
            .and_then(|n| client::ingame::encounter_hud::readout(n.remote_entities()))
            .map(|r| r.split(" - ").next().unwrap_or_default().to_string());
        // Only a rank going up shows a banner.
        if rank > self.rank || wave != self.wave {
            self.quiet = 0;
        } else {
            self.quiet += 1;
        }
        self.rank = rank;
        self.wave = wave;
    }

    fn clear(&self) -> bool {
        self.quiet > 110
    }
}

/// A frame worth keeping: the camera about level and no banner up.
fn composed(game: &InGame, banners: &Banners) -> bool {
    banners.clear() && game.controls.pitch.abs() < 0.45
}

/// Steering of a player fighting Moulds on frame `i`: aims at the nearest, strafes back and
/// forth, fires, and switches weapons (the Bellows up close, otherwise Quill and Arclash in
/// turn).
fn fight_step(game: &mut InGame, i: usize, strafe: f32) {
    let target = nearest_mould(game);
    let mut buttons = Buttons::NONE;
    if let Some(t) = target {
        aim_at(game, chest(&t));
        buttons = Buttons::FIRE;
        let close = eye(game).is_some_and(|e| t.position.distance(e) < 9.0);
        game.controls.weapons.current = if close {
            BELLOWS
        } else if (i / 150).is_multiple_of(2) {
            QUILL
        } else {
            ARCLASH
        };
    }
    let side = if (i / 80).is_multiple_of(2) {
        strafe
    } else {
        -strafe
    };
    game.controls.override_move = Some((0.0, side, buttons));
}

/// A run's best frames (up to [`Picks::KEEP`]), at least `gap` frames apart, best first.
struct Picks {
    gap: usize,
    list: Vec<Pick>,
}

struct Pick {
    score: f32,
    frame: usize,
    img: TextureData,
    /// The same moment in another look (the colour-vision mode).
    alt: Option<TextureData>,
}

impl Picks {
    const KEEP: usize = 4;

    fn new(gap: usize) -> Self {
        Self {
            gap,
            list: Vec::new(),
        }
    }

    /// Whether a frame scoring `score` would be kept.
    fn wants(&self, score: f32, frame: usize) -> bool {
        if score <= 0.0 {
            return false;
        }
        let mut near = self
            .list
            .iter()
            .filter(|p| frame.abs_diff(p.frame) < self.gap)
            .peekable();
        if near.peek().is_some() {
            near.all(|p| score >= p.score)
        } else {
            self.list.len() < Self::KEEP || self.list.iter().any(|p| score > p.score)
        }
    }

    fn add(&mut self, pick: Pick) {
        let gap = self.gap;
        self.list.retain(|p| pick.frame.abs_diff(p.frame) >= gap);
        self.list.push(pick);
        self.list.sort_by(|a, b| b.score.total_cmp(&a.score));
        self.list.truncate(Self::KEEP);
    }

    /// Writes `<name>_<n>.png` (and `<alt_name>_<n>.png`: it beside its other look), n = 1 for
    /// the best.
    fn save(&self, s: &Shots, name: &str, alt_name: &str) {
        if self.list.is_empty() {
            eprintln!("{name}: nothing worth keeping");
        }
        for (n, p) in self.list.iter().enumerate() {
            println!(
                "{name}_{}: score {:.1} at frame {}",
                n + 1,
                p.score,
                p.frame
            );
            s.save(&format!("{name}_{}", n + 1), &p.img);
            if let Some(alt) = &p.alt {
                s.save(&format!("{alt_name}_{}", n + 1), &pair(&p.img, alt));
            }
        }
    }
}

/// Single-player on BRUTAL (fuller waves, tougher Moulds; the player has `/god`): encounter
/// `encounter` of `level` started for real and fought for 20 s.
fn fight(s: &mut Shots, level: &str, encounter: usize, colour: bool) -> anyhow::Result<()> {
    let mut c = client(
        &mut s.renderer,
        StartMode::SinglePlayer {
            level: Some(level.into()),
        },
        "Shrike",
    )?;
    wait_in_game(s, &mut c)?;
    command(s, &mut c, "/god");
    command(s, &mut c, &format!("/goto encounter {encounter}"));
    let mut picks = Picks::new(45);
    let mut banners = Banners::default();
    // The scatterguns clear the encounter quickly and every rank-up banner spoils a few frames:
    // play it again (up to three times) until there are enough moments to pick from.
    for i in 0..3 * 1200 {
        if i % 1200 == 0 {
            if picks.list.len() >= Picks::KEEP {
                break;
            }
            command(s, &mut c, &format!("/encounter {encounter}"));
        }
        s.run(&mut c, 1, |g| fight_step(g, i, 0.6));
        banners.update(&c.game);
        if i % 1200 < 240 || !i.is_multiple_of(4) || !composed(&c.game, &banners) {
            continue;
        }
        let score = moulds_in_view(&c.game);
        if picks.wants(score, i)
            && let Some(img) = s.grab(&mut c)
        {
            let alt = if colour {
                s.grab_in(&mut c, ColorVisionMode::Deuteranopia)
            } else {
                None
            };
            picks.add(Pick {
                score,
                frame: i,
                img,
                alt,
            });
        }
    }
    picks.save(s, &format!("fight_{level}"), "colour_vision");
    c.game.shutdown(&mut s.renderer, &mut c.common);
    Ok(())
}

/// Both images at half size side by side (2x box filter: the 3D view is drawn at half the
/// window size anyway), with a thin dark seam between them.
fn pair(left: &TextureData, right: &TextureData) -> TextureData {
    let (w, h) = (left.width, left.height / 2);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (img, x0) in [(left, 0), (right, w / 2)] {
        for y in 0..h {
            for x in 0..w / 2 {
                for c in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        img.rgba[(((y * 2 + dy) * img.width + x * 2 + dx) * 4 + c) as usize] as u32
                    };
                    rgba[((y * w + x0 + x) * 4 + c) as usize] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
    }
    for y in 0..h {
        for x in w / 2 - 1..=w / 2 {
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&[12, 10, 10, 255]);
        }
    }
    TextureData {
        width: w,
        height: h,
        rgba,
    }
}

fn position(game: &InGame) -> Option<Vec3> {
    game.net()?.local_state().map(|s| s.position)
}

/// The other player as `game` sees it.
fn other(game: &InGame, other: &InGame) -> Option<EntityState> {
    let id = other.net()?.local_player_id()?;
    game.net()?.remote_entity(NetId::player(id)).copied()
}

fn two_players(
    s: &mut Shots,
    level: &str,
    names: [&str; 2],
) -> anyhow::Result<(ServerHandle, Client, Client)> {
    let server = ServerHandle::spawn(ServerConfig {
        bind: "127.0.0.1:0".parse()?,
        level: level.into(),
        max_players: 4,
        watch_tuning: false,
        dev_commands: true,
        ..Default::default()
    })?;
    let address = server.connect_addr().to_string();
    let mut a = client(
        &mut s.renderer,
        StartMode::Join {
            address: address.clone(),
        },
        names[0],
    )?;
    wait_in_game(s, &mut a)?;
    let mut b = client(&mut s.renderer, StartMode::Join { address }, names[1])?;
    for _ in 0..600 {
        s.run2(&mut a, &mut b, 1, |_, _| {});
        if b.game.is_playing() {
            break;
        }
    }
    anyhow::ensure!(b.game.is_playing(), "the second player never got in");
    s.run2(&mut a, &mut b, 60, |_, _| {});
    Ok((server, a, b))
}

/// Campaign co-op: both players in arena encounter `encounter` of `level`; the second stands
/// ahead and to the right of the first, so the first sees both it and the wave.
fn coop(s: &mut Shots, level: &str, encounter: usize) -> anyhow::Result<()> {
    let (server, mut a, mut b) = two_players(s, level, ["Sam", "Alex"])?;
    command(s, &mut a, "/god");
    command(s, &mut b, "/god");
    command(s, &mut a, &format!("/goto encounter {encounter}"));
    command(s, &mut b, &format!("/goto encounter {encounter}"));
    command(s, &mut a, &format!("/encounter {encounter}"));
    // Wait for the wave, then face it and put the second player ahead to the right.
    for _ in 0..300 {
        s.run2(&mut a, &mut b, 1, |_, _| {});
        if moulds(&a.game).len() >= 3 {
            break;
        }
    }
    let Some(p) = position(&a.game) else {
        anyhow::bail!("co-op: no position");
    };
    let wave = moulds(&a.game);
    let centre = wave.iter().map(|e| e.position).sum::<Vec3>() / wave.len().max(1) as f32;
    let fwd = Vec3::new(centre.x - p.x, 0.0, centre.z - p.z).normalize_or(Vec3::NEG_Z);
    let right = fwd.cross(Vec3::Y);
    // A step into the room (the rally point is the checkpoint, its lantern post stands there).
    let first = p + fwd * 2.5;
    let second = first + fwd * 4.0 + right * 2.5;
    for (c, at) in [(&mut a, first), (&mut b, second)] {
        command(
            s,
            c,
            &format!("/tp {:.2} {:.2} {:.2}", at.x, at.y + 0.1, at.z),
        );
    }
    if let Some(eye) = eye(&a.game) {
        aim_at(&mut a.game, eye + fwd * 10.0);
    }
    let mut picks = Picks::new(45);
    let mut banners = Banners::default();
    for i in 0..1200 {
        s.run2(&mut a, &mut b, 1, |ga, gb| {
            // The second player fights; the first fights the Moulds near its teammate, aiming
            // between the two so both stay in the frame.
            fight_step(gb, i + 75, 0.0);
            let mate = other(ga, gb);
            fight_step(ga, i, 0.0);
            if let (Some(m), Some(t)) = (mate, nearest_to(ga, mate.map(|m| m.position))) {
                aim_at(ga, chest(&m).lerp(chest(&t), 0.6));
            }
        });
        banners.update(&a.game);
        if i < 180 || !i.is_multiple_of(4) || !composed(&a.game, &banners) {
            continue;
        }
        let Some(mate) = other(&a.game, &b.game) else {
            continue;
        };
        if mate.flags & (player_flags::DEAD | player_flags::DOWNED) != 0
            || !in_view(&a.game, mate.position + Vec3::Y, 20.0)
        {
            continue;
        }
        let score = moulds_in_view(&a.game);
        if picks.wants(score, i)
            && let Some(img) = s.grab(&mut a)
        {
            picks.add(Pick {
                score,
                frame: i,
                img,
                alt: None,
            });
        }
    }
    picks.save(s, "coop", "");
    a.game.shutdown(&mut s.renderer, &mut a.common);
    b.game.shutdown(&mut s.renderer, &mut b.common);
    server.stop();
    Ok(())
}

/// Steering of a duellist: aims at the other player, strafes, fires the first slot.
fn duel_step(game: &mut InGame, target: Option<EntityState>, side: f32) {
    if let Some(e) = target {
        aim_at(game, e.position + Vec3::Y * 1.2);
    }
    game.controls.weapons.current = 0;
    game.controls.override_move = Some((0.0, side, Buttons::FIRE));
}

/// Deathmatch on Furnace Ring: the players on the pit floor about 6 m apart, the Great
/// Crucible behind, both strafing and shooting.
fn pvp(s: &mut Shots) -> anyhow::Result<()> {
    let (server, mut a, mut b) = two_players(s, "furnace_ring", ["Sam", "Alex"])?;
    command(s, &mut a, "/tp 0 0.1 16");
    command(s, &mut b, "/tp -3 0.1 11");
    command(s, &mut b, "/loadout arclash quill bellows tumbler");
    s.run2(&mut a, &mut b, 30, |_, _| {});
    // Four moments from 0.4 s into the exchange on, the earliest first.
    let mut picks = Picks::new(0);
    for i in 0..80usize {
        s.run2(&mut a, &mut b, 1, |ga, gb| {
            let side = if (i / 30).is_multiple_of(2) {
                0.6
            } else {
                -0.6
            };
            let (ta, tb) = (other(ga, gb), other(gb, ga));
            duel_step(ga, ta, side);
            duel_step(gb, tb, -side);
        });
        let visible = other(&a.game, &b.game).is_some_and(|e| {
            e.flags & player_flags::DEAD == 0 && in_view(&a.game, e.position + Vec3::Y, 30.0)
        });
        if i >= 24
            && i.is_multiple_of(12)
            && visible
            && a.game.controls.pitch.abs() < 0.35
            && let Some(img) = s.grab(&mut a)
        {
            picks.add(Pick {
                score: 100.0 - i as f32,
                frame: i,
                img,
                alt: None,
            });
        }
    }
    picks.save(s, "pvp", "");
    a.game.shutdown(&mut s.renderer, &mut a.common);
    b.game.shutdown(&mut s.renderer, &mut b.common);
    server.stop();
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/readme_shots"));
    std::fs::create_dir_all(&out)?;
    let mut s = Shots {
        renderer: Renderer::new_headless(W, H, &RetroSettings::default())?,
        buffers: FrameBuffers::default(),
        out,
    };
    fight(&mut s, "drowned_chandlery", 1, true)?;
    coop(&mut s, "crucible_of_veins", 1)?;
    pvp(&mut s)?;
    Ok(())
}
