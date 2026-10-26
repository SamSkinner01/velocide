//! Colour-vision screenshots: the same moments rendered in every colour-vision mode, plus what
//! a viewer with that deficiency actually sees (Machado 2009 simulation of the PNG), so people
//! with normal colour vision can check the palettes and the full-screen correction.
//!
//! Moments: the ACCESSIBILITY settings page over the menu backdrop, a hosted co-op game with
//! bots (team colours, name-tag markers, the campaign level's colours), an enemy in its
//! parryable window (telegraph glow, parry strobe + icon), the HUD after a hit (hard damage,
//! health popups for a hit and a heal) and the scoreboard.
//!
//! For each moment `<m>` it writes `<m>_<mode>.png` (what the screen shows) and, per
//! deficiency, `<m>_<mode>_compare.png`: three panels at half size — the standard look, how
//! that viewer sees the standard look, how that viewer sees the corrected look.
//!
//! ```text
//! cargo run -p client --release --example a11y_shots -- [out_dir]   # default target/a11y_shots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo, ScreenOutput};
use client::ingame::InGame;
use client::menus::lobby::LobbyScreen;
use client::menus::main_menu::MainMenu;
use client::menus::settings_menu::ACCESSIBILITY_TAB;
use client::settings::Settings;
use engine::audio::Audio;
use engine::color_vision::{self, ColorVision, ColorVisionMode};
use engine::gfx::{RetroSettings, TextureData, UiDrawList};
use engine::input::{GamepadButton, GamepadEvent, InputState};
use engine::render::Renderer;
use engine::ui::{UiInput, UiKey};
use glam::{Vec2, Vec3};
use server::{LobbyCommand, ServerCommand};
use shared::input::Buttons;
use shared::protocol::{enemy_flags, kind};

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

enum Screen<'a> {
    Lobby(&'a mut LobbyScreen),
    Menu(&'a mut MainMenu),
    Game(&'a mut InGame),
}

struct Harness {
    renderer: Renderer,
    common: Common,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
    out: PathBuf,
}

impl Harness {
    /// One frame of `dt` seconds (a tiny `dt` re-renders the same moment).
    fn frame(&mut self, screen: Screen, keys: &[UiKey], dt: f32) -> ScreenOutput {
        self.time += dt as f64;
        self.common.stats.update(DT);
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
                dt,
                time: self.time,
                window_size: (W, H),
            };
            match screen {
                Screen::Menu(m) => m.frame(&mut self.common, &mut io, &mut self.buffers),
                Screen::Lobby(l) => l.frame(&mut self.common, &mut io, &mut self.buffers),
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

    /// Real-time game frames (the in-process server runs on its own clock).
    fn play(&mut self, game: &mut InGame, n: usize, mut each: impl FnMut(&mut InGame)) {
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            each(game);
            self.frame(Screen::Game(game), &[], DT);
        }
    }

    fn set_mode(&mut self, mode: ColorVisionMode) {
        self.common.settings.accessibility.color_vision = ColorVision::new(mode, 1.0);
        self.common.apply_display_settings(&mut self.renderer);
    }

    /// Renders the current moment once per colour-vision mode and writes the PNGs.
    fn moment(&mut self, name: &str, mut screen: impl FnMut(&mut Harness) -> ScreenOutput) {
        let mut standard = None;
        for mode in ColorVisionMode::ALL {
            self.set_mode(mode);
            // Two frames: the first picks up the new theme, the second is the one we keep.
            screen(self);
            screen(self);
            let img = match self.renderer.read_pixels() {
                Ok(img) => img,
                Err(e) => {
                    eprintln!("{name}: {e:#}");
                    return;
                }
            };
            let tag = mode.label().to_lowercase();
            self.save(&format!("{name}_{tag}"), &img);
            match &standard {
                None => standard = Some(img),
                Some(std_img) => {
                    let panels = [
                        half(std_img),
                        half(&simulated(std_img, mode)),
                        half(&simulated(&img, mode)),
                    ];
                    self.save(&format!("{name}_{tag}_compare"), &side_by_side(&panels));
                }
            }
        }
        self.set_mode(ColorVisionMode::Off);
    }

    fn save(&self, name: &str, img: &TextureData) {
        let path = self.out.join(format!("{name}.png"));
        match engine::assets::save_png(&path, img) {
            Ok(()) => println!("wrote {}", path.display()),
            Err(e) => eprintln!("{name}: {e:#}"),
        }
    }
}

/// What a dichromat of `mode` sees of `img`.
fn simulated(img: &TextureData, mode: ColorVisionMode) -> TextureData {
    let mut out = img.clone();
    for px in out.rgba.chunks_exact_mut(4) {
        let c = Vec3::new(px[0] as f32, px[1] as f32, px[2] as f32) / 255.0;
        let s = color_vision::simulate(c, mode) * 255.0;
        px[0] = s.x.round() as u8;
        px[1] = s.y.round() as u8;
        px[2] = s.z.round() as u8;
    }
    out
}

/// 2x box downscale.
fn half(img: &TextureData) -> TextureData {
    let (w, h) = (img.width / 2, img.height / 2);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let at = |dx: u32, dy: u32| {
                    img.rgba[(((y * 2 + dy) * img.width + x * 2 + dx) * 4 + c) as usize] as u32
                };
                rgba.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8);
            }
        }
    }
    TextureData {
        width: w,
        height: h,
        rgba,
    }
}

