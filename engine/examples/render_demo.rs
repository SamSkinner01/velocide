//! Renderer test scene: procedural level, materials, a spinning glTF model, point lights,
//! sprites, decals, a viewmodel, debug lines, UI and screen flashes.
//!
//! Windowed (default): click to grab the mouse, WASD + mouse to fly, Space/Ctrl up/down, Shift
//! fast, Esc releases the mouse.
//!   1 snap  2 affine  3 dither  4 colour bits  5 fog  6 vertex lights  7 internal res
//!   8 vsync  9 snap strength  0 stress test  F2 capture  F5 reload shaders  F11 fullscreen
//!
//! Headless capture:
//!   cargo run -p engine --example render_demo -- --capture out.png [--frames N]
//!       [--width W --height H] [--res H] [--no-snap] [--snap-strength S] [--affine]
//!       [--no-dither] [--bits N] [--no-fog] [--no-lighting] [--stress]
//!       [--cam x,y,z,yaw_deg,pitch_deg] [--wall]
//! Windowed capture (tests swapchain readback; with `--features debug-ui` shows egui, F1):
//!   cargo run -p engine --example render_demo -- --window-capture out.png [--quit-after N]
//! Benchmark (headless, waits for the GPU every frame):
//!   cargo run -p engine --example render_demo --release -- --bench 500 --stress

#[path = "render_common/models.rs"]
mod models;

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::time::Instant;

use engine::app::{AppConfig, EngineContext, Game};
use engine::assets::meshgen::{self, BoxDesc, BoxUv, MeshBuilder};
use engine::assets::procedural::{self, SpriteTex};
use engine::gfx::{
    Camera, DebugLine, Decal, DrawItem, Fog, FrameDesc, MeshId, PointLight, RetroSettings, Sprite,
    TextureId, UiDrawList, UiQuad, UiTexture,
};
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use engine::input::{Key, MouseButton};
use engine::render::Renderer;
use engine::ui::font;

const FIXED_DT: f32 = 1.0 / 60.0;

// ---------------------------------------------------------------------------------------------
// Scene

struct StaticDraw {
    mesh: MeshId,
    texture: TextureId,
    tint: Vec4,
    unlit: bool,
}

struct Scene {
    level: Vec<StaticDraw>,
    model: Vec<StaticDraw>,
    model_bounds: (Vec3, Vec3),
    pedestal_top: f32,
    gun: MeshId,
    gun_glow: MeshId,
    gun_texture: TextureId,
    muzzle: Vec3,
    flash_quad: MeshId,
    cube: MeshId,
    materials: HashMap<&'static str, TextureId>,
    sprites: HashMap<SpriteTex, TextureId>,
    white: TextureId,
}

/// Collects level brushes per material, then uploads one mesh per material.
struct LevelBuilder {
    meshes: Vec<(&'static str, MeshBuilder)>,
}

impl LevelBuilder {
    fn builder(&mut self, material: &'static str) -> &mut MeshBuilder {
        let index = match self.meshes.iter().position(|(m, _)| *m == material) {
            Some(i) => i,
            None => {
                self.meshes.push((material, MeshBuilder::new()));
                self.meshes.len() - 1
            }
        };
        &mut self.meshes[index].1
    }

