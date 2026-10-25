//! Turns a [`LevelDef`] into renderable geometry.
//!
//! Brushes are batched into one mesh per material (tint baked into vertex colours, world-space
//! UVs so neighbouring brushes line up). Doors get their own mesh each so they can be hidden
//! when open. glTF props are loaded through `engine::assets::gltf` and cached per path.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use engine::assets::meshgen::{BoxDesc, MeshBuilder};
use engine::assets::{gltf, procedural};
use engine::gfx::{DrawItem, Fog, MeshData, PointLight, TextureId};
use engine::render::Renderer;
use glam::{EulerRot, Mat4, Quat, Vec3, Vec4};
use shared::level::{Brush, LevelDef};

/// Materials drawn fullbright (they glow).
pub fn material_is_unlit(material: &str) -> bool {
    matches!(material, "lava")
}

/// Opacity of a material (translucent materials are drawn in the translucent pass).
pub fn material_alpha(material: &str) -> f32 {
    if material == "glass" { 0.45 } else { 1.0 }
}

/// Largest glTF file a level prop may load.
pub const MAX_PROP_MODEL_BYTES: u64 = 16 * 1024 * 1024;

/// Resolves a level prop's `model` (a path relative to the assets folder) to a file inside
/// `assets`. Levels come from servers, so the path must be relative and made only of plain
/// names (no `..`, `.`, root, drive or `\`), end in `.gltf` / `.glb`, stay inside the
/// assets folder after resolving symlinks, and name a regular file of at most
/// [`MAX_PROP_MODEL_BYTES`]. Anything else (device files, pipes, network paths, files
/// elsewhere on disk) is refused with the reason.
pub fn resolve_prop_model(assets: &Path, model: &str) -> Result<PathBuf, String> {
    let rel = Path::new(model);
    if model.is_empty() || model.contains(['\\', ':', '\0']) {
        return Err("not a plain relative path".into());
    }
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err("must be a relative path inside assets/ without '.' or '..'".into());
    }
    let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("gltf") && !ext.eq_ignore_ascii_case("glb") {
        return Err("not a .gltf / .glb file".into());
    }
    let root = assets
        .canonicalize()
        .map_err(|e| format!("assets folder {}: {e}", assets.display()))?;
    let path = root.join(rel).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("resolves outside the assets folder".into());
    }
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_PROP_MODEL_BYTES {
        return Err(format!("larger than {MAX_PROP_MODEL_BYTES} bytes"));
    }
    Ok(path)
}

/// CPU-side level geometry (no GPU resources), useful for tools and tests.
#[derive(Default)]
pub struct LevelGeometry {
    /// One mesh per material.
    pub batches: Vec<(String, MeshData)>,
    /// (door id, material, mesh).
    pub doors: Vec<(u32, String, MeshData)>,
    /// Glowing pads marking jump pad triggers.
    pub jump_pads: MeshData,
}

fn brush_box(brush: &Brush) -> BoxDesc {
    let mut desc = BoxDesc::brush(brush.center(), brush.half_extents(), brush.rotation_quat());
    desc.color = brush.tint.extend(1.0);
    desc
}

pub fn build_geometry(level: &LevelDef) -> LevelGeometry {
    let mut by_material: BTreeMap<&str, MeshBuilder> = BTreeMap::new();
    for brush in &level.brushes {
        if brush.clip_only {
            continue;
        }
        by_material
            .entry(brush.material.as_str())
            .or_default()
            .add_box(&brush_box(brush));
    }
    let batches = by_material
        .into_iter()
        .filter(|(_, b)| !b.is_empty())
        .map(|(m, b)| (m.to_owned(), b.build()))
        .collect();

    let doors = level
        .doors
        .iter()
        .map(|door| {
            let mut b = MeshBuilder::new();
            b.add_box(&brush_box(&door.brush));
            (door.id, door.brush.material.clone(), b.build())
        })
        .collect();

    let mut pads = MeshBuilder::new();
    for pad in &level.jump_pads {
        let center = Vec3::new(
            (pad.trigger.min.x + pad.trigger.max.x) * 0.5,
            pad.trigger.min.y + 0.05,
            (pad.trigger.min.z + pad.trigger.max.z) * 0.5,
        );
        let half = Vec3::new(
            (pad.trigger.max.x - pad.trigger.min.x) * 0.5,
            0.05,
            (pad.trigger.max.z - pad.trigger.min.z) * 0.5,
        );
        pads.add_box(&BoxDesc {
            center,
            half_extents: half,
            color: Vec4::new(0.3, 1.0, 0.9, 1.0),
            ..BoxDesc::brush(center, half, Quat::IDENTITY)
        });
    }

    LevelGeometry {
        batches,
        doors,
        jump_pads: pads.build(),
    }
}

