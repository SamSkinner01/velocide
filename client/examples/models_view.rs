//! Headless renders of every model for visual checks at the game's 640x360 internal
//! resolution (saved 2x nearest-upscaled as 1280x720 PNGs):
//!
//! - `lineup.png`: every character side by side (scale check);
//! - `closeup_*.png`: each enemy from the front three-quarter and from behind;
//! - `lineup_third.png`: the third roster (Bellwether, Snuffer, Smelter, Effigy) beside a
//!   Shrike and a Gaoler for scale;
//! - `shrike_*.png`, `gnawer_*.png`, `censer_*.png`, `harrier_*.png`, `gaoler_*.png`,
//!   `heron_*.png`, `bellwether_*.png`, `snuffer_*.png`, `smelter_*.png`, `effigy_*.png`:
//!   animation states, every attack's wind-up and strike;
//! - `vm_*.png`: first-person viewmodels (rest, kick, alt modes, crest fold, bellows pump,
//!   discharge charge, drum turn, switch, punch, thorn throw...);
//! - `weapons.png`, `weapons_34.png`: the four guns at real scale (side, three-quarter front);
//! - `projectiles.png`, `thorns.png` (planted in a wall, lying, in flight), `props.png`.
//!
//! ```text
//! cargo run -p client --example models_view -- [--out DIR] [--only PREFIX]
//! ```

use std::path::{Path, PathBuf};

use client::models::{
    DrumTurn, EnemyAnim, EnemyKind, EnemyState, ModelId, ModelLibrary, PlayerAnim, Tint,
    ViewmodelAnim, projectiles, props,
};
use engine::assets::meshgen::{self, BoxDesc, BoxUv, MeshBuilder};
use engine::assets::procedural;
use engine::gfx::{
    Camera, DrawItem, Fog, FrameDesc, MeshId, PointLight, RetroSettings, TextureId, UiDrawList,
    UiQuad, UiTexture,
};
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use engine::render::Renderer;
use engine::ui::font;

const W: u32 = 1280;
const H: u32 = 720;

struct Stage {
    floor: MeshId,
    wall: MeshId,
    concrete: TextureId,
    brick: TextureId,
}

struct Shot {
    camera: Camera,
    draws: Vec<DrawItem>,
    viewmodel: Vec<DrawItem>,
    labels: Vec<(Vec3, String)>,
    lights: Vec<PointLight>,
}

impl Shot {
    fn new(camera: Camera) -> Self {
        Self {
            camera,
            draws: Vec::new(),
            viewmodel: Vec::new(),
            labels: Vec::new(),
            lights: Vec::new(),
        }
    }
}

fn look_at(position: Vec3, target: Vec3, fov: f32) -> Camera {
    let dir = (target - position).normalize();
    let yaw = (-dir.x).atan2(-dir.z);
    let pitch = dir.y.asin();
    Camera {
        position,
        rotation: Camera::rotation_from_angles(yaw, pitch, 0.0),
        fov_y_degrees: fov,
        near: 0.05,
        far: 200.0,
    }
}

fn text(ui: &mut UiDrawList, pos: Vec2, color: Vec4, s: &str) {
    let width = s.len() as f32 * font::GLYPH_ADVANCE;
    let mut x = pos.x - width * 0.5;
    ui.rect(
        Vec2::new(x - 2.0, pos.y - 1.0),
        Vec2::new(x + width + 1.0, pos.y + font::GLYPH_H + 1.0),
        Vec4::new(0.0, 0.0, 0.0, 0.6),
    );
    for c in s.chars() {
        if c != ' ' {
            let (uv_min, uv_max) = font::glyph_uv(c);
            ui.quads.push(UiQuad {
                min: Vec2::new(x, pos.y),
                max: Vec2::new(x + font::GLYPH_W, pos.y + font::GLYPH_H),
                uv_min,
                uv_max,
                color,
                texture: UiTexture::Font,
            });
        }
        x += font::GLYPH_ADVANCE;
    }
}

