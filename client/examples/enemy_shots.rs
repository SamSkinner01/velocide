//! Headless screenshots of the enemies in action (docs/enemies.md, docs/roster.md): a real
//! single-player session (in-process server) driven by dev commands — every attack of every
//! Mould from its wind-up through its strike: Gnawers biting and pouncing on a strafing
//! player, Censers whirling and lobbing ember bombs (in flight and bursting), Harriers
//! hovering, rearing and diving (and one swatted), the Gaoler's bash wind-up / parry window /
//! bash, its chain sweep and its rush, the Heron's aim laser tracking, locking and the needle
//! (also dodged with a dash), a crowd, then an arena encounter on the campaign level (doors
//! locked, a wave spawning, the HUD readout). The third roster (docs/enemies.md): a lineup of
//! all nine Moulds and a row of elites, the Bellwether hauling its bell back, the toll ringing
//! out and the frenzied pack, its bell shot mid-swing; the Snuffer rearing, latched onto you
//! (smothering the view) and punched off; the Smelter's slag line as a telegraph and burning;
//! the Effigy flinching under fire. The refit (docs/enemies.md "Sizes and hit shapes"): the
//! lineup again with every Mould's hit shapes drawn over it (white: the plain shapes, cyan: as
//! padded by the hit assist for the player's attacks), from the front and from above behind.
//!
//! ```text
//! cargo run -p client --release --example enemy_shots -- [out_dir] [--third]
//! # default out_dir target/enemy_shots; --third: only the third roster (Bellwether, Snuffer,
//! # Smelter, Effigy, elites)
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{DebugLine, RetroSettings, UiDrawList};
use engine::input::InputState;
use engine::render::Renderer;
use engine::ui::UiInput;
use glam::{Vec2, Vec3, Vec4};
use shared::input::Buttons;
use shared::protocol::{EntityState, enemy_flags, kind};
use shared::sim::{AiState, EnemyRegistry, HitPads, HitPart, HitTarget};

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

struct Harness {
    renderer: Renderer,
    common: Common,
    buffers: FrameBuffers,
    input: InputState,
    time: f64,
    out: PathBuf,
    /// Movement held by the player: (forward, right, buttons).
    moving: (f32, f32, Buttons),
    /// Draw every enemy's hit shapes over the frame.
    hit_shapes: bool,
}

impl Harness {
    fn frame(&mut self, game: &mut InGame) {
        std::thread::sleep(Duration::from_secs_f32(DT));
        self.time += DT as f64;
        self.common.stats.update(DT);
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
        if self.hit_shapes {
            hit_shape_lines(game, &mut self.buffers.lines);
        }
        if let Err(e) = self.renderer.render(&self.buffers.desc(&out.scene), None) {
            eprintln!("render: {e:#}");
        }
        self.input.end_frame();
    }