/// Panels of equal height next to each other with a 4 px gap.
fn side_by_side(panels: &[TextureData]) -> TextureData {
    let gap = 4;
    let h = panels.iter().map(|p| p.height).max().unwrap_or(0);
    let w = panels.iter().map(|p| p.width).sum::<u32>() + gap * (panels.len() as u32 - 1);
    let mut rgba = vec![255u8; (w * h * 4) as usize];
    let mut x0 = 0;
    for p in panels {
        for y in 0..p.height {
            let src = &p.rgba[(y * p.width * 4) as usize..((y + 1) * p.width * 4) as usize];
            let at = ((y * w + x0) * 4) as usize;
            rgba[at..at + src.len()].copy_from_slice(src);
        }
        x0 += p.width + gap;
    }
    TextureData {
        width: w,
        height: h,
        rgba,
    }
}

/// Eye of the local player.
fn eye(game: &InGame) -> Option<Vec3> {
    let net = game.net()?;
    Some(net.local_state()?.eye_position(&net.tuning().movement))
}

fn aim_at(game: &mut InGame, target: Vec3) {
    let Some(from) = eye(game) else {
        return;
    };
    let d = (target - from).normalize_or_zero();
    if d == Vec3::ZERO {
        return;
    }
    game.controls.yaw = (-d.x).atan2(-d.z);
    game.controls.pitch = d.y.clamp(-1.0, 1.0).asin();
}

fn hold(game: &mut InGame) {
    game.controls.override_move = Some((0.0, 0.0, Buttons::NONE));
}