    fn brush(
        &mut self,
        material: &'static str,
        center: Vec3,
        half: Vec3,
        rotation: Quat,
        texture_size: f32,
    ) {
        self.builder(material).add_box(&BoxDesc {
            uv: BoxUv::texel_density(meshgen::DEFAULT_TEXELS_PER_METRE, texture_size),
            ..BoxDesc::brush(center, half, rotation)
        });
    }
}

impl Scene {
    fn new(r: &mut Renderer) -> Scene {
        let mut materials = HashMap::new();
        let mut sizes = HashMap::new();
        for name in procedural::MATERIALS {
            let tex = procedural::material_texture(name);
            sizes.insert(*name, tex.width as f32);
            materials.insert(*name, r.create_texture(&tex));
        }
        let sprites = SpriteTex::ALL
            .iter()
            .map(|k| (*k, r.create_texture(&procedural::sprite_texture(*k))))
            .collect();

        // ---- level ----
        let mut lb = LevelBuilder { meshes: Vec::new() };
        let size = |m: &str| sizes.get(m).copied().unwrap_or(64.0);
        let q = Quat::IDENTITY;
        // Floor and walls.
        lb.brush(
            "concrete",
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(20.0, 0.5, 20.0),
            q,
            size("concrete"),
        );
        for (c, h) in [
            (Vec3::new(0.0, 3.0, -20.5), Vec3::new(21.0, 3.5, 0.5)),
            (Vec3::new(0.0, 3.0, 20.5), Vec3::new(21.0, 3.5, 0.5)),
            (Vec3::new(-20.5, 3.0, 0.0), Vec3::new(0.5, 3.5, 20.0)),
            (Vec3::new(20.5, 3.0, 0.0), Vec3::new(0.5, 3.5, 20.0)),
        ] {
            lb.brush("brick", c, h, q, size("brick"));
        }
        // Raised platform (panel sides, tile top) with a hazard edge and a wooden ramp.
        lb.brush(
            "panel",
            Vec3::new(13.0, 1.4, -13.0),
            Vec3::new(6.0, 1.4, 6.0),
            q,
            size("panel"),
        );
        lb.brush(
            "tile",
            Vec3::new(13.0, 2.9, -13.0),
            Vec3::new(6.0, 0.1, 6.0),
            q,
            size("tile"),
        );
        lb.brush(
            "hazard",
            Vec3::new(13.0, 2.9, -6.85),
            Vec3::new(6.0, 0.12, 0.15),
            q,
            size("hazard"),
        );
        let ramp_angle = (3.0f32 / 9.0).atan();
        lb.brush(
            "wood",
            Vec3::new(11.0, 1.35, -2.4),
            Vec3::new(1.6, 0.15, 4.75),
            Quat::from_rotation_x(ramp_angle),
            size("wood"),
        );
        // Lava basin with a concrete rim, crossed by a grate bridge.
        lb.brush(
            "concrete",
            Vec3::new(-11.0, 0.25, -7.2),
            Vec3::new(5.2, 0.25, 0.2),
            q,
            size("concrete"),
        );
        lb.brush(
            "concrete",
            Vec3::new(-11.0, 0.25, 3.2),
            Vec3::new(5.2, 0.25, 0.2),
            q,
            size("concrete"),
        );
        lb.brush(
            "concrete",
            Vec3::new(-16.0, 0.25, -2.0),
            Vec3::new(0.2, 0.25, 5.0),
            q,
            size("concrete"),
        );
        lb.brush(
            "concrete",
            Vec3::new(-6.0, 0.25, -2.0),
            Vec3::new(0.2, 0.25, 5.0),
            q,
            size("concrete"),
        );
        lb.brush(
            "grate",
            Vec3::new(-11.0, 0.55, -2.0),
            Vec3::new(5.0, 0.06, 1.0),
            q,
            size("grate"),
        );
        // Metal pillars, rusty crates, dirt patch, glass pane.
        for (x, z) in [(-17.0, -17.0), (17.0, 17.0), (-17.0, 17.0), (6.0, -17.0)] {
            lb.brush(
                "metal",
                Vec3::new(x, 3.0, z),
                Vec3::new(0.8, 3.0, 0.8),
                q,
                size("metal"),
            );
        }
        for (c, rot) in [
            (Vec3::new(-8.0, 0.5, 9.0), 0.2),
            (Vec3::new(-6.8, 0.5, 9.4), -0.1),
            (Vec3::new(-7.4, 1.5, 9.2), 0.5),
            (Vec3::new(7.5, 0.6, 3.0), 0.7),
        ] {
            lb.brush(
                "rust",
                c,
                Vec3::splat(0.5),
                Quat::from_rotation_y(rot),
                size("rust"),
            );
        }
        lb.brush(
            "dirt",
            Vec3::new(7.0, 0.01, 9.0),
            Vec3::new(4.0, 0.02, 3.5),
            q,
            size("dirt"),
        );
        lb.brush(
            "metal",
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(1.0, 0.5, 1.0),
            q,
            size("metal"),
        );

        let mut level = Vec::new();
        for (material, mut builder) in lb.meshes {
            if material != "grate" {
                builder.ao_near_floor(0.0, 1.5, 0.4);
            }
            builder.jitter_colors(7, 0.06);
            let tint = if material == "glass" {
                Vec4::new(1.0, 1.0, 1.0, 0.99)
            } else {
                Vec4::ONE
            };
            level.push(StaticDraw {
                mesh: r.create_mesh(&builder.build()),
                texture: materials[material],
                tint,
                unlit: false,
            });
        }
        // Lava surface: unlit so it glows.
        let mut lava = MeshBuilder::new();
        lava.add_box(&BoxDesc {
            uv: BoxUv::texel_density(meshgen::DEFAULT_TEXELS_PER_METRE, size("lava")),
            faces: meshgen::FACE_POS_Y,
            ..BoxDesc::brush(Vec3::new(-11.0, 0.1, -2.0), Vec3::new(4.8, 0.1, 5.0), q)
        });
        level.push(StaticDraw {
            mesh: r.create_mesh(&lava.build()),
            texture: materials["lava"],
            tint: Vec4::ONE,
            unlit: true,
        });
        // Glass pane (translucent).
        let mut glass = MeshBuilder::new();
        glass.add_box(&BoxDesc {
            center: Vec3::new(-3.0, 1.6, -9.0),
            half_extents: Vec3::new(1.6, 1.6, 0.05),
            ..Default::default()
        });
        level.push(StaticDraw {
            mesh: r.create_mesh(&glass.build()),
            texture: materials["glass"],
            tint: Vec4::new(1.0, 1.0, 1.0, 0.99),
            unlit: false,
        });
        // Wax columns: a poured-wax cylinder wrapped by wax-stone rings.
        let mut wax = meshgen::cylinder(0.9, 6.0, 10, Vec4::ONE);
        wax.transform(Mat4::from_translation(Vec3::new(9.0, 3.0, 7.0)));
        level.push(StaticDraw {
            mesh: r.create_mesh(&wax.build()),
            texture: materials["wax"],
            tint: Vec4::ONE,
            unlit: false,
        });
        let mut ribs = MeshBuilder::new();
        for i in 0..4 {
            let ring = meshgen::cylinder(1.05, 0.25, 10, Vec4::ONE);
            ribs.append_builder(
                &ring,
                Mat4::from_translation(Vec3::new(9.0, 1.0 + i as f32 * 1.3, 7.0)),
            );
        }
        let mut horn = meshgen::cone(0.35, 1.6, 6, Vec4::ONE);
        horn.transform(Mat4::from_rotation_translation(
            Quat::from_rotation_z(-0.9),
            Vec3::new(10.2, 4.8, 7.0),
        ));
        ribs.append_builder(&horn, Mat4::IDENTITY);
        level.push(StaticDraw {
            mesh: r.create_mesh(&ribs.build()),
            texture: materials["waxstone"],
            tint: Vec4::ONE,
            unlit: false,
        });

        // ---- glTF model ----
        let path = engine::assets_dir().join("models/warden.gltf");
        let parts = match engine::assets::gltf::load(&path) {
            Ok(parts) => parts,
            Err(e) => {
                log::warn!(
                    "{e:#}; generating the model in memory (run render_gen_models to save it)"
                );
                models::warden_parts()
            }
        };
        let mut model_lo = Vec3::splat(f32::MAX);
        let mut model_hi = Vec3::splat(f32::MIN);
        let white = r.white_texture();
        let model = parts
            .iter()
            .map(|p| {
                let (lo, hi) = meshgen::bounds(&p.mesh.vertices);
                model_lo = model_lo.min(lo);
                model_hi = model_hi.max(hi);
                StaticDraw {
                    mesh: r.create_mesh(&p.mesh),
                    texture: p
                        .texture
                        .as_ref()
                        .map(|t| r.create_texture(t))
                        .unwrap_or(white),
                    tint: p.base_color,
                    unlit: false,
                }
            })
            .collect();

        // ---- viewmodel ----
        let (gun, gun_glow, muzzle) = models::demo_gun();
        let flash_quad = meshgen::quad(Vec2::splat(0.16), Vec4::ONE).build();
        let cube = meshgen::cuboid(Vec3::splat(0.25), Vec4::ONE).build();

        Scene {
            level,
            model,
            model_bounds: (model_lo, model_hi),
            pedestal_top: 1.0,
            gun: r.create_mesh(&gun),
            gun_glow: r.create_mesh(&gun_glow),
            gun_texture: materials["metal"],
            muzzle,
            flash_quad: r.create_mesh(&flash_quad),
            cube: r.create_mesh(&cube),
            materials,
            sprites,
            white,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Per-frame state

struct FlyCamera {
    position: Vec3,
    yaw: f32,
    pitch: f32,
}

impl FlyCamera {
    fn rotation(&self) -> Quat {
        Camera::rotation_from_angles(self.yaw, self.pitch, 0.0)
    }
}

#[derive(Default)]
struct FrameData {
    draws: Vec<DrawItem>,
    viewmodel: Vec<DrawItem>,
    sprites: Vec<Sprite>,
    decals: Vec<Decal>,
    lights: Vec<PointLight>,
    lines: Vec<DebugLine>,
    ui: UiDrawList,
}

struct Demo {
    scene: Scene,
    camera: FlyCamera,
    settings: RetroSettings,
    time: f32,
    stress: bool,
    data: FrameData,
    fps: f32,
    status: String,
}

fn hash01(i: u32, salt: u32) -> f32 {
    procedural::rand01(i as i32, salt as i32, 99)
}

impl Demo {
    fn new(renderer: &mut Renderer, settings: RetroSettings) -> Demo {
        Demo {
            scene: Scene::new(renderer),
            camera: FlyCamera {
                position: Vec3::new(0.0, 2.2, 13.0),
                yaw: 0.0,
                pitch: -0.1,
            },
            settings,
            time: 0.0,
            stress: false,
            data: FrameData::default(),
            fps: 0.0,
            status: String::new(),
        }
    }

    /// Fills `self.data` for the current time; returns the camera.
    fn build_frame(&mut self, window: (u32, u32)) -> Camera {
        let t = self.time;
        let s = &self.scene;
        let d = &mut self.data;
        d.draws.clear();
        d.viewmodel.clear();
        d.sprites.clear();
        d.decals.clear();
        d.lights.clear();
        d.lines.clear();

        for item in &s.level {
            d.draws.push(DrawItem {
                mesh: item.mesh,
                texture: item.texture,
                transform: Mat4::IDENTITY,
                tint: item.tint,
                unlit: item.unlit,
            });
        }
        let model_transform = Mat4::from_rotation_translation(
            Quat::from_rotation_y(t * 0.8) * Quat::from_rotation_x((t * 1.3).sin() * 0.15),
            Vec3::new(0.0, s.pedestal_top + 1.5 + (t * 2.0).sin() * 0.15, 0.0),
        );
        for item in &s.model {
            d.draws.push(DrawItem {
                mesh: item.mesh,
                texture: item.texture,
                transform: model_transform,
                tint: item.tint,
                unlit: item.unlit,
            });
        }

        // Lights (a glow sprite marks each one).
        let orbit = Vec3::new((t * 1.5).cos() * 3.0, 3.6, (t * 1.5).sin() * 3.0);
        let flicker = 0.85 + (t * 17.0).sin() * 0.08 + (t * 31.0).sin() * 0.07;
        for (position, color, radius) in [
            (
                Vec3::new(-11.0, 1.2, -2.0),
                Vec3::new(1.0, 0.45, 0.1) * flicker,
                10.0,
            ),
            (Vec3::new(13.0, 4.6, -12.0), Vec3::new(0.2, 0.8, 1.0), 11.0),
            (Vec3::new(-7.5, 2.8, 7.5), Vec3::new(1.0, 0.2, 0.75), 8.0),
            (orbit, Vec3::new(1.0, 0.9, 0.6), 7.0),
            (Vec3::new(7.5, 2.2, 6.0), Vec3::new(1.0, 0.15, 0.1), 7.0),
            (Vec3::new(-3.0, 2.5, -7.5), Vec3::new(0.3, 1.0, 0.4), 7.0),
        ] {
            d.lights.push(PointLight {
                position,
                color,
                radius,
            });
            d.sprites.push(Sprite {
                position,
                size: Vec2::splat(0.9),
                color: color.extend(1.0),
                texture: s.sprites[&SpriteTex::Glow],
                uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                rotation: 0.0,
                additive: true,
            });
        }

        // Molten wax spray with gravity (alpha), looping every 1.6 s.
        let burst_t = t % 1.6;
        let burst_origin = Vec3::new(3.0, 1.4, 4.0);
        for i in 0..14 {
            let dir = Vec3::new(
                hash01(i, 1) - 0.5,
                hash01(i, 2) * 0.8 + 0.2,
                hash01(i, 3) - 0.5,
            )
            .normalize()
                * (2.0 + hash01(i, 4) * 2.5);
            let p = burst_origin + dir * burst_t + Vec3::new(0.0, -4.9, 0.0) * burst_t * burst_t;
            if p.y < 0.05 {
                continue;
            }
            d.sprites.push(Sprite {
                position: p,
                size: Vec2::splat(0.22 + hash01(i, 5) * 0.2),
                color: Vec4::ONE,
                texture: s.sprites[&SpriteTex::WaxDrops],
                uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                rotation: hash01(i, 6) * TAU + t * 3.0,
                additive: false,
            });
        }
        // Wax lumps resting on the floor.
        for i in 0..4 {
            d.sprites.push(Sprite {
                position: Vec3::new(2.2 + i as f32 * 0.5, 0.2, 4.6 + hash01(i, 9) * 0.6),
                size: Vec2::splat(0.4),
                color: Vec4::ONE,
                texture: s.sprites[&SpriteTex::Lump],
                uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                rotation: hash01(i, 10) * TAU,
                additive: false,
            });
        }
        // Smoke rising from the lava.
        for i in 0..8 {
            let phase = (t * 0.25 + hash01(i, 11)) % 1.0;
            d.sprites.push(Sprite {
                position: Vec3::new(
                    -14.0 + hash01(i, 12) * 6.0,
                    0.5 + phase * 4.0,
                    -5.0 + hash01(i, 13) * 6.0,
                ),
                size: Vec2::splat(1.0 + phase * 1.5),
                color: Vec4::new(0.7, 0.6, 0.55, 1.0 - phase),
                texture: s.sprites[&SpriteTex::Smoke],
                uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                rotation: hash01(i, 14) * TAU + phase,
                additive: false,
            });
        }
        // Sparks + impact flash on the pedestal side (additive).
        for i in 0..10 {
            let a = hash01(i, 15) * TAU;
            let r = ((t * 3.0 + hash01(i, 16)) % 1.0) * 0.9;
            d.sprites.push(Sprite {
                position: Vec3::new(-1.6 + a.cos() * r, 0.8 + a.sin().abs() * r, 2.4),
                size: Vec2::splat(0.18),
                color: Vec4::ONE,
                texture: s.sprites[&SpriteTex::Spark],
                uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                rotation: a,
                additive: true,
            });
        }
        d.sprites.push(Sprite {
            position: Vec3::new(-1.6, 0.8, 2.4),
            size: Vec2::splat(0.9),
            color: Vec4::ONE,
            texture: s.sprites[&SpriteTex::MuzzleFlash],
            uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
            rotation: t * 9.0,
            additive: true,
        });
        // A spinning ricochet shard and a shockwave ring.
        d.sprites.push(Sprite {
            position: Vec3::new(4.5, 2.2, 1.0),
            size: Vec2::splat(0.7),
            color: Vec4::ONE,
            texture: s.sprites[&SpriteTex::Shard],
            uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
            rotation: t * 4.0,
            additive: false,
        });
        let ring_t = (t * 0.7) % 1.0;
        d.sprites.push(Sprite {
            position: Vec3::new(-4.0, 1.4, 4.0),
            size: Vec2::splat(0.5 + ring_t * 3.0),
            color: Vec4::new(1.0, 0.7, 0.3, 1.0 - ring_t),
            texture: s.sprites[&SpriteTex::Ring],
            uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
            rotation: 0.0,
            additive: true,
        });

        // Decals.
        for (position, normal, size, rotation, kind) in [
            (
                Vec3::new(3.0, 0.0, 4.4),
                Vec3::Y,
                2.6,
                0.4,
                SpriteTex::WaxSplat,
            ),
            (
                Vec3::new(-2.5, 0.0, 7.0),
                Vec3::Y,
                1.5,
                2.0,
                SpriteTex::WaxSplat,
            ),
            (
                Vec3::new(1.5, 0.0, 9.0),
                Vec3::Y,
                2.2,
                0.0,
                SpriteTex::Scorch,
            ),
            (
                Vec3::new(2.0, 2.2, -20.0),
                Vec3::Z,
                2.4,
                1.0,
                SpriteTex::WaxSplat,
            ),
            (
                Vec3::new(-20.0, 1.5, 6.0),
                Vec3::X,
                2.0,
                0.3,
                SpriteTex::Scorch,
            ),
        ] {
            d.decals.push(Decal {
                position,
                normal,
                size,
                rotation,
                color: Vec4::ONE,
                texture: s.sprites[&kind],
            });
        }

        // Debug lines: rotating bounding box of the model + an axis gizmo.
        let (lo, hi) = s.model_bounds;
        let corners: Vec<Vec3> = (0..8)
            .map(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
                );
                model_transform.transform_point3(c)
            })
            .collect();
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            d.lines.push(DebugLine {
                a: corners[a],
                b: corners[b],
                color: Vec4::new(1.0, 0.9, 0.2, 1.0),
            });
        }
        let o = Vec3::new(-2.0, 0.05, 10.0);
        for (axis, color) in [
            (Vec3::X, Vec4::new(1.0, 0.2, 0.2, 1.0)),
            (Vec3::Y, Vec4::new(0.2, 1.0, 0.2, 1.0)),
            (Vec3::Z, Vec4::new(0.3, 0.4, 1.0, 1.0)),
        ] {
            d.lines.push(DebugLine {
                a: o,
                b: o + axis,
                color,
            });
        }

        // Stress test: lots of extra draws and sprites.
        if self.stress {
            let textures: Vec<TextureId> = s.materials.values().copied().collect();
            for i in 0..2000u32 {
                let p = Vec3::new(
                    hash01(i, 20) * 36.0 - 18.0,
                    0.3 + hash01(i, 21) * 5.0,
                    hash01(i, 22) * 36.0 - 18.0,
                );
                d.draws.push(DrawItem {
                    mesh: s.cube,
                    texture: textures[i as usize % textures.len()],
                    transform: Mat4::from_rotation_translation(
                        Quat::from_rotation_y(t + i as f32),
                        p,
                    ),
                    tint: Vec4::ONE,
                    unlit: false,
                });
            }
            let kinds = [
                SpriteTex::Glow,
                SpriteTex::Smoke,
                SpriteTex::WaxDrops,
                SpriteTex::Spark,
            ];
            for i in 0..3000u32 {
                let p = Vec3::new(
                    hash01(i, 30) * 36.0 - 18.0,
                    0.5 + hash01(i, 31) * 5.0 + (t + i as f32).sin() * 0.2,
                    hash01(i, 32) * 36.0 - 18.0,
                );
                let kind = kinds[i as usize % kinds.len()];
                d.sprites.push(Sprite {
                    position: p,
                    size: Vec2::splat(0.3),
                    color: Vec4::ONE,
                    texture: s.sprites[&kind],
                    uv_rect: Vec4::new(0.0, 0.0, 1.0, 1.0),
                    rotation: t,
                    additive: matches!(kind, SpriteTex::Glow | SpriteTex::Spark),
                });
            }
        }

        // Viewmodel: bobbing gun, muzzle flash on a 2 Hz cycle.
        let bob = Vec3::new((t * 4.0).sin() * 0.006, (t * 8.0).sin().abs() * -0.006, 0.0);
        let gun_transform = Mat4::from_rotation_translation(
            Quat::from_rotation_y(0.06),
            Vec3::new(0.17, -0.165, -0.4) + bob,
        );
        d.viewmodel.push(DrawItem {
            mesh: s.gun,
            texture: s.gun_texture,
            transform: gun_transform,
            tint: Vec4::ONE,
            unlit: false,
        });
        d.viewmodel.push(DrawItem {
            mesh: s.gun_glow,
            texture: s.white,
            transform: gun_transform,
            tint: Vec4::ONE,
            unlit: true,
        });
        if (t * 2.0).fract() < 0.15 {
            let muzzle = gun_transform.transform_point3(s.muzzle);
            d.viewmodel.push(DrawItem {
                mesh: s.flash_quad,
                texture: s.sprites[&SpriteTex::MuzzleFlash],
                transform: Mat4::from_rotation_translation(Quat::from_rotation_z(t * 20.0), muzzle),
                tint: Vec4::new(1.0, 1.0, 1.0, 0.99),
                unlit: true,
            });
        }

        // UI: title, settings, crosshair, health bar.
        let canvas = UiDrawList::canvas_for_window(window.0, window.1);
        d.ui = UiDrawList::new(canvas);
        let ui = &mut d.ui;
        ui.rect(
            Vec2::new(4.0, 4.0),
            Vec2::new(200.0, 30.0),
            Vec4::new(0.0, 0.0, 0.0, 0.55),
        );
        text(
            ui,
            Vec2::new(8.0, 8.0),
            1.0,
            Vec4::new(1.0, 0.85, 0.3, 1.0),
            "VELOCIDE RENDER DEMO",
        );
        text(ui, Vec2::new(8.0, 19.0), 1.0, Vec4::ONE, &self.status);
        let c = canvas * 0.5;
        ui.rect(
            c - Vec2::new(6.0, 0.5),
            c + Vec2::new(6.0, 0.5),
            Vec4::new(1.0, 1.0, 1.0, 0.8),
        );
        ui.rect(
            c - Vec2::new(0.5, 6.0),
            c + Vec2::new(0.5, 6.0),
            Vec4::new(1.0, 1.0, 1.0, 0.8),
        );
        let bar = Vec2::new(10.0, canvas.y - 22.0);
        ui.rect(
            bar,
            bar + Vec2::new(122.0, 12.0),
            Vec4::new(0.0, 0.0, 0.0, 0.6),
        );
        ui.rect(
            bar + Vec2::splat(1.0),
            bar + Vec2::new(1.0 + 120.0 * 0.7, 11.0),
            Vec4::new(0.85, 0.1, 0.12, 1.0),
        );
        ui.rect(
            bar + Vec2::new(85.0, 1.0),
            bar + Vec2::new(105.0, 11.0),
            Vec4::new(0.4, 0.05, 0.08, 1.0),
        );
        text(ui, bar + Vec2::new(2.0, -10.0), 1.0, Vec4::ONE, "HP 70");
        // A texture swatch drawn through the UI (UiTexture::Texture).
        let sw = Vec2::new(canvas.x - 40.0, canvas.y - 40.0);
        ui.quads.push(UiQuad {
            min: sw,
            max: sw + Vec2::splat(32.0),
            uv_min: Vec2::ZERO,
            uv_max: Vec2::ONE,
            color: Vec4::ONE,
            texture: UiTexture::Texture(s.materials["hazard"]),
        });

        Camera {
            position: self.camera.position,
            rotation: self.camera.rotation(),
            fov_y_degrees: 80.0,
            near: 0.05,
            far: 400.0,
        }
    }

