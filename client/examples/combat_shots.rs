//! Headless combat screenshots: a real single-player session (in-process server) driven by a
//! script — dev commands spawn enemies and needles, the local player aims and fires every weapon
//! mode and throws thorns — and PNGs of the moments worth looking at: Quill tracers, brand
//! sigils, Wicklines, Bellows pellets, gusts and the Bulwark, Arclash arcs / chains / wisps /
//! Discharge novas, Tumbler bombs, Breach and Gravewell, thorn ricochets, deflections and
//! parries, enemies walking / winding up / dying, the HUD with damage and hard damage, hit
//! markers, the kill feed and the death screen; and the feel pass (`feel_*.png`, docs/feel.md):
//! the muzzle flash on the shot's frame, own projectiles leaving the muzzle at once, bigger
//! glowing projectiles, the bow at rest / half / full draw, parryable needles flaring in punch
//! reach, a parry and the deflected needle.
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

/// Feet of the living enemies, nearest first.
fn enemy_feet(game: &InGame) -> Vec<Vec3> {
    let (Some(net), Some(me)) = (game.net(), eye(game)) else {
        return Vec::new();
    };
    let mut v: Vec<Vec3> = net
        .remote_entities()
        .iter()
        .filter(|e| kind::is_enemy(e.kind) && e.flags & shared::protocol::enemy_flags::DYING == 0)
        .map(|e| e.position)
        .collect();
    v.sort_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
    v
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

/// Fires the expansion's weapons and abilities for real: `/loadout` puts them in the slots on
/// the spot, the script aims at the Moulds and fires (`weapon_*.png`).
fn arsenal_weapon_shots(h: &mut Harness, game: &mut InGame) {
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);
    let aimed = |g: &mut InGame, buttons: Buttons| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, buttons);
    };
    game.send_chat("/killall");
    spawn(game, 4);
    h.frames(game, 70, still);
    game.send_chat(
        "/loadout lancet:tally,waxwork thresher:bolas,flak riveter:stoke,lantern cinderbox:limpet,kite",
    );
    h.frames(game, 20, still);

    // Lancet: needles, then a Tally held over the pack and let go.
    select(game, 0, 0);
    h.frames(game, 8, |g, _| aimed(g, Buttons::FIRE));
    h.shot("weapon_lancet");
    h.frames(game, 10, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 40, |g, _| aimed(g, Buttons::ALT_FIRE));
    h.shot("weapon_lancet_tally_held");
    h.frames(game, 3, |g, _| aimed(g, Buttons::NONE));
    h.shot("weapon_lancet_tally");
    h.frames(game, 30, still);

    // Thresher: the drum-fed blast, then a Flak shell bursting in the pack.
    select(game, 1, 0);
    h.frames(game, 12, still);
    h.frames(game, 3, |g, i| {
        aimed(g, if i == 0 { Buttons::FIRE } else { Buttons::NONE })
    });
    h.shot("weapon_thresher");
    select(game, 1, 1);
    h.frames(game, 40, still);
    let mut burst = false;
    for i in 0..30 {
        aimed(
            game,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        );
        h.frame(game);
        if !burst && i > 2 && find_entity(game, |e| e.kind == kind::FLAK_SHELL).is_none() {
            burst = true;
            h.frames(game, 2, still);
            h.shot("weapon_thresher_flak");
        }
    }
    if !burst {
        h.shot("weapon_thresher_flak");
    }

    // Riveter: a stream of hot rivets, then a Lantern tossed.
    game.send_chat("/killall");
    spawn(game, 3);
    h.frames(game, 60, still);
    select(game, 2, 0);
    h.frames(game, 14, |g, _| aimed(g, Buttons::FIRE));
    h.shot("weapon_riveter");
    h.frames(game, 10, still);
    select(game, 2, 1);
    h.frames(game, 20, still);
    h.frames(game, 2, |g, i| {
        aimed(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        )
    });
    h.frames(game, 70, still);
    h.shot("weapon_riveter_lantern");

    // Cinderbox: a salvo, its blasts; limpets thrown, stuck, detonated; a kite.
    select(game, 3, 0);
    h.frames(game, 20, still);
    h.frames(game, 9, |g, i| {
        aimed(g, if i == 0 { Buttons::FIRE } else { Buttons::NONE })
    });
    h.shot("weapon_cinderbox_salvo");
    h.frames(game, 20, still);
    h.shot("weapon_cinderbox_blasts");
    for _ in 0..3 {
        h.frames(game, 2, |g, i| {
            aimed(
                g,
                if i == 0 {
                    Buttons::ALT_FIRE
                } else {
                    Buttons::NONE
                },
            )
        });
        h.frames(game, 30, still);
    }
    h.frames(game, 20, still);
    h.shot("weapon_cinderbox_limpets");
    h.frames(game, 24, |g, _| aimed(g, Buttons::ALT_FIRE));
    h.frames(game, 3, still);
    h.shot("weapon_cinderbox_limpets_detonate");
    h.frames(game, 20, still);
    select(game, 3, 1);
    h.frames(game, 30, still);
    h.frames(game, 2, |g, i| {
        aimed(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        )
    });
    h.frames(game, 18, still);
    h.shot("weapon_cinderbox_kite");
    h.frames(game, 50, still);

    // The Longwick: a full draw, the release; a pinning arrow; the Kiln setting Moulds alight.
    game.send_chat("/killall");
    spawn(game, 4);
    h.frames(game, 60, still);
    game.send_chat("/loadout longwick:pinning,beacon kiln:lantern,discharge quill bellows");
    h.frames(game, 20, still);
    select(game, 0, 0);
    h.frames(game, 10, still);
    h.frames(game, 50, |g, _| aimed(g, Buttons::FIRE));
    h.shot("weapon_longwick_full_draw");
    h.frames(game, 2, |g, _| aimed(g, Buttons::NONE));
    h.shot("weapon_longwick_release");
    h.frames(game, 6, still);
    h.shot("weapon_longwick_arrow");
    h.frames(game, 20, still);
    // Arm the Pinning Arrow (a press), draw, loose.
    h.frames(game, 2, |g, i| {
        aimed(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        )
    });
    h.frames(game, 50, |g, _| aimed(g, Buttons::FIRE));
    h.shot("weapon_longwick_armed");
    h.frames(game, 12, |g, _| aimed(g, Buttons::NONE));
    h.shot("weapon_longwick_pinned");
    h.frames(game, 20, still);
    select(game, 1, 0);
    h.frames(game, 10, still);
    h.frames(game, 40, |g, _| aimed(g, Buttons::FIRE));
    h.shot("weapon_kiln");
    h.frames(game, 20, |g, _| aimed(g, Buttons::NONE));
    h.shot("weapon_kiln_burning");
    game.send_chat("/loadout quill bellows arclash tumbler");
    game.send_chat("/killall");
    h.frames(game, 30, still);
}

