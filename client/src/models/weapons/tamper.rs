//! Tamper: a foundry ramming hammer, the tool that packs moulding sand round a pattern, made
//! into a two-handed maul. A long ash haft with a leather grip and a brass pommel cap; iron
//! langets strapping the head on; the head a black-iron casting with the rammer's broad round
//! foot on its striking face (ringed in brass) and a wedge peen behind. The casting is still
//! hot: slag glows in a vent on each flank (`Core`), and a band round its waist takes the
//! selected ability's colour (`Glow`).
//!
//! Model space: the right hand's grip at the origin. The haft runs up the grip's axis (raked
//! like a pistol grip), so the head stands above the fist with the foot facing forward (-Z):
//! a swing pitches it forward and down onto what is in front.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{unit, wave};
use crate::models::PartSet;
use crate::models::kit::{self, BRASS, DARK_LEATHER, IRON, LEATHER, gem, limb, rgb, tube};
use crate::models::rig::{JointDef, Pose, Shade, joint, static_part};

pub const HAFT: usize = 0;
pub const HEAD: usize = 1;
pub const BAND: usize = 2;
pub const VENT: usize = 3;

/// The haft's axis (up the grip, raked back like the hand's grip).
const AXIS: Vec3 = Vec3::new(0.0, 0.955, -0.296);
/// The head's axis: perpendicular to the haft, the foot facing forward.
const FACE: Vec3 = Vec3::new(0.0, -0.296, -0.955);
/// How far up the haft the head's centre sits.
const HEAD_AT: f32 = 0.66;
/// The head's centre (its pivot: a hurled head spins about it).
pub const HEAD_CENTER: [f32; 3] = [0.0, 0.955 * HEAD_AT, -0.296 * HEAD_AT];
/// Where a grip on the haft is natural for the left hand (two-handed swings), model space.
pub const LEFT_HAND: Vec3 = Vec3::new(0.0, 0.955 * 0.3, -0.296 * 0.3);

pub const JOINTS: [JointDef; 4] = [
    static_part("haft", Shade::Lit),
    joint("head", HAFT, HEAD_CENTER, Shade::Lit),
    joint("band", HEAD, HEAD_CENTER, Shade::Glow),
    joint("vent", HEAD, HEAD_CENTER, Shade::Core),
];

/// The centre of the striking foot (slam effects start here).
pub const MUZZLE: Vec3 = Vec3::new(
    0.0,
    0.955 * HEAD_AT - 0.296 * 0.16,
    -0.296 * HEAD_AT - 0.955 * 0.16,
);

/// Ash wood, the cast iron's darker foot.
const ASH: Vec4 = Vec4::new(0.6, 0.46, 0.3, 1.0);
const SLAG: Vec4 = Vec4::new(1.0, 0.45, 0.12, 1.0);

/// Model-space point of a head-local one: `x` across, `up` along the haft, `f` along the
/// head (+ = towards the foot), about the head's centre.
fn head(x: f32, up: f32, f: f32) -> Vec3 {
    Vec3::from(HEAD_CENTER) + Vec3::X * x + AXIS * up + FACE * f
}