    fn frame_desc<'a>(&'a self, camera: Camera) -> FrameDesc<'a> {
        let fog = Fog {
            color: Vec3::new(0.2, 0.07, 0.06),
            start: 14.0,
            end: 60.0,
        };
        // Damage flash every 3 s, decaying over 0.4 s.
        let flash = (1.0 - (self.time % 3.0) / 0.4).max(0.0) * 0.45;
        FrameDesc {
            camera,
            clear_color: fog.color,
            fog,
            ambient: Vec3::new(0.3, 0.27, 0.32),
            light_dir: Vec3::new(0.4, -1.0, 0.3),
            light_color: Vec3::new(0.55, 0.5, 0.45),
            lights: &self.data.lights,
            draws: &self.data.draws,
            sprites: &self.data.sprites,
            decals: &self.data.decals,
            viewmodel: &self.data.viewmodel,
            viewmodel_fov_y_degrees: 62.0,
            debug_lines: &self.data.lines,
            screen_flash: Vec4::new(1.0, 0.1, 0.05, flash),
            ui: &self.data.ui,
        }
    }

    fn update_status(&mut self) {
        let s = &self.settings;
        let on = |b: bool| if b { "ON" } else { "OFF" };
        self.status = format!(
            "FPS {:.0} SNAP {} AFF {} DITH {} BITS {} FOG {} LIT {} RES {}",
            self.fps,
            on(s.vertex_snap),
            on(s.affine_textures),
            on(s.dithering),
            s.color_bits,
            on(s.fog),
            on(s.vertex_lighting),
            s.internal_height
        );
    }
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