/// The feel pass (`docs/feel.md`, `feel_*.png`): the muzzle flash on the frame the shot is
/// predicted, tracers, own rivets / bombs / arrows leaving the muzzle before the server reports
/// them, the bow at rest, half and full draw, a needle seen far off, flaring in punch reach,
/// parried and flying back.
fn feel_shots(h: &mut Harness, game: &mut InGame) {
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);
    let aimed = |g: &mut InGame, buttons: Buttons| {
        if let Some(t) = nearest_enemy(g, false) {
            aim_at(g, t);
        }
        hold(g, buttons);
    };
    game.send_chat("/killall");
    spawn(game, 3);
    h.frames(game, 70, still);
    game.send_chat("/loadout quill riveter:stoke,lantern longwick:blink,pinning tumbler");
    h.frames(game, 20, still);
    let claimed = game.combat.stand_ins.stats.claimed;

    // The Quill: the flash on the shot's own frame, the tracer a few frames on.
    select(game, 0, 0);
    h.frames(game, 12, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 1, |g, _| aimed(g, Buttons::FIRE));
    h.shot("feel_muzzle_flash");
    h.frames(game, 4, |g, _| aimed(g, Buttons::NONE));
    h.shot("feel_tracer");
    h.frames(game, 25, still);

    // The Riveter: rivets streaming from the muzzle; stoked.
    select(game, 1, 0);
    h.frames(game, 12, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 7, |g, _| aimed(g, Buttons::FIRE));
    h.shot("feel_rivets");
    h.frames(game, 20, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 1, |g, _| aimed(g, Buttons::ALT_FIRE));
    h.frames(game, 3, still);
    h.shot("feel_stoke");
    h.frames(game, 40, still);

    // The Tumbler: a bomb out of the barrel, glowing as it flies.
    select(game, 3, 0);
    h.frames(game, 12, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 1, |g, _| aimed(g, Buttons::FIRE));
    h.frames(game, 7, still);
    h.shot("feel_bomb");
    h.frames(game, 60, still);

    // The Longwick: at rest, half drawn, full draw (zoomed, the ring), the arrow flying.
    select(game, 2, 0);
    h.frames(game, 20, |g, _| aimed(g, Buttons::NONE));
    h.shot("feel_bow_rest");
    h.frames(game, 18, |g, _| aimed(g, Buttons::FIRE));
    h.shot("feel_bow_half");
    h.frames(game, 22, |g, _| aimed(g, Buttons::FIRE));
    h.shot("feel_bow_full_cue");
    h.frames(game, 20, |g, _| aimed(g, Buttons::FIRE));
    h.shot("feel_bow_full");
    h.frames(game, 1, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 4, still);
    h.shot("feel_bow_arrow");
    h.frames(game, 30, still);
    // The Blink Arrow: armed (a press), drawn, loosed.
    h.frames(game, 2, |g, i| {
        aimed(
            g,
            if i == 0 {
                Buttons::ALT_FIRE
            } else {
                Buttons::NONE
            },
        )
    });
    h.frames(game, 45, |g, _| aimed(g, Buttons::FIRE));
    h.frames(game, 1, |g, _| aimed(g, Buttons::NONE));
    h.frames(game, 3, still);
    h.shot("feel_blink_arrow");
    h.frames(game, 40, still);
    let stats = game.combat.stand_ins.stats;
    println!(
        "stand-ins: {stats:?} ({} handed over in this pass; own projectiles appeared {:.0} ms \
         after the shot on average, drawn from the muzzle meanwhile)",
        stats.claimed - claimed,
        stats.mean_wait() * 1000.0
    );

    // A needle: far off, then in punch reach (its ring flares), parried, flying back.
    game.send_chat("/killall");
    select(game, 0, 0);
    h.frames(game, 30, still);
    game.controls.pitch = 0.0;
    let enemy_shot = |e: &shared::protocol::EntityState| {
        kind::is_projectile(e.kind)
            && e.flags & projectile_flags::ENEMY != 0
            && e.flags & projectile_flags::PARRIED == 0
    };
    game.send_chat("/orb 18");
    let (mut far, mut reach, mut punched) = (false, false, false);
    for _ in 0..150 {
        let me = eye(game).unwrap_or(Vec3::ZERO);
        let orb = find_entity(game, enemy_shot);
        if let Some(p) = orb {
            aim_at(game, p);
        }
        let d = orb.map(|p| p.distance(me));
        let b = if !punched && d.is_some_and(|d| d < 2.2) {
            punched = true;
            Buttons::PUNCH
        } else {
            Buttons::NONE
        };
        hold(game, b);
        h.frame(game);
        if !far && d.is_some_and(|d| d < 11.0) {
            far = true;
            h.shot("feel_needle_incoming");
        }
        if !reach && d.is_some_and(|d| d < 3.2) {
            reach = true;
            h.shot("feel_needle_in_reach");
        }
        if punched {
            h.shot("feel_parry");
            break;
        }
    }
    h.frames(game, 12, still);
    h.shot("feel_parry_deflected");
    h.frames(game, 30, still);
    game.send_chat("/loadout quill bellows arclash tumbler");
    h.frames(game, 20, still);
}

