//! Rigid hierarchical rigs: parts with pivots, parents and procedural local transforms.
//!
//! A model is a list of parts ([`JointDef`]s, parents before children). Each part's mesh is
//! stored in *joint space* (model-space bind pose minus the joint pivot), so a part's world
//! transform is
//!
//! ```text
//! world[i] = world[parent] * translate(pivot[i] - pivot[parent]) * pose.locals[i]
//! world[root part] = root * translate(pivot[i]) * pose.locals[i]
//! ```
//!
//! With every local = identity the model is drawn in its bind pose. Pose functions only ever
//! rotate / translate / scale parts around their pivots.

use engine::gfx::{DrawItem, MeshId, TextureId};
use engine::glam::{Mat4, Quat, Vec3, Vec4};

/// How a part is tinted and lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shade {
    /// Lit normally; colour = vertex colour x texture x base colour x `Tint::body`.
    Lit,
    /// Lit, additionally multiplied by `Tint::team` (player / team colour slot; vertex colours
    /// of these parts are near-white).
    Team,
    /// Emissive (unlit) *telegraph* part, multiplied by `Pose::glow` and `Tint::glow`. Pose
    /// functions drive `Pose::glow` from the attack state (dim idle, flaring through the
    /// wind-up); the client can additionally flash it through `Tint::glow`.
    Glow,
    /// Emissive (unlit) steady part — weak points, lamps, the Shrike's blood core —
    /// multiplied by `Pose::core`.
    Core,
    /// Emissive part in the team colour (Shrike visor): `Tint::team x Pose::glow`.
    TeamGlow,
}

impl Shade {
    pub fn is_emissive(self) -> bool {
        matches!(self, Shade::Glow | Shade::Core | Shade::TeamGlow)
    }
}

/// Static description of one part of a model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointDef {
    /// Unique within the model; also the glTF mesh / node name.
    pub name: &'static str,
    /// Index of the parent part (always lower than this part's index).
    pub parent: Option<usize>,
    /// Joint position in model space (bind pose); the part rotates around it.
    pub pivot: Vec3,
    pub shade: Shade,
}

/// `parent` value for root parts in [`joint`].
pub const ROOT: usize = usize::MAX;

/// Const constructor for joint tables.
pub const fn joint(name: &'static str, parent: usize, pivot: [f32; 3], shade: Shade) -> JointDef {
    JointDef {
        name,
        parent: if parent == ROOT { None } else { Some(parent) },
        pivot: Vec3::new(pivot[0], pivot[1], pivot[2]),
        shade,
    }
}

/// A single unanimated part at the model origin.
pub const fn static_part(name: &'static str, shade: Shade) -> JointDef {
    joint(name, ROOT, [0.0, 0.0, 0.0], shade)
}

/// Draw tints applied on top of the pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    /// Multiplies every part (hit flash: > 1; fade-out: alpha < 1).
    pub body: Vec4,
    /// Player / team colour for [`Shade::Team`] and [`Shade::TeamGlow`] parts.
    pub team: Vec4,
    /// Extra multiplier for [`Shade::Glow`] parts (client-side telegraph flashing).
    /// Emissive colours are not clamped before the framebuffer, so values > 1 push towards
    /// white.
    pub glow: Vec4,
}

/// The Shrikes' default colour (single-player / co-op): blood red.
pub const DEFAULT_TEAM_COLOR: Vec4 = Vec4::new(0.85, 0.07, 0.09, 1.0);

impl Default for Tint {
    fn default() -> Self {
        Self {
            body: Vec4::ONE,
            team: DEFAULT_TEAM_COLOR,
            glow: Vec4::ONE,
        }
    }
}

impl Tint {
    /// Default tint with a different team colour.
    pub fn team(color: Vec4) -> Self {
        Self {
            team: color,
            ..Self::default()
        }
    }
}

/// Per-part local transforms plus glow state, produced by the pose functions.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    /// One transform per part, applied at the part's pivot (in the parent joint's frame).
    pub locals: Vec<Mat4>,
    /// Multiplier for [`Shade::Glow`] / [`Shade::TeamGlow`] parts (telegraph intensity /
    /// colour; may exceed 1 for a white-hot flash).
    pub glow: Vec4,
    /// Multiplier for [`Shade::Core`] parts.
    pub core: Vec4,
    /// Bit `i` set = part `i` (and nothing else) is not drawn.
    pub hidden: u64,
}