// ---------------------------------------------------------------------------------------------
// Windowed mode

/// Options for windowed mode that `Game::init` can't receive as parameters.
#[derive(Clone, Default)]
struct WindowOptions {
    /// Capture this path after `quit_after / 2` frames.
    capture: Option<PathBuf>,
    /// Quit after this many frames (0 = never).
    quit_after: u64,
}

static WINDOW_OPTIONS: std::sync::OnceLock<WindowOptions> = std::sync::OnceLock::new();

struct DemoGame {
    demo: Demo,
    fps_timer: f32,
    fps_frames: u32,
    frame_count: u64,
    options: WindowOptions,
    #[cfg(feature = "debug-ui")]
    debug: engine::debug_ui::DebugUi,
    #[cfg(feature = "debug-ui")]
    show_debug: bool,
}

impl Game for DemoGame {
    fn init(ctx: &mut EngineContext) -> anyhow::Result<Self> {
        let settings = ctx.renderer.settings().clone();
        ctx.renderer.set_shader_hot_reload(true);
        Ok(Self {
            demo: Demo::new(&mut ctx.renderer, settings),
            fps_timer: 0.0,
            fps_frames: 0,
            frame_count: 0,
            options: WINDOW_OPTIONS.get().cloned().unwrap_or_default(),
            #[cfg(feature = "debug-ui")]
            debug: engine::debug_ui::DebugUi::new(ctx),
            #[cfg(feature = "debug-ui")]
            show_debug: true,
        })
    }