/// The expansion arsenal's projectiles, blasts, burning patch and status effects, staged
/// client-side with `InGame::preview_arsenal` (`arsenal_*.png`).
fn arsenal_previews(h: &mut Harness, game: &mut InGame) {
    use client::ingame::combat::arsenal::{
        ArsenalPreview as P, ArsenalProjectile as K, Blast, Status,
    };
    game.send_chat("/killall");
    spawn(game, 3);
    h.frames(game, 70, |g, _| hold(g, Buttons::NONE));
    if let Some(t) = nearest_enemy(game, false) {
        aim_at(game, t - Vec3::Y * 0.6);
    }
    let Some(eye) = eye(game) else {
        return;
    };
    let fwd = shared::input::look_dir(game.controls.yaw, 0.0);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let launch = |side: f32| eye + fwd * 0.8 + right * side - Vec3::Y * 0.25;
    let fire = |game: &mut InGame, kind, side: f32, speed: f32, up: f32, gravity: f32| {
        game.preview_arsenal(P::Projectile {
            kind,
            from: launch(side),
            velocity: fwd * speed + Vec3::Y * up + right * side * 2.0,
            gravity,
            life: 4.0,
        });
    };
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);

    // Arrows in flight (taper, pinning, beacon), then stuck.
    fire(game, K::Arrow, -0.4, 45.0, 1.5, 6.0);
    fire(game, K::PinningArrow, 0.0, 45.0, 2.5, 6.0);
    fire(game, K::BeaconArrow, 0.4, 45.0, 3.5, 6.0);
    h.frames(game, 4, still);
    h.shot("arsenal_arrows");
    h.frames(game, 40, still);
    h.shot("arsenal_arrows_landed");

    // Rivets streaking, a cinder salvo (three rockets 0.08 s apart).
    for k in 0..5 {
        fire(
            game,
            K::Rivet,
            -0.2 + k as f32 * 0.1,
            90.0,
            1.0 + k as f32 * 0.3,
            4.0,
        );
    }
    for k in 0..3 {
        fire(
            game,
            K::CinderRocket,
            (k as f32 - 1.0) * 0.15,
            32.0,
            0.5,
            0.0,
        );
        h.frames(game, 5, still);
    }
    h.shot("arsenal_rivets_cinders");
    h.frames(game, 30, still);
    h.shot("arsenal_cinder_blasts");
    h.frames(game, 40, still);

    // A flak shell bursting into its ring of pellets.
    fire(game, K::FlakShell, 0.0, 28.0, 2.0, 2.0);
    h.frames(game, 8, still);
    h.shot("arsenal_flak_shell");
    game.preview_arsenal(P::FlakBurst {
        position: eye + fwd * 9.0 + Vec3::Y * 0.8,
        dir: fwd,
    });
    h.frames(game, 3, still);
    h.shot("arsenal_flak_burst");
    h.frames(game, 30, still);

    // Limpets thrown onto the floor ahead, blinking, then detonated.
    for k in 0..3 {
        fire(game, K::Limpet, (k as f32 - 1.0) * 0.5, 9.0, 3.0, 18.0);
    }
    h.frames(game, 70, still);
    h.shot("arsenal_limpets");
    let ground = launch(0.0) + fwd * 6.0 - Vec3::Y * 1.3;
    game.preview_arsenal(P::Blast {
        blast: Blast::Limpet,
        position: ground,
        radius: 3.5,
    });
    h.frames(game, 3, still);
    h.shot("arsenal_limpet_blast");
    h.frames(game, 40, still);

    // A kite (placeholder look) and its blast.
    fire(game, K::Kite, 0.0, 14.0, 9.0, 20.0);
    h.frames(game, 20, still);
    h.shot("arsenal_kite");
    h.frames(game, 60, |g, _| hold(g, Buttons::NONE));
    game.preview_arsenal(P::Blast {
        blast: Blast::Kite,
        position: ground + fwd * 4.0,
        radius: 4.5,
    });
    h.frames(game, 4, still);
    h.shot("arsenal_kite_blast");
    h.frames(game, 60, still);

    // A bolas (placeholder look) thrown ahead; statuses on the Moulds.
    fire(game, K::Bolas, 0.0, 10.0, 4.0, 8.0);
    h.frames(game, 12, still);
    h.shot("arsenal_bolas");
    let moulds: Vec<_> = game
        .net()
        .map(|n| {
            n.remote_entities()
                .iter()
                .filter(|e| kind::is_enemy(e.kind))
                .map(|e| e.id)
                .collect()
        })
        .unwrap_or_default();
    for (i, id) in moulds.iter().enumerate() {
        game.preview_arsenal(P::Status {
            entity: *id,
            status: if i % 2 == 0 {
                Status::Burning
            } else {
                Status::Pinned
            },
            seconds: 3.0,
        });
    }
    h.frames(game, 50, still);
    h.shot("arsenal_status");
    h.frames(game, 200, still);
}

