//! Runtime model library: loads `assets/models/*.gltf` (on disk, else embedded in the game),
//! falls back to in-memory generation, uploads meshes / textures and offers one-call draw
//! helpers for the client.

use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};

use engine::assets::gltf::ModelPart;
use engine::gfx::{DrawItem, MeshData, TextureData, TextureId};
use engine::glam::{Mat4, Vec3, Vec4};
use engine::render::Renderer;

use super::rig::{Rig, Shade, Tint, neutral_mesh, validate_joints};
use super::weapons::WeaponLook as _;
use super::{EnemyAnim, ModelId, PlayerAnim, ViewmodelAnim, cosmetics, enemies, shrike, weapons};

/// All models, uploaded and ready to draw.
pub struct ModelLibrary {
    rigs: Vec<Rig>,
    /// Model-space bounds of each model in its rest pose (min, max): menu previews frame it.
    bounds: Vec<(Vec3, Vec3)>,
    /// Models that could not be loaded (from disk or embedded) and were generated in memory
    /// instead.
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
    /// View-space position of the held thorn's tip / fist knuckles (punch sparks), if visible.
    pub fist: Option<Vec3>,
}

/// Bounds of joint-space parts (from [`match_parts`]) put back at their pivots: the model in
/// its rest pose.
fn rest_bounds(id: ModelId, parts: &[(MeshData, Option<TextureData>, Vec4)]) -> (Vec3, Vec3) {
    let mut out: Option<(Vec3, Vec3)> = None;
    for (j, (mesh, _, _)) in id.joints().iter().zip(parts) {
        let (lo, hi) = engine::assets::meshgen::bounds(&mesh.vertices);
        let (lo, hi) = (lo + j.pivot, hi + j.pivot);
        out = Some(out.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
    }
    out.unwrap_or_default()
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

/// An enemy model's root with its drawn scale (`EnemyKind::scale`, about the model origin) and
/// the crouch / leap / melt squash of its state (`enemies::body_transform`).
fn enemy_root(anim: &EnemyAnim, root: Mat4) -> Mat4 {
    root * enemies::body_transform(anim) * Mat4::from_scale(Vec3::splat(anim.kind.scale()))
}

fn texture_key(t: &TextureData) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (t.width, t.height).hash(&mut h);
    t.rgba.hash(&mut h);
    h.finish()
}

impl ModelLibrary {
    /// Loads every model from `models/` in the assets folder (`engine::asset_files`: the file
    /// on disk if there is one, else the copy embedded in the game); models whose file is
    /// missing or broken are generated in memory instead (logged as a warning).
    pub fn load(renderer: &mut Renderer) -> anyhow::Result<ModelLibrary> {
        let mut textures: HashMap<u64, TextureId> = HashMap::new();
        let white = renderer.white_texture();
        let mut rigs = Vec::with_capacity(ModelId::ALL.len());
        let mut bounds = Vec::with_capacity(ModelId::ALL.len());
        let mut regenerated = Vec::new();
        for id in ModelId::ALL {
            let path = format!("models/{}", id.file_name());
            let loaded =
                engine::assets::gltf::load_asset(&path).and_then(|parts| match_parts(id, &parts));
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
            bounds.push(rest_bounds(id, &parts));
            let mut rig = Rig::new(id.joints(), |i| {
                let (mesh, texture, color) = &parts[i];
                let texture = match texture {
                    Some(t) => *textures
                        .entry(texture_key(t))
                        .or_insert_with(|| renderer.create_texture(t)),
                    None => white,
                };
                (renderer.create_mesh(mesh), texture, *color)
            });
            // Telegraph glows and weak points can be recoloured by the colour-blind palettes
            // (`Tint::glow_color` / `core_color`): upload a greyscale copy of each. The Shrike
            // and its kit get one of every part (`Tint::flat`: the Waxwork's wax double).
            let figure = matches!(id, ModelId::Shrike | ModelId::ShrikeKit);
            for (part, (mesh, _, _)) in rig.parts.iter_mut().zip(&parts) {
                if figure || matches!(part.shade, Shade::Glow | Shade::Core) {
                    part.recolour_mesh = Some(renderer.create_mesh(&neutral_mesh(mesh)));
                }
            }
            rigs.push(rig);
        }
        Ok(ModelLibrary {
            rigs,
            bounds,
            regenerated,
        })
    }

    pub fn get(&self, id: ModelId) -> &Rig {
        &self.rigs[id.index()]
    }

    /// Model-space bounds (min, max) of a model in its rest pose (menu previews frame it).
    pub fn bounds(&self, id: ModelId) -> (Vec3, Vec3) {
        self.bounds.get(id.index()).copied().unwrap_or_default()
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
        self.draw_player_scaled(anim, feet, 1.0, tint, out)
    }

    /// [`ModelLibrary::draw_player`] scaled about the feet (other players use
    /// `shrike::REMOTE_SCALE`: drawn 10 % larger to read at a distance; visual only).
    pub fn draw_player_scaled(
        &self,
        anim: &PlayerAnim,
        feet: Vec3,
        scale: f32,
        tint: &Tint,
        out: &mut Vec<DrawItem>,
    ) -> PlayerDrawInfo {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let root = shrike::root_transform(anim, feet) * Mat4::from_scale(Vec3::splat(scale));
        self.draw_shrike(anim, root, tint, out)
    }

    /// Draws a Shrike posed from `anim` (and its cosmetics: `anim.cosmetics`, painted through
    /// `Tint::with_cosmetics`; `tint.team` stays the identity colour) with its model origin
    /// (feet) at `root` — world space, or view space for previews in the viewmodel pass.
    pub fn draw_shrike(
        &self,
        anim: &PlayerAnim,
        root: Mat4,
        tint: &Tint,
        out: &mut Vec<DrawItem>,
    ) -> PlayerDrawInfo {
        let rig = self.get(ModelId::Shrike);
        let (pose, drape) = shrike::pose_full(anim);
        let tint = &tint.with_cosmetics(&anim.cosmetics);
        rig.draw(&pose, root, tint, out);
        let mut world = Vec::with_capacity(rig.parts.len());
        rig.world_transforms(&pose, root, &mut world);
        cosmetics::draw(
            self.get(ModelId::ShrikeKit),
            &anim.cosmetics,
            &world,
            &pose,
            &drape,
            tint,
            out,
        );
        let hand = world[shrike::HAND_R];
        let weapon = weapons::weapon_kind(anim.weapon);
        let gun_root = hand * shrike::grip_transform();
        let ability = weapons::Ability::from_id(anim.ability)
            .filter(|a| a.fits(weapon))
            .unwrap_or(weapon.default_build().abilities[0]);
        let gun_pose = weapons::weapon_pose(&weapons::HeldWeapon {
            weapon,
            ability,
            kick: anim.firing,
            charge: anim.charge,
            draw: anim.draw,
            time: anim.time,
            ..Default::default()
        });
        let head = world[shrike::HEAD];
        // The Kindle Flask in the left hand while it is drunk (`weapons::drink`).
        if let Some(local) = weapons::drink::third_person_flask(anim.drinking) {
            let mut flask = self.get(ModelId::KindleFlask).rest_pose();
            let g = weapons::drink::flask_glow(anim.drinking);
            flask.glow = Vec4::new(g, g, g, 1.0);
            if weapons::drink::unstoppered(anim.drinking) {
                flask.hide(super::knacks::FLASK_STOPPER);
            }
            self.get(ModelId::KindleFlask)
                .draw(&flask, world[shrike::HAND_L] * local, tint, out);
        }
        // Melee: the Trimmer's halves in both hands, the Tamper; the muzzle is the striking
        // edge / foot (`weapons::melee`).
        if let Some(m) = weapons::melee::of_id(anim.weapon).filter(|_| !anim.unarmed) {
            let color = weapons::ability_color(ability).truncate();
            let kick = anim.swing.flare(m).max(anim.firing);
            let mut pose = weapons::melee::pose(m, color, kick, anim.time);
            let left = world[shrike::HAND_L] * shrike::grip_transform();
            if m == weapons::melee::MeleeWeapon::Trimmer {
                pose.set(
                    weapons::trimmer::BLADE_L,
                    weapons::melee::left_blade_local(gun_root, left),
                );
            }
            // The hurled piece is in the air (it comes back): the hand is empty of it.
            if anim.hurled || weapons::melee::hurl_hidden(&anim.swing) {
                weapons::melee::hide_hurled(m, &mut pose);
            }
            if !anim.dead {
                self.get(weapons::melee::model(m))
                    .draw(&pose, gun_root, tint, out);
            }
            let hand = if anim.swing.left && m == weapons::melee::MeleeWeapon::Trimmer {
                left
            } else {
                gun_root
            };
            return PlayerDrawInfo {
                muzzle: hand.transform_point3(weapons::melee::muzzle(m)),
                muzzle_dir: hand
                    .transform_vector3(Vec3::NEG_Z)
                    .normalize_or(Vec3::NEG_Z),
                head: head.transform_point3(Vec3::new(0.0, 0.16, 0.0)),
            };
        }
        let gun = self.get(weapon.model());
        if !anim.dead && !anim.unarmed {
            gun.draw(&gun_pose, gun_root, tint, out);
        }
        PlayerDrawInfo {
            muzzle: gun_root.transform_point3(weapon.muzzle()),
            muzzle_dir: gun_root
                .transform_vector3(Vec3::NEG_Z)
                .normalize_or(Vec3::NEG_Z),
            head: head.transform_point3(Vec3::new(0.0, 0.16, 0.0)),
        }
    }

    /// Draws an enemy posed from `anim` with its model origin (feet; the Harrier's hover
    /// centre) at `root`, [`EnemyKind::scale`](enemies::EnemyKind::scale) times its built size.
    pub fn draw_enemy(&self, anim: &EnemyAnim, root: Mat4, tint: &Tint, out: &mut Vec<DrawItem>) {
        let id = anim.kind.model();
        let pose = enemies::pose(anim);
        self.get(id).draw(&pose, enemy_root(anim, root), tint, out);
    }

    /// World-space point an enemy attacks from in its current pose: the Gnawer's jaw, the
    /// Censer's censer pot (ember bombs leave it), the Harrier's beak, the Gaoler's door (bash)
    /// or flail ball, the Heron's lens (needle and laser), the Bellwether's bell (toll) or fist,
    /// the Snuffer's cap, the Smelter's crucible lip (pour) or crucible, the Effigy's sigil.
    /// For muzzle flashes / impact effects; gameplay positions come from the server.
    pub fn enemy_attack_point(&self, anim: &EnemyAnim, root: Mat4) -> Vec3 {
        use enemies::{
            EnemyKind, bellwether, censer, effigy, gaoler, gnawer, harrier, heron, smelter, snuffer,
        };
        let (part, offset) = match anim.kind {
            EnemyKind::Gnawer => (gnawer::JAW, Vec3::new(0.0, -0.1, -0.55)),
            EnemyKind::Censer => (censer::CENSER, Vec3::ZERO),
            EnemyKind::Harrier => (harrier::HEAD, Vec3::new(0.0, -0.1, -0.4)),
            EnemyKind::Gaoler if anim.attack == gaoler::BASH => {
                (gaoler::SHIELD, Vec3::new(0.38, 0.1, -0.3))
            }
            EnemyKind::Gaoler => (gaoler::FLAIL, Vec3::ZERO),
            EnemyKind::Heron => (heron::LENS, Vec3::new(0.0, 0.14, -0.26)),
            EnemyKind::Bellwether if anim.attack == bellwether::TOLL => {
                (bellwether::BELL, Vec3::new(0.0, -0.5, 0.0))
            }
            EnemyKind::Bellwether => (bellwether::FOREARM_R, Vec3::new(0.0, -0.85, -0.2)),
            EnemyKind::Snuffer => (snuffer::CAP, Vec3::new(0.0, -0.4, -0.14)),
            EnemyKind::Smelter if anim.attack == smelter::POUR => {
                (smelter::CRUCIBLE, Vec3::new(0.0, 0.05, -0.08))
            }
            EnemyKind::Smelter => (smelter::CRUCIBLE, Vec3::new(0.0, -0.5, 0.55)),
            EnemyKind::Effigy => (effigy::SIGIL, Vec3::new(0.0, 0.85, -0.35)),
        };
        let pose = enemies::pose(anim);
        self.get(anim.kind.model())
            .part_transform(&pose, enemy_root(anim, root), part)
            .transform_point3(offset)
    }

    /// Draws the first-person viewmodel (view space) for `anim`: the weapon in the right hand,
    /// the punching left fist and the Thorn throw. Push the items into
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The standalone game draws the embedded models: each must match its part table, or it
    /// would be generated in memory at every start.
    #[test]
    fn every_model_is_embedded_and_matches_its_part_table() {
        for id in ModelId::ALL {
            let path = format!("models/{}", id.file_name());
            let bytes = engine::asset_files::embedded(&path)
                .unwrap_or_else(|| panic!("{path} is not embedded"));
            let parts =
                engine::assets::gltf::load_slice(bytes).unwrap_or_else(|e| panic!("{path}: {e:#}"));
            match_parts(id, &parts).unwrap_or_else(|e| panic!("{path}: {e:#}"));
        }
    }
}
