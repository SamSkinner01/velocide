//! Longwick: recurve bow of wax-hardened wood. A dark wooden riser with brass limb pockets, a
//! leather-wrapped grip and an arrow shelf; two amber, wax-hardened limbs that sweep back
//! towards the archer and flick forward at the tips; a string that is a glowing wick; and a
//! taper arrow nocked on it: a slim wax shaft with parchment fletching, an iron head and a
//! small flame burning behind the head.
//!
//! Drawing ([`super::HeldWeapon::draw`] 0..1) pulls the string and the arrow back by up to
//! [`DRAW_LENGTH`], flexes both limbs (their tips come back and in), heats the wick string
//! (brighter, whiter, strobing at full draw) and fans the arrow's flame (both `Core`: the
//! wick's own warm colour); the riser's inlays glow in the selected ability's colour. In first
//! person the left hand pinches the nock ([`draw_hand`]). On the release (`kick`) the string
//! snaps and quivers and the arrow is gone until the next one is nocked as the kick fades.
//!
//! The strings are two root parts whose pose is a full transform: each is rotated and
//! stretched from its limb tip (which moves with the flex) to the nock.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, smoothstep, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, IRON, block, gem, limb, rgb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const RISER: usize = 0;
pub const LIMB_U: usize = 1;
pub const LIMB_L: usize = 2;
pub const STRING_U: usize = 3;
pub const STRING_L: usize = 4;
pub const ARROW: usize = 5;
pub const FLAME: usize = 6;
pub const GLOW: usize = 7;

/// Height the arrow rests at on the shelf.
const ARROW_Y: f32 = 0.042;
/// Limb roots (where the limbs leave the riser's pockets).
const ROOT_U: [f32; 3] = [0.0, 0.16, 0.0];
const ROOT_L: [f32; 3] = [0.0, -0.16, 0.0];
/// String grooves at the limb tips (bind pose, undrawn).
pub const TIP_U: [f32; 3] = [0.0, 0.5, 0.14];
pub const TIP_L: [f32; 3] = [0.0, -0.5, 0.14];
/// The nock point on the string at rest (brace height 0.14 m).
const NOCK: [f32; 3] = [0.0, ARROW_Y, 0.14];
/// How far the nock comes back at full draw (m).
pub const DRAW_LENGTH: f32 = 0.3;
/// Arrow length from nock to head tip.
const ARROW_LEN: f32 = 0.5;
/// How far each limb flexes at full draw (radians).
const FLEX: f32 = 0.2;

pub const JOINTS: [JointDef; 8] = [
    static_part("riser", Shade::Lit),
    joint("limb_upper", ROOT, ROOT_U, Shade::Lit),
    joint("limb_lower", ROOT, ROOT_L, Shade::Lit),
    joint("string_upper", ROOT, TIP_U, Shade::Core),
    joint("string_lower", ROOT, TIP_L, Shade::Core),
    joint("arrow", ROOT, NOCK, Shade::Lit),
    joint("flame", ARROW, NOCK, Shade::Core),
    static_part("glow", Shade::Glow),
];

/// The nocked arrow's head at rest.
pub const MUZZLE: Vec3 = Vec3::new(0.0, ARROW_Y, NOCK[2] - ARROW_LEN);

/// First-person placement (`docs/feel.md`): the bow is held small and low on the left, canted
/// so the upper limb leans away from the crosshair and the lower one runs off the bottom of the
/// screen; the arrow points at the crosshair from below-left. As it is drawn the bow comes up
/// and in ([`DRAW_SHIFT`]) until, at full draw, the arrowhead sits just under the crosshair.
/// Nothing of it ever covers the centre of the screen (`models::tests`).
pub const VIEW_OFFSET: Vec3 = Vec3::new(-0.14, -0.14, -0.42);
pub const DRAW_SHIFT: Vec3 = Vec3::new(0.115, 0.06, 0.0);
/// Drawn smaller than the other weapons (× the common viewmodel scale).
pub const VIEW_SCALE: f32 = 0.8;

/// The bow's view rotation at draw `draw` (0..1, eased): canted about the arrow's own axis (the
/// upper limb leans left, away from the centre; more at full draw), then turned so the arrow
/// points at the crosshair from where the bow is held (straight ahead at full draw, when the
/// bow has come up under the crosshair).
pub fn view_rotation(draw: f32) -> Quat {
    let d = unit(draw);
    let lerp = |a: f32, b: f32| a + (b - a) * d;
    Quat::from_rotation_y(lerp(-0.1, -0.02))
        * Quat::from_rotation_x(lerp(0.1, 0.03))
        * Quat::from_rotation_z(lerp(0.62, 0.78))
}

