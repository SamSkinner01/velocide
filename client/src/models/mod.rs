//! Procedural low-poly PS1-style models: the Shrike (player), enemies, weapons, projectiles and
//! props, plus their rigs and pure procedural pose functions.
//!
//! Pipeline: every model is built in code ([`ModelId::build`], `engine::assets::meshgen` +
//! small procedural textures), exported to `assets/models/<file>.gltf` by [`generate_all`]
//! (`cargo run -p client --example models_gen`), embedded in the game at build time, and loaded
//! at runtime with `engine::assets::gltf::load_asset` (the file on disk wins over the embedded
//! copy) by [`ModelLibrary::load`]. If a file is missing or does not match the model's part
//! table, that model is regenerated in memory (with a warning) so the game always starts.
//!
//! Rig structure (part names, parents, pivots, shading) lives in code ([`ModelId::joints`]);
//! the glTF files hold each part's mesh in bind pose (model space), its texture and base
//! colour. See `docs/models.md`.

/// The Shrike's cosmetics kit: masks, crests, cloaks, emblems (`shared::cosmetics`).
pub mod cosmetics;
pub mod enemies;
/// The rework's ability gear: the Bolas, the Bulwark's slab, the Lantern, the Kite.
pub mod gear;
/// Warm-up hub props: the ready board and the practice console (docs/lobby.md).
pub mod hub;
pub mod kit;
/// The knacks' thrown things and traps: the Kindle flask, the Snare, the Soot pot.
pub mod knacks;
mod library;
pub mod projectiles;
pub mod props;
pub mod rig;
pub mod shrike;
/// The rework's arrows (docs/rework.md): the Blink and Beacon arrows.
pub mod tricks;
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
pub use weapons::{
    Ability, DrumTurn, HeldWeapon, ViewmodelAnim, WeaponKind, WeaponLook, ability_color,
    weapon_kind,
};

/// Every model in the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ModelId {
    /// Player character (third person).
    Shrike,
    /// Every cosmetic part of the Shrike (`cosmetics`), attached to its joints when drawn.
    ShrikeKit,
    // Enemies (docs/roster.md).
    Gnawer,
    Censer,
    Harrier,
    Gaoler,
    Heron,
    Bellwether,
    Snuffer,
    Smelter,
    /// The warm-up's training dummy.
    Effigy,
    // Weapons: used both as first-person viewmodels and in the Shrike's hand.
    Quill,
    Bellows,
    Arclash,
    Tumbler,
    // The expansion arsenal (docs/expansion.md).
    Lancet,
    Thresher,
    Riveter,
    Cinderbox,
    Longwick,
    Kiln,
    // Melee (docs/refit.md).
    /// Hooked wick-shears carried as two blades.
    Trimmer,
    /// Foundry ramming hammer.
    Tamper,
    // The rework (docs/rework.md).
    /// The Lamplighter: a lamplighter's long pole turned rifle (the sniper).
    Lamplighter,
    /// First-person right forearm + gloved hand that grips every weapon.
    ArmRight,
    /// First-person left forearm + fist (punch / parry, Thorn throw).
    Fist,
    // Projectiles.
    /// Tumbler bomb.
    Bomb,
    /// Tumbler alt B canister.
    Gravewell,
    /// Arclash alt A ball of lightning.
    Wisp,
    /// The Censer's lobbed ember bomb.
    EmberBomb,
    /// The Heron's needle.
    Needle,
    /// Unit glowing beam (the Heron's aim laser, needle streaks).
    AimLaser,
    /// Unit strip of molten slag on the floor (the Smelter's slag lines).
    SlagStrip,
    /// Thrown relay spike (also shown between the knuckles while throwing).
    Thorn,
    /// Longwick taper arrow, the Pinning Arrow and the Ember Arrow.
    Arrow,
    PinningArrow,
    EmberArrow,
    /// Riveter hot rivet.
    Rivet,
    /// Cinderbox salvo rocket.
    CinderRocket,
    /// Flak shell (airbursts into a ring of pellets).
    FlakShell,
    /// Limpet sticky charge.
    Limpet,
    /// Mortar shell.
    MortarShell,
    /// Flare (leaves a burning patch).
    Flare,
    /// Optional glowing core for ember-burst effects (a cluster of live coals).
    EmberCore,
    // Level props.
    JumpPad,
    Checkpoint,
    ExitGate,
    Door,
    // Warm-up hub props (level props by file name).
    ReadyBoard,
    Console,
    // The knacks (docs/refit.md): thrown things and traps.
    KindleFlask,
    Snare,
    SootPot,
    /// The wax puddle a stranded Mould melts into and re-forms out of.
    WaxPuddle,
    // The rework's arrows (docs/rework.md).
    /// Blink Arrow: the arrow you step to.
    BlinkArrow,
    /// Beacon Arrow: the arrow that lights a beacon where it sticks.
    BeaconArrow,
    // The rework's ability gear (docs/rework.md, `gear`).
    /// Two wax balls on a wick cord (Bolas).
    Bolas,
    /// The hardened wax slab raised in front of a Bulwark.
    Bulwark,
    /// The turret lantern set down by Lantern.
    Lantern,
    /// The steered ember kite (Kite).
    Kite,
}