impl Pose {
    /// Bind pose for `parts` parts.
    pub fn rest(parts: usize) -> Pose {
        Pose {
            locals: vec![Mat4::IDENTITY; parts],
            glow: Vec4::ONE,
            core: Vec4::ONE,
            hidden: 0,
        }
    }

    pub fn set(&mut self, part: usize, local: Mat4) {
        if let Some(m) = self.locals.get_mut(part) {
            *m = local;
        }
    }

    /// Sets a pure rotation for `part`.
    pub fn rotate(&mut self, part: usize, rotation: Quat) {
        self.set(part, Mat4::from_quat(rotation));
    }

    /// Rotation plus translation (translation in the parent's joint frame).
    pub fn rotate_move(&mut self, part: usize, rotation: Quat, offset: Vec3) {
        self.set(part, Mat4::from_rotation_translation(rotation, offset));
    }

    /// Post-multiplies the part's current local transform.
    pub fn then(&mut self, part: usize, m: Mat4) {
        if let Some(l) = self.locals.get_mut(part) {
            *l *= m;
        }
    }

    pub fn hide(&mut self, part: usize) {
        if part < 64 {
            self.hidden |= 1 << part;
        }
    }

    pub fn is_hidden(&self, part: usize) -> bool {
        part < 64 && self.hidden & (1 << part) != 0
    }

    /// True when every matrix and the glow are finite.
    pub fn is_finite(&self) -> bool {
        self.glow.is_finite() && self.core.is_finite() && self.locals.iter().all(|m| m.is_finite())
    }
}

/// One uploaded part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigPart {
    pub name: &'static str,
    pub mesh: MeshId,
    pub texture: TextureId,
    pub parent: Option<usize>,
    /// Joint position in model space (bind pose).
    pub pivot: Vec3,
    /// `translate(pivot - parent pivot)`: the part's bind transform relative to its parent.
    pub bind: Mat4,
    pub shade: Shade,
    /// Material base colour from the glTF (alpha < 1 = translucent part).
    pub base_color: Vec4,
}

/// A drawable model: parts with pivots and parents (static models have root parts only).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rig {
    pub parts: Vec<RigPart>,
}

/// Most parts a rig may have (the hidden mask is a `u64`).
pub const MAX_PARTS: usize = 64;

impl Rig {
    /// Builds a rig from a joint table; `resources(i)` gives part `i`'s mesh, texture and
    /// base colour.
    pub fn new(
        joints: &[JointDef],
        mut resources: impl FnMut(usize) -> (MeshId, TextureId, Vec4),
    ) -> Rig {
        let parts = joints
            .iter()
            .enumerate()
            .map(|(i, j)| {
                let parent_pivot = j.parent.map(|p| joints[p].pivot).unwrap_or(Vec3::ZERO);
                let (mesh, texture, base_color) = resources(i);
                RigPart {
                    name: j.name,
                    mesh,
                    texture,
                    parent: j.parent,
                    pivot: j.pivot,
                    bind: Mat4::from_translation(j.pivot - parent_pivot),
                    shade: j.shade,
                    base_color,
                }
            })
            .collect();
        Rig { parts }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.name == name)
    }

    pub fn rest_pose(&self) -> Pose {
        Pose::rest(self.parts.len())
    }

    /// World transform of every part (`out` is cleared first). Missing pose entries count as
    /// identity.
    pub fn world_transforms(&self, pose: &Pose, root: Mat4, out: &mut Vec<Mat4>) {
        out.clear();
        for (i, part) in self.parts.iter().enumerate() {
            let parent = match part.parent {
                Some(p) if p < out.len() => out[p],
                _ => root,
            };
            let local = pose.locals.get(i).copied().unwrap_or(Mat4::IDENTITY);
            out.push(parent * part.bind * local);
        }
    }

    /// World transform of a single part (walks up the parent chain).
    pub fn part_transform(&self, pose: &Pose, root: Mat4, part: usize) -> Mat4 {
        let Some(p) = self.parts.get(part) else {
            return root;
        };
        let parent = match p.parent {
            Some(parent) if parent < part => self.part_transform(pose, root, parent),
            _ => root,
        };
        parent * p.bind * pose.locals.get(part).copied().unwrap_or(Mat4::IDENTITY)
    }

    /// Appends one [`DrawItem`] per visible part.
    pub fn draw(&self, pose: &Pose, root: Mat4, tint: &Tint, out: &mut Vec<DrawItem>) {
        let mut world = [Mat4::IDENTITY; MAX_PARTS];
        for (i, part) in self.parts.iter().enumerate().take(MAX_PARTS) {
            let parent = match part.parent {
                Some(p) if p < i => world[p],
                _ => root,
            };
            let local = pose.locals.get(i).copied().unwrap_or(Mat4::IDENTITY);
            world[i] = parent * part.bind * local;
            if pose.is_hidden(i) {
                continue;
            }
            let color = part.base_color * tint.body;
            let (color, unlit) = match part.shade {
                Shade::Lit => (color, false),
                Shade::Team => (color * tint.team, false),
                Shade::Glow => (color * pose.glow * tint.glow, true),
                Shade::Core => (color * pose.core, true),
                Shade::TeamGlow => (color * tint.team * pose.glow, true),
            };
            out.push(DrawItem {
                mesh: part.mesh,
                texture: part.texture,
                transform: world[i],
                tint: color,
                unlit,
            });
        }
    }

    /// Draws the bind pose (static props, projectiles).
    pub fn draw_static(&self, root: Mat4, tint: &Tint, out: &mut Vec<DrawItem>) {
        self.draw(&self.rest_pose(), root, tint, out);
    }
}