    fn frames(&mut self, game: &mut InGame, n: usize) {
        for _ in 0..n {
            game.controls.override_move = Some(self.moving);
            self.frame(game);
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

    /// Runs frames (aiming at the nearest `kind` enemy) until `pred` holds; returns whether it
    /// did within `max` frames.
    fn until(
        &mut self,
        game: &mut InGame,
        kind: u16,
        max: usize,
        pred: impl Fn(&EntityState) -> bool,
    ) -> bool {
        for _ in 0..max {
            if let Some(t) = nearest(game, kind) {
                aim_at(game, chest(&t));
            }
            game.controls.override_move = Some(self.moving);
            self.frame(game);
            let hit = game.net().is_some_and(|n| {
                n.remote_entities()
                    .iter()
                    .any(|e| e.kind == kind && e.flags & enemy_flags::DYING == 0 && pred(e))
            });
            if hit {
                return true;
            }
        }
        false
    }
}

/// Wireframes of every living enemy's hit shapes, built from the replicated position and yaw
/// like the server builds them (`EnemyDef::hitbox`): white the plain shapes (enemy attacks,
/// lag compensation), weak points orange; cyan the same padded by the hit assist (what the
/// player's attacks test).
fn hit_shape_lines(game: &InGame, out: &mut Vec<DebugLine>) {
    let Some(net) = game.net() else {
        return;
    };
    let assist = &net.tuning().combat.hit_assist;
    let pads = HitPads {
        body: assist.enemy_body_pad,
        weak: assist.enemy_weak_pad,
    };
    let defs = EnemyRegistry::builtin();
    for e in net.remote_entities() {
        if !kind::is_enemy(e.kind) || e.flags & enemy_flags::DYING != 0 {
            continue;
        }
        let Some(def) = defs.by_kind(e.kind) else {
            continue;
        };
        let hitbox = def.hitbox(HitTarget::Entity(e.id), e.position, e.yaw);
        for s in &hitbox.shapes {
            let plain = if s.part == HitPart::Body {
                Vec4::new(1.0, 1.0, 1.0, 0.9)
            } else {
                Vec4::new(1.0, 0.55, 0.1, 1.0)
            };
            capsule_wire(s.a, s.b, s.radius, plain, out);
            let pad = pads.for_part(s.part);
            capsule_wire(s.a, s.b, s.radius + pad, Vec4::new(0.2, 0.9, 1.0, 0.7), out);
        }
    }
}

/// A capsule as lines: a ring around each end, four lines along it, and the end caps as two
/// half circles each.
fn capsule_wire(a: Vec3, b: Vec3, r: f32, color: Vec4, out: &mut Vec<DebugLine>) {
    const N: usize = 16;
    let axis = (b - a).try_normalize().unwrap_or(Vec3::Y);
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let mut line = |p: Vec3, q: Vec3| out.push(DebugLine { a: p, b: q, color });
    for c in [a, b] {
        for i in 0..N {
            let t0 = i as f32 / N as f32 * std::f32::consts::TAU;
            let t1 = (i + 1) as f32 / N as f32 * std::f32::consts::TAU;
            line(
                c + (u * t0.cos() + v * t0.sin()) * r,
                c + (u * t1.cos() + v * t1.sin()) * r,
            );
        }
    }
    for side in [u, v, -u, -v] {
        line(a + side * r, b + side * r);
    }
    // Caps: half circles over each end, in the two planes through the axis.
    for (c, out_dir) in [(a, -axis), (b, axis)] {
        for side in [u, v] {
            for i in 0..N / 2 {
                let t0 = i as f32 / (N / 2) as f32 * std::f32::consts::PI;
                let t1 = (i + 1) as f32 / (N / 2) as f32 * std::f32::consts::PI;
                line(
                    c + (side * t0.cos() + out_dir * t0.sin()) * r,
                    c + (side * t1.cos() + out_dir * t1.sin()) * r,
                );
            }
        }
    }
}

fn eye(game: &InGame) -> Option<Vec3> {
    let net = game.net()?;
    Some(net.local_state()?.eye_position(&net.tuning().movement))
}

/// Where to look at an enemy (about the middle of its body; Harriers hover).
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
                kind::EFFIGY => 1.4,
                _ => 1.0,
            }
}

fn nearest(game: &InGame, k: u16) -> Option<EntityState> {
    let net = game.net()?;
    let me = eye(game)?;
    net.remote_entities()
        .iter()
        .filter(|e| e.kind == k && e.flags & enemy_flags::DYING == 0)
        .min_by(|a, b| a.position.distance(me).total_cmp(&b.position.distance(me)))
        .copied()
}

fn aim_at(game: &mut InGame, target: Vec3) {
    let Some(from) = eye(game) else {
        return;
    };
    let d = (target - from).normalize();
    game.controls.yaw = (-d.x).atan2(-d.z);
    game.controls.pitch = d.y.clamp(-1.0, 1.0).asin();
}

fn attack(e: &EntityState) -> u8 {
    ((e.aux >> 8) & 0xFF) as u8
}

/// Wind-up progress (0..255) of attack `index`, 0 otherwise.
fn windup_of(e: &EntityState, index: u8) -> u32 {
    if e.state == AiState::Windup as u16 && attack(e) == index {
        e.aux & 0xFF
    } else {
        0
    }
}

fn striking(e: &EntityState, index: u8) -> bool {
    e.state == AiState::Strike as u16 && attack(e) == index
}

/// A dev command, paced to the server's chat rate limit (a burst of 5, then 1 per second).
fn cmd(h: &mut Harness, game: &mut InGame, text: &str) {
    game.send_chat(text);
    h.frames(game, 62);
}

