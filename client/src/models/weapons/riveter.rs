//! Riveter: pneumatic rivet driver. A green-enamelled motor housing with a slotted exhaust cap
//! at the back, a steel driving nose that ends in a rivet-set cup holding a red-hot rivet, and
//! a slanted brass feed of glowing rivets on top. Under the nose a riveted copper pressure tank
//! doubles as the fore grip, with a pressure gauge on the left flank; a black rubber hose runs
//! from the back of the housing out along the left flank and into the back of the tank (in
//! first person it hangs in view). The nose hammers back on every shot and a pulse of air
//! runs down the hose from the tank to the tool.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, COPPER, IRON, MIRROR_STEEL, block, gem, limb, rgb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const NOSE: usize = 1;
pub const RIVET: usize = 2;
pub const HEADS: usize = 3;
pub const GLOW: usize = 4;
/// First of the hose segments (`HOSE + i`, from the tool to the tank).
pub const HOSE: usize = 5;
pub const HOSE_SEGMENTS: usize = 4;

const BORE_Y: f32 = 0.09;
const HOUSING_R: f32 = 0.044;
/// The nose's back end (it slides back from here).
const NOSE_Z: f32 = -0.165;
const TANK_Y: f32 = -0.002;
const TANK_R: f32 = 0.041;

/// Hose path (back of the housing → out along the left flank → back of the tank), a cubic
/// Bézier.
const HOSE_PATH: [[f32; 3]; 4] = [
    [-0.03, BORE_Y - 0.022, 0.07],
    [-0.115, 0.07, 0.03],
    [-0.105, -0.03, -0.07],
    [-0.022, TANK_Y, -0.118],
];

/// Point `t` of [`HOSE_PATH`] (const: the hose joints sit on it).
const fn hose_point(t: f32) -> [f32; 3] {
    let p = HOSE_PATH;
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        p[0][0] * a + p[1][0] * b + p[2][0] * c + p[3][0] * d,
        p[0][1] * a + p[1][1] * b + p[2][1] * c + p[3][1] * d,
        p[0][2] * a + p[1][2] * b + p[2][2] * c + p[3][2] * d,
    ]
}

/// Middle of hose segment `i` (the pulse scales each segment about it).
const fn hose_mid(i: usize) -> [f32; 3] {
    hose_point((i as f32 + 0.5) / HOSE_SEGMENTS as f32)
}

pub const JOINTS: [JointDef; 9] = [
    static_part("body", Shade::Lit),
    joint("nose", ROOT, [0.0, BORE_Y, NOSE_Z], Shade::Lit),
    joint("rivet", NOSE, [0.0, BORE_Y, NOSE_Z], Shade::Core),
    static_part("heads", Shade::Core),
    static_part("glow", Shade::Glow),
    joint("hose_0", ROOT, hose_mid(0), Shade::Lit),
    joint("hose_1", ROOT, hose_mid(1), Shade::Lit),
    joint("hose_2", ROOT, hose_mid(2), Shade::Lit),
    joint("hose_3", ROOT, hose_mid(3), Shade::Lit),
];

pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, -0.45);

/// Green enamel.
const ENAMEL: Vec4 = Vec4::new(0.32, 0.47, 0.38, 1.0);
/// Red-hot rivet heads.
const HOT: Vec4 = Vec4::new(1.0, 0.45, 0.18, 1.0);

