//! glTF 2.0 models: a loader (node transforms applied, embedded base64 or external `.bin`
//! buffers, PNG/JPEG images; from a file, from bytes, or from the assets folder with its
//! embedded fallback) and a minimal exporter that writes a self-contained `.gltf` (JSON with
//! base64 data URIs) so procedurally generated models can be saved and reloaded.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context as _;
use glam::{Mat3, Mat4, Vec3, Vec4};

use crate::gfx::{MeshData, TextureData, Vertex};

/// One drawable piece of a model: a mesh with a single material.
#[derive(Clone, Debug, Default)]
pub struct ModelPart {
    pub name: String,
    pub mesh: MeshData,
    /// Base colour texture (RGBA8 sRGB), if the material has one.
    pub texture: Option<TextureData>,
    /// Material base colour factor; use it as the draw tint.
    pub base_color: Vec4,
}

/// Loads every triangle primitive of the default scene, with node transforms baked into the
/// vertices. Primitives without normals get flat normals.
pub fn load(path: &Path) -> anyhow::Result<Vec<ModelPart>> {
    let (doc, buffers, images) =
        ::gltf::import(path).with_context(|| format!("loading {}", path.display()))?;
    parts_of(&doc, &buffers, &images)
}

/// Like [`load`] from the bytes of a `.gltf` / `.glb` whose buffers and images are embedded
/// (data URIs or the GLB binary chunk); external files can't be resolved.
pub fn load_slice(bytes: &[u8]) -> anyhow::Result<Vec<ModelPart>> {
    let ::gltf::Gltf { document, blob } = ::gltf::Gltf::from_slice(bytes)?;
    let buffers = ::gltf::import_buffers(&document, None, blob)?;
    // `gltf::import_slice` refuses images with `data:` URIs (it only decodes them when it has
    // a folder for relative URIs): give those a folder, which a data URI never reads from.
    // Any other URI still fails.
    let images = document
        .images()
        .map(|image| {
            let source = image.source();
            let base = match source {
                ::gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
                    Some(Path::new("."))
                }
                _ => None,
            };
            ::gltf::image::Data::from_source(source, base, &buffers)
        })
        .collect::<Result<Vec<_>, _>>()?;
    parts_of(&document, &buffers, &images)
}

/// Loads a model of the assets folder (`models/shrike.gltf`) through
/// [`crate::asset_files`]: the file on disk if there is one, else the copy compiled into the
/// binary.
pub fn load_asset(path: &str) -> anyhow::Result<Vec<ModelPart>> {
    match crate::asset_files::find(path) {
        Some(crate::asset_files::Source::Disk(file)) => load(&file),
        Some(crate::asset_files::Source::Embedded(bytes)) => {
            load_slice(bytes).with_context(|| format!("loading embedded {path}"))
        }
        None => anyhow::bail!("{path} not found (neither in the assets folder nor embedded)"),
    }
}

fn parts_of(
    doc: &::gltf::Document,
    buffers: &[::gltf::buffer::Data],
    images: &[::gltf::image::Data],
) -> anyhow::Result<Vec<ModelPart>> {
    let mut parts = Vec::new();
    let mut ctx = LoadCtx {
        buffers,
        images,
        parts: &mut parts,
    };
    match doc.default_scene().or_else(|| doc.scenes().next()) {
        Some(scene) => {
            for node in scene.nodes() {
                ctx.visit(&node, Mat4::IDENTITY)?;
            }
        }
        None => {
            for mesh in doc.meshes() {
                ctx.add_mesh(&mesh, Mat4::IDENTITY, "")?;
            }
        }
    }
    Ok(parts)
}

struct LoadCtx<'a> {
    buffers: &'a [::gltf::buffer::Data],
    images: &'a [::gltf::image::Data],
    parts: &'a mut Vec<ModelPart>,
}

