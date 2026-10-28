//! Compiles the workspace's `assets/` folder into the engine (`engine::asset_files`), so the
//! game and the dedicated server run without one next to them.
//!
//! Writes `$OUT_DIR/embedded_assets.rs`: a sorted `&[(path, include_bytes!(file))]` table of
//! every file under `assets/` (paths relative to it, `/`-separated; hidden files and editor
//! backups skipped). Without the `client` feature (the headless server) only what the
//! simulation reads goes in: `tuning.ron`, `levels/` and `enemies/`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// What a build without the `client` feature embeds; models, shaders and anything else are
/// only read by the game.
const HEADLESS: &[&str] = &["tuning.ron", "levels/", "enemies/"];

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let assets = manifest
        .parent()
        .expect("engine/ is in the workspace")
        .join("assets");
    // Cargo scans a directory recursively: adding, removing or editing a file under it reruns
    // this script (`include_bytes!` also makes rustc track the contents of each file).
    println!("cargo:rerun-if-changed={}", assets.display());
    println!("cargo:rerun-if-changed=build.rs");

    let client = std::env::var_os("CARGO_FEATURE_CLIENT").is_some();
    let mut files = Vec::new();
    collect(&assets, "", &mut files);
    files.retain(|rel| {
        client
            || HEADLESS
                .iter()
                .any(|p| rel == p || (p.ends_with('/') && rel.starts_with(p)))
    });
    files.sort();

    let mut table = String::from("&[\n");
    for rel in &files {
        let path = assets.join(rel);
        let path = path.to_str().expect("asset paths are UTF-8");
        writeln!(table, "    ({rel:?}, include_bytes!({path:?})),").unwrap();
    }
    table.push_str("]\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by cargo"));
    std::fs::write(out.join("embedded_assets.rs"), table).expect("writing embedded_assets.rs");
}

/// Appends the paths (relative to the assets folder, `/`-separated) of the files under `dir`.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            println!(
                "cargo:warning=not embedding {} (name is not UTF-8)",
                path.display()
            );
            continue;
        };
        if name.starts_with('.') || name.ends_with('~') {
            continue;
        }
        let rel = format!("{prefix}{name}");
        if path.is_dir() {
            collect(&path, &format!("{rel}/"), out);
        } else if path.is_file() {
            out.push(rel);
        }
    }
}