/// Wax-hardened limbs (amber) and the riser's dark wood.
const WAX: Vec4 = Vec4::new(0.88, 0.66, 0.36, 1.0);
const PARCHMENT: Vec4 = Vec4::new(0.9, 0.84, 0.68, 1.0);
/// The burning wick string.
const WICK: Vec4 = Vec4::new(1.0, 0.78, 0.42, 1.0);

/// Points along the upper limb from its root to the forward-flicking tip (the lower limb is
/// the mirror image in Y).
const LIMB_PATH: [[f32; 3]; 5] = [
    ROOT_U,
    [0.0, 0.29, 0.06],
    [0.0, 0.41, 0.115],
    TIP_U,
    [0.0, 0.555, 0.105],
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wood = rgb(0x3e2416);
    // Riser: lower and upper halves either side of the grip, the upper one offset left to
    // make the arrow's window; an arrow shelf and brass pockets for the limbs.
    p.add(
        RISER,
        limb(
            Vec3::new(0.0, -0.175, 0.004),
            Vec3::new(0.0, -0.035, -0.012),
            Vec2::new(0.016, 0.022),
            Vec2::new(0.02, 0.028),
            wood,
        ),
    );
    p.add(
        RISER,
        limb(
            Vec3::new(-0.008, 0.03, -0.012),
            Vec3::new(-0.004, 0.175, 0.004),
            Vec2::new(0.014, 0.03),
            Vec2::new(0.016, 0.022),
            wood,
        ),
    );
    p.add(
        RISER,
        limb(
            Vec3::new(0.0, -0.045, 0.006),
            Vec3::new(0.0, 0.034, 0.002),
            Vec2::new(0.021, 0.026),
            Vec2::new(0.02, 0.025),
            DARK_LEATHER,
        ),
    );
    p.add(
        RISER,
        block(
            Vec3::new(0.004, ARROW_Y - 0.01, -0.012),
            Vec3::new(0.014, 0.006, 0.024),
            Quat::IDENTITY,
            wood,
        ),
    );
    for y in [0.165f32, -0.165] {
        p.add(
            RISER,
            block(
                Vec3::new(0.0, y, 0.002),
                Vec3::new(0.024, 0.016, 0.026),
                Quat::IDENTITY,
                BRASS,
            ),
        );
    }
    // A brass sight pin and a wax seal on the riser's face.
    p.add(
        RISER,
        limb(
            Vec3::new(-0.02, 0.09, -0.02),
            Vec3::new(-0.045, 0.09, -0.026),
            Vec2::splat(0.0035),
            Vec2::splat(0.003),
            BRASS,
        ),
    );
    p.add(
        RISER,
        kit::ellipsoid(
            Vec3::new(0.0, -0.1, -0.03),
            Vec3::new(0.016, 0.02, 0.008),
            Quat::IDENTITY,
            2,
            6,
            Vec4::new(0.72, 0.1, 0.1, 1.0),
            false,
        ),
    );
    kit::finish(p.mesh(RISER));

    // Limbs: flat amber laminations, widest at the root, the tip flicking forward.
    for (part, sign) in [(LIMB_U, 1.0f32), (LIMB_L, -1.0)] {
        let pts: Vec<Vec3> = LIMB_PATH
            .iter()
            .map(|q| Vec3::new(q[0], q[1] * sign, q[2]))
            .collect();
        let n = pts.len() - 1;
        for i in 0..n {
            let w = |k: usize| 0.024 - 0.013 * k as f32 / n as f32;
            let t = |k: usize| 0.011 - 0.005 * k as f32 / n as f32;
            p.add(
                part,
                kit::limb_oriented(
                    pts[i],
                    pts[i + 1],
                    Vec3::X,
                    Vec2::new(w(i), t(i)),
                    Vec2::new(w(i + 1), t(i + 1)),
                    WAX,
                ),
            );
        }
        // A dark wrap where the limb leaves its pocket, and the horn tip.
        p.add(
            part,
            tube(
                pts[0] + Vec3::Y * sign * 0.012,
                pts[0] + Vec3::Y * sign * 0.04,
                0.02,
                0.019,
                6,
                DARK_LEATHER,
            ),
        );
        p.add(part, gem(pts[n], 0.012, 0, IRON));
        kit::finish(p.mesh(part));
    }

    // The wick string: a thin glowing cord from each tip to the nock point.
    for (part, tip) in [(STRING_U, TIP_U), (STRING_L, TIP_L)] {
        p.add(
            part,
            limb(
                Vec3::from(tip),
                Vec3::from(NOCK),
                Vec2::splat(0.0038),
                Vec2::splat(0.0038),
                WICK,
            ),
        );
    }

    // The taper arrow, nocked: shaft, fletching, nock, iron head.
    let nock = Vec3::from(NOCK);
    let head = nock - Vec3::Z * ARROW_LEN;
    p.add(
        ARROW,
        kit::tube(
            nock - Vec3::Z * 0.01,
            head + Vec3::Z * 0.05,
            0.0055,
            0.0045,
            6,
            PARCHMENT,
        ),
    );
    p.add(
        ARROW,
        kit::tube(
            nock + Vec3::Z * 0.004,
            nock - Vec3::Z * 0.014,
            0.0065,
            0.0065,
            6,
            BLACK_STEEL,
        ),
    );
    for k in 0..3 {
        let a = k as f32 / 3.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
        let d = Vec3::new(a.cos(), a.sin(), 0.0);
        p.add(
            ARROW,
            kit::plate(
                &[
                    nock + d * 0.005 - Vec3::Z * 0.02,
                    nock + d * 0.022 - Vec3::Z * 0.035,
                    nock + d * 0.02 - Vec3::Z * 0.085,
                    nock + d * 0.005 - Vec3::Z * 0.1,
                ],
                0.002,
                if k == 0 {
                    Vec4::new(0.78, 0.12, 0.1, 1.0)
                } else {
                    PARCHMENT
                },
            ),
        );
    }
    p.add(
        ARROW,
        kit::spike(head + Vec3::Z * 0.04, head, 0.011, 4, IRON),
    );
    p.add(
        ARROW,
        kit::tube(
            head + Vec3::Z * 0.04,
            head + Vec3::Z * 0.052,
            0.008,
            0.007,
            6,
            BRASS,
        ),
    );
    kit::finish(p.mesh(ARROW));
    // The taper's flame, burning just behind the head.
    p.add(
        FLAME,
        kit::crystal(
            head + Vec3::Z * 0.07,
            Vec3::NEG_Z,
            (0.014, 0.026),
            0.009,
            5,
            Vec4::new(1.0, 0.72, 0.32, 1.0),
        ),
    );
    // Glow: inlaid lines down the riser's face and a ring at each limb pocket.
    p.add(
        GLOW,
        limb(
            Vec3::new(0.0, -0.15, -0.022),
            Vec3::new(0.0, -0.06, -0.033),
            Vec2::new(0.003, 0.002),
            Vec2::new(0.003, 0.002),
            Vec4::ONE,
        ),
    );
    p.add(
        GLOW,
        limb(
            Vec3::new(-0.008, 0.07, -0.04),
            Vec3::new(-0.004, 0.15, -0.024),
            Vec2::new(0.003, 0.002),
            Vec2::new(0.003, 0.002),
            Vec4::ONE,
        ),
    );
    for y in [0.183f32, -0.183] {
        p.add(
            GLOW,
            block(
                Vec3::new(0.0, y, 0.002),
                Vec3::new(0.025, 0.003, 0.027),
                Quat::IDENTITY,
                Vec4::ONE,
            ),
        );
    }

    p.texture(RISER, kit::tex_leather());
    p.texture(LIMB_U, kit::tex_wax());
    p.texture(LIMB_L, kit::tex_wax());
    p.texture(STRING_U, kit::tex_glow());
    p.texture(STRING_L, kit::tex_glow());
    p.texture(ARROW, kit::tex_bone());
    p.texture(FLAME, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    p.finish()
}

/// Nock position (model space) for a draw / release state.
pub fn nock_position(draw: f32, kick: f32, time: f64) -> Vec3 {
    let d = smoothstep(unit(draw));
    let kick = unit(kick);
    // After the release the string quivers about its rest line, dying away with the kick.
    let quiver = 0.018 * kick * kick * wave(time, 23.0);
    Vec3::from(NOCK) + Vec3::Z * (DRAW_LENGTH * d + quiver)
}

/// A string part's transform: from its bind segment (`tip` → rest nock) to the segment from the
/// flexed tip to the drawn nock.
fn string_local(tip: Vec3, flexed_tip: Vec3, nock: Vec3) -> Mat4 {
    let d0 = Vec3::from(NOCK) - tip;
    let d1 = nock - flexed_tip;
    let (l0, l1) = (d0.length(), d1.length());
    if l0 < 1e-5 || l1 < 1e-5 {
        return Mat4::IDENTITY;
    }
    let a = d0 / l0;
    let s = l1 / l0;
    // Stretch along the bind direction only.
    let stretch = Mat4::from_cols(
        (Vec3::X + a * (a.x * (s - 1.0))).extend(0.0),
        (Vec3::Y + a * (a.y * (s - 1.0))).extend(0.0),
        (Vec3::Z + a * (a.z * (s - 1.0))).extend(0.0),
        Vec4::W,
    );
    let turn = Quat::from_rotation_arc(a, d1 / l1);
    Mat4::from_translation(flexed_tip - tip) * Mat4::from_quat(turn) * stretch
}

/// Draw: limbs flex, strings stretch to the nock, the arrow rides back with it; the wick
/// string heats up and strobes at full draw; the release snaps the string and hides the arrow
/// until the next is nocked.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32, color: &mut Vec3) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let draw = unit(h.draw);
    let kick = unit(h.kick);
    let flex = FLEX * smoothstep(draw) + 0.03 * kick * wave(h.time, 23.0);
    let rot_u = Quat::from_rotation_x(flex);
    let rot_l = Quat::from_rotation_x(-flex);
    pose.rotate(LIMB_U, rot_u);
    pose.rotate(LIMB_L, rot_l);
    let (root_u, root_l) = (Vec3::from(ROOT_U), Vec3::from(ROOT_L));
    let (tip_u, tip_l) = (Vec3::from(TIP_U), Vec3::from(TIP_L));
    let tip_u2 = root_u + rot_u * (tip_u - root_u);
    let tip_l2 = root_l + rot_l * (tip_l - root_l);
    let nock = nock_position(draw, kick, h.time);
    pose.set(STRING_U, string_local(tip_u, tip_u2, nock));
    pose.set(STRING_L, string_local(tip_l, tip_l2, nock));
    if kick > 0.35 {
        pose.hide(ARROW);
        pose.hide(FLAME);
    } else {
        pose.set(ARROW, Mat4::from_translation(nock - Vec3::from(NOCK)));
    }
    // The wick string and the arrow's flame (`Core`) heat up with the draw and strobe white at
    // full draw; the riser's inlays (`Glow`, the ability's colour) brighten with them.
    let full = draw >= 0.98;
    let strobe = if full {
        0.7 * (h.time * 12.0).sin().abs() as f32
    } else {
        0.0
    };
    *glow += 1.1 * draw + 0.5 * strobe + kick * 0.8;
    *color += Vec3::splat(0.3 * draw * draw + if full { 0.2 } else { 0.0 });
    let flicker = 0.1 * wave(h.time, 9.0) * wave(h.time, 2.3);
    let heat = 1.05 + 1.1 * draw + strobe + 0.6 * kick + flicker;
    pose.core = Vec4::new(
        heat,
        heat * (1.0 + 0.12 * draw),
        heat * (1.0 + 0.3 * draw),
        1.0,
    );
    pose
}

