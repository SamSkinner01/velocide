//! Headless combat screenshots: a real single-player session (in-process server) driven by a
//! script — dev commands spawn enemies and needles, the local player aims and fires every weapon
//! mode and throws thorns — and PNGs of the moments worth looking at: Quill tracers, brand
//! sigils, splinter bursts, Bellows pellets and gusts, Arclash arcs / chains / wisps /
//! Discharge novas, Tumbler bombs, Breach and Gravewell, thorn ricochets, deflections and
//! parries, enemies walking / winding up / dying, the HUD with damage and hard damage, hit
//! markers, the kill feed and the death screen.
//!
//! ```text
//! cargo run -p client --release --example combat_shots -- [out_dir]   # default target/combat_shots
//! ```

use std::path::PathBuf;
use std::time::Duration;

use client::app::Common;
use client::cli::{LaunchOptions, StartMode};
use client::frame::{FrameBuffers, FrameIo};
use client::ingame::InGame;
use client::net::{ClientConfig, NetClient};
use client::settings::Settings;
use engine::audio::Audio;
use engine::gfx::{RetroSettings, UiDrawList};
use engine::input::InputState;
use engine::render::Renderer;
use engine::ui::UiInput;
use glam::{Vec2, Vec3};
use shared::input::{Buttons, PlayerInput};
use shared::protocol::{kind, projectile_flags};

/// Enemy the script spawns (`/spawn <name>`): any ground enemy of the roster will do.
const TARGET: &str = "gnawer";

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
    shots: Vec<String>,
    /// Frame + render time (ms) of every frame, without the real-time sleep.
    frame_ms: Vec<f32>,
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
        let started = std::time::Instant::now();
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
        self.frame_ms.push(started.elapsed().as_secs_f32() * 1000.0);
        self.input.end_frame();
    }

    fn frames(&mut self, game: &mut InGame, n: usize, mut each: impl FnMut(&mut InGame, usize)) {
        for i in 0..n {
            each(game, i);
            self.frame(game);
        }
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
}

/// Eye of the local player.
fn eye(game: &InGame) -> Option<Vec3> {
    let net = game.net()?;
    let t = &net.tuning().movement;
    Some(net.local_state()?.eye_position(t))
}

/// Nearest living enemy's chest (or head).
fn nearest_enemy(game: &InGame, head: bool) -> Option<Vec3> {
    let net = game.net()?;
    let me = eye(game)?;
    net.remote_entities()
        .iter()
        .filter(|e| kind::is_enemy(e.kind) && e.flags & shared::protocol::enemy_flags::DYING == 0)
        .map(|e| e.position + Vec3::Y * if head { 1.66 } else { 0.95 })
        .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
}

fn aim_at(game: &mut InGame, target: Vec3) {
    let Some(from) = eye(game) else {
        return;
    };
    let d = (target - from).normalize();
    game.controls.yaw = (-d.x).atan2(-d.z);
    game.controls.pitch = d.y.clamp(-1.0, 1.0).asin();
}

fn hold(game: &mut InGame, buttons: Buttons) {
    game.controls.override_move = Some((0.0, 0.0, buttons));
}

fn select(game: &mut InGame, weapon: u8, mode: u8) {
    game.controls.weapons.current = weapon;
    game.controls.weapons.modes[weapon as usize] = mode;
}

/// Where a replicated entity of `pick` is drawn (projectiles / thorns by trajectory).
fn find_entity(
    game: &InGame,
    pick: impl Fn(&shared::protocol::EntityState) -> bool,
) -> Option<Vec3> {
    let n = game.net()?;
    n.remote_entities().iter().find(|e| pick(e)).map(|e| {
        shared::sim::projectiles::replicated_position(
            e,
            n.render_tick(),
            n.tick_rate(),
            &n.tuning().combat,
        )
    })
}