impl LoadCtx<'_> {
    fn visit(&mut self, node: &::gltf::Node, parent: Mat4) -> anyhow::Result<()> {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            self.add_mesh(&mesh, world, node.name().unwrap_or(""))?;
        }
        for child in node.children() {
            self.visit(&child, world)?;
        }
        Ok(())
    }

    fn add_mesh(
        &mut self,
        mesh: &::gltf::Mesh,
        world: Mat4,
        node_name: &str,
    ) -> anyhow::Result<()> {
        let name = mesh.name().unwrap_or(node_name).to_owned();
        for prim in mesh.primitives() {
            if prim.mode() != ::gltf::mesh::Mode::Triangles {
                log::warn!(
                    "glTF mesh '{name}': skipping non-triangle primitive ({:?})",
                    prim.mode()
                );
                continue;
            }
            let reader = prim.reader(|b| self.buffers.get(b.index()).map(|d| d.0.as_slice()));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let count = positions.len();
            let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|t| t.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0; 2]; count]);
            let colors: Vec<[f32; 4]> = reader
                .read_colors(0)
                .map(|c| c.into_rgba_f32().collect())
                .unwrap_or_else(|| vec![[1.0; 4]; count]);
            let mut indices: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..count as u32).collect(),
            };
            anyhow::ensure!(
                uvs.len() == count
                    && colors.len() == count
                    && normals.as_ref().is_none_or(|n| n.len() == count),
                "glTF mesh '{name}': attribute counts differ"
            );
            anyhow::ensure!(
                indices.len().is_multiple_of(3) && indices.iter().all(|&i| (i as usize) < count),
                "glTF mesh '{name}': invalid indices"
            );

            let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
            if world.determinant() < 0.0 {
                for tri in indices.chunks_exact_mut(3) {
                    tri.swap(1, 2);
                }
            }
            let mut vertices: Vec<Vertex> = (0..count)
                .map(|i| Vertex {
                    position: world.transform_point3(Vec3::from(positions[i])).to_array(),
                    normal: normals
                        .as_ref()
                        .map(|n| {
                            (normal_matrix * Vec3::from(n[i]))
                                .normalize_or(Vec3::Y)
                                .to_array()
                        })
                        .unwrap_or([0.0; 3]),
                    uv: uvs[i],
                    color: colors[i],
                })
                .collect();
            if normals.is_none() {
                (vertices, indices) = flat_normals(&vertices, &indices);
            }

            let pbr = prim.material().pbr_metallic_roughness();
            let texture = match pbr.base_color_texture() {
                Some(info) => {
                    let index = info.texture().source().index();
                    let image = self
                        .images
                        .get(index)
                        .context("glTF texture refers to a missing image")?;
                    Some(
                        image_to_rgba(image)
                            .with_context(|| format!("glTF mesh '{name}' texture"))?,
                    )
                }
                None => None,
            };
            self.parts.push(ModelPart {
                name: name.clone(),
                mesh: MeshData { vertices, indices },
                texture,
                base_color: Vec4::from(pbr.base_color_factor()),
            });
        }
        Ok(())
    }
}

/// Un-indexes the mesh so every triangle gets its face normal.
fn flat_normals(vertices: &[Vertex], indices: &[u32]) -> (Vec<Vertex>, Vec<u32>) {
    let mut out = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| vertices[i as usize]);
        let n = (Vec3::from(b.position) - Vec3::from(a.position))
            .cross(Vec3::from(c.position) - Vec3::from(a.position))
            .normalize_or(Vec3::Y)
            .to_array();
        for mut v in [a, b, c] {
            v.normal = n;
            out.push(v);
        }
    }
    let count = out.len() as u32;
    (out, (0..count).collect())
}

fn image_to_rgba(image: &::gltf::image::Data) -> anyhow::Result<TextureData> {
    use ::gltf::image::Format;
    let channels = match image.format {
        Format::R8 => 1,
        Format::R8G8 => 2,
        Format::R8G8B8 => 3,
        Format::R8G8B8A8 => 4,
        other => anyhow::bail!("unsupported image format {other:?} (only 8-bit images)"),
    };
    let pixel_count = image.width as usize * image.height as usize;
    anyhow::ensure!(
        image.pixels.len() == pixel_count * channels,
        "image data size mismatch"
    );
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for px in image.pixels.chunks_exact(channels) {
        match channels {
            1 => rgba.extend_from_slice(&[px[0], px[0], px[0], 255]),
            2 => rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            3 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
            _ => rgba.extend_from_slice(px),
        }
    }
    Ok(TextureData {
        width: image.width,
        height: image.height,
        rgba,
    })
}

