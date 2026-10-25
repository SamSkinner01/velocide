//! Builds every procedural model and writes it as glTF into `assets/models/` (or the
//! directory given as the first argument). Each file is read back and verified.
//!
//! ```text
//! cargo run -p client --example models_gen [-- DIR]
//! ```

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| engine::assets_dir().join("models"));
    client::models::generate_all(&dir)?;
    log::info!(
        "{} models written to {}",
        client::models::ModelId::ALL.len(),
        dir.display()
    );
    Ok(())
}
