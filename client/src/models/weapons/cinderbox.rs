//! Cinderbox: salvo box launcher. A squat, sooty iron box that flares out towards a brass
//! face plate and band, three short clay-red tubes standing out of the face in a triangle
//! (one on top, two below), slatted vents on both flanks glowing with cinders, an iron carry
//! handle on top, a pistol grip and an upright fore grip. Three lamps on the back face, laid
//! out like the tubes, show which are loaded. A salvo fires the tubes in turn, top, left,
//! right, 0.08 s apart: each mouth flares, its lamp goes dark and the box kicks once per
//! rocket; the lamps relight as the box reloads.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, IRON, add_mirrored, block, chamfer_box_z, gem, limb,
    rgb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const CINDERS: usize = 1;
pub const GLOW: usize = 2;
/// First of the three mouth flares (`FLARE + i`, in firing order).
pub const FLARE: usize = 3;
/// First of the three "loaded" lamps on the back face (`LAMP + i`).
pub const LAMP: usize = 6;
pub const TUBES: usize = 3;

/// Seconds between the rockets of a salvo.
pub const SALVO_GAP: f32 = 0.08;
/// Seconds after a salvo starts until the lamps relight (the box has reloaded).
pub const RELOAD: f32 = 0.9;
const BOX_Y: f32 = 0.11;
const BACK_Z: f32 = 0.07;
const FACE_Z: f32 = -0.33;
/// The box's half size at the back and at the face.
const BACK_HALF: [f32; 2] = [0.058, 0.055];
const FACE_HALF: [f32; 2] = [0.084, 0.076];
const MOUTH_Z: f32 = -0.435;
const TUBE_R: f32 = 0.03;

/// Tube centres (x, y) in firing order: top, lower left, lower right.
const TUBE_XY: [[f32; 2]; TUBES] = [[0.0, 0.152], [-0.042, 0.078], [0.042, 0.078]];

const fn mouth(i: usize) -> [f32; 3] {
    [TUBE_XY[i][0], TUBE_XY[i][1], MOUTH_Z]
}

pub const JOINTS: [JointDef; 9] = [
    static_part("body", Shade::Lit),
    static_part("cinders", Shade::Core),
    static_part("glow", Shade::Glow),
    joint("flare_0", ROOT, mouth(0), Shade::Glow),
    joint("flare_1", ROOT, mouth(1), Shade::Glow),
    joint("flare_2", ROOT, mouth(2), Shade::Glow),
    static_part("lamp_0", Shade::Glow),
    static_part("lamp_1", Shade::Glow),
    static_part("lamp_2", Shade::Glow),
];

/// The box's half size (x, y) at depth `z` (it flares from the back to the face).
fn half_at(z: f32) -> Vec2 {
    let t = ((BACK_Z - z) / (BACK_Z - FACE_Z)).clamp(0.0, 1.0);
    Vec2::from(BACK_HALF).lerp(Vec2::from(FACE_HALF), t)
}

/// Between the three mouths.
pub const MUZZLE: Vec3 = Vec3::new(0.0, 0.103, -0.44);

/// Clay-red tubes.
const CLAY: Vec4 = Vec4::new(0.66, 0.3, 0.2, 1.0);

/// Tube `i`'s mouth (model space).
pub fn tube_mouth(i: usize) -> Vec3 {
    Vec3::from(mouth(i.min(TUBES - 1)))
}

/// Recoil of a salvo `salvo` seconds after it started: one sharp kick per rocket.
pub fn salvo_kick(salvo: Option<f32>) -> f32 {
    let Some(s) = salvo.filter(|s| s.is_finite()) else {
        return 0.0;
    };
    (0..TUBES)
        .map(|i| {
            let x = s - i as f32 * SALVO_GAP;
            if x < 0.0 { 0.0 } else { (-x / 0.07).exp() }
        })
        .sum::<f32>()
        .min(1.0)
}

