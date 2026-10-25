//! Runtime model library: loads `assets/models/*.gltf`, falls back to in-memory generation,
//! uploads meshes / textures and offers one-call draw helpers for the client.

use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};
use std::path::Path;

use engine::assets::gltf::ModelPart;
use engine::gfx::{DrawItem, MeshData, TextureData, TextureId};
use engine::glam::{Mat4, Vec3, Vec4};
use engine::render::Renderer;

use super::rig::{Rig, Tint, validate_joints};
use super::{EnemyAnim, ModelId, PlayerAnim, ViewmodelAnim, enemies, shrike, weapons};

/// All models, uploaded and ready to draw.
pub struct ModelLibrary {
    rigs: Vec<Rig>,
    /// Models that could not be loaded from disk and were generated in memory instead.
    pub regenerated: Vec<ModelId>,
}

/// Result of [`ModelLibrary::draw_player`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerDrawInfo {
    /// World-space muzzle of the held weapon (tracers / muzzle flash of remote players).
    pub muzzle: Vec3,
    /// World-space direction the muzzle points.
    pub muzzle_dir: Vec3,
    /// World-space head position (name tags go a little above it).
    pub head: Vec3,
}

/// Result of [`ModelLibrary::draw_viewmodel`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewmodelDrawInfo {
    /// View-space muzzle point (camera at origin looking down -Z).
    pub muzzle: Vec3,
    /// View-space direction the barrel points.
    pub muzzle_dir: Vec3,
    /// View-space position of the thrown prism / fist knuckles (punch sparks), if visible.
    pub fist: Option<Vec3>,
}

/// Turns loaded / generated parts into joint-space meshes in joint-table order.
/// Errors if a part is missing or invalid.
pub(crate) fn match_parts(
    id: ModelId,
    parts: &[ModelPart],
) -> anyhow::Result<Vec<(MeshData, Option<TextureData>, Vec4)>> {
    let joints = id.joints();
    validate_joints(joints).map_err(|e| anyhow::anyhow!("{id:?}: {e}"))?;
    joints
        .iter()
        .map(|j| {
            let part = parts
                .iter()
                .find(|p| p.name == j.name)
                .ok_or_else(|| anyhow::anyhow!("{id:?}: part '{}' missing", j.name))?;
            let mesh = &part.mesh;
            anyhow::ensure!(
                !mesh.indices.is_empty()
                    && mesh.indices.len().is_multiple_of(3)
                    && mesh
                        .indices
                        .iter()
                        .all(|&i| (i as usize) < mesh.vertices.len()),
                "{id:?}: part '{}' has an empty or invalid mesh",
                j.name
            );
            let mut local = mesh.clone();
            for v in &mut local.vertices {
                v.position = (Vec3::from(v.position) - j.pivot).to_array();
            }
            Ok((local, part.texture.clone(), part.base_color))
        })
        .collect()
}

fn texture_key(t: &TextureData) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (t.width, t.height).hash(&mut h);
    t.rgba.hash(&mut h);
    h.finish()
}

impl ModelLibrary {
    /// Loads every model from `engine::assets_dir()/models`.
    pub fn load(renderer: &mut Renderer) -> anyhow::Result<ModelLibrary> {
        Self::load_from(renderer, &engine::assets_dir().join("models"))
    }

    /// Loads every model from `dir`; models whose file is missing or broken are generated in
    /// memory instead (logged as a warning).
    pub fn load_from(renderer: &mut Renderer, dir: &Path) -> anyhow::Result<ModelLibrary> {
        let mut textures: HashMap<u64, TextureId> = HashMap::new();
        let white = renderer.white_texture();
        let mut rigs = Vec::with_capacity(ModelId::ALL.len());
        let mut regenerated = Vec::new();
        for id in ModelId::ALL {
            let path = dir.join(id.file_name());
            let loaded =
                engine::assets::gltf::load(&path).and_then(|parts| match_parts(id, &parts));
            let parts = match loaded {
                Ok(parts) => parts,
                Err(e) => {
                    log::warn!(
                        "model {id:?}: {e:#}; generating it in memory (run `cargo run -p client --example models_gen` to rebuild assets/models)"
                    );
                    regenerated.push(id);
                    match_parts(id, &id.build())?
                }
            };
            let rig = Rig::new(id.joints(), |i| {
                let (mesh, texture, color) = &parts[i];
                let texture = match texture {
                    Some(t) => *textures
                        .entry(texture_key(t))
                        .or_insert_with(|| renderer.create_texture(t)),
                    None => white,
                };
                (renderer.create_mesh(mesh), texture, *color)
            });
            rigs.push(rig);
        }
        Ok(ModelLibrary { rigs, regenerated })
    }

