//! Asset loading: glTF models, PNG textures, procedural textures and procedural meshes.
//!
//! - [`procedural`]: deterministic PS1-style material and sprite textures.
//! - [`meshgen`]: [`meshgen::MeshBuilder`] and primitives (level brushes, weapons, enemies).
//! - [`gltf`]: glTF 2.0 loader and a minimal exporter.

pub mod gltf;
pub mod meshgen;
pub mod procedural;

use std::path::Path;

use anyhow::Context as _;

use crate::gfx::TextureData;

/// Loads a PNG file as RGBA8.
pub fn load_png(path: &Path) -> anyhow::Result<TextureData> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    decode_png(&bytes).with_context(|| format!("decoding {}", path.display()))
}

/// Decodes PNG bytes as RGBA8.
pub fn decode_png(bytes: &[u8]) -> anyhow::Result<TextureData> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.to_rgba8();
    Ok(TextureData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

/// Encodes RGBA8 pixels as PNG and writes them to `path`.
pub fn save_png(path: &Path, texture: &TextureData) -> anyhow::Result<()> {
    anyhow::ensure!(
        texture.rgba.len() == texture.width as usize * texture.height as usize * 4,
        "texture data size mismatch"
    );
    image::save_buffer_with_format(
        path,
        &texture.rgba,
        texture.width,
        texture.height,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let tex = procedural::sprite_texture(procedural::SpriteTex::Glow);
        let path = std::env::temp_dir().join(format!("velocide-{}-glow.png", std::process::id()));
        save_png(&path, &tex).unwrap();
        let back = load_png(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!((back.width, back.height), (tex.width, tex.height));
        assert_eq!(back.rgba, tex.rgba);
        assert!(decode_png(b"not a png").is_err());
    }
}
