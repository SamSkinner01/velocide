//! Headless level viewer: renders PNG captures of a level for checking layout, lighting and
//! markup without running the game.
//!
//! ```text
//! cargo run -p client --example level_view -- <level> [options]
//!   --out DIR            output directory (default captures/levels); files go to DIR/<level>/
//!   --only GROUPS        comma list of: overview, areas, spawns, checkpoints, encounters
//!   --lines              debug overlay on first-person views too (always on for overviews)
//!   --cam x,y,z,yaw,pitch  one extra first-person view (degrees), saved as cam.png
//!   --doors open|closed  force every door open/closed (default: each door's start state)
//! ```
//!
//! Views: every player spawn and checkpoint (looking along its yaw), each encounter from its
//! centre looking N/E/S/W, a top-down overview of the whole level (fog off, roofs removed for
//! indoor levels) and a top-down + angled view of every encounter area. Each group is also
//! saved as a contact sheet (`sheet_<group>.png`).
//!
//! Overlay colours: encounter triggers orange, checkpoints green, exit white, kill volumes red,
//! doors (closed red / open green), player spawns cyan (with facing), jump pads cyan with
//! their ballistic arc, enemies: husk yellow, spitter magenta, gnat light blue, bastion red.

use std::path::{Path, PathBuf};

use client::level_mesh::{self, LevelRender};
use engine::gfx::{
    Camera, DebugLine, DrawItem, FrameDesc, RetroSettings, TextureData, UiDrawList, UiQuad,
    UiTexture,
};
use engine::physics::MASK_MOVEMENT;
use engine::render::Renderer;
use engine::ui::font;
use glam::{Quat, Vec2, Vec3, Vec4};
use shared::level::{Brush, LevelDef, LevelKind, Volume};

const FP_SIZE: (u32, u32) = (640, 360);

struct Args {
    level: String,
    out: PathBuf,
    only: Option<Vec<String>>,
    lines: bool,
    cam: Option<[f32; 5]>,
    doors: Option<bool>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        level: String::new(),
        out: PathBuf::from("captures/levels"),
        only: None,
        lines: false,
        cam: None,
        doors: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| anyhow::anyhow!("{a} needs a value"));
        match a.as_str() {
            "--out" => args.out = value()?.into(),
            "--only" => args.only = Some(value()?.split(',').map(str::to_owned).collect()),
            "--lines" => args.lines = true,
            "--doors" => args.doors = Some(value()? == "open"),
            "--cam" => {
                let v: Vec<f32> = value()?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
                anyhow::ensure!(v.len() == 5, "--cam wants x,y,z,yaw,pitch");
                args.cam = Some([v[0], v[1], v[2], v[3], v[4]]);
            }
            s if s.starts_with("--") => anyhow::bail!("unknown option {s}"),
            s => args.level = s.to_owned(),
        }
    }
    anyhow::ensure!(
        !args.level.is_empty(),
        "usage: level_view <level> [--out DIR] [--only GROUPS] [--lines] [--cam x,y,z,yaw,pitch] [--doors open|closed]\nlevels: {:?}",
        LevelDef::list()
    );
    Ok(args)
}

// ------------------------------------------------------------------------------------------
// Overlay

fn line(out: &mut Vec<DebugLine>, a: Vec3, b: Vec3, color: Vec4) {
    out.push(DebugLine { a, b, color });
}

fn box_lines(out: &mut Vec<DebugLine>, min: Vec3, max: Vec3, color: Vec4) {
    let c = |i: u32| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    };
    for (a, b) in [
        (0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7),
    ] {
        line(out, c(a), c(b), color);
    }
}

fn volume_lines(out: &mut Vec<DebugLine>, v: &Volume, color: Vec4) {
    box_lines(out, v.min, v.max, color);
}

fn yaw_dir(yaw_deg: f32) -> Vec3 {
    shared::input::look_dir(yaw_deg.to_radians(), 0.0)
}

fn marker(out: &mut Vec<DebugLine>, p: Vec3, height: f32, size: f32, color: Vec4) {
    line(out, p, p + Vec3::Y * height, color);
    line(out, p - Vec3::X * size, p + Vec3::X * size, color);
    line(out, p - Vec3::Z * size, p + Vec3::Z * size, color);
}