/// Model-space points along the bow as drawn at `draw` / `kick` (the riser, both flexed limbs,
/// both strings to the nock, the nocked arrow while it is shown): the first-person framing
/// test projects them to check the bow never covers the crosshair.
pub fn outline_points(draw: f32, kick: f32, time: f64) -> Vec<Vec3> {
    let h = HeldWeapon {
        draw,
        kick,
        time,
        ..HeldWeapon::default()
    };
    let (mut glow, mut color) = (0.0, Vec3::ZERO);
    let pose = pose(&h, &mut glow, &mut color);
    let flex = |part: usize, q: [f32; 3], root: [f32; 3]| {
        let rot = Quat::from_mat4(&pose.locals[part]);
        Vec3::from(root) + rot * (Vec3::from(q) - Vec3::from(root))
    };
    let nock = nock_position(draw, kick, time);
    let mut points = vec![Vec3::new(0.0, -0.175, 0.0), Vec3::new(0.0, 0.175, 0.0)];
    for (part, sign, root) in [(LIMB_U, 1.0f32, ROOT_U), (LIMB_L, -1.0, ROOT_L)] {
        let path: Vec<Vec3> = LIMB_PATH
            .iter()
            .map(|q| flex(part, [q[0], q[1] * sign, q[2]], root))
            .collect();
        let tip = path[path.len() - 2];
        points.extend(path);
        points.extend((1..8).map(|i| tip.lerp(nock, i as f32 / 8.0)));
    }
    if !pose.is_hidden(ARROW) {
        let head = nock - Vec3::Z * ARROW_LEN;
        points.extend((0..=8).map(|i| nock.lerp(head, i as f32 / 8.0)));
    }
    points
}

/// View-space transform of the drawing hand (the left fist model) for the bow drawn at `gun`
/// (its view-space grip transform): knuckles pinching the nock, the forearm running down and
/// out to the bottom right of the view (never straight back into the camera).
pub fn draw_hand(gun: Mat4, draw: f32, time: f64) -> Mat4 {
    let nock = gun.transform_point3(nock_position(draw, 0.0, time));
    let forearm = Vec3::new(0.55, -0.7, 0.45).normalize();
    let rot = Quat::from_rotation_arc(Vec3::Z, forearm) * Quat::from_rotation_z(0.9);
    Mat4::from_scale_rotation_translation(
        Vec3::splat(super::VIEWMODEL_SCALE * VIEW_SCALE * 0.7),
        rot,
        nock + Vec3::new(0.006, -0.008, 0.008),
    )
}