/// GPU resources for a level.
pub struct LevelRender {
    items: Vec<DrawItem>,
    doors: Vec<(u32, DrawItem)>,
    meshes: Vec<engine::gfx::MeshId>,
    textures: Vec<TextureId>,
    lights: Vec<PointLight>,
}

impl LevelRender {
    pub fn new(renderer: &mut Renderer, level: &LevelDef) -> Self {
        let geometry = build_geometry(level);
        let mut textures: HashMap<String, TextureId> = HashMap::new();
        let mut owned_textures = Vec::new();
        let mut meshes = Vec::new();
        let mut texture_for = |renderer: &mut Renderer, material: &str| -> TextureId {
            *textures.entry(material.to_owned()).or_insert_with(|| {
                let id = renderer.create_texture(&procedural::material_texture(material));
                owned_textures.push(id);
                id
            })
        };
        let item = |mesh, texture, material: &str| DrawItem {
            mesh,
            texture,
            transform: Mat4::IDENTITY,
            tint: Vec4::new(1.0, 1.0, 1.0, material_alpha(material)),
            unlit: material_is_unlit(material),
        };

        let mut items = Vec::new();
        for (material, mesh) in &geometry.batches {
            let texture = texture_for(renderer, material);
            let mesh = renderer.create_mesh(mesh);
            meshes.push(mesh);
            items.push(item(mesh, texture, material));
        }
        let mut doors = Vec::new();
        for (id, material, mesh) in &geometry.doors {
            let texture = texture_for(renderer, material);
            let mesh = renderer.create_mesh(mesh);
            meshes.push(mesh);
            doors.push((*id, item(mesh, texture, material)));
        }
        if !geometry.jump_pads.indices.is_empty() {
            let mesh = renderer.create_mesh(&geometry.jump_pads);
            meshes.push(mesh);
            let texture = texture_for(renderer, "panel");
            items.push(DrawItem {
                unlit: true,
                ..item(mesh, texture, "panel")
            });
        }

        // glTF props, loaded once per distinct path.
        let mut prop_cache: HashMap<String, Vec<(engine::gfx::MeshId, TextureId, Vec4)>> =
            HashMap::new();
        let assets = engine::assets_dir();
        for prop in &level.props {
            let parts = prop_cache.entry(prop.model.clone()).or_insert_with(|| {
                // The level may come from a server: only files inside assets/ are read.
                let path = match resolve_prop_model(&assets, &prop.model) {
                    Ok(path) => path,
                    Err(err) => {
                        log::warn!("level prop {:?} skipped: {err}", prop.model);
                        return Vec::new();
                    }
                };
                match gltf::load(&path) {
                    Ok(parts) => parts
                        .into_iter()
                        .map(|part| {
                            let mesh = renderer.create_mesh(&part.mesh);
                            meshes.push(mesh);
                            let texture = match &part.texture {
                                Some(t) => {
                                    let id = renderer.create_texture(t);
                                    owned_textures.push(id);
                                    id
                                }
                                None => renderer.white_texture(),
                            };
                            (mesh, texture, part.base_color)
                        })
                        .collect(),
                    Err(err) => {
                        log::warn!("level prop {}: {err}", path.display());
                        Vec::new()
                    }
                }
            });
            let rotation = Quat::from_euler(
                EulerRot::YXZ,
                prop.rotation.y.to_radians(),
                prop.rotation.x.to_radians(),
                prop.rotation.z.to_radians(),
            );
            let transform = Mat4::from_scale_rotation_translation(
                Vec3::splat(prop.scale),
                rotation,
                prop.position,
            );
            for &(mesh, texture, color) in parts.iter() {
                items.push(DrawItem {
                    mesh,
                    texture,
                    transform,
                    tint: color,
                    unlit: false,
                });
            }
        }

        let lights = level
            .lights
            .iter()
            .map(|l| PointLight {
                position: l.position,
                color: l.color,
                radius: l.radius,
            })
            .collect();

        Self {
            items,
            doors,
            meshes,
            textures: owned_textures,
            lights,
        }
    }

