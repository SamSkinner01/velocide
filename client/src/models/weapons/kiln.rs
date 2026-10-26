//! Kiln: handheld ember sprayer. A long terracotta spout runs the length of the gun on top,
//! glazed in bands and strapped with iron, from a brass feed cap at the back to a flared,
//! glazed mouth at the front. Under it hangs the firebox: a fat ceramic belly with a grated
//! door on each flank glowing with live coals. A short chimney rises from the back of the
//! spout with a hinged iron damper that rattles while it roars. A wooden pistol grip under the
//! back, a leather-wrapped fore grip under the firebox.

use engine::assets::gltf::ModelPart;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, IRON, add_mirrored, block, gem, limb, rgb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const DAMPER: usize = 1;
pub const COALS: usize = 2;
pub const GLOW: usize = 3;

/// The spout's axis height.
const SPOUT_Y: f32 = 0.105;
/// Firebox centre and radii (the belly under the spout).
const BOX_C: [f32; 3] = [0.0, 0.035, -0.2];
const BOX_R: [f32; 3] = [0.07, 0.066, 0.105];
/// The chimney's axis (x, z) and the damper's hinge on its lip.
const CHIMNEY: [f32; 2] = [0.0, -0.035];
pub const DAMPER_HINGE: [f32; 3] = [0.0, 0.205, -0.012];

pub const JOINTS: [JointDef; 4] = [
    static_part("body", Shade::Lit),
    joint("damper", ROOT, DAMPER_HINGE, Shade::Lit),
    static_part("coals", Shade::Core),
    static_part("glow", Shade::Glow),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, SPOUT_Y, -0.5);