// ---------------------------------------------------------------------------------------------
// Export

/// Writes one untextured mesh as a self-contained `.gltf`.
pub fn write(path: &Path, mesh: &MeshData) -> anyhow::Result<()> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mesh")
        .to_owned();
    write_parts(
        path,
        &[ModelPart {
            name,
            mesh: mesh.clone(),
            texture: None,
            base_color: Vec4::ONE,
        }],
    )
}

/// Writes several parts (each with an optional embedded PNG texture) as one `.gltf`.
pub fn write_parts(path: &Path, parts: &[ModelPart]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let json = to_gltf_json(parts)?;
    std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))
}

const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const NEAREST: u32 = 9728;
const REPEAT: u32 = 10497;

/// Builds the glTF JSON document for `parts` (buffers and images embedded as data URIs).
pub fn to_gltf_json(parts: &[ModelPart]) -> anyhow::Result<String> {
    let parts: Vec<&ModelPart> = parts
        .iter()
        .filter(|p| !p.mesh.vertices.is_empty() && !p.mesh.indices.is_empty())
        .collect();
    anyhow::ensure!(!parts.is_empty(), "nothing to export: every mesh is empty");

    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut textures = Vec::new();

    // Appends a buffer view and an accessor; returns the accessor index.
    let mut add_accessor = |bin: &mut Vec<u8>,
                            bytes: &[u8],
                            target: u32,
                            component: u32,
                            count: usize,
                            ty: &str,
                            bounds: Option<(Vec3, Vec3)>| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{},"target":{target}}}"#,
            bin.len(),
            bytes.len()
        ));
        bin.extend_from_slice(bytes);
        let bounds = bounds
            .map(|(lo, hi)| {
                format!(
                    r#","min":[{},{},{}],"max":[{},{},{}]"#,
                    num(lo.x),
                    num(lo.y),
                    num(lo.z),
                    num(hi.x),
                    num(hi.y),
                    num(hi.z)
                )
            })
            .unwrap_or_default();
        accessors.push(format!(
            r#"{{"bufferView":{},"componentType":{component},"count":{count},"type":"{ty}"{bounds}}}"#,
            views.len() - 1
        ));
        accessors.len() - 1
    };

    for part in &parts {
        let v = &part.mesh.vertices;
        let n = v.len();
        anyhow::ensure!(
            part.mesh.indices.iter().all(|&i| (i as usize) < n),
            "mesh '{}' has out-of-range indices",
            part.name
        );
        let positions: Vec<[f32; 3]> = v.iter().map(|v| v.position).collect();
        let normals: Vec<[f32; 3]> = v.iter().map(|v| v.normal).collect();
        let uvs: Vec<[f32; 2]> = v.iter().map(|v| v.uv).collect();
        let colors: Vec<[f32; 4]> = v.iter().map(|v| v.color).collect();
        let bounds = super::meshgen::bounds(v);
        let pos = add_accessor(
            &mut bin,
            bytemuck::cast_slice(&positions),
            ARRAY_BUFFER,
            FLOAT,
            n,
            "VEC3",
            Some(bounds),
        );
        let nrm = add_accessor(
            &mut bin,
            bytemuck::cast_slice(&normals),
            ARRAY_BUFFER,
            FLOAT,
            n,
            "VEC3",
            None,
        );
        let uv = add_accessor(
            &mut bin,
            bytemuck::cast_slice(&uvs),
            ARRAY_BUFFER,
            FLOAT,
            n,
            "VEC2",
            None,
        );
        let col = add_accessor(
            &mut bin,
            bytemuck::cast_slice(&colors),
            ARRAY_BUFFER,
            FLOAT,
            n,
            "VEC4",
            None,
        );
        let idx = add_accessor(
            &mut bin,
            bytemuck::cast_slice(&part.mesh.indices),
            ELEMENT_ARRAY_BUFFER,
            UNSIGNED_INT,
            part.mesh.indices.len(),
            "SCALAR",
            None,
        );

        let c = part.base_color;
        let mut pbr = format!(
            r#""baseColorFactor":[{},{},{},{}],"metallicFactor":0,"roughnessFactor":1"#,
            num(c.x),
            num(c.y),
            num(c.z),
            num(c.w)
        );
        if let Some(texture) = &part.texture {
            let png = encode_png(texture)?;
            images.push(format!(
                r#"{{"uri":"data:image/png;base64,{}"}}"#,
                base64(&png)
            ));
            textures.push(format!(r#"{{"source":{},"sampler":0}}"#, images.len() - 1));
            let _ = write!(
                pbr,
                r#","baseColorTexture":{{"index":{}}}"#,
                textures.len() - 1
            );
        }
        let alpha_mode = if c.w < 1.0 {
            r#","alphaMode":"BLEND""#
        } else {
            ""
        };
        materials.push(format!(
            r#"{{"name":{},"pbrMetallicRoughness":{{{pbr}}}{alpha_mode}}}"#,
            json_string(&part.name)
        ));
        meshes.push(format!(
            r#"{{"name":{},"primitives":[{{"attributes":{{"POSITION":{pos},"NORMAL":{nrm},"TEXCOORD_0":{uv},"COLOR_0":{col}}},"indices":{idx},"material":{},"mode":4}}]}}"#,
            json_string(&part.name),
            materials.len() - 1
        ));
    }

    let nodes: Vec<String> = parts
        .iter()
        .enumerate()
        .map(|(i, p)| format!(r#"{{"name":{},"mesh":{i}}}"#, json_string(&p.name)))
        .collect();
    let node_list: Vec<String> = (0..parts.len()).map(|i| i.to_string()).collect();

    let mut json = String::new();
    json.push_str(
        r#"{"asset":{"version":"2.0","generator":"velocide engine::assets::gltf"},"scene":0,"#,
    );
    let _ = write!(json, r#""scenes":[{{"nodes":[{}]}}],"#, node_list.join(","));
    let _ = write!(json, r#""nodes":[{}],"#, nodes.join(","));
    let _ = write!(json, r#""meshes":[{}],"#, meshes.join(","));
    let _ = write!(json, r#""materials":[{}],"#, materials.join(","));
    if !textures.is_empty() {
        let _ = write!(
            json,
            r#""samplers":[{{"magFilter":{NEAREST},"minFilter":{NEAREST},"wrapS":{REPEAT},"wrapT":{REPEAT}}}],"textures":[{}],"images":[{}],"#,
            textures.join(","),
            images.join(",")
        );
    }
    let _ = write!(json, r#""accessors":[{}],"#, accessors.join(",\n"));
    let _ = write!(json, r#""bufferViews":[{}],"#, views.join(",\n"));
    let _ = write!(
        json,
        r#""buffers":[{{"byteLength":{},"uri":"data:application/octet-stream;base64,{}"}}]}}"#,
        bin.len(),
        base64(&bin)
    );
    json.push('\n');
    Ok(json)
}

/// JSON number for a float (non-finite values become 0).
fn num(x: f32) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "0".to_owned()
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn encode_png(texture: &TextureData) -> anyhow::Result<Vec<u8>> {
    use image::ImageEncoder as _;
    anyhow::ensure!(
        texture.rgba.len() == texture.width as usize * texture.height as usize * 4,
        "texture data size mismatch"
    );
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(
        &texture.rgba,
        texture.width,
        texture.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png)
}

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::meshgen;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("velocide-{}-{name}", std::process::id()))
    }

    #[test]
    fn round_trip_mesh() {
        let mut b = meshgen::cuboid(Vec3::new(1.0, 0.5, 0.25), Vec4::new(0.8, 0.2, 0.1, 1.0));
        b.append(
            &meshgen::cone(0.3, 0.6, 5, Vec4::ONE).build(),
            Mat4::from_translation(Vec3::Y),
        );
        let mesh = b.build();
        let path = temp_path("roundtrip.gltf");
        write(&path, &mesh).unwrap();
        let parts = load(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(parts.len(), 1);
        let got = &parts[0].mesh;
        assert_eq!(got.indices, mesh.indices);
        assert_eq!(got.vertices.len(), mesh.vertices.len());
        for (a, b) in got.vertices.iter().zip(&mesh.vertices) {
            assert_eq!(a.position, b.position);
            assert_eq!(a.uv, b.uv);
            assert_eq!(a.color, b.color);
            for k in 0..3 {
                assert!((a.normal[k] - b.normal[k]).abs() < 1e-5);
            }
        }
        assert_eq!(parts[0].base_color, Vec4::ONE);
        assert!(parts[0].texture.is_none());
    }

    #[test]
    fn round_trip_textured_parts() {
        let texture = crate::assets::procedural::material_texture("hazard");
        let parts = [
            ModelPart {
                name: "body \"main\"".into(),
                mesh: meshgen::icosphere(1.0, 1, Vec4::ONE).build(),
                texture: Some(texture.clone()),
                base_color: Vec4::new(1.0, 0.5, 0.5, 1.0),
            },
            ModelPart {
                name: "empty".into(),
                mesh: MeshData::default(),
                texture: None,
                base_color: Vec4::ONE,
            },
            ModelPart {
                name: "fin".into(),
                mesh: meshgen::quad(glam::Vec2::ONE, Vec4::ONE).build(),
                texture: None,
                base_color: Vec4::new(0.2, 0.9, 0.9, 0.5),
            },
        ];
        let path = temp_path("parts.gltf");
        write_parts(&path, &parts).unwrap();
        let loaded = load(&path).unwrap();
        let from_bytes = load_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(from_bytes.len(), loaded.len());
        for (a, b) in from_bytes.iter().zip(&loaded) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.mesh.indices, b.mesh.indices);
            assert_eq!(
                a.texture.as_ref().map(|t| &t.rgba),
                b.texture.as_ref().map(|t| &t.rgba)
            );
        }

        assert_eq!(loaded.len(), 2, "empty parts are skipped");
        assert_eq!(loaded[0].name, "body \"main\"");
        let tex = loaded[0].texture.as_ref().unwrap();
        assert_eq!((tex.width, tex.height), (texture.width, texture.height));
        assert_eq!(tex.rgba, texture.rgba);
        assert_eq!(loaded[0].base_color, Vec4::new(1.0, 0.5, 0.5, 1.0));
        assert_eq!(loaded[1].base_color.w, 0.5);
    }

    #[test]
    fn node_transforms_are_applied() {
        // A hand-written file with a translated + scaled node, no normals, external-free.
        let mesh = meshgen::quad(glam::Vec2::ONE, Vec4::ONE).build();
        let json = to_gltf_json(&[ModelPart {
            name: "q".into(),
            mesh,
            texture: None,
            base_color: Vec4::ONE,
        }])
        .unwrap()
        .replace(
            r#"{"name":"q","mesh":0}"#,
            r#"{"name":"q","mesh":0,"translation":[0,5,0],"scale":[2,2,2]}"#,
        );
        let path = temp_path("node.gltf");
        std::fs::write(&path, json).unwrap();
        let parts = load(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let (lo, hi) = meshgen::bounds(&parts[0].mesh.vertices);
        assert_eq!(lo, Vec3::new(-1.0, 4.0, 0.0));
        assert_eq!(hi, Vec3::new(1.0, 6.0, 0.0));
    }

    #[test]
    fn embedded_models_load_from_their_bytes() {
        let bytes = crate::asset_files::embedded("models/warden.gltf").expect("embedded");
        assert!(!load_slice(bytes).unwrap().is_empty());
        assert!(!load_asset("models/warden.gltf").unwrap().is_empty());
        let error = load_asset("models/missing.gltf").unwrap_err().to_string();
        assert!(error.contains("models/missing.gltf"), "{error}");
        // Bytes have no folder: external buffers and images are refused, not read.
        for external in [
            r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"x.bin"}]}"#,
            r#"{"asset":{"version":"2.0"},"images":[{"uri":"x.png"}]}"#,
            r#"{"asset":{"version":"2.0"},"images":[{"uri":"file:///etc/passwd"}]}"#,
        ] {
            assert!(load_slice(external.as_bytes()).is_err(), "{external}");
        }
    }
}