fn spawn(game: &mut InGame, count: u32) {
    game.send_chat(&format!("/spawn {TARGET} {count}"));
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/combat_shots"));
    std::fs::create_dir_all(&out)?;
    let settings = Settings::default();
    let renderer = Renderer::new_headless(W, H, &RetroSettings::default())?;
    let scratch = std::env::temp_dir().join("velocide-combat-shots-settings.ron");
    let mut h = Harness {
        renderer,
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
        shots: Vec::new(),
        frame_ms: Vec::new(),
    };
    let mut game = InGame::start(
        &StartMode::SinglePlayer {
            level: Some(shared::sim::BUILTIN_LEVEL.into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.frames(&mut game, 200, |g, _| hold(g, Buttons::NONE));
    anyhow::ensure!(game.is_playing(), "not in game");

    // Enemies in front, us invulnerable and ignored for the weapon shots.
    game.send_chat("/god");
    game.send_chat("/notarget");
    spawn(&mut game, 4);
    h.frames(&mut game, 60, |g, _| hold(g, Buttons::NONE));

    // Quill: muzzle flash, tracer, hit marker, damage number.
    select(&mut game, 0, 0);
    h.frames(&mut game, 10, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, if i == 6 { Buttons::FIRE } else { Buttons::NONE });
    });
    h.shot("combat_quill");
    h.frames(&mut game, 4, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_quill_marker");

    // Brand: the marking shot, then the sigil over the target.
    h.frames(&mut game, 12, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_brand");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_brand_sigil");

    // Splinter: the shot bursts into splinters behind the target.
    select(&mut game, 0, 1);
    h.frames(&mut game, 120, |g, _| hold(g, Buttons::NONE));
    h.frames(&mut game, 8, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_splinter");
    h.frames(&mut game, 5, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_splinter_burst");

    // Bellows: pellets, then a Gust, then both barrels.
    spawn(&mut game, 3);
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));
    select(&mut game, 1, 0);
    h.frames(&mut game, 3, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, if i == 0 { Buttons::FIRE } else { Buttons::NONE });
    });
    h.shot("combat_bellows");
    h.frames(&mut game, 50, |g, _| hold(g, Buttons::NONE));
    h.frames(&mut game, 5, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_gust");
    select(&mut game, 1, 1);
    h.frames(&mut game, 100, |g, _| hold(g, Buttons::NONE));
    h.frames(&mut game, 3, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_both_barrels");
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_heat_release");

    // Arclash: a held stream of zaps chaining through a pack.
    game.send_chat("/killall");
    spawn(&mut game, 5);
    h.frames(&mut game, 50, |g, _| hold(g, Buttons::NONE));
    select(&mut game, 2, 0);
    h.frames(&mut game, 14, |g, _| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, Buttons::FIRE);
    });
    h.shot("combat_arc");
    h.frames(&mut game, 3, |g, _| hold(g, Buttons::FIRE));
    h.shot("combat_arc_chain");
    // Wisp drifting into them, zapping.
    h.frames(&mut game, 30, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t + Vec3::Y * 0.4);
        }
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_wisp");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_wisp_2");
    // Discharge: charge, then the nova.
    spawn(&mut game, 3);
    h.frames(&mut game, 60, |g, _| hold(g, Buttons::NONE));
    select(&mut game, 2, 1);
    h.frames(&mut game, 55, |g, _| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, Buttons::ALT_FIRE);
    });
    h.shot("combat_discharge_charging");
    h.frames(&mut game, 3, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_discharge_nova");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));

    // Tumbler: a bomb in flight, then the explosion.
    spawn(&mut game, 3);
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));
    select(&mut game, 3, 0);
    h.frames(&mut game, 5, |g, i| {
        if let Some(t) = nearest_enemy(g, true) {
            aim_at(g, t);
        }
        hold(g, if i == 0 { Buttons::FIRE } else { Buttons::NONE });
    });
    h.shot("combat_bomb_flight");
    for i in 0..30 {
        h.frame(&mut game);
        if i == 6 {
            h.shot("combat_explosion");
        }
    }
    h.shot("combat_after_explosion");
    // Breach at the floor in front.
    h.frames(&mut game, 4, |g, i| {
        g.controls.pitch = -0.55;
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_breach");
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));
    // Gravewell: lobbed among them, pulling, imploding.
    spawn(&mut game, 4);
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));
    select(&mut game, 3, 1);
    h.frames(&mut game, 70, |g, i| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        g.controls.pitch -= 0.1;
        hold(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_gravewell_pull");
    for _ in 0..90 {
        hold(&mut game, Buttons::NONE);
        h.frame(&mut game);
        let pulling = find_entity(&game, |e| {
            e.kind == kind::GRAVEWELL && e.flags & projectile_flags::STUCK != 0
        });
        if pulling.is_none() {
            h.frames(&mut game, 3, |g, _| hold(g, Buttons::NONE));
            h.shot("combat_gravewell_implode");
            break;
        }
    }
    h.frames(&mut game, 60, |g, _| hold(g, Buttons::NONE));

    // Thorns: one planted in the wall far ahead, one lying after hitting an enemy; then a
    // Quill shot relays off the planted one.
    game.send_chat("/killall");
    spawn(&mut game, 2);
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));
    select(&mut game, 0, 0);
    h.frames(&mut game, 50, |g, i| {
        g.controls.yaw = 0.3;
        g.controls.pitch = -0.12;
        hold(
            g,
            if i == 0 {
                Buttons::THROW
            } else {
                Buttons::NONE
            },
        );
    });
    h.shot("combat_thorn_planted");
    let mut fired = false;
    for _ in 0..40 {
        let thorn = find_entity(&game, |e| e.kind == kind::THORN);
        let mut b = Buttons::NONE;
        if let Some(p) = thorn {
            aim_at(&mut game, p);
            if !fired {
                b = Buttons::FIRE;
                fired = true;
            }
        }
        hold(&mut game, b);
        h.frame(&mut game);
        if fired && b == Buttons::NONE {
            break;
        }
    }
    h.frames(&mut game, 9, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_ricochet");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));

    // Enemies attacking: walking, winding up (we're still god-moded).
    game.send_chat("/notarget");
    spawn(&mut game, 4);
    h.frames(&mut game, 50, |g, _| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, Buttons::NONE);
    });
    h.shot("combat_enemies_walking");
    let mut windup = false;
    for _ in 0..240 {
        if let Some(t) = nearest_enemy(&game, false) {
            aim_at(&mut game, t);
        }
        hold(&mut game, Buttons::NONE);
        h.frame(&mut game);
        let w = game.net().is_some_and(|n| {
            n.remote_entities().iter().any(|e| {
                kind::is_enemy(e.kind)
                    && e.state == shared::sim::AiState::Windup as u16
                    && (e.aux & 0xFF) > 150
            })
        });
        if w {
            windup = true;
            break;
        }
    }
    println!("caught a wind-up: {windup}");
    h.shot("combat_enemy_windup");

    // Gust an orb back, then parry another.
    game.send_chat("/killall");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));
    game.controls.pitch = 0.0;
    let enemy_shot = |e: &shared::protocol::EntityState| {
        kind::is_projectile(e.kind)
            && e.flags & projectile_flags::ENEMY != 0
            && e.flags & projectile_flags::PARRIED == 0
    };
    select(&mut game, 1, 0);
    game.send_chat("/orb 16");
    let mut gusted = false;
    for _ in 0..120 {
        let me = eye(&game).unwrap_or(Vec3::ZERO);
        let orb = find_entity(&game, enemy_shot).map(|p| p.distance(me));
        let b = if !gusted && orb.is_some_and(|d| d < 5.0) {
            gusted = true;
            Buttons::ALT_FIRE
        } else {
            Buttons::NONE
        };
        hold(&mut game, b);
        h.frame(&mut game);
        if gusted {
            break;
        }
    }
    h.frames(&mut game, 4, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_deflect");
    h.frames(&mut game, 60, |g, _| hold(g, Buttons::NONE));
    game.send_chat("/orb 16");
    let mut punched = false;
    for _ in 0..120 {
        let me = eye(&game).unwrap_or(Vec3::ZERO);
        let orb = find_entity(&game, enemy_shot).map(|p| p.distance(me));
        let b = if !punched && orb.is_some_and(|d| d < 2.2) {
            punched = true;
            Buttons::PUNCH
        } else {
            Buttons::NONE
        };
        if orb.is_some_and(|d| d < 8.0) && orb.is_some_and(|d| d > 6.0) {
            h.shot("combat_orb_incoming");
        }
        hold(&mut game, b);
        h.frame(&mut game);
        if punched {
            break;
        }
    }
    h.frames(&mut game, 3, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_parry");
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));

    // An enemy hits us from the side: damage flash + direction indicator.
    game.send_chat("/god");
    spawn(&mut game, 1);
    h.frames(&mut game, 20, |g, _| hold(g, Buttons::NONE));
    let mut hurt = false;
    for _ in 0..300 {
        let h0 = game
            .net()
            .and_then(|n| n.own_combat())
            .map(|c| c.combat.damage);
        hold(&mut game, Buttons::NONE);
        // Look away to the right so the indicator points left.
        game.controls.yaw -= 0.004;
        h.frame(&mut game);
        let h1 = game
            .net()
            .and_then(|n| n.own_combat())
            .map(|c| c.combat.damage);
        if h1 > h0 {
            hurt = true;
            break;
        }
    }
    h.frames(&mut game, 3, |g, _| hold(g, Buttons::NONE));
    println!("enemy hit us: {hurt}");
    h.shot("combat_hit_by_enemy");
    game.send_chat("/killall");
    game.send_chat("/give");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));

    // Damage + hard damage on the HUD.
    h.frames(&mut game, 10, |g, _| hold(g, Buttons::NONE));
    game.send_chat("/hurt 45");
    h.frames(&mut game, 6, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_hud_damage");
    game.send_chat("/hurt 60");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_low_health");

    // Kindling: walk up to a Mould, punch it and draw its embers into the wick (a stream of
    // sparks into the chest lantern + the heal flash), then its heat release on the kill.
    // Turn away from the wall; space the commands out (the chat flood limit).
    game.controls.yaw += std::f32::consts::PI;
    game.controls.pitch = 0.0;
    h.frames(&mut game, 90, |g, _| hold(g, Buttons::NONE));
    game.send_chat("/god");
    h.frames(&mut game, 90, |g, _| hold(g, Buttons::NONE));
    spawn(&mut game, 1);
    let damage = |g: &InGame| {
        g.net()
            .and_then(|n| n.own_combat())
            .map(|c| c.combat.damage)
    };
    let mut healed = false;
    for _ in 0..400 {
        let before = damage(&game);
        let target = nearest_enemy(&game, false);
        let close = match (target, eye(&game)) {
            (Some(t), Some(me)) => {
                aim_at(&mut game, t);
                t.distance(me) < 2.4
            }
            _ => false,
        };
        game.controls.override_move = Some(if close {
            (0.0, 0.0, Buttons::PUNCH)
        } else {
            (1.0, 0.0, Buttons::NONE)
        });
        h.frame(&mut game);
        if let (Some(b), Some(a)) = (before, damage(&game))
            && a < b
        {
            healed = true;
            break;
        }
    }
    println!("kindling heal seen: {healed}");
    h.frames(&mut game, 2, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_kindling");
    h.frames(&mut game, 4, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_kindling_2");
    game.send_chat("/killall");
    h.frames(&mut game, 30, |g, _| hold(g, Buttons::NONE));
    game.send_chat("/god");

    // Death screen.
    game.send_chat("/kill");
    h.frames(&mut game, 40, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_death");
    h.frames(&mut game, 120, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_respawned");

    game.shutdown(&mut h.renderer, &mut h.common);

    // ---- another player shooting at us (hosted game + a scripted rival client) -------------
    let mut game = InGame::start(
        &StartMode::Host {
            port: Some(0),
            level: Some(shared::sim::BUILTIN_LEVEL.into()),
        },
        &mut h.common,
        &mut h.renderer,
    )?;
    h.frames(&mut game, 120, |g, _| hold(g, Buttons::NONE));
    let addr = game.server_addr().expect("hosting");
    let mut rival = NetClient::connect(addr, ClientConfig::new("Rival"))?;
    let mut rival_ticks = 0u32;
    let mut step_rival = |rival: &mut NetClient, fire: bool| {
        rival.update(Duration::from_secs_f32(DT));
        rival.drain_events();
        rival.drain_fx();
        if !rival.is_in_game() {
            return;
        }
        let me = rival
            .local_state()
            .map(|s| s.eye_position(&rival.tuning().movement));
        let target = rival
            .remote_players()
            .next()
            .map(|e| e.position + Vec3::Y * 1.0);
        let mut inp = PlayerInput {
            weapon: 1,
            right: if (rival_ticks / 50).is_multiple_of(2) {
                100
            } else {
                -100
            },
            ..Default::default()
        };
        if let (Some(me), Some(t)) = (me, target) {
            let d = (t - me).normalize();
            inp.yaw = (-d.x).atan2(-d.z);
            inp.pitch = d.y.clamp(-1.0, 1.0).asin();
        }
        inp.buttons.set(Buttons::FIRE, fire);
        rival.tick(inp);
        rival_ticks += 1;
    };
    for _ in 0..180 {
        step_rival(&mut rival, false);
        if let Some(r) = game.net().and_then(|n| {
            n.remote_players()
                .next()
                .map(|e| e.position + Vec3::Y * 1.2)
        }) {
            aim_at(&mut game, r);
        }
        hold(&mut game, Buttons::NONE);
        h.frame(&mut game);
    }
    println!("rival in game: {}", rival.is_in_game());
    let mut captured = false;
    for i in 0..60 {
        step_rival(&mut rival, i == 0);
        if let Some(r) = game.net().and_then(|n| {
            n.remote_players()
                .next()
                .map(|e| e.position + Vec3::Y * 1.2)
        }) {
            aim_at(&mut game, r);
        }
        hold(&mut game, Buttons::NONE);
        h.frame(&mut game);
        let fired = game
            .net()
            .and_then(|n| n.remote_players().next().map(|e| e.id))
            .and_then(|id| game.remote.visual(id).map(|v| v.firing > 0.6))
            .unwrap_or(false);
        if fired && !captured {
            captured = true;
            h.shot("combat_remote_shot");
        }
    }
    h.shot("combat_remote_after");
    rival.disconnect();

    println!("{} screenshots in {}", h.shots.len(), h.out.display());
    let mut ms = h.frame_ms.clone();
    ms.sort_by(f32::total_cmp);
    let avg = ms.iter().sum::<f32>() / ms.len().max(1) as f32;
    println!(
        "frame + render time over {} frames: avg {avg:.2} ms, p95 {:.2} ms, max {:.2} ms",
        ms.len(),
        ms[(ms.len() as f32 * 0.95) as usize],
        ms.last().copied().unwrap_or(0.0)
    );
    game.shutdown(&mut h.renderer, &mut h.common);
    Ok(())
}
