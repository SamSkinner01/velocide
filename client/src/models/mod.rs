//! Procedural low-poly PS1-style models: the Shrike (player), enemies, weapons, projectiles and
//! props, plus their rigs and pure procedural pose functions.
//!
//! Pipeline: every model is built in code ([`ModelId::build`], `engine::assets::meshgen` +
//! small procedural textures), exported to `assets/models/<file>.gltf` by [`generate_all`]
//! (`cargo run -p client --example models_gen`) and loaded at runtime with
//! `engine::assets::gltf::load` by [`ModelLibrary::load`]. If a file is missing or does not
//! match the model's part table, that model is regenerated in memory (with a warning) so the
//! game always starts.
//!
//! Rig structure (part names, parents, pivots, shading) lives in code ([`ModelId::joints`]);
//! the glTF files hold each part's mesh in bind pose (model space), its texture and base
//! colour. See `docs/models.md`.

pub mod enemies;
pub mod kit;
mod library;
pub mod props;
pub mod rig;
pub mod shrike;
pub mod weapons;

#[cfg(test)]
mod tests;

use std::path::Path;

use anyhow::Context as _;
use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::gfx::TextureData;
use engine::glam::Vec4;

pub use enemies::{EnemyAnim, EnemyKind, EnemyState};
pub use library::{ModelLibrary, PlayerDrawInfo, ViewmodelDrawInfo};
pub use rig::{DEFAULT_TEAM_COLOR, JointDef, Pose, Rig, RigPart, Shade, Tint};
pub use shrike::PlayerAnim;
pub use weapons::{BarrelSpin, ViewmodelAnim, WeaponKind};

/// Every model in the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelId {
    /// Player character (third person).
    Shrike,
    // Enemies.
    Husk,
    Spitter,
    Gnat,
    Bastion,
    // Weapons: used both as first-person viewmodels and in the Shrike's hand.
    Lancet,
    Thresher,
    Riveter,
    Cinderbox,
    /// First-person right forearm + hand that grips every weapon.
    ArmRight,
    /// First-person left forearm + fist (punch / parry, prism throw).
    Fist,
    // Projectiles.
    Rivet,
    Sawblade,
    Rocket,
    FlakGrenade,
    StickyCharge,
    MortarShell,
    SpitterOrb,
    GnatBolt,
    /// Thrown ricochet shard (also shown spinning above the fist while throwing).
    Prism,
    /// Optional core for blood-burst effects.
    BloodCore,
    // Level props.
    JumpPad,
    Checkpoint,
    ExitGate,
    Door,
}

impl ModelId {
    pub const ALL: [ModelId; 25] = [
        ModelId::Shrike,
        ModelId::Husk,
        ModelId::Spitter,
        ModelId::Gnat,
        ModelId::Bastion,
        ModelId::Lancet,
        ModelId::Thresher,
        ModelId::Riveter,
        ModelId::Cinderbox,
        ModelId::ArmRight,
        ModelId::Fist,
        ModelId::Rivet,
        ModelId::Sawblade,
        ModelId::Rocket,
        ModelId::FlakGrenade,
        ModelId::StickyCharge,
        ModelId::MortarShell,
        ModelId::SpitterOrb,
        ModelId::GnatBolt,
        ModelId::Prism,
        ModelId::BloodCore,
        ModelId::JumpPad,
        ModelId::Checkpoint,
        ModelId::ExitGate,
        ModelId::Door,
    ];