fn bezier(t: f32) -> Vec3 {
    Vec3::from(hose_point(t))
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let rubber = rgb(0x1e1b1a);
    let z = |y: f32, z: f32| Vec3::new(0.0, y, z);
    // Motor housing, a slotted exhaust cap and a brass collar where the nose comes out.
    p.add(
        BODY,
        tube(
            z(BORE_Y, 0.085),
            z(BORE_Y, NOSE_Z),
            HOUSING_R,
            HOUSING_R,
            10,
            ENAMEL,
        ),
    );
    p.add(
        BODY,
        tube(
            z(BORE_Y, 0.1),
            z(BORE_Y, 0.08),
            HOUSING_R - 0.008,
            HOUSING_R + 0.002,
            10,
            BLACK_STEEL,
        ),
    );
    for k in 0..3 {
        let zz = 0.074 - k as f32 * 0.02;
        p.add(
            BODY,
            tube(
                z(BORE_Y, zz),
                z(BORE_Y, zz - 0.006),
                HOUSING_R + 0.003,
                HOUSING_R + 0.003,
                10,
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        BODY,
        tube(
            z(BORE_Y, NOSE_Z + 0.02),
            z(BORE_Y, NOSE_Z - 0.008),
            HOUSING_R + 0.004,
            0.032,
            10,
            BRASS,
        ),
    );
    // Brass feed rail on top with four rivets waiting their turn (the heads glow).
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, BORE_Y + 0.04, -0.07),
            Vec3::new(0.0, BORE_Y + 0.115, -0.15),
            Vec2::new(0.016, 0.01),
            Vec2::new(0.016, 0.01),
            BRASS,
        ),
    );
    // Pressure tank: riveted copper with domed ends, fixed under the housing by two straps.
    p.add(
        BODY,
        tube(
            z(TANK_Y, -0.135),
            z(TANK_Y, -0.39),
            TANK_R,
            TANK_R,
            10,
            COPPER,
        ),
    );
    for zz in [-0.135, -0.39] {
        p.add(
            BODY,
            kit::ellipsoid(
                z(TANK_Y, zz),
                Vec3::new(TANK_R, TANK_R, 0.022),
                Quat::IDENTITY,
                3,
                10,
                COPPER,
                false,
            ),
        );
    }
    for zz in [-0.18, -0.34] {
        p.add(
            BODY,
            tube(
                z(TANK_Y, zz + 0.008),
                z(TANK_Y, zz - 0.008),
                TANK_R + 0.004,
                TANK_R + 0.004,
                10,
                BLACK_STEEL,
            ),
        );
        p.add(
            BODY,
            block(
                Vec3::new(0.0, (TANK_Y + BORE_Y) * 0.5, zz),
                Vec3::new(0.012, 0.03, 0.008),
                Quat::IDENTITY,
                BLACK_STEEL,
            ),
        );
    }
    for k in 0..6 {
        let a = k as f32 / 6.0 * std::f32::consts::TAU;
        let d = Vec3::new(a.cos(), a.sin(), 0.0) * (TANK_R + 0.002);
        p.add(BODY, gem(z(TANK_Y, -0.26) + d, 0.005, 0, BRASS));
    }
    // Gauge housing on the left flank (its dial glows).
    p.add(
        BODY,
        tube(
            Vec3::new(-HOUSING_R + 0.004, BORE_Y + 0.01, -0.06),
            Vec3::new(-HOUSING_R - 0.014, BORE_Y + 0.01, -0.06),
            0.026,
            0.026,
            10,
            BRASS,
        ),
    );
    // Hose couplings at both ends.
    let (start, end) = (bezier(0.0), bezier(1.0));
    let (d0, d1) = (
        (bezier(0.05) - start).normalize(),
        (end - bezier(0.95)).normalize(),
    );
    p.add(
        BODY,
        tube(
            start - d0 * 0.008,
            start + d0 * 0.018,
            0.018,
            0.016,
            8,
            BRASS,
        ),
    );
    p.add(
        BODY,
        tube(end - d1 * 0.018, end + d1 * 0.006, 0.018, 0.018, 8, BRASS),
    );
    // Pistol grip and the long trigger paddle.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, BORE_Y - 0.03, 0.035),
            Vec3::new(0.0, -0.105, 0.065),
            Vec2::new(0.02, 0.026),
            Vec2::new(0.019, 0.025),
            rubber,
        ),
    );
    for k in 0..4 {
        let y = 0.03 - k as f32 * 0.035;
        p.add(
            BODY,
            block(
                Vec3::new(0.0, y, 0.044 + (0.06 - y) * 0.2),
                Vec3::new(0.022, 0.004, 0.026),
                Quat::from_rotation_x(-0.2),
                IRON,
            ),
        );
    }
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, BORE_Y - 0.045, -0.02),
            Vec3::new(0.0, -0.04, -0.008),
            Vec2::new(0.006, 0.008),
            Vec2::new(0.006, 0.01),
            BLACK_STEEL,
        ),
    );
    kit::finish(p.mesh(BODY));

    // The driving nose: steel shaft, three rings, the rivet-set cup.
    p.add(
        NOSE,
        tube(
            z(BORE_Y, NOSE_Z),
            z(BORE_Y, -0.41),
            0.025,
            0.022,
            8,
            MIRROR_STEEL,
        ),
    );
    for zz in [-0.22, -0.28, -0.34] {
        p.add(
            NOSE,
            tube(
                z(BORE_Y, zz),
                z(BORE_Y, zz - 0.012),
                0.03,
                0.03,
                8,
                BLACK_STEEL,
            ),
        );
    }
    p.add(
        NOSE,
        kit::loft(
            &[
                kit::ngon_ring(
                    z(BORE_Y, -0.405),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(0.024),
                    8,
                    0.0,
                ),
                kit::ngon_ring(
                    z(BORE_Y, -0.43),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(0.034),
                    8,
                    0.0,
                ),
                kit::ngon_ring(
                    z(BORE_Y, -0.445),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::splat(0.03),
                    8,
                    0.0,
                ),
            ],
            IRON,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    kit::finish(p.mesh(NOSE));
    // The red-hot rivet sitting in the cup.
    p.add(
        RIVET,
        kit::ellipsoid(
            z(BORE_Y, -0.448),
            Vec3::new(0.02, 0.02, 0.011),
            Quat::IDENTITY,
            2,
            8,
            HOT,
            false,
        ),
    );
    // Rivet heads in the feed (red-hot).
    for k in 0..4 {
        let t = 0.18 + k as f32 * 0.22;
        let at =
            Vec3::new(0.0, BORE_Y + 0.04, -0.07).lerp(Vec3::new(0.0, BORE_Y + 0.115, -0.15), t);
        p.add(
            HEADS,
            kit::ellipsoid(
                at + Vec3::new(0.0, 0.014, -0.004),
                Vec3::new(0.014, 0.008, 0.014),
                Quat::from_rotation_x(0.75),
                2,
                6,
                HOT,
                false,
            ),
        );
    }
    // Glow: the gauge dial and a lamp on the exhaust cap.
    p.add(
        GLOW,
        tube(
            Vec3::new(-HOUSING_R - 0.014, BORE_Y + 0.01, -0.06),
            Vec3::new(-HOUSING_R - 0.017, BORE_Y + 0.01, -0.06),
            0.02,
            0.02,
            10,
            Vec4::ONE,
        ),
    );
    p.add(GLOW, gem(z(BORE_Y + 0.03, 0.098), 0.009, 0, Vec4::ONE));
    // Hose: four rubber segments along the Bézier, each bulging as the pulse passes.
    for i in 0..HOSE_SEGMENTS {
        let steps = 3;
        for s in 0..steps {
            let t0 = (i * steps + s) as f32 / (HOSE_SEGMENTS * steps) as f32;
            let t1 = (i * steps + s + 1) as f32 / (HOSE_SEGMENTS * steps) as f32;
            p.add(
                HOSE + i,
                tube(bezier(t0), bezier(t1), 0.0125, 0.0125, 6, rubber),
            );
        }
        kit::finish(p.mesh(HOSE + i));
    }

    p.texture(BODY, kit::tex_plate());
    p.texture(NOSE, kit::tex_mirror());
    p.texture(RIVET, kit::tex_glow());
    p.texture(HEADS, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    for i in 0..HOSE_SEGMENTS {
        p.texture(HOSE + i, kit::tex_grip());
    }
    p.finish()
}

/// Where the air pulse is along the hose (in segments from the tool; ≥ 4 or < 0 = none),
/// from the recoil: `fire_kick` falls by 0.3 in the 0.09 s between rivets, which carries one
/// pulse from the tank (segment 3) to the tool (segment 0).
pub fn pulse_position(kick: f32) -> f32 {
    let since = 1.0 - unit(kick);
    HOSE_SEGMENTS as f32 - since / 0.3 * HOSE_SEGMENTS as f32
}

/// The nose hammers back on the shot, a pulse runs down the hose, the rivets glow red-hot.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let kick = unit(h.kick);
    let hammer = (kick * 3.0 - 2.0).clamp(0.0, 1.0);
    pose.set(
        NOSE,
        Mat4::from_translation(Vec3::new(0.0, 0.0, 0.03 * hammer)),
    );
    let at = pulse_position(kick);
    let breathe = 0.03 * wave(h.time, 0.8);
    for i in 0..HOSE_SEGMENTS {
        let bump = if kick > 0.0 {
            (1.0 - (at - i as f32 - 0.5).abs()).max(0.0)
        } else {
            0.0
        };
        let s = 1.0 + 0.32 * bump + breathe * (1.0 - bump);
        pose.set(HOSE + i, Mat4::from_scale(Vec3::splat(s)));
    }
    *glow += kick * 0.5 + 0.1 * wave(h.time, 3.7);
    let heat = 1.2 + 0.5 * kick + 0.1 * wave(h.time, 6.1);
    pose.core = Vec4::new(heat, heat, heat, 1.0);
    pose
}