/// Walks up to the nearest Mould until it is within `reach` (for melee), aiming at it.
fn close_in(h: &mut Harness, game: &mut InGame, reach: f32) {
    for _ in 0..240 {
        let close = match (nearest_enemy(game, false), eye(game)) {
            (Some(t), Some(me)) => {
                aim_at(game, t);
                Vec3::new(t.x - me.x, 0.0, t.z - me.z).length() < reach
            }
            _ => true,
        };
        if close {
            break;
        }
        game.controls.override_move = Some((1.0, 0.0, Buttons::NONE));
        h.frame(game);
    }
    hold(game, Buttons::NONE);
}

/// The melee weapons and knacks for real (`/loadout trimmer tamper knack:...`): the
/// viewmodel mid-swing with its trail, a hit, the abilities, each knack used, the Q slot
/// (`melee_weapon_*.png`, `knack_use_*.png`).
fn melee_weapon_shots(h: &mut Harness, game: &mut InGame) {
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);
    let press = |h: &mut Harness, game: &mut InGame, b: Buttons, after: usize, name: &str| {
        if let Some(t) = nearest_enemy(game, false) {
            aim_at(game, t);
        }
        hold(game, b);
        h.frame(game);
        for _ in 0..after {
            hold(game, Buttons::NONE);
            h.frame(game);
        }
        h.shot(name);
    };
    game.send_chat("/killall");
    game.send_chat("/loadout trimmer:lunge,hurl tamper:whirl,riposte knack:kindle");
    spawn(game, 3);
    h.frames(game, 70, still);
    close_in(h, game, 2.6);

    // The Trimmer: right blade, then left; the HUD's Q slot shows the flask.
    select(game, 0, 0);
    h.frames(game, 20, still);
    h.shot("melee_weapon_trimmer_rest");
    press(h, game, Buttons::FIRE, 3, "melee_weapon_trimmer_slash");
    h.frames(game, 18, still);
    press(h, game, Buttons::FIRE, 3, "melee_weapon_trimmer_slash_left");
    h.frames(game, 12, still);
    h.shot("melee_weapon_trimmer_hit");
    h.frames(game, 30, still);
    // Lunge, then Hurl.
    press(h, game, Buttons::ALT_FIRE, 6, "melee_weapon_trimmer_lunge");
    h.frames(game, 40, still);
    select(game, 0, 1);
    h.frames(game, 20, still);
    press(h, game, Buttons::ALT_FIRE, 8, "melee_weapon_trimmer_hurl");
    h.frames(game, 40, still);

    // The Tamper: the slam, a Whirl, a Riposte's guard.
    game.send_chat("/give");
    spawn(game, 2);
    h.frames(game, 40, still);
    close_in(h, game, 2.8);
    select(game, 1, 0);
    h.frames(game, 20, still);
    h.shot("melee_weapon_tamper_rest");
    press(h, game, Buttons::FIRE, 5, "melee_weapon_tamper_slam");
    h.frames(game, 50, still);
    press(h, game, Buttons::ALT_FIRE, 8, "melee_weapon_tamper_whirl");
    h.frames(game, 40, still);
    select(game, 1, 1);
    h.frames(game, 20, still);
    press(
        h,
        game,
        Buttons::ALT_FIRE,
        10,
        "melee_weapon_tamper_riposte",
    );
    h.frames(game, 40, still);

    // The knacks, used for real.
    for (knack, name, after, settle) in [
        ("kindle", "knack_use_drink", 18, 30),
        ("snare", "knack_use_snare", 4, 60),
        ("soot", "knack_use_soot", 40, 60),
        ("tallow", "knack_use_tallow", 20, 200),
        ("surge", "knack_use_surge", 4, 40),
    ] {
        game.send_chat(&format!("/loadout knack:{knack}"));
        game.send_chat("/give");
        h.frames(game, 20, still);
        if let Some(t) = nearest_enemy(game, false) {
            aim_at(game, t - Vec3::Y * 0.8);
        }
        hold(game, Buttons::THROW);
        h.frame(game);
        h.frames(game, after, still);
        h.shot(name);
        h.frames(game, settle, still);
        if knack == "kindle" {
            h.shot("knack_use_haste");
        }
        if knack == "snare" {
            h.shot("knack_use_snare_armed");
        }
    }
    game.send_chat("/loadout quill bellows arclash tumbler knack:thorn");
    game.send_chat("/killall");
    h.frames(game, 30, still);
}