    pub fn get(&self, id: ModelId) -> &Rig {
        &self.rigs[id.index()]
    }

    /// Draws a model in its bind pose (projectiles, props).
    pub fn draw_model(&self, id: ModelId, root: Mat4, tint: &Tint, out: &mut Vec<DrawItem>) {
        self.get(id).draw_static(root, tint, out);
    }

    /// Draws a third-person Shrike (posed from `anim`) standing at `feet` and the weapon in
    /// its hand. The body faces `anim.yaw`.
    pub fn draw_player(
        &self,
        anim: &PlayerAnim,
        feet: Vec3,
        tint: &Tint,
        out: &mut Vec<DrawItem>,
    ) -> PlayerDrawInfo {
        let rig = self.get(ModelId::Shrike);
        let pose = shrike::pose(anim);
        let root = shrike::root_transform(anim, feet);
        rig.draw(&pose, root, tint, out);
        let hand = rig.part_transform(&pose, root, shrike::HAND_R);
        let weapon = weapons::WeaponKind::from_index(anim.weapon);
        let gun_root = hand * shrike::grip_transform();
        let gun_pose = weapons::third_person_pose(weapon, anim.firing, anim.time);
        let gun = self.get(weapon.model());
        if !anim.dead {
            gun.draw(&gun_pose, gun_root, tint, out);
        }
        let head = rig.part_transform(&pose, root, shrike::HEAD);
        PlayerDrawInfo {
            muzzle: gun_root.transform_point3(weapon.muzzle()),
            muzzle_dir: gun_root
                .transform_vector3(Vec3::NEG_Z)
                .normalize_or(Vec3::NEG_Z),
            head: head.transform_point3(Vec3::new(0.0, 0.16, 0.0)),
        }
    }

    /// Draws an enemy posed from `anim` with its feet (or hover centre for Gnats) at `root`.
    pub fn draw_enemy(&self, anim: &EnemyAnim, root: Mat4, tint: &Tint, out: &mut Vec<DrawItem>) {
        let id = anim.kind.model();
        let pose = enemies::pose(anim);
        self.get(id).draw(&pose, root, tint, out);
    }

    /// World-space point an enemy attacks from in its current pose: Husk right claw tips,
    /// Spitter maw (orb spawn), Gnat emitters (bolt spawn), Bastion right fist (slam impact).
    /// For muzzle flashes / impact effects; gameplay positions come from the server.
    pub fn enemy_attack_point(&self, anim: &EnemyAnim, root: Mat4) -> Vec3 {
        let (part, offset) = match anim.kind {
            enemies::EnemyKind::Husk => (enemies::husk::CLAWS_R, Vec3::new(0.0, -0.25, -0.09)),
            enemies::EnemyKind::Spitter => (enemies::spitter::MAW, Vec3::new(0.0, 0.012, -0.2)),
            enemies::EnemyKind::Gnat => (enemies::gnat::EMITTERS, Vec3::new(0.0, -0.045, -0.21)),
            enemies::EnemyKind::Bastion => (enemies::bastion::FIST_R, Vec3::new(0.0, -0.5, -0.04)),
        };
        let pose = enemies::pose(anim);
        self.get(anim.kind.model())
            .part_transform(&pose, root, part)
            .transform_point3(offset)
    }

    /// Draws the first-person viewmodel (view space) for `anim`: the weapon in the right hand,
    /// the punching left fist and the prism throw. Push the items into
    /// `FrameDesc::viewmodel`.
    pub fn draw_viewmodel(
        &self,
        anim: &ViewmodelAnim,
        tint: &Tint,
        out: &mut Vec<DrawItem>,
    ) -> ViewmodelDrawInfo {
        weapons::draw_viewmodel(self, anim, tint, out)
    }
}