/// Fired terracotta and a celadon glaze.
const TERRACOTTA: Vec4 = Vec4::new(0.68, 0.37, 0.24, 1.0);
const GLAZE: Vec4 = Vec4::new(0.45, 0.64, 0.56, 1.0);
/// Live coals.
const COAL: Vec4 = Vec4::new(1.0, 0.5, 0.14, 1.0);

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let c = Vec3::from(BOX_C);
    let r = Vec3::from(BOX_R);
    let wood = rgb(0x5a3722);
    let ring = |z: f32, rr: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, SPOUT_Y, z),
            Vec3::X,
            Vec3::Y,
            Vec2::splat(rr),
            8,
            std::f32::consts::PI / 8.0,
        )
    };
    // The spout: a long ceramic barrel tapering forward, then a flared glazed mouth.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(0.06, 0.036),
                ring(-0.1, 0.042),
                ring(-0.3, 0.032),
                ring(-0.42, 0.026),
            ],
            TERRACOTTA,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        BODY,
        kit::loft(
            &[ring(-0.418, 0.027), ring(-0.47, 0.036), ring(-0.5, 0.05)],
            GLAZE,
            false,
            (false, true),
            kit::Uv::PerFace,
        ),
    );
    // Glazed bands and iron straps along it.
    for (z, rr, color) in [
        (-0.06, 0.045, GLAZE),
        (-0.16, 0.043, IRON),
        (-0.26, 0.037, GLAZE),
        (-0.36, 0.031, IRON),
    ] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, SPOUT_Y, z + 0.01),
                Vec3::new(0.0, SPOUT_Y, z - 0.01),
                rr,
                rr,
                8,
                color,
            ),
        );
    }
    // Brass feed cap at the back.
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, SPOUT_Y, 0.05),
            Vec3::new(0.0, SPOUT_Y, 0.085),
            0.04,
            0.032,
            8,
            BRASS,
        ),
    );
    p.add(BODY, gem(Vec3::new(0.0, SPOUT_Y, 0.09), 0.015, 0, BRASS));
    // Chimney stack rising from the back of the spout.
    let chimney = |y: f32| Vec3::new(CHIMNEY[0], y, CHIMNEY[1]);
    p.add(
        BODY,
        tube(
            chimney(SPOUT_Y + 0.02),
            chimney(DAMPER_HINGE[1] - 0.004),
            0.024,
            0.02,
            8,
            TERRACOTTA,
        ),
    );
    p.add(
        BODY,
        tube(
            chimney(DAMPER_HINGE[1] - 0.014),
            chimney(DAMPER_HINGE[1]),
            0.025,
            0.025,
            8,
            IRON,
        ),
    );
    // The firebox belly, its neck up into the spout and the grated doors on both flanks.
    p.add(
        BODY,
        kit::ellipsoid(c, r, Quat::IDENTITY, 5, 10, TERRACOTTA, false),
    );
    p.add(
        BODY,
        kit::band(
            Vec3::new(0.0, c.y, c.z),
            Vec3::Z,
            r.x + 0.002,
            0.014,
            0.008,
            10,
            IRON,
        ),
    );
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, c.y + r.y - 0.02, c.z),
            Vec3::new(0.0, SPOUT_Y - 0.02, c.z),
            0.034,
            0.03,
            8,
            GLAZE,
        ),
    );
    let door = block(
        Vec3::new(r.x - 0.006, c.y, c.z),
        Vec3::new(0.006, 0.034, 0.05),
        Quat::IDENTITY,
        IRON,
    );
    add_mirrored(p.mesh(BODY), &door);
    for k in 0..3 {
        let bar = block(
            Vec3::new(r.x + 0.002, c.y, c.z - 0.028 + k as f32 * 0.028),
            Vec3::new(0.004, 0.032, 0.004),
            Quat::IDENTITY,
            BLACK_STEEL,
        );
        add_mirrored(p.mesh(BODY), &bar);
    }
    // Wooden pistol grip and the trigger, the wrapped fore grip under the firebox.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, SPOUT_Y - 0.03, 0.035),
            Vec3::new(0.0, -0.105, 0.066),
            Vec2::new(0.02, 0.026),
            Vec2::new(0.019, 0.025),
            wood,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.035, -0.015),
            Vec3::new(0.006, 0.016, 0.004),
            Quat::from_rotation_x(0.3),
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, c.y - r.y + 0.01, c.z - 0.03),
            Vec3::new(0.0, -0.1, c.z - 0.02),
            Vec2::new(0.018, 0.022),
            Vec2::new(0.017, 0.021),
            DARK_LEATHER,
        ),
    );
    kit::finish(p.mesh(BODY));

    // The damper: a round iron flap hinged at the back of the chimney's lip.
    let h = Vec3::from(DAMPER_HINGE);
    let lid = Vec3::new(CHIMNEY[0], h.y, CHIMNEY[1]);
    p.add(
        DAMPER,
        tube(lid, lid + Vec3::Y * 0.006, 0.028, 0.028, 8, IRON),
    );
    p.add(
        DAMPER,
        block(
            h + Vec3::new(0.0, 0.004, 0.0),
            Vec3::new(0.012, 0.004, 0.006),
            Quat::IDENTITY,
            BRASS,
        ),
    );
    kit::finish(p.mesh(DAMPER));

    // Coals behind the door grates, and the embers showing in the chimney's mouth.
    let coals = block(
        Vec3::new(r.x - 0.004, c.y, c.z),
        Vec3::new(0.004, 0.028, 0.044),
        Quat::IDENTITY,
        COAL,
    );
    add_mirrored(p.mesh(COALS), &coals);
    p.add(
        COALS,
        tube(
            Vec3::new(CHIMNEY[0], DAMPER_HINGE[1] - 0.003, CHIMNEY[1]),
            Vec3::new(CHIMNEY[0], DAMPER_HINGE[1] + 0.001, CHIMNEY[1]),
            0.017,
            0.017,
            8,
            COAL,
        ),
    );
    // Glow (the selected ability's colour): the spout's throat and a lamp on the feed cap.
    p.add(
        GLOW,
        tube(
            Vec3::new(0.0, SPOUT_Y, -0.492),
            Vec3::new(0.0, SPOUT_Y, -0.5),
            0.038,
            0.038,
            8,
            Vec4::ONE,
        ),
    );
    p.add(
        GLOW,
        gem(Vec3::new(0.0, SPOUT_Y + 0.035, 0.07), 0.01, 0, Vec4::ONE),
    );

    p.texture(BODY, kit::tex_grime());
    p.texture(DAMPER, kit::tex_plate());
    p.texture(COALS, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}

/// Spraying: the coals roar brighter, the damper rattles, the throat glows; at rest the coals
/// breathe and the damper ticks now and then.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let spray = unit(h.spray);
    let rattle = 0.03 * wave(h.time, 1.9).max(0.0).powi(8)
        + spray * 0.3 * wave(h.time, 13.0).abs() * (0.6 + 0.4 * wave(h.time, 3.1));
    pose.rotate(DAMPER, Quat::from_rotation_x(rattle));
    *glow += spray * 1.3 + unit(h.kick) * 0.3;
    let flicker = 0.12 * wave(h.time, 2.3) + 0.08 * wave(h.time, 7.9) * wave(h.time, 1.1);
    let heat = 1.15 + spray * (0.9 + 0.25 * wave(h.time, 17.0)) + flicker;
    pose.core = Vec4::new(heat, heat, heat, 1.0);
    pose
}