impl ModelId {
    pub const ALL: [ModelId; 60] = [
        ModelId::Shrike,
        ModelId::ShrikeKit,
        ModelId::Gnawer,
        ModelId::Censer,
        ModelId::Harrier,
        ModelId::Gaoler,
        ModelId::Heron,
        ModelId::Bellwether,
        ModelId::Snuffer,
        ModelId::Smelter,
        ModelId::Effigy,
        ModelId::Quill,
        ModelId::Bellows,
        ModelId::Arclash,
        ModelId::Tumbler,
        ModelId::Lancet,
        ModelId::Thresher,
        ModelId::Riveter,
        ModelId::Cinderbox,
        ModelId::Longwick,
        ModelId::Kiln,
        ModelId::Trimmer,
        ModelId::Tamper,
        ModelId::Lamplighter,
        ModelId::ArmRight,
        ModelId::Fist,
        ModelId::Bomb,
        ModelId::Gravewell,
        ModelId::Wisp,
        ModelId::EmberBomb,
        ModelId::Needle,
        ModelId::AimLaser,
        ModelId::SlagStrip,
        ModelId::Thorn,
        ModelId::Arrow,
        ModelId::PinningArrow,
        ModelId::EmberArrow,
        ModelId::Rivet,
        ModelId::CinderRocket,
        ModelId::FlakShell,
        ModelId::Limpet,
        ModelId::MortarShell,
        ModelId::Flare,
        ModelId::EmberCore,
        ModelId::JumpPad,
        ModelId::Checkpoint,
        ModelId::ExitGate,
        ModelId::Door,
        ModelId::ReadyBoard,
        ModelId::Console,
        ModelId::KindleFlask,
        ModelId::Snare,
        ModelId::SootPot,
        ModelId::WaxPuddle,
        ModelId::BlinkArrow,
        ModelId::BeaconArrow,
        ModelId::Bolas,
        ModelId::Bulwark,
        ModelId::Lantern,
        ModelId::Kite,
    ];

    /// Index into [`ModelId::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }

    /// File name (without directory) in `assets/models/`.
    pub fn file_name(self) -> &'static str {
        match self {
            ModelId::Shrike => "shrike.gltf",
            ModelId::ShrikeKit => "shrike_kit.gltf",
            ModelId::Gnawer => "enemy_gnawer.gltf",
            ModelId::Censer => "enemy_censer.gltf",
            ModelId::Harrier => "enemy_harrier.gltf",
            ModelId::Gaoler => "enemy_gaoler.gltf",
            ModelId::Heron => "enemy_heron.gltf",
            ModelId::Bellwether => "enemy_bellwether.gltf",
            ModelId::Snuffer => "enemy_snuffer.gltf",
            ModelId::Smelter => "enemy_smelter.gltf",
            ModelId::Effigy => "enemy_effigy.gltf",
            ModelId::Quill => "weapon_quill.gltf",
            ModelId::Bellows => "weapon_bellows.gltf",
            ModelId::Arclash => "weapon_arclash.gltf",
            ModelId::Tumbler => "weapon_tumbler.gltf",
            ModelId::Lancet => "weapon_lancet.gltf",
            ModelId::Thresher => "weapon_thresher.gltf",
            ModelId::Riveter => "weapon_riveter.gltf",
            ModelId::Cinderbox => "weapon_cinderbox.gltf",
            ModelId::Longwick => "weapon_longwick.gltf",
            ModelId::Kiln => "weapon_kiln.gltf",
            ModelId::Trimmer => "weapon_trimmer.gltf",
            ModelId::Tamper => "weapon_tamper.gltf",
            ModelId::Lamplighter => "weapon_lamplighter.gltf",
            ModelId::ArmRight => "vm_arm.gltf",
            ModelId::Fist => "vm_fist.gltf",
            ModelId::Bomb => "proj_bomb.gltf",
            ModelId::Gravewell => "proj_gravewell.gltf",
            ModelId::Wisp => "proj_wisp.gltf",
            ModelId::EmberBomb => "proj_ember_bomb.gltf",
            ModelId::Needle => "proj_needle.gltf",
            ModelId::AimLaser => "fx_aim_laser.gltf",
            ModelId::SlagStrip => "fx_slag_strip.gltf",
            ModelId::Thorn => "proj_thorn.gltf",
            ModelId::Arrow => "proj_arrow.gltf",
            ModelId::PinningArrow => "proj_arrow_pinning.gltf",
            ModelId::EmberArrow => "proj_arrow_ember.gltf",
            ModelId::Rivet => "proj_rivet.gltf",
            ModelId::CinderRocket => "proj_cinder_rocket.gltf",
            ModelId::FlakShell => "proj_flak_shell.gltf",
            ModelId::Limpet => "proj_limpet.gltf",
            ModelId::MortarShell => "proj_mortar_shell.gltf",
            ModelId::Flare => "proj_flare.gltf",
            ModelId::EmberCore => "fx_ember_core.gltf",
            ModelId::JumpPad => "prop_jump_pad.gltf",
            ModelId::Checkpoint => "prop_checkpoint.gltf",
            ModelId::ExitGate => "prop_exit_gate.gltf",
            ModelId::Door => "prop_door.gltf",
            ModelId::ReadyBoard => "prop_ready_board.gltf",
            ModelId::Console => "prop_console.gltf",
            ModelId::KindleFlask => "prop_kindle_flask.gltf",
            ModelId::Snare => "prop_snare.gltf",
            ModelId::SootPot => "prop_soot_pot.gltf",
            ModelId::WaxPuddle => "fx_wax_puddle.gltf",
            ModelId::BlinkArrow => "proj_arrow_blink.gltf",
            ModelId::BeaconArrow => "proj_arrow_beacon.gltf",
            ModelId::Bolas => "proj_bolas.gltf",
            ModelId::Bulwark => "prop_bulwark.gltf",
            ModelId::Lantern => "prop_lantern.gltf",
            ModelId::Kite => "proj_kite.gltf",
        }
    }

    /// The part table (names, parents, pivots, shading) of this model.
    pub fn joints(self) -> &'static [JointDef] {
        match self {
            ModelId::Shrike => &shrike::JOINTS,
            ModelId::ShrikeKit => &cosmetics::JOINTS,
            ModelId::ReadyBoard | ModelId::Console => hub::joints(self),
            ModelId::Gnawer => &enemies::gnawer::JOINTS,
            ModelId::Censer => &enemies::censer::JOINTS,
            ModelId::Harrier => &enemies::harrier::JOINTS,
            ModelId::Gaoler => &enemies::gaoler::JOINTS,
            ModelId::Heron => &enemies::heron::JOINTS,
            ModelId::Bellwether => &enemies::bellwether::JOINTS,
            ModelId::Snuffer => &enemies::snuffer::JOINTS,
            ModelId::Smelter => &enemies::smelter::JOINTS,
            ModelId::Effigy => &enemies::effigy::JOINTS,
            ModelId::SlagStrip => &enemies::slag::JOINTS,
            ModelId::WaxPuddle => &enemies::puddle::JOINTS,
            ModelId::Quill => &weapons::quill::JOINTS,
            ModelId::Bellows => &weapons::bellows::JOINTS,
            ModelId::Arclash => &weapons::arclash::JOINTS,
            ModelId::Tumbler => &weapons::tumbler::JOINTS,
            ModelId::Lancet => &weapons::lancet::JOINTS,
            ModelId::Thresher => &weapons::thresher::JOINTS,
            ModelId::Riveter => &weapons::riveter::JOINTS,
            ModelId::Cinderbox => &weapons::cinderbox::JOINTS,
            ModelId::Longwick => &weapons::longwick::JOINTS,
            ModelId::Kiln => &weapons::kiln::JOINTS,
            ModelId::Trimmer => &weapons::trimmer::JOINTS,
            ModelId::Tamper => &weapons::tamper::JOINTS,
            ModelId::Lamplighter => &weapons::lamplighter::JOINTS,
            ModelId::KindleFlask | ModelId::Snare | ModelId::SootPot => knacks::joints(self),
            ModelId::BlinkArrow | ModelId::BeaconArrow => tricks::joints(self),
            ModelId::Bolas | ModelId::Bulwark | ModelId::Lantern | ModelId::Kite => {
                gear::joints(self)
            }
            ModelId::ArmRight => &weapons::arm::JOINTS,
            ModelId::Fist => &weapons::fist::JOINTS,
            id if id.is_player_projectile() => projectiles::joints(self),
            other => props::joints(other),
        }
    }

    /// Builds the model's parts (model-space bind pose, in [`ModelId::joints`] order, named
    /// after the joints).
    pub fn build(self) -> Vec<ModelPart> {
        match self {
            ModelId::Shrike => shrike::build(),
            ModelId::ShrikeKit => cosmetics::build(),
            ModelId::ReadyBoard | ModelId::Console => hub::build(self),
            ModelId::Gnawer => enemies::gnawer::build(),
            ModelId::Censer => enemies::censer::build(),
            ModelId::Harrier => enemies::harrier::build(),
            ModelId::Gaoler => enemies::gaoler::build(),
            ModelId::Heron => enemies::heron::build(),
            ModelId::Bellwether => enemies::bellwether::build(),
            ModelId::Snuffer => enemies::snuffer::build(),
            ModelId::Smelter => enemies::smelter::build(),
            ModelId::Effigy => enemies::effigy::build(),
            ModelId::SlagStrip => enemies::slag::build(),
            ModelId::WaxPuddle => enemies::puddle::build(),
            ModelId::Quill => weapons::quill::build(),
            ModelId::Bellows => weapons::bellows::build(),
            ModelId::Arclash => weapons::arclash::build(),
            ModelId::Tumbler => weapons::tumbler::build(),
            ModelId::Lancet => weapons::lancet::build(),
            ModelId::Thresher => weapons::thresher::build(),
            ModelId::Riveter => weapons::riveter::build(),
            ModelId::Cinderbox => weapons::cinderbox::build(),
            ModelId::Longwick => weapons::longwick::build(),
            ModelId::Kiln => weapons::kiln::build(),
            ModelId::Trimmer => weapons::trimmer::build(),
            ModelId::Tamper => weapons::tamper::build(),
            ModelId::Lamplighter => weapons::lamplighter::build(),
            ModelId::KindleFlask | ModelId::Snare | ModelId::SootPot => knacks::build(self),
            ModelId::BlinkArrow | ModelId::BeaconArrow => tricks::build(self),
            ModelId::Bolas | ModelId::Bulwark | ModelId::Lantern | ModelId::Kite => {
                gear::build(self)
            }
            ModelId::ArmRight => weapons::arm::build(),
            ModelId::Fist => weapons::fist::build(),
            id if id.is_player_projectile() => projectiles::build(self),
            other => props::build(other),
        }
    }

    /// Built by `models::projectiles` (the players' projectiles and the Thorn).
    fn is_player_projectile(self) -> bool {
        matches!(
            self,
            ModelId::Bomb
                | ModelId::Gravewell
                | ModelId::Wisp
                | ModelId::Thorn
                | ModelId::Arrow
                | ModelId::PinningArrow
                | ModelId::EmberArrow
                | ModelId::Rivet
                | ModelId::CinderRocket
                | ModelId::FlakShell
                | ModelId::Limpet
                | ModelId::MortarShell
                | ModelId::Flare
        )
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