/// Melee and knacks (docs/refit.md), staged client-side with `InGame::preview_melee` /
/// `preview_knack` (`melee_*.png`, `knack_*.png`): slash trails, hits, the abilities' streak /
/// ring / hurled pieces / launch, the flask and its pool, the snare arming and springing, the
/// soot cloud hiding the Moulds, Tallow Skin's sheen and the Surge's lines. (The Kindle Flask
/// is `kindle_previews`.)
fn refit_previews(h: &mut Harness, game: &mut InGame) {
    use client::ingame::combat::knacks::KnackPreview as K;
    use client::ingame::combat::melee::{MeleePreview as M, MeleeWeapon, SwingKind};
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);
    game.send_chat("/killall");
    spawn(game, 3);
    h.frames(game, 70, still);
    if let Some(t) = nearest_enemy(game, false) {
        aim_at(game, t - Vec3::Y * 0.4);
    }
    let Some(eye) = eye(game) else {
        return;
    };
    let (yaw, pitch) = (game.controls.yaw, game.controls.pitch);
    let fwd = shared::input::look_dir(yaw, 0.0);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let floor = eye - Vec3::Y * 1.6;
    let swing = |g: &mut InGame, weapon, kind, left| {
        g.preview_melee(M::Swing {
            weapon,
            kind,
            left,
            eye,
            yaw,
            pitch,
        })
    };

    // Slash trails: the Trimmer's right then left blade, the Tamper's slam.
    swing(game, MeleeWeapon::Trimmer, SwingKind::Slash, false);
    h.frames(game, 4, still);
    h.shot("melee_trimmer_slash");
    h.frames(game, 16, still);
    swing(game, MeleeWeapon::Trimmer, SwingKind::Slash, true);
    h.frames(game, 6, still);
    h.shot("melee_trimmer_slash_left");
    h.frames(game, 20, still);
    swing(game, MeleeWeapon::Tamper, SwingKind::Slash, false);
    h.frames(game, 9, still);
    h.shot("melee_tamper_slam");
    game.preview_melee(M::Hit {
        weapon: MeleeWeapon::Tamper,
        at: floor + fwd * 2.4 + Vec3::Y * 0.1,
        dir: -Vec3::Y,
    });
    h.frames(game, 3, still);
    h.shot("melee_tamper_hit");
    h.frames(game, 30, still);

    // Hits on the Moulds.
    if let Some(t) = nearest_enemy(game, false) {
        game.preview_melee(M::Hit {
            weapon: MeleeWeapon::Trimmer,
            at: t,
            dir: fwd,
        });
    }
    h.frames(game, 3, still);
    h.shot("melee_trimmer_hit");
    h.frames(game, 30, still);

    // The abilities: a Lunge streak across the view, the Whirl round a spot ahead, hurled
    // pieces, the Upswing under a Mould.
    game.preview_melee(M::Lunge {
        from: floor + fwd * 5.0 + right * 4.0,
        velocity: -right * 16.0,
        seconds: 0.4,
    });
    h.frames(game, 18, still);
    h.shot("melee_lunge");
    h.frames(game, 20, still);
    game.preview_melee(M::Whirl {
        feet: floor + fwd * 5.0,
        radius: 3.5,
    });
    swing(game, MeleeWeapon::Trimmer, SwingKind::Whirl, false);
    h.frames(game, 12, still);
    h.shot("melee_whirl");
    h.frames(game, 30, still);
    for (weapon, side) in [(MeleeWeapon::Trimmer, -0.5), (MeleeWeapon::Tamper, 0.5)] {
        game.preview_melee(M::Hurl {
            weapon,
            from: eye + fwd * 0.8 + right * side - Vec3::Y * 0.2,
            velocity: fwd * 30.0 + Vec3::Y * 1.5,
        });
    }
    h.frames(game, 5, still);
    h.shot("melee_hurl");
    h.frames(game, 40, still);
    if let Some(t) = nearest_enemy(game, false) {
        game.preview_melee(M::Upswing {
            feet: t - Vec3::Y * 0.95,
        });
        swing(game, MeleeWeapon::Tamper, SwingKind::Upswing, false);
    }
    h.frames(game, 10, still);
    h.shot("melee_upswing");
    h.frames(game, 40, still);

    // A snare thrown down, arming, armed, then sprung.
    game.preview_knack(K::Snare {
        position: floor + fwd * 3.5,
        arm: 0.6,
        spring: Some(2.0),
    });
    h.frames(game, 10, still);
    h.shot("knack_snare_arming");
    h.frames(game, 50, still);
    h.shot("knack_snare_armed");
    h.frames(game, 64, still);
    h.shot("knack_snare_sprung");
    h.frames(game, 30, still);

    // A soot pot bursting between us and the Moulds: they disappear in it.
    h.shot("knack_soot_before");
    let target = nearest_enemy(game, false).unwrap_or(eye + fwd * 10.0);
    let mid = eye + (target - eye) * 0.45;
    game.preview_knack(K::Cloud {
        position: Vec3::new(mid.x, floor.y, mid.z),
        radius: 5.0,
        seconds: 7.0,
    });
    h.frames(game, 70, still);
    h.shot("knack_soot_cloud");
    h.frames(game, 330, still);
    h.shot("knack_soot_gone");

    // Tallow Skin on you, then a Surge.
    game.preview_knack(K::TallowSelf { seconds: 3.0 });
    h.frames(game, 40, still);
    h.shot("knack_tallow_skin");
    h.frames(game, 160, still);
    game.preview_knack(K::SurgeSelf { seconds: 0.35 });
    game.preview_melee(M::Lunge {
        from: floor + fwd * 6.0 - right * 3.0,
        velocity: right * 18.0,
        seconds: 0.35,
    });
    h.frames(game, 6, still);
    h.shot("knack_surge");
    h.frames(game, 30, still);
}