    #[cfg(feature = "debug-ui")]
    fn window_event(&mut self, ctx: &mut EngineContext, event: &winit::event::WindowEvent) -> bool {
        // While mouselooking the overlay must not steal input.
        !ctx.cursor_grabbed() && self.show_debug && self.debug.on_window_event(&ctx.window, event)
    }

    fn frame(&mut self, ctx: &mut EngineContext) {
        self.frame_count += 1;
        let quit_after = self.options.quit_after;
        if quit_after > 0 {
            if self.frame_count == quit_after / 2
                && let Some(path) = &self.options.capture
            {
                ctx.renderer.capture_png(path);
            }
            if self.frame_count >= quit_after {
                ctx.quit();
            }
        }
        let demo = &mut self.demo;
        let dt = ctx.frame_dt;

        if ctx.input.mouse_pressed(MouseButton::Left) && !ctx.cursor_grabbed() {
            ctx.set_cursor_grab(true);
        }
        if ctx.input.key_pressed(Key::Escape) {
            ctx.set_cursor_grab(false);
        }
        if ctx.input.key_pressed(Key::F11) {
            let fs = !ctx.is_fullscreen();
            ctx.set_fullscreen(fs);
        }
        let input = &ctx.input;
        if ctx.cursor_grabbed() {
            let delta = input.mouse_delta() * 0.0025;
            demo.camera.yaw -= delta.x;
            demo.camera.pitch = (demo.camera.pitch - delta.y).clamp(-1.55, 1.55);
        }
        let look = input.right_stick() * dt * 2.5;
        demo.camera.yaw -= look.x;
        demo.camera.pitch = (demo.camera.pitch + look.y).clamp(-1.55, 1.55);

        let rot = Quat::from_rotation_y(demo.camera.yaw);
        let mut wish = Vec3::ZERO;
        for (key, dir) in [
            (Key::KeyW, -Vec3::Z),
            (Key::KeyS, Vec3::Z),
            (Key::KeyA, -Vec3::X),
            (Key::KeyD, Vec3::X),
            (Key::Space, Vec3::Y),
            (Key::ControlLeft, -Vec3::Y),
        ] {
            if input.key_down(key) {
                wish += if dir.y != 0.0 { dir } else { rot * dir };
            }
        }
        let stick = input.left_stick();
        wish += rot * Vec3::new(stick.x, 0.0, -stick.y);
        let speed = if input.key_down(Key::ShiftLeft) {
            18.0
        } else {
            6.0
        };
        demo.camera.position += wish.clamp_length_max(1.0) * speed * dt;

        // Effect toggles.
        let s = &mut demo.settings;
        let mut changed = true;
        if input.key_pressed(Key::Digit1) {
            s.vertex_snap = !s.vertex_snap;
        } else if input.key_pressed(Key::Digit2) {
            s.affine_textures = !s.affine_textures;
        } else if input.key_pressed(Key::Digit3) {
            s.dithering = !s.dithering;
        } else if input.key_pressed(Key::Digit4) {
            s.color_bits = match s.color_bits {
                5 => 4,
                4 => 3,
                3 => 8,
                _ => 5,
            };
        } else if input.key_pressed(Key::Digit5) {
            s.fog = !s.fog;
        } else if input.key_pressed(Key::Digit6) {
            s.vertex_lighting = !s.vertex_lighting;
        } else if input.key_pressed(Key::Digit7) {
            s.internal_height = match s.internal_height {
                360 => 240,
                240 => 480,
                480 => 0,
                _ => 360,
            };
        } else if input.key_pressed(Key::Digit8) {
            s.vsync = !s.vsync;
        } else if input.key_pressed(Key::Digit9) {
            s.vertex_snap_strength = match s.vertex_snap_strength as u32 {
                1 => 2.0,
                2 => 4.0,
                _ => 1.0,
            };
        } else {
            changed = false;
        }
        if input.key_pressed(Key::Digit0) {
            demo.stress = !demo.stress;
        }
        if changed {
            ctx.renderer.apply_settings(&demo.settings);
        }
        if input.key_pressed(Key::F5)
            && let Err(e) = ctx.renderer.reload_shaders()
        {
            log::error!("{e}");
        }
        if input.key_pressed(Key::F2) {
            let path = PathBuf::from(format!("captures/render_demo_{}.png", ctx.time as u64));
            ctx.renderer.capture_png(&path);
        }

        self.fps_timer += dt;
        self.fps_frames += 1;
        if self.fps_timer >= 0.5 {
            demo.fps = self.fps_frames as f32 / self.fps_timer;
            self.fps_timer = 0.0;
            self.fps_frames = 0;
        }
        demo.update_status();
        demo.time += dt;
        let camera = demo.build_frame(ctx.renderer.window_size());

        #[cfg(feature = "debug-ui")]
        let overlay: Option<&mut dyn engine::render::OverlayPainter> = {
            if ctx.input.key_pressed(Key::F1) {
                self.show_debug = !self.show_debug;
            }
            if self.show_debug {
                let stats = ctx.renderer.stats();
                let adapter = ctx.renderer.adapter_name();
                let shader_error = ctx.renderer.shader_error().map(str::to_owned);
                let mut settings = demo.settings.clone();
                let fps = demo.fps;
                self.debug.run(&ctx.window, |ui_ctx| {
                    use engine::debug_ui::egui;
                    egui::Window::new("Renderer (F1)")
                        .default_pos([10.0, 80.0])
                        .show(ui_ctx, |ui| {
                            ui.label(format!("{adapter}  {fps:.0} fps"));
                            ui.label(format!(
                                "internal {:?}  draws {} (culled {})  sprites {}  tris {}",
                                stats.internal_size,
                                stats.draws,
                                stats.culled,
                                stats.sprites,
                                stats.triangles
                            ));
                            ui.checkbox(&mut settings.vertex_snap, "vertex snap");
                            ui.add(
                                egui::Slider::new(&mut settings.vertex_snap_strength, 1.0..=8.0)
                                    .text("snap strength"),
                            );
                            ui.checkbox(&mut settings.affine_textures, "affine textures");
                            ui.checkbox(&mut settings.dithering, "dithering");
                            ui.add(
                                egui::Slider::new(&mut settings.color_bits, 2..=8)
                                    .text("colour bits"),
                            );
                            ui.checkbox(&mut settings.fog, "fog");
                            ui.checkbox(&mut settings.vertex_lighting, "point lights");
                            ui.add(
                                egui::Slider::new(&mut settings.internal_height, 0..=1080)
                                    .text("internal height"),
                            );
                            if let Some(e) = &shader_error {
                                ui.colored_label(egui::Color32::RED, e);
                            }
                        });
                });
                if settings != demo.settings {
                    demo.settings = settings;
                    ctx.renderer.apply_settings(&demo.settings);
                }
                Some(&mut self.debug)
            } else {
                None
            }
        };
        #[cfg(not(feature = "debug-ui"))]
        let overlay: Option<&mut dyn engine::render::OverlayPainter> = None;

        let frame = demo.frame_desc(camera);
        if let Err(e) = ctx.renderer.render(&frame, overlay) {
            log::error!("render failed: {e:#}");
            ctx.quit();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Headless capture / benchmark

struct Args {
    capture: Option<PathBuf>,
    window: WindowOptions,
    bench: Option<u32>,
    frames: u32,
    width: u32,
    height: u32,
    stress: bool,
    camera: Option<[f32; 5]>,
    settings: RetroSettings,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        capture: None,
        window: WindowOptions::default(),
        bench: None,
        frames: 30,
        width: 1280,
        height: 720,
        stress: false,
        camera: None,
        settings: RetroSettings::default(),
    };
    let mut it = std::env::args().skip(1);
    let value = |name: &str, it: &mut dyn Iterator<Item = String>| -> anyhow::Result<String> {
        it.next()
            .ok_or_else(|| anyhow::anyhow!("{name} needs a value"))
    };
    while let Some(arg) = it.next() {
        let s = &mut args.settings;
        match arg.as_str() {
            "--capture" => args.capture = Some(value(&arg, &mut it)?.into()),
            "--window-capture" => args.window.capture = Some(value(&arg, &mut it)?.into()),
            "--quit-after" => args.window.quit_after = value(&arg, &mut it)?.parse()?,
            "--bench" => args.bench = Some(value(&arg, &mut it)?.parse()?),
            "--frames" => args.frames = value(&arg, &mut it)?.parse()?,
            "--width" => args.width = value(&arg, &mut it)?.parse()?,
            "--height" => args.height = value(&arg, &mut it)?.parse()?,
            "--res" => s.internal_height = value(&arg, &mut it)?.parse()?,
            "--no-snap" => s.vertex_snap = false,
            "--snap-strength" => s.vertex_snap_strength = value(&arg, &mut it)?.parse()?,
            "--affine" => s.affine_textures = true,
            "--no-dither" => s.dithering = false,
            "--bits" => s.color_bits = value(&arg, &mut it)?.parse()?,
            "--no-fog" => s.fog = false,
            "--no-lighting" => s.vertex_lighting = false,
            "--stress" => args.stress = true,
            "--wall" => args.camera = Some([16.5, 1.7, -2.0, -90.0, 0.0]),
            "--cam" => {
                let v: Vec<f32> = value(&arg, &mut it)?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
                anyhow::ensure!(v.len() == 5, "--cam wants x,y,z,yaw_deg,pitch_deg");
                args.camera = Some([v[0], v[1], v[2], v[3], v[4]]);
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    Ok(args)
}

fn run_headless(args: Args) -> anyhow::Result<()> {
    let mut renderer = Renderer::new_headless(args.width, args.height, &args.settings)?;
    let mut demo = Demo::new(&mut renderer, args.settings.clone());
    demo.stress = args.stress;
    if let Some([x, y, z, yaw, pitch]) = args.camera {
        demo.camera = FlyCamera {
            position: Vec3::new(x, y, z),
            yaw: yaw.to_radians(),
            pitch: pitch.to_radians(),
        };
    }
    demo.update_status();

    if let Some(frames) = args.bench {
        // Warm up, then time frames including GPU completion.
        let mut times = Vec::with_capacity(frames as usize);
        let (mut build_ms, mut render_ms) = (0.0, 0.0);
        for i in 0..frames + 20 {
            let start = Instant::now();
            demo.time += FIXED_DT;
            let camera = demo.build_frame((args.width, args.height));
            let built = Instant::now();
            renderer.render(&demo.frame_desc(camera), None)?;
            let rendered = Instant::now();
            renderer
                .device()
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            if i >= 20 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                build_ms += (built - start).as_secs_f64() * 1000.0;
                render_ms += (rendered - built).as_secs_f64() * 1000.0;
            }
        }
        println!(
            "per frame: demo scene build {:.3} ms, Renderer::render (CPU) {:.3} ms",
            build_ms / frames.max(1) as f64,
            render_ms / frames.max(1) as f64
        );
        times.sort_by(f64::total_cmp);
        let avg = times.iter().sum::<f64>() / times.len().max(1) as f64;
        let p99 = times.get(times.len() * 99 / 100).copied().unwrap_or(avg);
        let stats = renderer.stats();
        println!(
            "{} frames at {}x{} (internal {:?}): avg {avg:.3} ms ({:.0} fps), p99 {p99:.3} ms | draws {} culled {} sprites {} tris {}",
            times.len(),
            args.width,
            args.height,
            stats.internal_size,
            1000.0 / avg,
            stats.draws,
            stats.culled,
            stats.sprites,
            stats.triangles
        );
        return Ok(());
    }

    let path = args
        .capture
        .unwrap_or_else(|| PathBuf::from("captures/render_demo.png"));
    for i in 0..args.frames.max(1) {
        demo.time += FIXED_DT;
        let camera = demo.build_frame((args.width, args.height));
        if i + 1 == args.frames.max(1) {
            renderer.capture_png(&path);
        }
        renderer.render(&demo.frame_desc(camera), None)?;
    }
    let stats = renderer.stats();
    println!("captured {} | {stats:?}", path.display());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = parse_args()?;
    if args.capture.is_some() || args.bench.is_some() {
        return run_headless(args);
    }
    let mut window = args.window.clone();
    if window.capture.is_some() && window.quit_after == 0 {
        window.quit_after = 120;
    }
    let _ = WINDOW_OPTIONS.set(window);
    engine::app::run::<DemoGame>(AppConfig {
        title: "VELOCIDE render demo".into(),
        retro: args.settings,
        ..Default::default()
    })
}