fn look_from(h: &mut Harness, game: &mut InGame, at: Vec3, target: Vec3) {
    cmd(h, game, &format!("/tp {} {} {}", at.x, at.y, at.z));
    aim_at(game, target);
    h.frames(game, 2);
}

fn any_enemy(game: &InGame, k: u16, pred: impl Fn(&EntityState) -> bool) -> bool {
    game.net().is_some_and(|n| {
        n.remote_entities()
            .iter()
            .any(|e| e.kind == k && e.flags & enemy_flags::DYING == 0 && pred(e))
    })
}

/// The Bellwether, Snuffer, Smelter and Effigy, and elites.
fn third_roster(h: &mut Harness, game: &mut InGame) {
    // Lineup: every Mould in a row (spawned 8 m in front of where we stand, facing us), then
    // a row of elites behind them; nobody targets us while we look.
    cmd(h, game, "/notarget");
    let names = [
        "gnawer",
        "censer",
        "harrier",
        "gaoler",
        "heron",
        "bellwether",
        "snuffer",
        "smelter",
        "effigy",
    ];
    for (i, name) in names.iter().enumerate() {
        let x = -16.0 + 4.0 * i as f32;
        game.controls.yaw = 0.0;
        game.controls.pitch = 0.0;
        cmd(h, game, &format!("/tp {x} 0 10"));
        h.frames(game, 4);
        cmd(h, game, &format!("/spawn {name}"));
        h.frames(game, 2);
    }
    look_from(h, game, Vec3::new(0.0, 0.0, 19.0), Vec3::new(0.0, 1.2, 2.0));
    h.frames(game, 30);
    h.shot("third_lineup");
    // The same Moulds with their hit shapes (docs/enemies.md "Sizes and hit shapes"), three at
    // a time from the front, then the middle three from above and behind (the Gaoler's
    // lantern, the Bellwether's bell). No gun in the way.
    h.hit_shapes = true;
    h.common.settings.show_viewmodel = false;
    // (They stand 8 m in front of where each was spawned from: z = 2.)
    for (i, x) in [-12.0f32, 0.0, 12.0].into_iter().enumerate() {
        look_from(h, game, Vec3::new(x, 0.0, 9.5), Vec3::new(x, 1.5, 2.0));
        h.frames(game, 2);
        h.shot(&format!("hit_shapes_{}", i + 1));
    }
    look_from(h, game, Vec3::new(0.0, 3.0, -6.0), Vec3::new(0.0, 1.5, 2.0));
    h.frames(game, 2);
    h.shot("hit_shapes_behind");
    h.hit_shapes = false;
    h.common.settings.show_viewmodel = true;
    cmd(h, game, "/killall");
    h.frames(game, 60);
    for (i, name) in ["gnawer", "gaoler", "bellwether", "snuffer", "smelter"]
        .iter()
        .enumerate()
    {
        let x = -10.0 + 5.0 * i as f32;
        game.controls.yaw = 0.0;
        cmd(h, game, &format!("/tp {x} 0 10"));
        h.frames(game, 4);
        cmd(h, game, &format!("/spawn {name} 1 elite"));
        h.frames(game, 2);
    }
    look_from(h, game, Vec3::new(0.0, 0.0, 15.0), Vec3::new(0.0, 1.4, 2.0));
    h.frames(game, 40);
    h.shot("elites");
    cmd(h, game, "/killall");
    h.frames(game, 60);
    cmd(h, game, "/notarget");
    h.frames(game, 4);

    // Bellwether: it hangs back with its pack (two Censers holding their range beside it),
    // hauls the bell back (the bell glows), tolls; the pack is frenzied. Set up unnoticed.
    cmd(h, game, "/notarget");
    game.controls.yaw = 0.0;
    cmd(h, game, "/tp 0 0 10");
    cmd(h, game, "/spawn bellwether 1");
    cmd(h, game, "/spawn censer 2");
    cmd(h, game, "/tp 0 0 18");
    if let Some(b) = nearest(game, kind::BELLWETHER) {
        aim_at(game, b.position + Vec3::Y * 2.0);
    }
    cmd(h, game, "/notarget");
    let ok = h.until(game, kind::BELLWETHER, 900, |e| windup_of(e, 1) > 150);
    println!("bellwether toll wind-up: {ok}");
    h.shot("bellwether_toll_windup");
    let ok = h.until(game, kind::BELLWETHER, 120, |e| striking(e, 1));
    h.frames(game, 3);
    println!("bellwether toll: {ok}");
    h.shot("bellwether_toll");
    let ok = (0..60).any(|_| {
        h.frames(game, 1);
        any_enemy(game, kind::CENSER, |e| e.flags & enemy_flags::FRENZIED != 0)
    });
    if let Some(c) = nearest(game, kind::CENSER) {
        aim_at(game, chest(&c));
    }
    h.frames(game, 8);
    println!("frenzied pack: {ok}");
    h.shot("frenzied_pack");
    // The next toll: shoot the bell mid-swing (it cracks, the toll is cancelled).
    let ok = h.until(game, kind::BELLWETHER, 900, |e| windup_of(e, 1) > 60);
    if let Some(b) = nearest(game, kind::BELLWETHER) {
        // The bell hangs behind its centre line (its weak point in the definition).
        let offset = EnemyRegistry::builtin()
            .get("bellwether")
            .and_then(|d| d.weak_points.first().map(|w| w.offset))
            .unwrap_or(Vec3::new(0.0, 3.17, 0.36));
        let bell = b.position + glam::Quat::from_rotation_y(b.yaw) * offset;
        aim_at(game, bell);
    }
    h.frames(game, 1);
    h.moving = (0.0, 0.0, Buttons::FIRE);
    h.frames(game, 1);
    h.moving = (0.0, 0.0, Buttons::NONE);
    let cracked = (0..60).any(|_| {
        h.frames(game, 1);
        any_enemy(game, kind::BELLWETHER, |e| {
            e.state == AiState::Stunned as u16
        })
    });
    h.frames(game, 4);
    println!("bell shot mid-swing ({ok}): cracked {cracked}");
    h.shot("bellwether_bell_cracked");
    cmd(h, game, "/killall");
    h.frames(game, 60);

    // Snuffer: rears up (the cap lifts off its wick), leaps and latches on; punched off.
    game.controls.yaw = 0.0;
    cmd(h, game, "/spawn snuffer 1");
    let ok = h.until(game, kind::SNUFFER, 600, |e| windup_of(e, 0) > 150);
    println!("snuffer rearing: {ok}");
    h.shot("snuffer_windup");
    let ok = h.until(game, kind::SNUFFER, 120, |e| {
        e.flags & enemy_flags::LATCHED != 0
    });
    h.frames(game, 20);
    println!("snuffer latched: {ok}");
    h.shot("snuffer_latched");
    h.moving = (0.0, 0.0, Buttons::PUNCH);
    h.frames(game, 1);
    h.moving = (0.0, 0.0, Buttons::NONE);
    h.frames(game, 10);
    if let Some(s) = nearest(game, kind::SNUFFER) {
        aim_at(game, chest(&s));
    }
    h.frames(game, 2);
    println!(
        "snuffer punched off: {}",
        any_enemy(game, kind::SNUFFER, |e| e.state == AiState::Stunned as u16)
    );
    h.shot("snuffer_punched_off");
    cmd(h, game, "/killall");
    h.frames(game, 60);

    // Smelter: the pour's telegraph line, then the burning slag. It pours as soon as it sees
    // you at range (5-24 m; closer it heaves): catch the first half of that wind-up, the line
    // laid and glowing, not burning yet.
    game.controls.yaw = 0.0;
    cmd(h, game, "/tp 0 0 20");
    game.send_chat("/spawn smelter 1");
    let ok = h.until(game, kind::SMELTER, 300, |e| {
        (25..120).contains(&windup_of(e, 1))
    });
    // Step aside to see the line from an angle.
    h.moving = (0.0, 1.0, Buttons::NONE);
    h.frames(game, 8);
    h.moving = (0.0, 0.0, Buttons::NONE);
    if let Some(s) = nearest(game, kind::SMELTER) {
        aim_at(game, s.position + Vec3::Y * 0.5);
    }
    h.frames(game, 1);
    println!("smelter pour wind-up: {ok}");
    h.shot("smelter_pour_telegraph");
    let ok = h.until(game, kind::SMELTER, 120, |e| striking(e, 1));
    h.frames(game, 20);
    if let Some(s) = nearest(game, kind::SMELTER) {
        aim_at(game, s.position + Vec3::Y * 0.5);
    }
    h.frames(game, 1);
    println!("smelter pour: {ok}");
    h.shot("smelter_slag_burning");
    cmd(h, game, "/killall");
    h.frames(game, 60);

    // Effigy: a few shots into it (it flinches, sprays wax; never dies).
    game.controls.yaw = 0.0;
    cmd(h, game, "/spawn effigy 1");
    h.frames(game, 10);
    if let Some(e) = nearest(game, kind::EFFIGY) {
        aim_at(game, chest(&e));
    }
    for _ in 0..3 {
        h.moving = (0.0, 0.0, Buttons::FIRE);
        h.frames(game, 1);
        h.moving = (0.0, 0.0, Buttons::NONE);
        h.frames(game, 18);
    }
    h.frames(game, 2);
    h.shot("effigy_hit");
    cmd(h, game, "/killall");
    h.frames(game, 60);
}

