//! Generates the procedural models used by the demos and saves them as glTF into
//! `assets/models/`.
//!
//! ```text
//! cargo run -p engine --example render_gen_models
//! ```

#[path = "render_common/models.rs"]
mod models;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let dir = engine::assets_dir().join("models");
    std::fs::create_dir_all(&dir)?;

    let warden = dir.join("warden.gltf");
    engine::assets::gltf::write_parts(&warden, &models::warden_parts())?;
    // Read it back to make sure the file is valid.
    let parts = engine::assets::gltf::load(&warden)?;
    let triangles: usize = parts.iter().map(|p| p.mesh.indices.len() / 3).sum();
    log::info!(
        "wrote {} ({} parts, {triangles} triangles)",
        warden.display(),
        parts.len()
    );

    let (gun, glow, _) = models::demo_gun();
    let demo_gun = dir.join("demo_gun.gltf");
    engine::assets::gltf::write_parts(
        &demo_gun,
        &[
            engine::assets::gltf::ModelPart {
                name: "demo_gun".into(),
                mesh: gun,
                texture: Some(engine::assets::procedural::material_texture("metal")),
                base_color: engine::glam::Vec4::ONE,
            },
            engine::assets::gltf::ModelPart {
                name: "demo_gun_cell".into(),
                mesh: glow,
                texture: None,
                base_color: engine::glam::Vec4::ONE,
            },
        ],
    )?;
    log::info!("wrote {}", demo_gun.display());
    Ok(())
}