/// The Kindle Flask drunk (docs/rework.md §3), staged client-side with `InGame::preview_kindle`
/// (`kindle_*.png`): the drink in first person, your haste's embers and edges while running,
/// a heal tick, and another hasted Shrike's trail and wick embers running across the view.
fn kindle_previews(h: &mut Harness, game: &mut InGame) {
    use client::ingame::combat::kindle::KindlePreview as K;
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);
    game.send_chat("/killall");
    h.frames(game, 10, still);
    let Some(eye) = eye(game) else {
        return;
    };
    let fwd = shared::input::look_dir(game.controls.yaw, 0.0);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let floor = eye - Vec3::Y * 1.6;
    game.preview_kindle(K::Drink { seconds: 5.0 });
    for (frames, name) in [
        (5, "kindle_drink_raise"),
        (8, "kindle_drink_tip"),
        (10, "kindle_drink_away"),
    ] {
        h.frames(game, frames, still);
        h.shot(name);
    }
    // Hasted: running forward (embers past the view, the edges streaming).
    game.controls.override_move = Some((1.0, 0.0, Buttons::NONE));
    h.frames(game, 40, |_, _| {});
    h.shot("kindle_haste_run");
    game.preview_kindle(K::Tick { amount: 8.0 });
    h.frames(game, 6, |_, _| {});
    h.shot("kindle_heal_tick");
    hold(game, Buttons::NONE);
    h.frames(game, 200, still);
    // Another hasted Shrike running across the view.
    let Some(eye) = self::eye(game) else {
        return;
    };
    let floor = Vec3::new(eye.x, floor.y.min(eye.y - 1.6), eye.z);
    game.preview_kindle(K::Runner {
        from: floor + fwd * 7.0 - right * 6.0,
        velocity: right * 14.0,
        seconds: 1.2,
    });
    h.frames(game, 26, still);
    h.shot("kindle_haste_other");
    game.preview_kindle(K::Runner {
        from: floor + fwd * 9.0 + right * 0.5,
        velocity: -fwd * 3.0,
        seconds: 2.0,
    });
    h.frames(game, 40, still);
    h.shot("kindle_haste_other_slow");
    h.frames(game, 90, still);
}