    /// Index into [`ModelId::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// File name (without directory) in `assets/models/`.
    pub fn file_name(self) -> &'static str {
        match self {
            ModelId::Shrike => "shrike.gltf",
            ModelId::Husk => "enemy_husk.gltf",
            ModelId::Spitter => "enemy_spitter.gltf",
            ModelId::Gnat => "enemy_gnat.gltf",
            ModelId::Bastion => "enemy_bastion.gltf",
            ModelId::Lancet => "weapon_lancet.gltf",
            ModelId::Thresher => "weapon_thresher.gltf",
            ModelId::Riveter => "weapon_riveter.gltf",
            ModelId::Cinderbox => "weapon_cinderbox.gltf",
            ModelId::ArmRight => "vm_arm.gltf",
            ModelId::Fist => "vm_fist.gltf",
            ModelId::Rivet => "proj_rivet.gltf",
            ModelId::Sawblade => "proj_sawblade.gltf",
            ModelId::Rocket => "proj_rocket.gltf",
            ModelId::FlakGrenade => "proj_flak.gltf",
            ModelId::StickyCharge => "proj_sticky.gltf",
            ModelId::MortarShell => "proj_mortar.gltf",
            ModelId::SpitterOrb => "proj_spitter_orb.gltf",
            ModelId::GnatBolt => "proj_gnat_bolt.gltf",
            ModelId::Prism => "proj_prism.gltf",
            ModelId::BloodCore => "fx_blood_core.gltf",
            ModelId::JumpPad => "prop_jump_pad.gltf",
            ModelId::Checkpoint => "prop_checkpoint.gltf",
            ModelId::ExitGate => "prop_exit_gate.gltf",
            ModelId::Door => "prop_door.gltf",
        }
    }

    /// The part table (names, parents, pivots, shading) of this model.
    pub fn joints(self) -> &'static [JointDef] {
        match self {
            ModelId::Shrike => &shrike::JOINTS,
            ModelId::Husk => &enemies::husk::JOINTS,
            ModelId::Spitter => &enemies::spitter::JOINTS,
            ModelId::Gnat => &enemies::gnat::JOINTS,
            ModelId::Bastion => &enemies::bastion::JOINTS,
            ModelId::Lancet => &weapons::lancet::JOINTS,
            ModelId::Thresher => &weapons::thresher::JOINTS,
            ModelId::Riveter => &weapons::riveter::JOINTS,
            ModelId::Cinderbox => &weapons::cinderbox::JOINTS,
            ModelId::ArmRight => &weapons::arm::JOINTS,
            ModelId::Fist => &weapons::fist::JOINTS,
            other => props::joints(other),
        }
    }

    /// Builds the model's parts (model-space bind pose, in [`ModelId::joints`] order, named
    /// after the joints).
    pub fn build(self) -> Vec<ModelPart> {
        match self {
            ModelId::Shrike => shrike::build(),
            ModelId::Husk => enemies::husk::build(),
            ModelId::Spitter => enemies::spitter::build(),
            ModelId::Gnat => enemies::gnat::build(),
            ModelId::Bastion => enemies::bastion::build(),
            ModelId::Lancet => weapons::lancet::build(),
            ModelId::Thresher => weapons::thresher::build(),
            ModelId::Riveter => weapons::riveter::build(),
            ModelId::Cinderbox => weapons::cinderbox::build(),
            ModelId::ArmRight => weapons::arm::build(),
            ModelId::Fist => weapons::fist::build(),
            other => props::build(other),
        }
    }
}

/// Builds every model and writes one self-contained `.gltf` per model into `dir`; each file
/// is read back and checked against the model's part table.
pub fn generate_all(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for id in ModelId::ALL {
        let parts = id.build();
        let path = dir.join(id.file_name());
        engine::assets::gltf::write_parts(&path, &parts)
            .with_context(|| format!("writing model {id:?}"))?;
        load_parts(id, dir).with_context(|| format!("verifying {}", path.display()))?;
        let triangles: usize = parts.iter().map(|p| p.mesh.indices.len() / 3).sum();
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        log::info!(
            "wrote {} ({} parts, {triangles} triangles, {} KiB)",
            path.display(),
            parts.len(),
            bytes / 1024
        );
    }
    Ok(())
}

/// Loads `dir/<file>` for `id` with `engine::assets::gltf::load` and checks that every part
/// of the model's table is present and valid. Returns the parts in model space (bind pose).
pub fn load_parts(id: ModelId, dir: &Path) -> anyhow::Result<Vec<ModelPart>> {
    let parts = engine::assets::gltf::load(&dir.join(id.file_name()))?;
    library::match_parts(id, &parts)?;
    Ok(parts)
}

/// Collects one mesh / texture / base colour per joint while a model is being built.
pub(crate) struct PartSet {
    joints: &'static [JointDef],
    meshes: Vec<MeshBuilder>,
    textures: Vec<Option<TextureData>>,
    colors: Vec<Vec4>,
}

impl PartSet {
    pub fn new(joints: &'static [JointDef]) -> Self {
        Self {
            joints,
            meshes: vec![MeshBuilder::new(); joints.len()],
            textures: vec![None; joints.len()],
            colors: vec![Vec4::ONE; joints.len()],
        }
    }

    pub fn joints(&self) -> &'static [JointDef] {
        self.joints
    }

    /// Mesh builder of part `i` (model space).
    pub fn mesh(&mut self, i: usize) -> &mut MeshBuilder {
        &mut self.meshes[i]
    }

    /// Appends `b` to part `i`.
    pub fn add(&mut self, i: usize, b: MeshBuilder) {
        self.meshes[i].append(&b.build(), engine::glam::Mat4::IDENTITY);
    }

    pub fn texture(&mut self, i: usize, texture: TextureData) {
        self.textures[i] = Some(texture);
    }

    /// Material base colour (alpha < 1 = translucent part).
    pub fn color(&mut self, i: usize, color: Vec4) {
        self.colors[i] = color;
    }

    /// Applies `f` to every part's mesh builder.
    pub fn for_each_mesh(&mut self, mut f: impl FnMut(usize, &mut MeshBuilder)) {
        for (i, m) in self.meshes.iter_mut().enumerate() {
            f(i, m);
        }
    }

    pub fn finish(self) -> Vec<ModelPart> {
        self.joints
            .iter()
            .zip(self.meshes)
            .zip(self.textures)
            .zip(self.colors)
            .map(|(((j, mesh), texture), base_color)| ModelPart {
                name: j.name.to_owned(),
                mesh: mesh.build(),
                texture,
                base_color,
            })
            .collect()
    }
}