    /// Appends the level's draw items; doors are drawn only while `door_open(id)` is false.
    pub fn draw(&self, door_open: impl Fn(u32) -> bool, out: &mut Vec<DrawItem>) {
        out.extend_from_slice(&self.items);
        out.extend(
            self.doors
                .iter()
                .filter(|(id, _)| !door_open(*id))
                .map(|(_, item)| *item),
        );
    }

    pub fn lights(&self) -> &[PointLight] {
        &self.lights
    }

    /// Releases GPU resources (call when changing levels).
    pub fn free(self, renderer: &mut Renderer) {
        for mesh in self.meshes {
            renderer.free_mesh(mesh);
        }
        for texture in self.textures {
            renderer.free_texture(texture);
        }
    }
}

/// Scene lighting/fog values from the level's environment.
pub struct SceneEnvironment {
    pub clear_color: Vec3,
    pub fog: Fog,
    pub ambient: Vec3,
    pub light_dir: Vec3,
    pub light_color: Vec3,
}

pub fn environment(level: &LevelDef) -> SceneEnvironment {
    let env = &level.environment;
    SceneEnvironment {
        clear_color: env.sky_color,
        fog: Fog {
            color: env.fog_color,
            start: env.fog_start,
            end: env.fog_end,
        },
        ambient: env.ambient,
        light_dir: env.light_dir.normalize_or(Vec3::NEG_Y),
        light_color: env.light_color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_level_builds_geometry() {
        let level = shared::sim::builtin_level(shared::sim::BUILTIN_LEVEL).expect("builtin level");
        let geometry = build_geometry(&level);
        assert!(!geometry.batches.is_empty());
        for (_, mesh) in &geometry.batches {
            assert!(!mesh.indices.is_empty());
            assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()));
        }
        assert_eq!(geometry.doors.len(), level.doors.len());
    }

    #[test]
    fn prop_models_stay_inside_the_assets_folder() {
        let base = std::env::temp_dir().join(format!("velocide-props-{}", std::process::id()));
        let assets = base.join("assets");
        let models = assets.join("models");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(models.join("dir.glb")).unwrap();
        std::fs::write(models.join("crate.glb"), b"glTF").unwrap();
        std::fs::write(models.join("Crate.GLTF"), b"{}").unwrap();
        std::fs::write(models.join("notes.txt"), b"hi").unwrap();
        std::fs::write(base.join("outside.glb"), b"glTF").unwrap();
        let big = std::fs::File::create(models.join("big.glb")).unwrap();
        big.set_len(MAX_PROP_MODEL_BYTES + 1).unwrap(); // sparse
        drop(big);

        let ok = |m: &str| resolve_prop_model(&assets, m);
        let expected = models.join("crate.glb").canonicalize().unwrap();
        assert_eq!(ok("models/crate.glb"), Ok(expected));
        assert!(ok("models/Crate.GLTF").is_ok());
        for evil in [
            "",
            "/etc/passwd",
            "/dev/zero",
            "/dev/zero.glb",
            "../outside.glb",
            "models/../../outside.glb",
            "models/../models/crate.glb",
            "./models/crate.glb",
            "models//../crate.glb",
            "models\\..\\..\\outside.glb",
            "\\\\server\\share\\x.glb",
            "C:/Windows/x.glb",
            "C:x.glb",
            "models/notes.txt",
            "models/crate",
            "models/missing.glb",
            "models/dir.glb",
            "models/big.glb",
        ] {
            assert!(ok(evil).is_err(), "{evil:?} must be refused");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("outside.glb"), models.join("link.glb")).unwrap();
            std::os::unix::fs::symlink("/dev/zero", models.join("zero.glb")).unwrap();
            assert!(ok("models/link.glb").unwrap_err().contains("outside"));
            assert!(ok("models/zero.glb").is_err());
        }
        std::fs::remove_dir_all(&base).unwrap();
    }
}