/// How hard tube `i`'s mouth flares `salvo` seconds into a salvo (0 = not firing).
pub fn tube_flash(salvo: Option<f32>, i: usize) -> f32 {
    let Some(s) = salvo.filter(|s| s.is_finite()) else {
        return 0.0;
    };
    let x = s - i as f32 * SALVO_GAP;
    if !(0.0..0.14).contains(&x) {
        0.0
    } else {
        1.0 - x / 0.14
    }
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let soot = rgb(0x2c2623);
    // The box, flaring towards the face; a dark rim at the back, a brass band and face plate.
    p.add(
        BODY,
        chamfer_box_z(
            BOX_Y,
            BACK_Z,
            FACE_Z,
            Vec2::from(BACK_HALF),
            Vec2::from(FACE_HALF),
            0.02,
            soot,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            BOX_Y,
            BACK_Z + 0.004,
            BACK_Z - 0.012,
            Vec2::from(BACK_HALF) + 0.004,
            half_at(BACK_Z - 0.012) + 0.004,
            0.02,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            BOX_Y,
            -0.29,
            -0.316,
            half_at(-0.29) + 0.005,
            half_at(-0.316) + 0.005,
            0.022,
            BRASS,
        ),
    );
    p.add(
        BODY,
        chamfer_box_z(
            BOX_Y,
            FACE_Z + 0.004,
            FACE_Z - 0.014,
            Vec2::new(0.08, 0.072),
            Vec2::new(0.074, 0.066),
            0.022,
            BRASS,
        ),
    );
    // Three short clay tubes with iron collars and dark bores.
    for [x, y] in TUBE_XY {
        let c = |z: f32| Vec3::new(x, y, z);
        p.add(
            BODY,
            tube(c(FACE_Z), c(MOUTH_Z + 0.015), TUBE_R, TUBE_R, 8, CLAY),
        );
        p.add(
            BODY,
            tube(
                c(MOUTH_Z + 0.02),
                c(MOUTH_Z),
                TUBE_R + 0.006,
                TUBE_R + 0.004,
                8,
                IRON,
            ),
        );
        p.add(
            BODY,
            tube(
                c(MOUTH_Z + 0.002),
                c(MOUTH_Z - 0.001),
                TUBE_R - 0.004,
                TUBE_R - 0.004,
                8,
                BLACK_STEEL,
            ),
        );
    }
    // Slatted vents on both flanks.
    let vent_x = half_at(-0.14).x + 0.003;
    for k in 0..3 {
        let slat = block(
            Vec3::new(vent_x, BOX_Y + 0.03 - k as f32 * 0.03, -0.14),
            Vec3::new(0.006, 0.005, 0.07),
            Quat::from_rotation_z(-0.4),
            IRON,
        );
        add_mirrored(p.mesh(BODY), &slat);
    }
    // Carry handle and sight on top.
    for z in [0.02, -0.24] {
        p.add(
            BODY,
            limb(
                Vec3::new(0.0, BOX_Y + half_at(z).y - 0.004, z),
                Vec3::new(0.0, BOX_Y + 0.11, z - 0.01),
                Vec2::new(0.009, 0.008),
                Vec2::new(0.008, 0.008),
                IRON,
            ),
        );
    }
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, BOX_Y + 0.112, 0.015),
            Vec3::new(0.0, BOX_Y + 0.112, -0.255),
            Vec2::new(0.012, 0.009),
            Vec2::new(0.012, 0.009),
            IRON,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, BOX_Y + 0.128, -0.24),
            Vec3::new(0.004, 0.01, 0.006),
            Quat::IDENTITY,
            BRASS,
        ),
    );
    // Rivets on the band.
    for (x, y) in [(-0.06, 0.08), (0.06, 0.08), (-0.06, -0.08), (0.06, -0.08)] {
        p.add(BODY, gem(Vec3::new(x, BOX_Y + y, -0.304), 0.008, 0, BRASS));
    }
    // Lamp sockets on the back face.
    for i in 0..TUBES {
        p.add(
            BODY,
            tube(
                lamp_at(i) - Vec3::Z * 0.004,
                lamp_at(i) + Vec3::Z * 0.004,
                0.014,
                0.014,
                8,
                BRASS,
            ),
        );
    }
    // Pistol grip and fore grip.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, 0.05, 0.03),
            Vec3::new(0.0, -0.105, 0.068),
            Vec2::new(0.021, 0.027),
            Vec2::new(0.02, 0.026),
            DARK_LEATHER,
        ),
    );
    p.add(
        BODY,
        block(
            Vec3::new(0.0, 0.03, -0.045),
            Vec3::new(0.006, 0.006, 0.026),
            Quat::IDENTITY,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, BOX_Y - 0.07, -0.25),
            Vec3::new(0.0, -0.06, -0.235),
            Vec2::new(0.018, 0.022),
            Vec2::new(0.017, 0.021),
            DARK_LEATHER,
        ),
    );
    kit::finish(p.mesh(BODY));

    // Cinders glowing behind the vent slats.
    let bed = block(
        Vec3::new(vent_x - 0.006, BOX_Y, -0.14),
        Vec3::new(0.004, 0.045, 0.068),
        Quat::IDENTITY,
        Vec4::new(1.0, 0.45, 0.12, 1.0),
    );
    add_mirrored(p.mesh(CINDERS), &bed);
    // Glow: a ring inside each tube mouth and a lamp on the back of the box.
    for [x, y] in TUBE_XY {
        p.add(
            GLOW,
            kit::band(
                Vec3::new(x, y, MOUTH_Z + 0.006),
                Vec3::Z,
                TUBE_R - 0.007,
                0.008,
                0.005,
                8,
                Vec4::ONE,
            ),
        );
    }
    // "Loaded" lamps, one per tube.
    for i in 0..TUBES {
        p.add(
            LAMP + i,
            gem(lamp_at(i) + Vec3::Z * 0.004, 0.0095, 1, Vec4::ONE),
        );
    }
    // Mouth flares (shown only while that tube fires).
    for i in 0..TUBES {
        let c = tube_mouth(i);
        p.add(
            FLARE + i,
            gem(
                c - Vec3::Z * 0.01,
                TUBE_R * 0.95,
                1,
                Vec4::new(1.0, 0.85, 0.6, 1.0),
            ),
        );
        p.add(
            FLARE + i,
            kit::spike(
                c - Vec3::Z * 0.01,
                c - Vec3::Z * 0.09,
                TUBE_R * 0.7,
                5,
                Vec4::ONE,
            ),
        );
    }

    p.texture(BODY, kit::tex_grime());
    p.texture(CINDERS, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    for i in 0..TUBES {
        p.texture(FLARE + i, kit::tex_glow());
        p.texture(LAMP + i, kit::tex_glow());
    }
    p.finish()
}