fn session(h: &mut Harness, level: &str) -> anyhow::Result<InGame> {
    let mut game = InGame::start(
        &StartMode::SinglePlayer {
            level: Some(level.into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.frames(&mut game, 200);
    anyhow::ensure!(game.is_playing(), "not in game");
    game.send_chat("/god");
    h.frames(&mut game, 10);
    Ok(game)
}

fn clear(h: &mut Harness, game: &mut InGame) {
    h.moving = (0.0, 0.0, Buttons::NONE);
    game.send_chat("/killall");
    h.frames(game, 90);
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let out = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/enemy_shots"));
    let only_third = std::env::args().any(|a| a == "--third");
    std::fs::create_dir_all(&out)?;
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let scratch = std::env::temp_dir().join("velocide-enemy-shots-settings.ron");
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
        moving: (0.0, 0.0, Buttons::NONE),
        hit_shapes: false,
    };

    // ---- Proving ground: one type at a time. ----
    let mut game = session(&mut h, shared::sim::BUILTIN_LEVEL)?;
    if only_third {
        third_roster(&mut h, &mut game);
        game.shutdown(&mut h.renderer, &mut h.common);
        return Ok(());
    }

    // Gnawers: the pack comes knuckle-running, the jaw unhinges, the bite; then the player
    // strafes and gets pounced on.
    game.send_chat("/spawn gnawer 3");
    h.frames(&mut game, 25);
    h.shot("gnawer_pack");
    let ok = h.until(&mut game, kind::GNAWER, 600, |e| windup_of(e, 0) > 160);
    println!("gnawer bite wind-up: {ok}");
    h.shot("gnawer_bite_windup");
    let ok = h.until(&mut game, kind::GNAWER, 60, |e| striking(e, 0));
    println!("gnawer bite: {ok}");
    h.shot("gnawer_bite");
    clear(&mut h, &mut game);
    game.send_chat("/spawn gnawer 2");
    h.moving = (0.0, 0.4, Buttons::NONE);
    let ok = h.until(&mut game, kind::GNAWER, 600, |e| windup_of(e, 1) > 120);
    println!("gnawer pounce wind-up: {ok}");
    h.shot("gnawer_pounce_windup");
    let ok = h.until(&mut game, kind::GNAWER, 60, |e| striking(e, 1));
    h.frames(&mut game, 5);
    println!("gnawer pounce: {ok}");
    h.shot("gnawer_pounce");
    clear(&mut h, &mut game);

    // Censers: back off, whirl the censer overhead, lob three ember bombs that burst.
    game.send_chat("/spawn censer 2");
    h.frames(&mut game, 30);
    h.shot("censer_spawn");
    let ok = h.until(&mut game, kind::CENSER, 900, |e| windup_of(e, 1) > 180);
    println!("censer whirl: {ok}");
    h.shot("censer_whirl");
    let ok = h.until(&mut game, kind::CENSER, 120, |e| striking(e, 1));
    h.frames(&mut game, 18);
    println!("censer lob: {ok}");
    h.shot("censer_embers_in_flight");
    let burst = (0..200).any(|_| {
        h.frames(&mut game, 1);
        !game.net().is_some_and(|n| {
            n.remote_entities()
                .iter()
                .any(|e| e.kind == kind::EMBER_BOMB)
        })
    });
    h.frames(&mut game, 2);
    println!("ember bombs burst: {burst}");
    h.shot("censer_embers_burst");
    // The swing when crowded.
    clear(&mut h, &mut game);
    game.send_chat("/spawn censer 1");
    h.frames(&mut game, 2);
    h.moving = (1.0, 0.0, Buttons::NONE);
    let ok = h.until(&mut game, kind::CENSER, 300, |e| windup_of(e, 0) > 150);
    h.moving = (0.0, 0.0, Buttons::NONE);
    println!("censer swing wind-up: {ok}");
    h.shot("censer_swing_windup");
    clear(&mut h, &mut game);

    // Harriers: hover and circle, rear up, dive; one gets swatted.
    game.send_chat("/spawn harrier 3");
    h.frames(&mut game, 150);
    if let Some(g) = nearest(&game, kind::HARRIER) {
        aim_at(&mut game, chest(&g));
    }
    h.frames(&mut game, 2);
    h.shot("harriers_hovering");
    let ok = h.until(&mut game, kind::HARRIER, 600, |e| windup_of(e, 0) > 170);
    println!("harrier rearing: {ok}");
    h.shot("harrier_windup");
    let ok = h.until(&mut game, kind::HARRIER, 60, |e| striking(e, 0));
    h.frames(&mut game, 6);
    println!("harrier dive: {ok}");
    h.shot("harrier_dive");
    // Swat: punch the next diving one when it is close.
    let mut swatted = false;
    for _ in 0..900 {
        let me = eye(&game).unwrap_or_default();
        let close = game.net().and_then(|n| {
            n.remote_entities()
                .iter()
                .filter(|e| e.kind == kind::HARRIER && striking(e, 0))
                .find(|e| chest(e).distance(me) < 4.5)
                .copied()
        });
        if let Some(e) = close {
            aim_at(&mut game, chest(&e));
            h.moving = (0.0, 0.0, Buttons::PUNCH);
            h.frames(&mut game, 1);
            h.moving = (0.0, 0.0, Buttons::NONE);
            h.frames(&mut game, 8);
            swatted = true;
            break;
        }
        if let Some(t) = nearest(&game, kind::HARRIER) {
            aim_at(&mut game, chest(&t));
        }
        h.frames(&mut game, 1);
    }
    println!("harrier swatted: {swatted}");
    h.shot("harrier_swatted");
    clear(&mut h, &mut game);

    // Gaoler: the door drawn back, the parry window strobe, the bash; the chain sweep; the
    // rush from afar.
    game.send_chat("/spawn gaoler 1");
    h.frames(&mut game, 30);
    h.shot("gaoler_spawn");
    let ok = h.until(&mut game, kind::GAOLER, 900, |e| windup_of(e, 0) > 60);
    println!("gaoler bash wind-up: {ok}");
    h.shot("gaoler_bash_windup");
    let ok = h.until(&mut game, kind::GAOLER, 60, |e| {
        e.flags & enemy_flags::PARRYABLE != 0
    });
    h.frames(&mut game, 6);
    println!("gaoler parry window: {ok}");
    h.shot("gaoler_parryable");
    let ok = h.until(&mut game, kind::GAOLER, 60, |e| striking(e, 0));
    h.frames(&mut game, 3);
    println!("gaoler bash: {ok}");
    h.shot("gaoler_bash");
    // Step back a little: the sweep.
    h.moving = (-1.0, 0.0, Buttons::NONE);
    h.frames(&mut game, 14);
    h.moving = (0.0, 0.0, Buttons::NONE);
    let ok = h.until(&mut game, kind::GAOLER, 600, |e| windup_of(e, 1) > 150);
    println!("gaoler sweep wind-up: {ok}");
    h.shot("gaoler_sweep_windup");
    let ok = h.until(&mut game, kind::GAOLER, 60, |e| striking(e, 1));
    h.frames(&mut game, 6);
    println!("gaoler sweep: {ok}");
    h.shot("gaoler_sweep");
    clear(&mut h, &mut game);
    // The rush: back away first so it is 8-20 m out.
    game.send_chat("/spawn gaoler 1");
    h.moving = (-1.0, 0.0, Buttons::NONE);
    h.frames(&mut game, 40);
    h.moving = (0.0, 0.0, Buttons::NONE);
    let ok = h.until(&mut game, kind::GAOLER, 900, |e| windup_of(e, 2) > 150);
    println!("gaoler rush wind-up: {ok}");
    h.shot("gaoler_rush_windup");
    let ok = h.until(&mut game, kind::GAOLER, 90, |e| striking(e, 2));
    h.frames(&mut game, 12);
    println!("gaoler rush: {ok}");
    h.shot("gaoler_rush");
    clear(&mut h, &mut game);

    // Heron: walks off to sniping range, the laser tracks, locks, the needle; then a second
    // shot dodged with a dash so the locked laser and the needle go past.
    game.send_chat("/spawn heron 1");
    h.frames(&mut game, 30);
    h.shot("heron_spawn");
    let ok = h.until(&mut game, kind::HERON, 900, |e| {
        windup_of(e, 1) > 100 && e.flags & enemy_flags::AIMING != 0
    });
    println!("heron laser: {ok}");
    h.shot("heron_laser");
    let ok = h.until(&mut game, kind::HERON, 90, |e| {
        e.flags & enemy_flags::AIM_LOCKED != 0
    });
    h.frames(&mut game, 4);
    println!("heron locked: {ok}");
    h.shot("heron_laser_locked");
    let ok = h.until(&mut game, kind::HERON, 60, |e| striking(e, 1));
    h.frames(&mut game, 1);
    println!("heron needle: {ok}");
    h.shot("heron_needle");
    let ok = h.until(&mut game, kind::HERON, 600, |e| {
        e.flags & enemy_flags::AIM_LOCKED != 0
    });
    h.moving = (0.0, 1.0, Buttons::DASH);
    h.frames(&mut game, 1);
    h.moving = (0.0, 1.0, Buttons::NONE);
    h.frames(&mut game, 9);
    h.moving = (0.0, 0.0, Buttons::NONE);
    if let Some(e) = nearest(&game, kind::HERON) {
        aim_at(&mut game, chest(&e));
    }
    h.frames(&mut game, 2);
    println!("heron dodged: {ok}");
    h.shot("heron_dodged");
    clear(&mut h, &mut game);

    third_roster(&mut h, &mut game);

    // A mixed crowd.
    game.send_chat("/spawn gnawer 4");
    game.send_chat("/spawn censer 2");
    game.send_chat("/spawn harrier 2");
    h.frames(&mut game, 60);
    game.send_chat("/spawn gaoler 1");
    game.send_chat("/spawn heron 1");
    h.frames(&mut game, 60);
    // Back off so the crowd fits the view.
    h.moving = (-1.0, 0.0, Buttons::NONE);
    h.frames(&mut game, 50);
    h.moving = (0.0, 0.0, Buttons::NONE);
    if let Some(b) = nearest(&game, kind::GAOLER) {
        aim_at(&mut game, chest(&b));
    }
    h.frames(&mut game, 2);
    h.shot("mixed_crowd");
    // Kills: ember bursts, melting corpses.
    game.send_chat("/killall");
    h.frames(&mut game, 6);
    h.shot("mixed_crowd_killed");
    h.frames(&mut game, 30);
    h.shot("mixed_crowd_melting");
    game.shutdown(&mut h.renderer, &mut h.common);

    // ---- Campaign: an arena encounter (Crucible Heart: the gaoler on the island). ----
    let mut game = session(&mut h, "crucible_of_veins")?;
    game.send_chat("/encounter 4");
    h.frames(&mut game, 20);
    h.shot("arena4_locked");
    h.frames(&mut game, 60);
    h.shot("arena4_wave1");
    let ok = h.until(&mut game, kind::GAOLER, 600, |e| {
        e.state == AiState::Windup as u16
    });
    println!("arena gaoler wind-up: {ok}");
    h.shot("arena4_gaoler");
    game.send_chat("/killall");
    h.frames(&mut game, 150);
    h.shot("arena4_wave2");
    game.send_chat("/killall");
    h.frames(&mut game, 30);
    game.send_chat("/encounters");
    h.frames(&mut game, 60);
    h.shot("arena4_cleared");
    game.shutdown(&mut h.renderer, &mut h.common);
    Ok(())
}