fn render(
    r: &mut Renderer,
    stage: &Stage,
    shot: &Shot,
    path: &Path,
    title: &str,
) -> anyhow::Result<()> {
    let mut draws = vec![
        DrawItem {
            mesh: stage.floor,
            texture: stage.concrete,
            transform: Mat4::IDENTITY,
            tint: Vec4::ONE,
            unlit: false,
        },
        DrawItem {
            mesh: stage.wall,
            texture: stage.brick,
            transform: Mat4::IDENTITY,
            tint: Vec4::ONE,
            unlit: false,
        },
    ];
    draws.extend_from_slice(&shot.draws);
    let canvas = UiDrawList::canvas_for_window(W, H);
    let mut ui = UiDrawList::new(canvas);
    text(
        &mut ui,
        Vec2::new(canvas.x * 0.5, 4.0),
        Vec4::new(1.0, 0.85, 0.3, 1.0),
        title,
    );
    let cam = &shot.camera;
    let view = cam.view_matrix();
    let tan = (cam.fov_y_degrees.to_radians() * 0.5).tan();
    let aspect = W as f32 / H as f32;
    for (p, label) in &shot.labels {
        let v = view.transform_point3(*p);
        if v.z >= -0.01 {
            continue;
        }
        let ndc = Vec2::new(v.x / -v.z / (tan * aspect), v.y / -v.z / tan);
        let pos = Vec2::new(
            (ndc.x * 0.5 + 0.5) * canvas.x,
            (0.5 - ndc.y * 0.5) * canvas.y,
        );
        text(&mut ui, pos, Vec4::ONE, label);
    }
    let fog = Fog {
        color: Vec3::new(0.2, 0.07, 0.06),
        start: 18.0,
        end: 70.0,
    };
    let mut lights = vec![PointLight {
        position: cam.position + Vec3::new(0.0, 2.0, 0.0),
        color: Vec3::new(0.35, 0.3, 0.28),
        radius: 14.0,
    }];
    lights.extend_from_slice(&shot.lights);
    let frame = FrameDesc {
        camera: shot.camera,
        clear_color: fog.color,
        fog,
        ambient: Vec3::new(0.3, 0.27, 0.32),
        light_dir: Vec3::new(0.4, -1.0, -0.3),
        light_color: Vec3::new(0.6, 0.55, 0.5),
        lights: &lights,
        draws: &draws,
        sprites: &[],
        decals: &[],
        viewmodel: &shot.viewmodel,
        viewmodel_fov_y_degrees: 60.0,
        debug_lines: &[],
        screen_flash: Vec4::ZERO,
        ui: &ui,
    };
    r.capture_png(path);
    r.render(&frame, None)?;
    Ok(())
}

fn player(t: f64) -> PlayerAnim {
    PlayerAnim {
        on_ground: true,
        time: t,
        ..Default::default()
    }
}

/// Places `n` models in a row along X, facing the camera (+Z), returning their root
/// positions.
fn row(n: usize, spacing: f32) -> Vec<Vec3> {
    (0..n)
        .map(|i| Vec3::new((i as f32 - (n as f32 - 1.0) * 0.5) * spacing, 0.0, 0.0))
        .collect()
}

/// Facing the camera: the models face -Z, so turn them around.
const FACE_CAMERA: f32 = std::f32::consts::PI - 0.45;
/// Profile view (facing screen-left, turned slightly towards the camera).
const FACE_SIDE: f32 = std::f32::consts::FRAC_PI_2 + 0.35;

fn shrike_shots(lib: &ModelLibrary) -> Vec<(String, String, Shot)> {
    let t = 3.3;
    let run = |cycle: f32| PlayerAnim {
        velocity: Quat::from_rotation_y(FACE_SIDE) * Vec3::new(0.0, 0.0, -11.0),
        cycle,
        ..player(t)
    };
    let states: Vec<(&str, PlayerAnim)> = vec![
        ("IDLE", player(t)),
        ("RUN A", run(0.0)),
        ("RUN B", run(0.25)),
        ("RUN C", run(0.5)),
        (
            "STRAFE",
            PlayerAnim {
                velocity: Quat::from_rotation_y(FACE_SIDE) * Vec3::new(-9.0, 0.0, 0.0),
                cycle: 0.2,
                ..player(t)
            },
        ),
        (
            "AIM UP",
            PlayerAnim {
                pitch: 0.8,
                weapon: 3,
                ..player(t)
            },
        ),
        (
            "AIM DOWN",
            PlayerAnim {
                pitch: -0.7,
                weapon: 1,
                ..player(t)
            },
        ),
        (
            "FIRE",
            PlayerAnim {
                firing: 1.0,
                weapon: 2,
                ..player(t)
            },
        ),
        (
            "PUNCH",
            PlayerAnim {
                punching: 0.25,
                ..player(t)
            },
        ),
        (
            "RISE",
            PlayerAnim {
                on_ground: false,
                velocity: Vec3::new(0.0, 7.0, 0.0),
                ..player(t)
            },
        ),
        (
            "FALL",
            PlayerAnim {
                on_ground: false,
                velocity: Vec3::new(0.0, -9.0, 0.0),
                weapon: 1,
                ..player(t)
            },
        ),
        (
            "SLIDE",
            PlayerAnim {
                sliding: true,
                velocity: Quat::from_rotation_y(FACE_SIDE) * Vec3::new(0.0, 0.0, -14.0),
                weapon: 1,
                ..player(t)
            },
        ),
        (
            "DASH",
            PlayerAnim {
                dashing: true,
                on_ground: false,
                velocity: Quat::from_rotation_y(FACE_SIDE) * Vec3::new(0.0, 0.0, -25.0),
                ..player(t)
            },
        ),
        (
            "SLAM",
            PlayerAnim {
                slamming: true,
                on_ground: false,
                velocity: Vec3::new(0.0, -30.0, 0.0),
                ..player(t)
            },
        ),
        (
            "WALL",
            PlayerAnim {
                wall_cling: Some(Quat::from_rotation_y(FACE_SIDE) * Vec3::X),
                on_ground: false,
                ..player(t)
            },
        ),
        (
            "DOWNED",
            PlayerAnim {
                downed: true,
                ..player(t)
            },
        ),
        (
            "DEAD",
            PlayerAnim {
                dead: true,
                ..player(t)
            },
        ),
        (
            "TEAM",
            PlayerAnim {
                weapon: 2,
                ..player(t)
            },
        ),
    ];
    let mut shots = Vec::new();
    for (chunk_index, chunk) in states.chunks(5).enumerate() {
        let positions = row(chunk.len(), 1.3);
        let mut shot = Shot::new(look_at(
            Vec3::new(0.0, 1.4, 4.6),
            Vec3::new(0.0, 0.85, 0.0),
            60.0,
        ));
        for ((name, anim), pos) in chunk.iter().zip(&positions) {
            let anim = PlayerAnim {
                yaw: FACE_SIDE,
                ..*anim
            };
            let tint = if *name == "TEAM" {
                Tint::team(Vec4::new(0.15, 0.55, 1.0, 1.0))
            } else {
                Tint::default()
            };
            lib.draw_player(&anim, *pos, &tint, &mut shot.draws);
            shot.labels
                .push((*pos + Vec3::new(0.0, -0.25, 0.0), name.to_string()));
        }
        shots.push((
            format!("shrike_{chunk_index}"),
            format!("SHRIKE POSES {}", chunk_index + 1),
            shot,
        ));
    }
    // Close-up from behind / side.
    let mut shot = Shot::new(look_at(
        Vec3::new(1.6, 1.7, 2.2),
        Vec3::new(0.0, 1.1, 0.0),
        55.0,
    ));
    let anim = PlayerAnim {
        yaw: 0.6,
        weapon: 1,
        ..player(t)
    };
    lib.draw_player(
        &anim,
        Vec3::new(-0.6, 0.0, 0.0),
        &Tint::default(),
        &mut shot.draws,
    );
    let anim = PlayerAnim {
        yaw: FACE_CAMERA + 0.6,
        weapon: 0,
        ..player(t)
    };
    lib.draw_player(
        &anim,
        Vec3::new(0.7, 0.0, -0.4),
        &Tint::default(),
        &mut shot.draws,
    );
    shots.push(("shrike_closeup".into(), "SHRIKE CLOSE UP".into(), shot));
    shots
}