/// Tube `i`'s lamp on the back face (laid out like the tubes).
fn lamp_at(i: usize) -> Vec3 {
    let [x, y] = TUBE_XY[i.min(TUBES - 1)];
    Vec3::new(x * 0.5, BOX_Y + (y - BOX_Y) * 0.5 + 0.005, BACK_Z + 0.004)
}

/// Whether tube `i` is loaded `salvo` seconds into a salvo (its lamp is lit).
pub fn tube_loaded(salvo: Option<f32>, i: usize) -> bool {
    match salvo.filter(|s| s.is_finite()) {
        Some(s) => s < i as f32 * SALVO_GAP || s >= RELOAD,
        None => true,
    }
}

/// The salvo: each mouth flares in turn (hidden otherwise); the cinders breathe and flare.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let mut firing = 0.0f32;
    for i in 0..TUBES {
        let f = tube_flash(h.salvo, i);
        firing = firing.max(f);
        if f <= 0.0 {
            pose.hide(FLARE + i);
        } else {
            pose.set(FLARE + i, Mat4::from_scale(Vec3::splat(0.6 + 0.9 * f)));
        }
        if !tube_loaded(h.salvo, i) {
            pose.hide(LAMP + i);
        }
    }
    *glow += firing * 1.4 + unit(h.kick) * 0.5;
    let heat = 1.1 + 0.6 * firing + 0.15 * wave(h.time, 1.7) * wave(h.time, 4.9);
    pose.core = Vec4::new(heat, heat, heat, 1.0);
    pose
}