fn enemy_color(name: &str) -> Vec4 {
    match name {
        "husk" => Vec4::new(1.0, 0.9, 0.2, 1.0),
        "spitter" => Vec4::new(1.0, 0.3, 1.0, 1.0),
        "gnat" => Vec4::new(0.5, 0.8, 1.0, 1.0),
        "bastion" => Vec4::new(1.0, 0.15, 0.1, 1.0),
        _ => Vec4::ONE,
    }
}

fn overlay(level: &LevelDef) -> Vec<DebugLine> {
    let mut out = Vec::new();
    let orange = Vec4::new(1.0, 0.55, 0.1, 1.0);
    let green = Vec4::new(0.2, 1.0, 0.3, 1.0);
    let red = Vec4::new(1.0, 0.1, 0.1, 1.0);
    let cyan = Vec4::new(0.2, 1.0, 1.0, 1.0);
    for e in &level.encounters {
        volume_lines(&mut out, &e.trigger, orange);
        for w in &e.waves {
            for s in &w.spawns {
                let c = enemy_color(&s.enemy);
                let (h, r) = match s.enemy.as_str() {
                    "bastion" => (3.2, 1.2),
                    "gnat" => (0.8, 0.8),
                    _ => (2.0, 0.5),
                };
                marker(&mut out, s.position, h, r, c);
                line(&mut out, s.position + Vec3::Y, s.position + Vec3::Y + yaw_dir(s.yaw) * 1.5, c);
            }
        }
    }
    for c in &level.checkpoints {
        volume_lines(&mut out, &c.trigger, green);
        marker(&mut out, c.spawn.position, 1.8, 0.5, green);
        let eye = c.spawn.position + Vec3::Y * 1.6;
        line(&mut out, eye, eye + yaw_dir(c.spawn.yaw) * 2.0, green);
    }
    if let Some(x) = &level.exit {
        volume_lines(&mut out, x, Vec4::ONE);
    }
    for k in &level.kill_volumes {
        if (k.max - k.min).max_element() < 500.0 {
            volume_lines(&mut out, k, red);
        }
    }
    for d in &level.doors {
        let c = if d.starts_open { green } else { red };
        box_lines(&mut out, d.brush.min, d.brush.max, c);
    }
    for s in &level.spawns {
        marker(&mut out, s.position, 1.8, 0.5, cyan);
        let eye = s.position + Vec3::Y * 1.6;
        line(&mut out, eye, eye + yaw_dir(s.yaw) * 2.0, cyan);
    }
    for p in &level.jump_pads {
        volume_lines(&mut out, &p.trigger, cyan);
        // Ballistic arc (no air control) with the default gravity.
        let gravity = shared::tuning::MovementTuning::default().gravity;
        let start = Vec3::new(p.trigger.center().x, p.trigger.min.y, p.trigger.center().z);
        let mut prev = start;
        for i in 1..=40 {
            let t = i as f32 * 0.05;
            let pos = start + p.velocity * t - Vec3::Y * 0.5 * gravity * t * t;
            line(&mut out, prev, pos, cyan);
            prev = pos;
        }
    }
    out
}

// ------------------------------------------------------------------------------------------
// Roof removal for top-down views