/// One labelled pose of an enemy sheet: (label, state, speed, attack index, aim pitch).
type Pose = (&'static str, EnemyState, f32, u8, f32);

fn enemy_shot(
    lib: &ModelLibrary,
    kind: EnemyKind,
    states: &[Pose],
    spacing: f32,
    cam_dist: f32,
    cam_y: f32,
    target_y: f32,
) -> Shot {
    let positions = row(states.len(), spacing);
    let mut shot = Shot::new(look_at(
        Vec3::new(0.0, cam_y, cam_dist),
        Vec3::new(0.0, target_y, 0.0),
        60.0,
    ));
    for ((name, state, speed, attack, pitch), pos) in states.iter().zip(&positions) {
        let anim = EnemyAnim {
            state: *state,
            speed: *speed,
            attack: *attack,
            aim_pitch: *pitch,
            aim_locked: name.contains("LOCK"),
            cycle: 0.2,
            time: 2.1,
            seed: 7,
            ..EnemyAnim::new(kind)
        };
        let root_pos = if kind == EnemyKind::Harrier {
            *pos + Vec3::new(0.0, 1.4, 0.0)
        } else {
            *pos
        };
        lib.draw_enemy(
            &anim,
            EnemyAnim::root(root_pos, FACE_CAMERA),
            &Tint::default(),
            &mut shot.draws,
        );
        shot.labels
            .push((*pos + Vec3::new(0.0, -0.2, 0.0), name.to_string()));
    }
    shot
}

/// One pose sheet: (file name, title, kind, poses, spacing, camera distance, camera height,
/// look-at height).
type Sheet = (
    &'static str,
    &'static str,
    EnemyKind,
    Vec<Pose>,
    f32,
    f32,
    f32,
    f32,
);

/// The pose sheets of every enemy.
fn enemy_sheets() -> Vec<Sheet> {
    use EnemyState::*;
    vec![
        (
            "gnawer_1",
            "GNAWER: BITE",
            EnemyKind::Gnawer,
            vec![
                ("IDLE", Idle, 0.0, 0, 0.0),
                ("WALK", Walk, 8.0, 0, 0.0),
                ("BITE .5", Windup(0.5), 0.0, 0, 0.0),
                ("BITE HI", Windup(0.9), 0.0, 0, 0.0),
                ("SNAP", Attack(0.3), 0.0, 0, 0.0),
            ],
            1.9,
            6.2,
            1.8,
            1.0,
        ),
        (
            "gnawer_2",
            "GNAWER: POUNCE, STUN, DEATH",
            EnemyKind::Gnawer,
            vec![
                ("POUNCE WIND", Windup(0.8), 0.0, 1, 0.0),
                ("POUNCE", Attack(0.2), 0.0, 1, 0.0),
                ("RECOVER", Recover(0.3), 0.0, 0, 0.0),
                ("STUNNED", Stunned, 0.0, 0, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            1.8,
            6.8,
            1.8,
            0.9,
        ),
        (
            "censer_1",
            "CENSER: VOLLEY",
            EnemyKind::Censer,
            vec![
                ("IDLE", Idle, 0.0, 1, 0.0),
                ("WALK", Walk, 4.0, 1, 0.0),
                ("WHIRL .3", Windup(0.3), 0.0, 1, 0.0),
                ("WHIRL .9", Windup(0.9), 0.0, 1, 0.0),
                ("LOB", Attack(0.4), 0.0, 1, 0.0),
            ],
            2.0,
            7.0,
            2.0,
            1.4,
        ),
        (
            "censer_2",
            "CENSER: SWING, STUN, DEATH",
            EnemyKind::Censer,
            vec![
                ("SWING WIND", Windup(0.8), 0.0, 0, 0.0),
                ("SWING", Attack(0.5), 0.0, 0, 0.0),
                ("STUNNED", Stunned, 0.0, 0, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            2.0,
            7.0,
            2.0,
            1.3,
        ),
        (
            "harrier_1",
            "HARRIER: DIVE",
            EnemyKind::Harrier,
            vec![
                ("HOVER", Idle, 0.0, 0, 0.0),
                ("REAR .5", Windup(0.5), 0.0, 0, 0.0),
                ("REAR HI", Windup(0.95), 0.0, 0, 0.0),
                ("DIVE", Attack(0.4), 0.0, 0, -0.6),
                ("CLIMB", Recover(0.3), 0.0, 0, 0.0),
            ],
            3.2,
            8.0,
            2.4,
            1.4,
        ),
        (
            "harrier_2",
            "HARRIER: SWATTED, DEATH",
            EnemyKind::Harrier,
            vec![
                ("SWATTED", Stunned, 0.0, 0, 0.0),
                ("DYING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            3.4,
            6.5,
            2.2,
            1.3,
        ),
        (
            "gaoler_1",
            "GAOLER: SHIELD BASH",
            EnemyKind::Gaoler,
            vec![
                ("IDLE", Idle, 0.0, 0, 0.0),
                ("WALK", Walk, 4.0, 0, 0.0),
                ("BASH .4", Windup(0.4), 0.0, 0, 0.0),
                ("PARRY NOW", Windup(0.85), 0.0, 0, 0.0),
                ("BASH", Attack(0.5), 0.0, 0, 0.0),
            ],
            3.4,
            12.0,
            3.0,
            1.9,
        ),
        (
            "gaoler_2",
            "GAOLER: CHAIN SWEEP, RUSH",
            EnemyKind::Gaoler,
            vec![
                ("SWEEP WIND", Windup(0.7), 0.0, 1, 0.0),
                ("SWEEP", Attack(0.35), 0.0, 1, 0.0),
                ("RUSH WIND", Windup(0.8), 0.0, 2, 0.0),
                ("RUSH", Attack(0.5), 0.0, 2, 0.0),
            ],
            3.8,
            12.0,
            3.0,
            1.8,
        ),
        (
            "gaoler_3",
            "GAOLER: PARRIED, DEATH",
            EnemyKind::Gaoler,
            vec![
                ("PARRIED", Stunned, 0.0, 0, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            4.0,
            11.0,
            3.0,
            1.6,
        ),
        (
            "heron_1",
            "HERON: NEEDLE",
            EnemyKind::Heron,
            vec![
                ("IDLE", Idle, 0.0, 1, 0.0),
                ("WALK", Walk, 6.0, 1, 0.0),
                ("AIM .5", Windup(0.5), 0.0, 1, -0.1),
                ("AIM LOCKED", Windup(0.9), 0.0, 1, -0.1),
                ("FIRE", Attack(0.3), 0.0, 1, -0.1),
            ],
            2.0,
            9.0,
            2.6,
            1.9,
        ),
        (
            "heron_2",
            "HERON: KICK, STUN, DEATH",
            EnemyKind::Heron,
            vec![
                ("KICK WIND", Windup(0.8), 0.0, 0, 0.0),
                ("KICK", Attack(0.4), 0.0, 0, 0.0),
                ("STUNNED", Stunned, 0.0, 0, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            2.2,
            9.0,
            2.6,
            1.7,
        ),
        // The third roster (docs/enemies.md).
        (
            "bellwether_1",
            "BELLWETHER: TOLL",
            EnemyKind::Bellwether,
            vec![
                ("IDLE", Idle, 0.0, 1, 0.0),
                ("WALK", Walk, 5.0, 1, 0.0),
                ("TOLL .4", Windup(0.4), 0.0, 1, 0.0),
                ("TOLL .9", Windup(0.9), 0.0, 1, 0.0),
                ("RING", Attack(0.5), 0.0, 1, 0.0),
            ],
            2.4,
            10.0,
            2.6,
            1.8,
        ),
        (
            "bellwether_2",
            "BELLWETHER: CLAPPER, CRACKED, DEATH",
            EnemyKind::Bellwether,
            vec![
                ("CLAP WIND", Windup(0.8), 0.0, 0, 0.0),
                ("CLAPPER", Attack(0.4), 0.0, 0, 0.0),
                ("CRACKED", Stunned, 0.0, 1, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            2.4,
            10.0,
            2.6,
            1.7,
        ),
        (
            "snuffer_1",
            "SNUFFER: LATCH",
            EnemyKind::Snuffer,
            vec![
                ("IDLE", Idle, 0.0, 0, 0.0),
                ("WALK", Walk, 10.0, 0, 0.0),
                ("REAR .5", Windup(0.5), 0.0, 0, 0.0),
                ("REAR .9", Windup(0.9), 0.0, 0, 0.0),
                ("LEAP", Attack(0.4), 0.0, 0, 0.0),
                ("LATCHED", Latched, 0.0, 0, 0.0),
            ],
            2.4,
            8.0,
            2.0,
            0.7,
        ),
        (
            "snuffer_2",
            "SNUFFER: NIP, THROWN OFF, DEATH",
            EnemyKind::Snuffer,
            vec![
                ("NIP WIND", Windup(0.8), 0.0, 1, 0.0),
                ("NIP", Attack(0.4), 0.0, 1, 0.0),
                ("THROWN", Stunned, 0.0, 0, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            2.4,
            8.0,
            2.0,
            0.7,
        ),
        (
            "smelter_1",
            "SMELTER: POUR",
            EnemyKind::Smelter,
            vec![
                ("IDLE", Idle, 0.0, 1, 0.0),
                ("WALK", Walk, 3.6, 1, 0.0),
                ("TIP .5", Windup(0.5), 0.0, 1, 0.0),
                ("TIP .9", Windup(0.9), 0.0, 1, 0.0),
                ("POURING", Attack(0.5), 0.0, 1, 0.0),
            ],
            3.0,
            12.0,
            2.8,
            1.8,
        ),
        (
            "smelter_2",
            "SMELTER: HEAVE, STUN, DEATH",
            EnemyKind::Smelter,
            vec![
                ("HEAVE WIND", Windup(0.8), 0.0, 0, 0.0),
                ("HEAVE", Attack(0.4), 0.0, 0, 0.0),
                ("STUNNED", Stunned, 0.0, 0, 0.0),
                ("MELTING", Dying(0.5), 0.0, 0, 0.0),
                ("DEAD", Dying(1.0), 0.0, 0, 0.0),
            ],
            3.0,
            12.0,
            2.8,
            1.8,
        ),
        (
            "effigy_1",
            "EFFIGY: IDLE, HIT, HIT",
            EnemyKind::Effigy,
            vec![
                ("IDLE", Idle, 0.0, 0, 0.0),
                ("HIT .1", Recover(0.1), 0.0, 0, 0.0),
                ("HIT .3", Recover(0.3), 0.0, 0, 0.0),
                ("HIT .6", Recover(0.6), 0.0, 0, 0.0),
            ],
            2.0,
            7.0,
            2.0,
            1.2,
        ),
    ]
}

fn viewmodel_shots(lib: &ModelLibrary) -> Vec<(String, String, Shot)> {
    let mut shots = Vec::new();
    let base = |weapon: u8| ViewmodelAnim {
        weapon,
        switch: 1.0,
        time: 1.7,
        ..Default::default()
    };
    let names = ["QUILL", "BELLOWS", "ARCLASH", "TUMBLER"];
    let mut cases: Vec<(String, ViewmodelAnim)> = Vec::new();
    for w in 0..4u8 {
        let n = names[w as usize];
        cases.push((format!("{n} REST"), base(w)));
        cases.push((
            format!("{n} KICK"),
            ViewmodelAnim {
                fire_kick: 1.0,
                ..base(w)
            },
        ));
        cases.push((
            format!("{n} ALT MODE"),
            ViewmodelAnim {
                alt_mode: 1,
                fire_kick: 0.4,
                ..base(w)
            },
        ));
    }
    cases.push((
        "QUILL CREST FOLDING".into(),
        ViewmodelAnim {
            fire_kick: 0.55,
            ..base(0)
        },
    ));
    cases.push((
        "BELLOWS PUMP".into(),
        ViewmodelAnim {
            fire_kick: 0.5,
            alt_mode: 1,
            ..base(1)
        },
    ));
    cases.push((
        "ARCLASH DISCHARGE .4".into(),
        ViewmodelAnim {
            alt_mode: 1,
            alt_charge: 0.4,
            ..base(2)
        },
    ));
    cases.push((
        "ARCLASH DISCHARGE FULL".into(),
        ViewmodelAnim {
            alt_mode: 1,
            alt_charge: 1.0,
            time: 1.71,
            ..base(2)
        },
    ));
    cases.push((
        "TUMBLER DRUM TURNING".into(),
        ViewmodelAnim {
            spin: DrumTurn::STEP * 0.5,
            fire_kick: 0.6,
            alt_mode: 1,
            ..base(3)
        },
    ));
    cases.push((
        "SWITCH .3".into(),
        ViewmodelAnim {
            switch: 0.3,
            ..base(1)
        },
    ));
    cases.push((
        "PUNCH .1".into(),
        ViewmodelAnim {
            punch: 0.1,
            ..base(0)
        },
    ));
    cases.push((
        "PUNCH .25".into(),
        ViewmodelAnim {
            punch: 0.25,
            ..base(3)
        },
    ));
    cases.push((
        "THROW .3".into(),
        ViewmodelAnim {
            throw: 0.3,
            ..base(2)
        },
    ));
    cases.push((
        "THROW .5".into(),
        ViewmodelAnim {
            throw: 0.5,
            ..base(2)
        },
    ));
    cases.push((
        "BOB + SWAY".into(),
        ViewmodelAnim {
            bob_phase: 1.2,
            bob_amount: 1.0,
            sway: Vec2::new(2.0, -1.0),
            ..base(1)
        },
    ));
    for (i, (name, anim)) in cases.into_iter().enumerate() {
        let mut shot = Shot::new(look_at(
            Vec3::new(0.0, 1.7, 7.0),
            Vec3::new(0.0, 1.5, 0.0),
            75.0,
        ));
        lib.draw_viewmodel(&anim, &Tint::default(), &mut shot.viewmodel);
        shots.push((format!("vm_{i:02}"), name, shot));
    }
    shots
}

fn prop_shots(lib: &ModelLibrary, t: f64) -> Vec<(String, String, Shot)> {
    let projectiles = [
        (ModelId::Bomb, "BOMB"),
        (ModelId::Gravewell, "GRAVEWELL"),
        (ModelId::Wisp, "WISP"),
        (ModelId::Thorn, "THORN"),
        (ModelId::EmberBomb, "EMBER"),
        (ModelId::Needle, "NEEDLE"),
        (ModelId::EmberCore, "CORE"),
    ];
    let mut shot = Shot::new(look_at(
        Vec3::new(0.0, 1.4, 3.3),
        Vec3::new(0.0, 0.9, 0.0),
        60.0,
    ));
    for ((id, name), pos) in projectiles.iter().zip(row(projectiles.len(), 0.8)) {
        let rot = Quat::from_rotation_y(-1.1) * Quat::from_rotation_x(0.35);
        let root = Mat4::from_rotation_translation(rot, pos + Vec3::new(0.0, 1.0, 0.0));
        lib.draw_model(*id, root, &Tint::default(), &mut shot.draws);
        shot.labels
            .push((pos + Vec3::new(0.0, 0.55, 0.0), name.to_string()));
    }
    let mut shots = vec![("projectiles".to_string(), "PROJECTILES".to_string(), shot)];

    // Thorns: planted in the wall at different angles, lying on the floor, in flight.
    let mut shot = Shot::new(look_at(
        Vec3::new(1.2, 1.6, -2.6),
        Vec3::new(0.0, 1.2, -5.5),
        60.0,
    ));
    for (x, y, yaw, pitch) in [
        (-1.0, 1.6, 0.3, -0.2),
        (0.0, 1.1, -0.25, 0.15),
        (0.9, 1.8, 0.0, 0.5),
    ] {
        // The wall face is at z = -5.5; the thorn flew towards -Z.
        let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
        let root = Mat4::from_rotation_translation(rot, Vec3::new(x, y, -5.5));
        lib.draw_model(ModelId::Thorn, root, &Tint::default(), &mut shot.draws);
    }
    let lying = Quat::from_rotation_y(0.9) * Quat::from_rotation_z(0.3);
    lib.draw_model(
        ModelId::Thorn,
        Mat4::from_rotation_translation(lying, Vec3::new(0.4, 0.04, -4.4)),
        &Tint::default(),
        &mut shot.draws,
    );
    lib.draw_model(
        ModelId::Thorn,
        Mat4::from_rotation_translation(Quat::from_rotation_y(-0.6), Vec3::new(-0.6, 0.9, -3.8)),
        &Tint::default(),
        &mut shot.draws,
    );
    shot.labels.push((
        Vec3::new(0.0, 0.6, -5.4),
        format!("PLANTED (TIP {:.2} M DEEP)", projectiles::THORN_TIP),
    ));
    shots.push(("thorns".into(), "THORNS".into(), shot));

    // Weapons at real scale, side and three-quarter views.
    for (name, title, cam, yaw) in [
        (
            "weapons",
            "WEAPONS (SIDE)",
            Vec3::new(0.0, 1.15, 1.75),
            std::f32::consts::FRAC_PI_2,
        ),
        (
            "weapons_34",
            "WEAPONS (3/4 FRONT)",
            Vec3::new(0.0, 1.5, 1.6),
            2.4,
        ),
    ] {
        let mut shot = Shot::new(look_at(cam, Vec3::new(0.0, 1.08, 0.0), 50.0));
        for (i, w) in client::models::WeaponKind::ALL.iter().enumerate() {
            let pos = Vec3::new(
                if i % 2 == 0 { -0.45 } else { 0.45 },
                if i < 2 { 1.35 } else { 0.8 },
                0.0,
            );
            let root = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw), pos);
            let pose = client::models::weapons::weapon_pose(*w, 0, 0.0, 0.0, 0.0, t);
            lib.get(w.model())
                .draw(&pose, root, &Tint::default(), &mut shot.draws);
            shot.labels.push((
                pos + Vec3::new(0.0, -0.22, 0.0),
                format!("{w:?}").to_uppercase(),
            ));
        }
        shots.push((name.into(), title.into(), shot));
    }

    let mut shot = Shot::new(look_at(
        Vec3::new(0.0, 3.2, 13.0),
        Vec3::new(0.0, 2.0, 0.0),
        60.0,
    ));
    let face = Quat::from_rotation_y(std::f32::consts::PI - 0.3);
    let at = |x: f32| Mat4::from_rotation_translation(face, Vec3::new(x, 0.0, 0.0));
    let lib_rig = |id: ModelId| lib.get(id);
    lib_rig(ModelId::JumpPad).draw(
        &props::jump_pad_pose(t),
        at(-7.0),
        &Tint::default(),
        &mut shot.draws,
    );
    lib_rig(ModelId::Checkpoint).draw(
        &props::checkpoint_pose(false, t),
        at(-4.6),
        &Tint::default(),
        &mut shot.draws,
    );
    lib_rig(ModelId::Checkpoint).draw(
        &props::checkpoint_pose(true, t),
        at(-3.2),
        &Tint::default(),
        &mut shot.draws,
    );
    lib_rig(ModelId::ExitGate).draw(
        &props::exit_gate_pose(t),
        at(1.0),
        &Tint::default(),
        &mut shot.draws,
    );
    lib_rig(ModelId::Door).draw(
        &props::door_pose(0.0, true),
        at(5.3),
        &Tint::default(),
        &mut shot.draws,
    );
    lib_rig(ModelId::Door).draw(
        &props::door_pose(0.6, false),
        Mat4::from_rotation_translation(face, Vec3::new(8.0, 0.0, -2.0)),
        &Tint::default(),
        &mut shot.draws,
    );
    for (x, name) in [
        (-7.0, "JUMP PAD"),
        (-4.6, "CHECKPOINT"),
        (-3.0, "ACTIVE"),
        (1.0, "EXIT GATE"),
        (5.3, "DOOR LOCKED"),
        (8.5, "DOOR OPENING"),
    ] {
        shot.labels
            .push((Vec3::new(x, -0.3, 0.0), name.to_string()));
    }
    shots.push(("props".into(), "PROPS".into(), shot));
    shots
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let mut out = PathBuf::from("target/models_view");
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(PathBuf::from).unwrap_or(out),
            "--only" => only = args.next(),
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    std::fs::create_dir_all(&out)?;

    let settings = RetroSettings {
        internal_height: 360,
        vsync: false,
        ..Default::default()
    };
    let mut r = Renderer::new_headless(W, H, &settings)?;
    let lib = ModelLibrary::load(&mut r)?;
    if !lib.regenerated.is_empty() {
        eprintln!(
            "note: generated in memory (missing glTF): {:?}",
            lib.regenerated
        );
    }

    let mut floor = MeshBuilder::new();
    floor.add_box(&BoxDesc {
        uv: BoxUv::texel_density(meshgen::DEFAULT_TEXELS_PER_METRE, 64.0),
        ..BoxDesc::brush(
            Vec3::new(0.0, -0.25, 0.0),
            Vec3::new(30.0, 0.25, 30.0),
            Quat::IDENTITY,
        )
    });
    floor.ao_near_floor(-0.5, 0.3, 0.0);
    let mut wall = MeshBuilder::new();
    wall.add_box(&BoxDesc {
        uv: BoxUv::texel_density(meshgen::DEFAULT_TEXELS_PER_METRE, 64.0),
        ..BoxDesc::brush(
            Vec3::new(0.0, 4.0, -6.0),
            Vec3::new(30.0, 4.0, 0.5),
            Quat::IDENTITY,
        )
    });
    wall.ao_near_floor(0.0, 1.5, 0.4);
    let stage = Stage {
        floor: r.create_mesh(&floor.build()),
        wall: r.create_mesh(&wall.build()),
        concrete: r.create_texture(&procedural::material_texture("concrete")),
        brick: r.create_texture(&procedural::material_texture("brick")),
    };

    let mut shots: Vec<(String, String, Shot)> = Vec::new();
    // Scale lineup.
    {
        let mut shot = Shot::new(look_at(
            Vec3::new(0.3, 2.3, 10.5),
            Vec3::new(0.3, 1.6, 0.0),
            60.0,
        ));
        let xs = [-5.6, -3.7, -1.7, 0.7, 3.6, 6.2];
        lib.draw_player(
            &PlayerAnim {
                yaw: FACE_CAMERA,
                ..player(1.0)
            },
            Vec3::new(xs[0], 0.0, 0.0),
            &Tint::default(),
            &mut shot.draws,
        );
        shot.labels
            .push((Vec3::new(xs[0], -0.2, 0.0), "SHRIKE".into()));
        for (kind, x) in EnemyKind::ALL.iter().zip(&xs[1..]) {
            let y = if *kind == EnemyKind::Harrier {
                1.6
            } else {
                0.0
            };
            let anim = EnemyAnim {
                time: 1.0,
                ..EnemyAnim::new(*kind)
            };
            lib.draw_enemy(
                &anim,
                EnemyAnim::root(Vec3::new(*x, y, 0.0), FACE_CAMERA),
                &Tint::default(),
                &mut shot.draws,
            );
            shot.labels
                .push((Vec3::new(*x, -0.2, 0.0), format!("{kind:?}").to_uppercase()));
        }
        shots.push(("lineup".into(), "LINEUP".into(), shot));
        // Same from behind: the models face the wall, the camera looks at their backs.
        let mut shot = Shot::new(look_at(
            Vec3::new(1.0, 2.5, 9.5),
            Vec3::new(0.3, 1.6, 0.0),
            60.0,
        ));
        lib.draw_player(
            &PlayerAnim {
                yaw: 0.35,
                weapon: 3,
                ..player(1.0)
            },
            Vec3::new(xs[0], 0.0, 0.0),
            &Tint::default(),
            &mut shot.draws,
        );
        for (kind, x) in EnemyKind::ALL.iter().zip(&xs[1..]) {
            let y = if *kind == EnemyKind::Harrier {
                1.6
            } else {
                0.0
            };
            let anim = EnemyAnim {
                time: 1.0,
                ..EnemyAnim::new(*kind)
            };
            lib.draw_enemy(
                &anim,
                EnemyAnim::root(Vec3::new(*x, y, 0.0), 0.35),
                &Tint::default(),
                &mut shot.draws,
            );
        }
        shots.push(("lineup_back".into(), "LINEUP FROM BEHIND".into(), shot));
        // The third roster beside a Shrike and a Gaoler (the five-slot lineup above is full).
        let mut shot = Shot::new(look_at(
            Vec3::new(0.3, 2.3, 10.5),
            Vec3::new(0.3, 1.6, 0.0),
            60.0,
        ));
        lib.draw_player(
            &PlayerAnim {
                yaw: FACE_CAMERA,
                ..player(1.0)
            },
            Vec3::new(xs[0], 0.0, 0.0),
            &Tint::default(),
            &mut shot.draws,
        );
        shot.labels
            .push((Vec3::new(xs[0], -0.2, 0.0), "SHRIKE".into()));
        let third = [
            EnemyKind::Gaoler,
            EnemyKind::Bellwether,
            EnemyKind::Snuffer,
            EnemyKind::Smelter,
            EnemyKind::Effigy,
        ];
        for (kind, x) in third.iter().zip(&xs[1..]) {
            let anim = EnemyAnim {
                time: 1.0,
                ..EnemyAnim::new(*kind)
            };
            lib.draw_enemy(
                &anim,
                EnemyAnim::root(Vec3::new(*x, 0.0, 0.0), FACE_CAMERA),
                &Tint::default(),
                &mut shot.draws,
            );
            shot.labels
                .push((Vec3::new(*x, -0.2, 0.0), format!("{kind:?}").to_uppercase()));
        }
        shots.push((
            "lineup_third".into(),
            "LINEUP: THE THIRD ROSTER".into(),
            shot,
        ));
    }
    // Close-ups: each enemy idle (front three-quarter) next to the same model from behind.
    for (kind, dist, height) in [
        (EnemyKind::Gnawer, 4.2, 1.1),
        (EnemyKind::Censer, 5.0, 1.4),
        (EnemyKind::Harrier, 5.6, 1.6),
        (EnemyKind::Gaoler, 7.6, 2.0),
        (EnemyKind::Heron, 6.4, 1.8),
        (EnemyKind::Bellwether, 7.0, 1.7),
        (EnemyKind::Snuffer, 5.0, 0.8),
        (EnemyKind::Smelter, 8.0, 1.8),
        (EnemyKind::Effigy, 5.0, 1.2),
    ] {
        let mut shot = Shot::new(look_at(
            Vec3::new(0.0, height + 0.4, dist),
            Vec3::new(0.0, height, 0.0),
            60.0,
        ));
        let y = if kind == EnemyKind::Harrier { 1.6 } else { 0.0 };
        let half = dist * 0.27;
        for (x, yaw) in [(-half, FACE_CAMERA), (half, 0.5)] {
            let anim = EnemyAnim {
                time: 1.3,
                seed: 3,
                ..EnemyAnim::new(kind)
            };
            lib.draw_enemy(
                &anim,
                EnemyAnim::root(Vec3::new(x, y, 0.0), yaw),
                &Tint::default(),
                &mut shot.draws,
            );
        }
        let name = format!("{kind:?}").to_lowercase();
        shots.push((
            format!("closeup_{name}"),
            format!("{} CLOSE UP", name.to_uppercase()),
            shot,
        ));
    }
    shots.extend(shrike_shots(&lib));
    for (name, title, kind, poses, spacing, dist, cam_y, target_y) in enemy_sheets() {
        shots.push((
            name.into(),
            title.into(),
            enemy_shot(&lib, kind, &poses, spacing, dist, cam_y, target_y),
        ));
    }
    shots.extend(viewmodel_shots(&lib));
    shots.extend(prop_shots(&lib, 2.0));

    for (name, title, shot) in &shots {
        if only.as_ref().is_some_and(|o| !name.starts_with(o.as_str())) {
            continue;
        }
        let path = out.join(format!("{name}.png"));
        render(&mut r, &stage, shot, &path, title)?;
        println!("{}", path.display());
    }
    Ok(())
}