/// Validates a joint table: parents precede children, names are unique, at most
/// [`MAX_PARTS`] parts.
pub fn validate_joints(joints: &[JointDef]) -> Result<(), String> {
    if joints.is_empty() || joints.len() > MAX_PARTS {
        return Err(format!("{} parts (must be 1..={MAX_PARTS})", joints.len()));
    }
    for (i, j) in joints.iter().enumerate() {
        if let Some(p) = j.parent
            && p >= i
        {
            return Err(format!("part '{}' has parent {p} >= its index {i}", j.name));
        }
        if joints[..i].iter().any(|o| o.name == j.name) {
            return Err(format!("duplicate part name '{}'", j.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOINTS: [JointDef; 3] = [
        joint("hip", ROOT, [0.0, 1.0, 0.0], Shade::Lit),
        joint("thigh", 0, [0.2, 1.0, 0.0], Shade::Team),
        joint("shin", 1, [0.2, 0.5, 0.0], Shade::Glow),
    ];

    fn rig() -> Rig {
        Rig::new(&JOINTS, |i| (MeshId(i as u32), TextureId(0), Vec4::ONE))
    }

    #[test]
    fn rest_pose_places_parts_at_pivots() {
        let r = rig();
        let mut w = Vec::new();
        r.world_transforms(&r.rest_pose(), Mat4::from_translation(Vec3::X), &mut w);
        for (i, j) in JOINTS.iter().enumerate() {
            let p = w[i].transform_point3(Vec3::ZERO);
            assert!(p.distance(j.pivot + Vec3::X) < 1e-6);
            assert!(
                r.part_transform(&r.rest_pose(), Mat4::from_translation(Vec3::X), i)
                    .abs_diff_eq(w[i], 1e-6)
            );
        }
    }

    #[test]
    fn rotation_pivots_around_joint() {
        let r = rig();
        let mut pose = r.rest_pose();
        // Swing the thigh forward 90 degrees: the knee moves from below the hip to in front.
        pose.rotate(1, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));
        let mut w = Vec::new();
        r.world_transforms(&pose, Mat4::IDENTITY, &mut w);
        let knee = w[2].transform_point3(Vec3::ZERO);
        assert!(knee.distance(Vec3::new(0.2, 1.0, -0.5)) < 1e-5, "{knee}");
    }

    #[test]
    fn draw_applies_shades_and_hiding() {
        let r = rig();
        let mut pose = r.rest_pose();
        pose.glow = Vec4::splat(2.0);
        pose.hide(0);
        let tint = Tint::team(Vec4::new(0.0, 1.0, 0.0, 1.0));
        let mut out = Vec::new();
        r.draw(&pose, Mat4::IDENTITY, &tint, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].tint, Vec4::new(0.0, 1.0, 0.0, 1.0));
        assert!(!out[0].unlit);
        assert_eq!(out[1].tint, Vec4::splat(2.0));
        assert!(out[1].unlit);
    }

    #[test]
    fn validation() {
        assert!(validate_joints(&JOINTS).is_ok());
        let bad = [
            joint("a", 1, [0.0; 3], Shade::Lit),
            joint("b", ROOT, [0.0; 3], Shade::Lit),
        ];
        assert!(validate_joints(&bad).is_err());
        let dup = [
            joint("a", ROOT, [0.0; 3], Shade::Lit),
            joint("a", 0, [0.0; 3], Shade::Lit),
        ];
        assert!(validate_joints(&dup).is_err());
    }
}