/// A point `d` metres up the haft from the grip.
fn haft(d: f32) -> Vec3 {
    AXIS * d
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    // The haft: ash, octagonal, slightly swelling towards the head; the leather grip over its
    // lower third; a brass pommel cap.
    p.add(
        HAFT,
        tube(haft(-0.17), haft(HEAD_AT - 0.04), 0.021, 0.025, 8, ASH),
    );
    p.add(
        HAFT,
        tube(haft(-0.13), haft(0.36), 0.026, 0.026, 8, DARK_LEATHER),
    );
    for d in [-0.08, 0.02, 0.12, 0.22, 0.32] {
        p.add(
            HAFT,
            tube(haft(d - 0.006), haft(d + 0.006), 0.029, 0.029, 8, LEATHER),
        );
    }
    p.add(HAFT, tube(haft(-0.2), haft(-0.15), 0.034, 0.03, 8, BRASS));
    p.add(HAFT, gem(haft(-0.205), 0.022, 0, BRASS));
    kit::finish(p.mesh(HAFT));

    // The head: a waisted octagonal casting along FACE.
    let ring = |f: f32, r: Vec2| {
        kit::ngon_ring(
            head(0.0, 0.0, f),
            Vec3::X,
            AXIS,
            r,
            8,
            std::f32::consts::PI / 8.0,
        )
    };
    p.add(
        HEAD,
        kit::loft(
            &[
                ring(-0.1, Vec2::new(0.062, 0.062)),
                ring(-0.02, Vec2::new(0.07, 0.07)),
                ring(0.06, Vec2::new(0.07, 0.07)),
                ring(0.1, Vec2::new(0.08, 0.08)),
            ],
            IRON,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    // The rammer's foot: a broad round disc, ringed in brass.
    p.add(
        HEAD,
        kit::loft(
            &[
                ring(0.1, Vec2::new(0.108, 0.108)),
                ring(0.14, Vec2::new(0.112, 0.112)),
                ring(0.165, Vec2::new(0.1, 0.1)),
            ],
            rgb(0x4a4642),
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        HEAD,
        kit::band(head(0.0, 0.0, 0.118), FACE, 0.113, 0.02, 0.008, 12, BRASS),
    );
    // Rivets round the foot's rim.
    for k in 0..6 {
        let a = k as f32 / 6.0 * std::f32::consts::TAU;
        p.add(
            HEAD,
            gem(
                head(a.cos() * 0.1, a.sin() * 0.1, 0.142),
                0.008,
                0,
                rgb(0x3a3632),
            ),
        );
    }
    // The wedge peen behind: wide across, thin along the haft.
    p.add(
        HEAD,
        kit::loft(
            &[
                kit::rect_ring(head(0.0, 0.0, -0.1), Vec3::X, AXIS, Vec2::new(0.055, 0.05)),
                kit::rect_ring(head(0.0, 0.0, -0.2), Vec3::X, AXIS, Vec2::new(0.05, 0.008)),
            ],
            IRON,
            false,
            (false, true),
            kit::Uv::PerFace,
        ),
    );
    // Langets: iron straps down the haft from the head's eye, and the wedge in the eye.
    for side in [-1.0f32, 1.0] {
        p.add(
            HEAD,
            limb(
                head(side * 0.028, -0.04, 0.0),
                haft(HEAD_AT - 0.22) + Vec3::X * side * 0.027,
                Vec2::new(0.004, 0.016),
                Vec2::new(0.003, 0.012),
                rgb(0x34312e),
            ),
        );
    }
    p.add(
        HEAD,
        tube(
            head(0.0, 0.062, 0.0),
            head(0.0, 0.08, 0.0),
            0.026,
            0.02,
            8,
            BRASS,
        ),
    );
    kit::finish(p.mesh(HEAD));

    // Glow band round the waist (the ability colour).
    p.add(
        BAND,
        kit::band(
            head(0.0, 0.0, 0.02),
            FACE,
            0.072,
            0.022,
            0.006,
            8,
            Vec4::ONE,
        ),
    );
    // Hot slag showing through two slits of a vent on each flank.
    for side in [-1.0f32, 1.0] {
        for f in [-0.03, -0.06] {
            p.add(
                VENT,
                kit::block(
                    head(side * 0.067, 0.0, f),
                    Vec3::new(0.004, 0.032, 0.006),
                    Quat::from_rotation_arc(Vec3::Y, AXIS),
                    SLAG,
                ),
            );
        }
    }

    p.texture(HAFT, kit::tex_leather());
    p.texture(HEAD, kit::tex_grime());
    p.texture(BAND, kit::tex_glow());
    p.texture(VENT, kit::tex_glow());
    p.finish()
}

/// The band flares on a swing (`kick` 0..1); the slag in the vents breathes.
pub(super) fn pose(kick: f32, time: f64, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    *glow += unit(kick) * 1.2;
    let heat = 1.15 + 0.12 * wave(time, 0.8) + unit(kick) * 0.5;
    pose.core = Vec4::new(heat, heat, heat, 1.0);
    pose
}

/// The head's centre in model space (a hurled head spins about it).
pub fn head_center() -> Vec3 {
    Vec3::from(HEAD_CENTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haft_and_face_are_perpendicular_unit_axes() {
        assert!((AXIS.length() - 1.0).abs() < 0.01);
        assert!((FACE.length() - 1.0).abs() < 0.01);
        assert!(AXIS.dot(FACE).abs() < 1e-3);
        // The foot faces forward and the head stands above the grip.
        const { assert!(MUZZLE.z < -0.3 && MUZZLE.y > 0.4) };
    }
}