fn brush_aabb(b: &Brush) -> (Vec3, Vec3) {
    let (c, h, q) = (b.center(), b.half_extents(), b.rotation_quat());
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for i in 0..8 {
        let s = Vec3::new(
            if i & 1 == 0 { -1.0 } else { 1.0 },
            if i & 2 == 0 { -1.0 } else { 1.0 },
            if i & 4 == 0 { -1.0 } else { 1.0 },
        );
        let p = c + q * (h * s);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

/// A copy of the level without "roofs": flat-ish brushes that nothing visible sits above.
fn roofless(level: &LevelDef) -> LevelDef {
    let boxes: Vec<_> = level.brushes.iter().map(brush_aabb).collect();
    let visible: Vec<bool> = level.brushes.iter().map(|b| !b.clip_only).collect();
    let mut out = level.clone();
    out.brushes = level
        .brushes
        .iter()
        .enumerate()
        .filter(|&(i, _)| {
            let (lo, hi) = boxes[i];
            if !visible[i] || hi.y - lo.y > 2.5 {
                return true;
            }
            // Keep it if something visible sits above it (it's a floor, not a roof).
            boxes.iter().enumerate().any(|(j, &(l2, h2))| {
                j != i
                    && visible[j]
                    && l2.y >= hi.y - 0.01
                    && l2.x < hi.x - 0.5
                    && h2.x > lo.x + 0.5
                    && l2.z < hi.z - 0.5
                    && h2.z > lo.z + 0.5
            })
        })
        .map(|(_, b)| b.clone())
        .collect();
    out
}

// ------------------------------------------------------------------------------------------
// Rendering

struct Scene {
    level: LevelDef,
    render: LevelRender,
    doors: Option<bool>,
}

impl Scene {
    fn new(renderer: &mut Renderer, level: LevelDef, doors: Option<bool>) -> Self {
        let render = LevelRender::new(renderer, &level);
        Self {
            level,
            render,
            doors,
        }
    }
}

struct Shot<'a> {
    camera: Camera,
    label: String,
    fog: bool,
    lines: &'a [DebugLine],
    bright: bool,
}

fn text(ui: &mut UiDrawList, pos: Vec2, scale: f32, color: Vec4, s: &str) {
    let mut x = pos.x;
    for c in s.chars() {
        if c != ' ' {
            let (uv_min, uv_max) = font::glyph_uv(c);
            ui.quads.push(UiQuad {
                min: Vec2::new(x, pos.y),
                max: Vec2::new(x + font::GLYPH_W * scale, pos.y + font::GLYPH_H * scale),
                uv_min,
                uv_max,
                color,
                texture: UiTexture::Font,
            });
        }
        x += font::GLYPH_ADVANCE * scale;
    }
}

fn render_shot(renderer: &mut Renderer, scene: &Scene, shot: &Shot) -> anyhow::Result<TextureData> {
    let env = level_mesh::environment(&scene.level);
    let mut draws: Vec<DrawItem> = Vec::new();
    let starts_open: std::collections::BTreeMap<u32, bool> =
        scene.level.doors.iter().map(|d| (d.id, d.starts_open)).collect();
    scene.render.draw(
        |id| scene.doors.unwrap_or_else(|| starts_open.get(&id).copied().unwrap_or(true)),
        &mut draws,
    );
    let (w, h) = renderer.window_size();
    let mut ui = UiDrawList::new(UiDrawList::canvas_for_window(w, h));
    let scale = 1.0;
    let label_w = shot.label.len() as f32 * font::GLYPH_ADVANCE * scale + 6.0;
    ui.rect(
        Vec2::new(2.0, 2.0),
        Vec2::new(2.0 + label_w, 4.0 + font::GLYPH_H * scale + 2.0),
        Vec4::new(0.0, 0.0, 0.0, 0.6),
    );
    text(&mut ui, Vec2::new(5.0, 4.0), scale, Vec4::ONE, &shot.label);
    let mut fog = env.fog;
    if !shot.fog {
        fog.start = 1.0e5;
        fog.end = 2.0e5;
    }
    let ambient = if shot.bright { env.ambient.max(Vec3::splat(0.8)) } else { env.ambient };
    let frame = FrameDesc {
        camera: shot.camera,
        clear_color: if shot.fog { env.clear_color } else { Vec3::new(0.05, 0.05, 0.07) },
        fog,
        ambient,
        light_dir: env.light_dir,
        light_color: env.light_color,
        lights: scene.render.lights(),
        draws: &draws,
        sprites: &[],
        decals: &[],
        viewmodel: &[],
        viewmodel_fov_y_degrees: 60.0,
        debug_lines: shot.lines,
        screen_flash: Vec4::ZERO,
        ui: &ui,
    };
    renderer.render(&frame, None)?;
    renderer.read_pixels()
}

fn fp_camera(eye: Vec3, yaw_deg: f32, pitch_deg: f32) -> Camera {
    Camera {
        position: eye,
        rotation: Camera::rotation_from_angles(yaw_deg.to_radians(), pitch_deg.to_radians(), 0.0),
        fov_y_degrees: 74.0,
        near: 0.05,
        far: 600.0,
    }
}

/// Camera straight above `center` that fits `half` (x/z half extents) into an image of
/// `aspect` with vertical FOV `fov`.
fn top_down(center: Vec3, half: Vec2, top: f32, fov: f32, aspect: f32) -> Camera {
    let t = (fov.to_radians() * 0.5).tan();
    let dist = (half.y / t).max(half.x / (t * aspect)) * 1.08;
    let rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    Camera {
        position: Vec3::new(center.x, top + dist, center.z),
        rotation,
        fov_y_degrees: fov,
        near: 1.0,
        far: dist + 400.0,
    }
}

fn save(img: &TextureData, path: &Path) -> anyhow::Result<()> {
    engine::assets::save_png(path, img)?;
    println!("  {}", path.display());
    Ok(())
}

/// Tiles images (all the same size) into a grid `cols` wide.
fn sheet(images: &[TextureData], cols: usize) -> Option<TextureData> {
    let first = images.first()?;
    let (w, h) = (first.width as usize, first.height as usize);
    let rows = images.len().div_ceil(cols);
    let (sw, sh) = (w * cols.min(images.len()), h * rows);
    let mut rgba = vec![0u8; sw * sh * 4];
    for (i, img) in images.iter().enumerate() {
        let (ox, oy) = ((i % cols) * w, (i / cols) * h);
        for y in 0..h {
            let src = &img.rgba[y * w * 4..(y + 1) * w * 4];
            let dst = ((oy + y) * sw + ox) * 4;
            rgba[dst..dst + w * 4].copy_from_slice(src);
        }
    }
    Some(TextureData {
        width: sw as u32,
        height: sh as u32,
        rgba,
    })
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = parse_args()?;
    let level = LevelDef::load_named(&args.level)?;
    let out = args.out.join(&args.level);
    std::fs::create_dir_all(&out)?;
    let want = |g: &str| args.only.as_ref().is_none_or(|o| o.iter().any(|x| x == g));
    let collision = level.build_collision();
    let all_lines = overlay(&level);
    let no_lines: Vec<DebugLine> = Vec::new();
    let fp_lines: &[DebugLine] = if args.lines { &all_lines } else { &no_lines };

    // Level bounds (rendered brushes only).
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for b in level.brushes.iter().filter(|b| !b.clip_only) {
        let (l, h) = brush_aabb(b);
        lo = lo.min(l);
        hi = hi.max(h);
    }
    println!(
        "{}: {} brushes, {} lights, bounds {lo} .. {hi}",
        level.name,
        level.brushes.len(),
        level.lights.len()
    );

    // ---- First-person views (640x360, internal 360 = the game's default look).
    let settings = RetroSettings::default();
    let mut fp = Renderer::new_headless(FP_SIZE.0, FP_SIZE.1, &settings)?;
    let fp_scene = Scene::new(&mut fp, level.clone(), args.doors);
    let mut groups: Vec<(&str, Vec<TextureData>)> = Vec::new();

    if want("spawns") {
        let mut imgs = Vec::new();
        for (i, s) in level.spawns.iter().enumerate() {
            let shot = Shot {
                camera: fp_camera(s.position + Vec3::Y * 1.6, s.yaw, -4.0),
                label: format!("SPAWN {i}"),
                fog: true,
                lines: fp_lines,
                bright: false,
            };
            let img = render_shot(&mut fp, &fp_scene, &shot)?;
            save(&img, &out.join(format!("spawn_{i}.png")))?;
            imgs.push(img);
        }
        groups.push(("spawns", imgs));
    }
    if want("checkpoints") {
        let mut imgs = Vec::new();
        for (i, c) in level.checkpoints.iter().enumerate() {
            let shot = Shot {
                camera: fp_camera(c.spawn.position + Vec3::Y * 1.6, c.spawn.yaw, -4.0),
                label: format!("CHECKPOINT {}", i + 1),
                fog: true,
                lines: fp_lines,
                bright: false,
            };
            let img = render_shot(&mut fp, &fp_scene, &shot)?;
            save(&img, &out.join(format!("checkpoint_{}.png", i + 1)))?;
            imgs.push(img);
        }
        groups.push(("checkpoints", imgs));
    }
    if want("encounters") {
        let mut imgs = Vec::new();
        for (i, e) in level.encounters.iter().enumerate() {
            let c = e.trigger.center();
            // Reference height: median height of the grounded enemies of the encounter.
            let mut ys: Vec<f32> = e
                .waves
                .iter()
                .flat_map(|w| &w.spawns)
                .filter(|s| s.enemy != "gnat")
                .map(|s| s.position.y)
                .collect();
            ys.sort_by(f32::total_cmp);
            let ref_y = ys.get(ys.len() / 2).copied().unwrap_or(e.trigger.min.y);
            let from = Vec3::new(c.x, ref_y + 6.0, c.z);
            let ground = collision
                .world
                .cast_ray(from, Vec3::NEG_Y, 12.0, MASK_MOVEMENT)
                .map(|h| h.point.y)
                .filter(|y| (y - ref_y).abs() < 3.0)
                .unwrap_or(ref_y);
            let eye = Vec3::new(c.x, ground + 2.4, c.z);
            for (dir, yaw) in [("N", 0.0), ("W", 90.0), ("S", 180.0), ("E", -90.0)] {
                let shot = Shot {
                    camera: fp_camera(eye, yaw, -3.0),
                    label: format!("ENCOUNTER {} LOOKING {dir}", i + 1),
                    fog: true,
                    lines: fp_lines,
                    bright: false,
                };
                let img = render_shot(&mut fp, &fp_scene, &shot)?;
                save(&img, &out.join(format!("encounter_{}_{dir}.png", i + 1)))?;
                imgs.push(img);
            }
        }
        groups.push(("encounters", imgs));
    }
    if let Some([x, y, z, yaw, pitch]) = args.cam {
        let shot = Shot {
            camera: fp_camera(Vec3::new(x, y, z), yaw, pitch),
            label: format!("CAM {x} {y} {z} YAW {yaw} PITCH {pitch}"),
            fog: true,
            lines: fp_lines,
            bright: false,
        };
        let img = render_shot(&mut fp, &fp_scene, &shot)?;
        save(&img, &out.join("cam.png"))?;
    }
    for (name, imgs) in &groups {
        if let Some(s) = sheet(imgs, 3) {
            save(&s, &out.join(format!("sheet_{name}.png")))?;
        }
    }

    // ---- Overviews: native resolution, fog off, roofs removed for indoor levels.
    let top_level = if level.kind == LevelKind::Campaign { roofless(&level) } else { level.clone() };
    if want("overview") {
        let size = (hi - lo).max(Vec3::ONE);
        let aspect = size.x / size.z;
        let (w, h) = if aspect >= 1.0 {
            (1400, (1400.0 / aspect).clamp(400.0, 1400.0) as u32)
        } else {
            ((1400.0 * aspect).clamp(500.0, 1400.0) as u32, 1400)
        };
        let settings = RetroSettings {
            internal_height: 0,
            vertex_snap: false,
            dithering: false,
            color_bits: 8,
            ..RetroSettings::default()
        };
        let mut top = Renderer::new_headless(w, h, &settings)?;
        let scene = Scene::new(&mut top, top_level.clone(), args.doors);
        let center = (lo + hi) * 0.5;
        let half = Vec2::new(size.x, size.z) * 0.5;
        for (name, lines) in [("overview", &all_lines), ("overview_clean", &no_lines)] {
            let shot = Shot {
                camera: top_down(center, half, hi.y, 20.0, w as f32 / h as f32),
                label: format!("{} - TOP DOWN", level.name.to_uppercase()),
                fog: false,
                lines,
                bright: true,
            };
            let img = render_shot(&mut top, &scene, &shot)?;
            save(&img, &out.join(format!("{name}.png")))?;
        }
    }
    if want("areas") {
        let settings = RetroSettings {
            internal_height: 0,
            vertex_snap: false,
            ..RetroSettings::default()
        };
        let (w, h) = (960, 720);
        let mut top = Renderer::new_headless(w, h, &settings)?;
        let scene = Scene::new(&mut top, top_level.clone(), args.doors);
        let full_scene = Scene::new(&mut top, level.clone(), args.doors);
        let mut imgs = Vec::new();
        for (i, e) in level.encounters.iter().enumerate() {
            let c = e.trigger.center();
            let half = Vec2::new(e.trigger.max.x - e.trigger.min.x, e.trigger.max.z - e.trigger.min.z)
                * 0.5
                + Vec2::splat(4.0);
            let shot = Shot {
                camera: top_down(c, half, e.trigger.max.y, 50.0, w as f32 / h as f32),
                label: format!("AREA {} TOP", i + 1),
                fog: false,
                lines: &all_lines,
                bright: true,
            };
            let img = render_shot(&mut top, &scene, &shot)?;
            save(&img, &out.join(format!("area_{}_top.png", i + 1)))?;
            imgs.push(img);
            // Angled views from two high corners inside the trigger volume, looking at the
            // centre of the encounter at enemy height (full level, with ceilings).
            let mut ys: Vec<f32> = e.waves.iter().flat_map(|w| &w.spawns).map(|s| s.position.y).collect();
            ys.sort_by(f32::total_cmp);
            let target_y = ys.get(ys.len() / 2).copied().unwrap_or(c.y);
            let target = Vec3::new(c.x, target_y, c.z);
            for (corner, (sx, sz)) in [("SE", (1.0, 1.0)), ("NW", (-1.0, -1.0))] {
                let eye = Vec3::new(
                    if sx > 0.0 { e.trigger.max.x - 1.5 } else { e.trigger.min.x + 1.5 },
                    (e.trigger.max.y - 2.0).min(target_y + 14.0),
                    if sz > 0.0 { e.trigger.max.z - 1.0 } else { e.trigger.min.z + 1.5 },
                );
                let d = target - eye;
                let yaw = (-d.x).atan2(-d.z).to_degrees();
                let pitch = (d.y / Vec2::new(d.x, d.z).length()).atan().to_degrees();
                let shot = Shot {
                    camera: Camera {
                        fov_y_degrees: 70.0,
                        ..fp_camera(eye, yaw, pitch)
                    },
                    label: format!("AREA {} FROM {corner}", i + 1),
                    fog: true,
                    lines: fp_lines,
                    bright: false,
                };
                let img = render_shot(&mut top, &full_scene, &shot)?;
                save(&img, &out.join(format!("area_{}_{corner}.png", i + 1)))?;
                imgs.push(img);
            }
        }
        if level.encounters.is_empty() {
            // Arena: four angled views from the corners.
            let center = (lo + hi) * 0.5;
            let half = Vec2::new(hi.x - lo.x, hi.z - lo.z) * 0.5;
            for (k, (sx, sz)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].into_iter().enumerate() {
                let eye = Vec3::new(center.x + sx * half.x * 0.9, hi.y + 4.0, center.z + sz * half.y * 0.9);
                let d = Vec3::new(center.x, lo.y, center.z) - eye;
                let yaw = (-d.x).atan2(-d.z).to_degrees();
                let pitch = (d.y / Vec2::new(d.x, d.z).length()).atan().to_degrees();
                let shot = Shot {
                    camera: Camera {
                        fov_y_degrees: 70.0,
                        ..fp_camera(eye, yaw, pitch)
                    },
                    label: format!("CORNER VIEW {}", k + 1),
                    fog: true,
                    lines: &no_lines,
                    bright: false,
                };
                let img = render_shot(&mut top, &scene, &shot)?;
                save(&img, &out.join(format!("corner_{}.png", k + 1)))?;
                imgs.push(img);
            }
        }
        if let Some(s) = sheet(&imgs, 2) {
            save(&s, &out.join("sheet_areas.png"))?;
        }
    }
    Ok(())
}