/// Nearest living enemy (position) and whether it is in its parry window.
fn nearest_enemy(game: &InGame) -> Option<(Vec3, bool)> {
    let net = game.net()?;
    let me = eye(game)?;
    net.remote_entities()
        .iter()
        .filter(|e| kind::is_enemy(e.kind) && e.flags & enemy_flags::DYING == 0)
        .min_by(|a, b| {
            let (da, db) = (a.position.distance(me), b.position.distance(me));
            da.total_cmp(&db)
        })
        .map(|e| (e.position, e.flags & enemy_flags::PARRYABLE != 0))
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/a11y_shots"));
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let scratch = std::env::temp_dir().join("velocide-a11y-shots-settings.ron");
    let mut h = Harness {
        renderer,
        common: Common::new(
            Settings::default(),
            scratch,
            Audio::disabled(),
            LaunchOptions::default(),
        ),
        buffers: FrameBuffers::default(),
        input: InputState::new(),
        time: 10.0,
        out,
    };

    // ---- the ACCESSIBILITY page over the menu backdrop -----------------------------------
    use UiKey::*;
    let mut menu = MainMenu::new(&mut h.renderer, &h.common);
    for keys in [&[][..], &[], &[Down], &[Down], &[Down], &[Enter], &[], &[]] {
        h.frame(Screen::Menu(&mut menu), keys, DT);
    }
    for _ in 0..ACCESSIBILITY_TAB {
        h.frame(Screen::Menu(&mut menu), &[Right], DT);
    }
    for _ in 0..30 {
        h.frame(Screen::Menu(&mut menu), &[], DT);
    }
    h.moment("settings", |h| h.frame(Screen::Menu(&mut menu), &[], 1e-4));
    menu.free(&mut h.renderer);

    // ---- hosted co-op game with bots: the lobby, then the match -----------------------------
    let mut lobby = LobbyScreen::start(
        &StartMode::Host {
            port: Some(0),
            level: Some("crucible_of_veins".into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    let lobby_frames = |h: &mut Harness, lobby: &mut LobbyScreen, n: usize| {
        let mut transition = None;
        for _ in 0..n {
            std::thread::sleep(Duration::from_secs_f32(DT));
            let out = h.frame(Screen::Lobby(lobby), &[], DT);
            transition = transition.or(out.transition);
        }
        transition
    };
    lobby_frames(&mut h, &mut lobby, 120);
    if let Some(s) = lobby.session() {
        for name in ["BOT EMBER", "BOT WICK", "BOT SLAG"] {
            s.command(ServerCommand::AddBot { name: name.into() });
        }
    }
    lobby_frames(&mut h, &mut lobby, 90);
    h.moment("lobby", |h| h.frame(Screen::Lobby(&mut lobby), &[], 1e-4));
    if let Some(s) = lobby.session() {
        s.command(ServerCommand::Lobby(LobbyCommand::StartMatch {
            force: true,
        }));
    }
    let mut started = None;
    for _ in 0..12 {
        started = lobby_frames(&mut h, &mut lobby, 60);
        if started.is_some() {
            break;
        }
    }
    println!("lobby transition: {started:?}");
    let handoff = lobby
        .into_handoff(&mut h.renderer)
        .ok_or_else(|| anyhow::anyhow!("lost the session"))?;
    let mut game = InGame::from_handoff(handoff, &mut h.common, &mut h.renderer);
    h.play(&mut game, 180, hold);
    anyhow::ensure!(game.is_playing(), "not in game");
    game.send_chat("/god");
    game.send_chat("/notarget");
    h.play(&mut game, 30, hold);
    // Look at the other players.
    let others: Vec<Vec3> = game
        .net()
        .map(|n| n.remote_players().map(|e| e.position).collect())
        .unwrap_or_default();
    println!("{} other players", others.len());
    if let Some(&p) = others.first() {
        h.play(&mut game, 2, |g| {
            hold(g);
            aim_at(g, p + Vec3::Y * 1.2);
        });
    }
    h.moment("players", |h| h.frame(Screen::Game(&mut game), &[], 1e-4));

    // Scoreboard (hold Select).
    h.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, true));
    h.play(&mut game, 10, hold);
    h.moment("scoreboard", |h| {
        h.frame(Screen::Game(&mut game), &[], 1e-4)
    });
    h.input
        .handle_gamepad_event(GamepadEvent::Button(GamepadButton::Select, false));
    h.play(&mut game, 5, hold);
    game.shutdown(&mut h.renderer, &mut h.common);

    // ---- an enemy in its parry window (single-player: no bots for it to pick on) -------------
    let mut game = InGame::start(
        &StartMode::SinglePlayer {
            level: Some("crucible_of_veins".into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.play(&mut game, 200, hold);
    anyhow::ensure!(game.is_playing(), "single-player didn't start");
    // God mode keeps us alive; enemies still target us.
    game.send_chat("/god");
    // Whichever heavy the roster has (the other name is just an unknown-enemy reply).
    game.send_chat("/spawn gaoler 1");
    game.send_chat("/spawn bastion 1");
    let mut caught = false;
    for _ in 0..1200 {
        let Some((pos, parryable)) = nearest_enemy(&game) else {
            h.play(&mut game, 1, hold);
            continue;
        };
        if parryable {
            caught = true;
            break;
        }
        h.play(&mut game, 1, |g| {
            hold(g);
            aim_at(g, pos + Vec3::Y * 1.6);
        });
    }
    println!("caught a parry window: {caught}");
    h.moment("parry", |h| h.frame(Screen::Game(&mut game), &[], 1e-4));
    game.send_chat("/killall");
    h.play(&mut game, 60, hold);

    // ---- HUD after a hit: hard damage, health popup ------------------------------------------
    game.send_chat("/god"); // off: /hurt needs to land
    game.send_chat("/hurt 45");
    h.play(&mut game, 12, hold);
    h.moment("hud", |h| h.frame(Screen::Game(&mut game), &[], 1e-4));
    // ...and a heal (the popup's sign and colour).
    h.play(&mut game, 90, hold);
    game.send_chat("/give");
    h.play(&mut game, 8, hold);
    h.moment("heal", |h| h.frame(Screen::Game(&mut game), &[], 1e-4));

    game.shutdown(&mut h.renderer, &mut h.common);
    Ok(())
}