/// The rework's Scatter / Rapid / Explosive abilities and the Thorn relays (docs/rework.md),
/// staged client-side with `InGame::preview_gear` (`gear_*.png`, `relay_*.png`).
fn gear_previews(h: &mut Harness, game: &mut InGame) {
    use client::ingame::combat::gear::GearPreview as G;
    use shared::loadout::WeaponKind;
    use shared::sim::Attack;
    let still = |g: &mut InGame, _| hold(g, Buttons::NONE);
    game.send_chat("/killall");
    spawn(game, 3);
    h.frames(game, 90, still);
    if let Some(t) = nearest_enemy(game, false) {
        aim_at(game, t - Vec3::Y * 0.3);
    }
    let Some(eye) = eye(game) else {
        return;
    };
    let fwd = shared::input::look_dir(game.controls.yaw, 0.0);
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let floor = Vec3::new(eye.x, eye.y - 1.6, eye.z);
    let ahead = |d: f32, side: f32| floor + fwd * d + right * side;

    // Bolas: thrown at the nearest Mould, the cord snapping on to the next one.
    let moulds = enemy_feet(game);
    let first = moulds.first().copied().unwrap_or(ahead(9.0, -1.2));
    let second = moulds.get(1).copied().unwrap_or(ahead(10.0, 2.4));
    game.preview_gear(G::Bolas {
        from: eye + fwd * 0.6 + right * 0.2 - Vec3::Y * 0.2,
        to: first,
        height: 2.0,
        link: Some(second),
        seconds: 3.0,
    });
    h.frames(game, 9, still);
    h.shot("gear_bolas_flight");
    h.frames(game, 16, still);
    h.shot("gear_bolas_bound");
    h.frames(game, 40, still);
    h.shot("gear_bolas_link");
    h.frames(game, 160, still);

    // Bulwark: yours, taking blows and cracking, then shattering.
    game.preview_gear(G::Bulwark {
        holder: None,
        seconds: 3.0,
        hits: 6,
        shatter: true,
    });
    h.frames(game, 12, still);
    h.shot("gear_bulwark_raised");
    h.frames(game, 100, still);
    h.shot("gear_bulwark_cracked");
    h.frames(game, 46, still);
    h.shot("gear_bulwark_shatter");
    h.frames(game, 60, still);
    // Another Shrike's, as its opponents see it.
    game.preview_gear(G::Bulwark {
        holder: Some((ahead(5.0, 0.5), game.controls.yaw + std::f32::consts::PI)),
        seconds: 2.5,
        hits: 3,
        shatter: false,
    });
    h.frames(game, 100, still);
    h.shot("gear_bulwark_other");
    h.frames(game, 80, still);

    // The wisp orbiting you, then flung; round another spot ahead.
    game.preview_gear(G::Wisp {
        around: None,
        seconds: 2.0,
        fling: Some(shared::input::look_dir(
            game.controls.yaw,
            game.controls.pitch,
        )),
    });
    game.preview_gear(G::Wisp {
        around: Some(ahead(6.0, -3.0)),
        seconds: 3.0,
        fling: None,
    });
    for (frames, name) in [
        (30, "gear_wisp_orbit"),
        (24, "gear_wisp_orbit_2"),
        (70, "gear_wisp_flung"),
    ] {
        h.frames(game, frames, still);
        h.shot(name);
    }
    h.frames(game, 100, still);

    // Stoke: your weapon (the Arclash) stoked (early, late), then venting.
    select(game, 2, 0);
    game.preview_gear(G::Stoke {
        stoke: 4.0,
        vent: 1.5,
    });
    h.frames(game, 20, still);
    h.shot("gear_stoke_early");
    h.frames(game, 200, still);
    h.shot("gear_stoke_late");
    h.frames(game, 30, still);
    h.shot("gear_stoke_vent");
    h.frames(game, 80, still);

    // Lanterns: an Arclash's, a Riveter's and a Kiln's, firing at a body ahead.
    let target = nearest_enemy(game, false).unwrap_or(ahead(12.0, 0.0) + Vec3::Y);
    for (k, carrier) in [WeaponKind::Arclash, WeaponKind::Riveter, WeaponKind::Kiln]
        .into_iter()
        .enumerate()
    {
        game.preview_gear(G::Lantern {
            from: eye + fwd * 0.6 - Vec3::Y * 0.3,
            to: ahead(3.5 + k as f32, -2.5 + 2.5 * k as f32),
            carrier,
            target,
            seconds: 3.0,
        });
    }
    h.frames(game, 40, still);
    h.shot("gear_lantern_set");
    h.frames(game, 50, still);
    h.shot("gear_lantern_firing");
    h.frames(game, 125, still);
    h.shot("gear_lantern_out");
    h.frames(game, 40, still);

    // A kite steered round to the left, then bursting.
    game.preview_gear(G::Kite {
        from: eye + fwd * 0.8 - Vec3::Y * 0.2,
        velocity: (fwd + Vec3::Y * 0.15) * 16.0,
        steer: ahead(9.0, -4.0) + Vec3::Y * 1.5,
        steer_for: 1.6,
        seconds: 2.2,
    });
    h.frames(game, 40, still);
    h.shot("gear_kite_steer");
    h.frames(game, 50, still);
    h.shot("gear_kite_free");
    h.frames(game, 60, still);

    // Thorn relays: a Bellows pellet ray, a rivet turning, a Trimmer slash batting it, a Kiln
    // cone; each thorn ahead, relayed at a body to the side.
    for (k, (attack, name)) in [
        (Attack::Blast, "relay_ray"),
        (Attack::Riveter, "relay_turn"),
        (Attack::Trimmer, "relay_bat"),
        (Attack::Kiln, "relay_blast"),
    ]
    .into_iter()
    .enumerate()
    {
        let thorn = ahead(4.5, -3.2 + k as f32 * 0.3) + Vec3::Y * 1.2;
        game.preview_gear(G::Relay {
            attack,
            turned: attack == Attack::Riveter,
            from: eye - Vec3::Y * 0.2 + right * 0.2,
            thorn,
            to: target,
        });
        h.frames(game, 5, still);
        h.shot(name);
        h.frames(game, 40, still);
    }
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

    // The rework (docs/rework.md); `COMBAT_SHOTS=rework` stops after them (quick look).
    kindle_previews(&mut h, &mut game);
    gear_previews(&mut h, &mut game);
    if std::env::var("COMBAT_SHOTS").is_ok_and(|v| v == "rework") {
        println!("{} screenshots in {}", h.shots.len(), h.out.display());
        game.shutdown(&mut h.renderer, &mut h.common);
        return Ok(());
    }

    // Melee and knacks; `COMBAT_SHOTS=refit` stops after them (quick look).
    refit_previews(&mut h, &mut game);
    melee_weapon_shots(&mut h, &mut game);
    if std::env::var("COMBAT_SHOTS").is_ok_and(|v| v == "refit") {
        println!("{} screenshots in {}", h.shots.len(), h.out.display());
        game.shutdown(&mut h.renderer, &mut h.common);
        return Ok(());
    }

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

    // Wickline: the thread to the target.
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
    h.shot("combat_wickline");
    h.frames(&mut game, 5, |g, _| hold(g, Buttons::NONE));
    h.shot("combat_wickline_after");

    // Bellows: pellets, then a Gust, then the Bulwark raised.
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
    h.shot("combat_bulwark");
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

    arsenal_previews(&mut h, &mut game);
    arsenal_weapon_shots(&mut h, &mut game);
    feel_shots(&mut h, &mut game);

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
