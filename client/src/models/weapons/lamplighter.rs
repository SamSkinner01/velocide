//! Lamplighter: a lamplighter's long pole turned rifle (docs/rework.md §2). A dark wooden stock
//! and pistol grip behind a brass-bound receiver; in front of it a lantern chamber: a glass
//! drum between two brass end plates, caged by four bars, with a chimney vent on top and a lit
//! wick standing inside it. Out of the chamber runs the long brass tube of the pole, bound
//! with iron bands, a knurled focus ring and a wooden fore grip under it; at the end a
//! telescoping sleeve carries the lens in a flared bell and the pole's brass hook curling over
//! it.
//!
//! Focusing ([`super::HeldWeapon::draw`] 0..1, the bow's draw field: the replicated level is
//! the same `DRAWING` + level) turns the focus ring, runs the sleeve and its lens out like a
//! spyglass being focused, and feeds the wick: its flame grows and brightens, the lens (`Core`,
//! lit by the same flame) warms with it and both strobe white at full focus. The bands' inlays
//! glow in the selected ability's colour. The shot (`kick`) snaps the sleeve back and flares
//! the lens.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{HeldWeapon, smoothstep, unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, IRON, add_mirrored, block, gem, limb, rgb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BODY: usize = 0;
pub const STOCK: usize = 1;
pub const GLASS: usize = 2;
pub const WICK: usize = 3;
pub const GLOW: usize = 4;
pub const RING: usize = 5;
pub const SLEEVE: usize = 6;
pub const LENS: usize = 7;
pub const SLEEVE_GLOW: usize = 8;

/// Height of the tube's axis above the grip.
pub const BORE_Y: f32 = 0.05;
/// The lantern chamber: from its back plate to its front plate, and its glass radius.
const CHAMBER_Z: (f32, f32) = (-0.12, -0.228);
const CHAMBER_R: f32 = 0.036;
const CHAMBER_MID: f32 = (CHAMBER_Z.0 + CHAMBER_Z.1) * 0.5;
/// Where the wick's flame stands (the base of the flame, on the chamber's floor).
const FLAME_BASE: [f32; 3] = [0.0, BORE_Y - 0.02, CHAMBER_MID];
/// The tube from the chamber to where the sleeve slides over it.
const TUBE_R: f32 = 0.0165;
/// The focus ring's middle.
const RING_Z: f32 = -0.3;
/// The sleeve's back end (it slides forward from here as the lens is focused).
const SLEEVE_Z: f32 = -0.7;
/// The lens, at the front of the bell (sleeve at rest).
const LENS_Z: f32 = -0.915;
const LENS_R: f32 = 0.028;
/// How far the sleeve runs out at full focus (m).
pub const FOCUS_TRAVEL: f32 = 0.05;

pub const JOINTS: [JointDef; 9] = [
    static_part("body", Shade::Lit),
    static_part("stock", Shade::Lit),
    static_part("glass", Shade::Lit),
    joint("wick", ROOT, FLAME_BASE, Shade::Core),
    static_part("glow", Shade::Glow),
    joint("ring", ROOT, [0.0, BORE_Y, RING_Z], Shade::Lit),
    joint("sleeve", ROOT, [0.0, BORE_Y, SLEEVE_Z], Shade::Lit),
    joint("lens", SLEEVE, [0.0, BORE_Y, LENS_Z], Shade::Core),
    joint("sleeve_glow", SLEEVE, [0.0, BORE_Y, SLEEVE_Z], Shade::Glow),
];

/// The lens (the beam leaves it) with the sleeve at rest.
pub const MUZZLE: Vec3 = Vec3::new(0.0, BORE_Y, LENS_Z - 0.004);

/// Aged brass for the long tube (a little darker than the fittings).
const OLD_BRASS: Vec4 = Vec4::new(0.7, 0.52, 0.24, 1.0);
/// The stock's dark, oiled wood and the receiver's blackened bronze.
const WOOD: u32 = 0x3a2215;
const BRONZE: u32 = 0x3b3430;
/// The wick's flame and the lens it lights (both `Core`).
const FLAME: Vec4 = Vec4::new(1.0, 0.74, 0.34, 1.0);
const LENS_GLASS: Vec4 = Vec4::new(1.0, 0.9, 0.66, 1.0);

/// The hook at the pole's end: from the sleeve's top, up and forward, curling down over the
/// lens and back (sleeve at rest).
const HOOK_PATH: [[f32; 3]; 7] = [
    [0.0, BORE_Y + 0.02, -0.79],
    [0.0, BORE_Y + 0.06, -0.815],
    [0.0, BORE_Y + 0.092, -0.86],
    [0.0, BORE_Y + 0.1, -0.91],
    [0.0, BORE_Y + 0.088, -0.952],
    [0.0, BORE_Y + 0.062, -0.962],
    [0.0, BORE_Y + 0.046, -0.946],
];

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wood = rgb(WOOD);
    let ring = |z: f32, y: f32, w: f32, h: f32, c: f32| {
        kit::chamfer_ring(Vec3::new(0.0, y, z), Vec3::X, Vec3::Y, Vec2::new(w, h), c)
    };

    // The stock: a long dark shoulder stock deepening to the butt, a brass butt plate, a
    // leather cheek pad; the pistol grip under the receiver and a wooden fore grip under the
    // tube (where the other hand holds it).
    p.add(
        STOCK,
        kit::loft(
            &[
                ring(0.055, 0.045, 0.019, 0.028, 0.008),
                ring(0.15, 0.03, 0.017, 0.036, 0.008),
                ring(0.29, 0.012, 0.019, 0.052, 0.009),
            ],
            wood,
            false,
            (false, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        BODY,
        kit::loft(
            &[
                ring(0.288, 0.012, 0.02, 0.053, 0.009),
                ring(0.302, 0.012, 0.02, 0.053, 0.009),
            ],
            BRASS,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        STOCK,
        limb(
            Vec3::new(0.0, 0.072, 0.1),
            Vec3::new(0.0, 0.068, 0.21),
            Vec2::new(0.017, 0.006),
            Vec2::new(0.019, 0.006),
            DARK_LEATHER,
        ),
    );
    p.add(
        STOCK,
        limb(
            Vec3::new(0.0, 0.026, 0.016),
            Vec3::new(0.0, -0.1, 0.058),
            Vec2::new(0.015, 0.021),
            Vec2::new(0.016, 0.024),
            DARK_LEATHER,
        ),
    );
    p.add(
        STOCK,
        kit::loft(
            &[
                ring(CHAMBER_Z.1 - 0.015, BORE_Y - 0.026, 0.015, 0.011, 0.005),
                ring(-0.33, BORE_Y - 0.027, 0.017, 0.012, 0.006),
                ring(-0.47, BORE_Y - 0.023, 0.013, 0.009, 0.005),
            ],
            wood,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    kit::finish(p.mesh(STOCK));

    // Receiver: a brass-bound block between the stock and the chamber, a trigger and its guard.
    p.add(
        BODY,
        kit::loft(
            &[
                ring(0.06, BORE_Y - 0.002, 0.02, 0.027, 0.008),
                ring(-0.02, BORE_Y, 0.022, 0.03, 0.009),
                ring(CHAMBER_Z.0 + 0.004, BORE_Y, 0.02, 0.026, 0.008),
            ],
            rgb(BRONZE),
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    let side_plate = block(
        Vec3::new(0.021, BORE_Y + 0.002, -0.03),
        Vec3::new(0.003, 0.016, 0.05),
        Quat::IDENTITY,
        BRASS,
    );
    add_mirrored(p.mesh(BODY), &side_plate);
    p.add(
        BODY,
        kit::plate(
            &[
                Vec3::new(0.0, 0.022, -0.03),
                Vec3::new(0.0, -0.012, -0.044),
                Vec3::new(0.0, -0.018, -0.036),
                Vec3::new(0.0, 0.018, -0.018),
            ],
            0.005,
            BLACK_STEEL,
        ),
    );
    p.add(
        BODY,
        kit::band(
            Vec3::new(0.0, -0.005, -0.03),
            Vec3::X,
            0.026,
            0.006,
            0.004,
            8,
            BRASS,
        ),
    );

    // The lantern chamber: two brass end plates, four cage bars, a chimney vent with its cap,
    // the wick's stub on the floor.
    for (z0, z1) in [
        (CHAMBER_Z.0, CHAMBER_Z.0 - 0.012),
        (CHAMBER_Z.1 + 0.012, CHAMBER_Z.1),
    ] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, BORE_Y, z0),
                Vec3::new(0.0, BORE_Y, z1),
                CHAMBER_R + 0.006,
                CHAMBER_R + 0.006,
                10,
                BRASS,
            ),
        );
    }
    for k in 0..4 {
        let a = (k as f32 + 0.5) / 4.0 * std::f32::consts::TAU;
        let off = Vec3::new(a.cos(), a.sin(), 0.0) * (CHAMBER_R + 0.003);
        p.add(
            BODY,
            limb(
                Vec3::new(0.0, BORE_Y, CHAMBER_Z.0 - 0.01) + off,
                Vec3::new(0.0, BORE_Y, CHAMBER_Z.1 + 0.01) + off,
                Vec2::splat(0.0032),
                Vec2::splat(0.0032),
                OLD_BRASS,
            ),
        );
    }
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y + CHAMBER_R - 0.004, CHAMBER_MID),
            Vec3::new(0.0, BORE_Y + CHAMBER_R + 0.024, CHAMBER_MID),
            0.009,
            0.0075,
            6,
            BRASS,
        ),
    );
    p.add(
        BODY,
        kit::ellipsoid(
            Vec3::new(0.0, BORE_Y + CHAMBER_R + 0.028, CHAMBER_MID),
            Vec3::new(0.014, 0.006, 0.014),
            Quat::IDENTITY,
            2,
            6,
            OLD_BRASS,
            false,
        ),
    );
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y - CHAMBER_R, CHAMBER_MID),
            Vec3::new(0.0, FLAME_BASE[1], CHAMBER_MID),
            0.0035,
            0.0028,
            5,
            BLACK_STEEL,
        ),
    );

    // The pole's tube: a collar out of the chamber, iron bands down its length, tapering a
    // little to where the sleeve rides over it.
    p.add(
        BODY,
        kit::smooth_tube(
            Vec3::new(0.0, BORE_Y, CHAMBER_Z.1),
            Vec3::new(0.0, BORE_Y, SLEEVE_Z - 0.05),
            TUBE_R,
            TUBE_R - 0.0025,
            8,
            OLD_BRASS,
        ),
    );
    p.add(
        BODY,
        tube(
            Vec3::new(0.0, BORE_Y, CHAMBER_Z.1),
            Vec3::new(0.0, BORE_Y, CHAMBER_Z.1 - 0.03),
            0.024,
            0.019,
            8,
            BRASS,
        ),
    );
    for z in [-0.42f32, -0.55, -0.66] {
        p.add(
            BODY,
            tube(
                Vec3::new(0.0, BORE_Y, z + 0.007),
                Vec3::new(0.0, BORE_Y, z - 0.007),
                TUBE_R + 0.004,
                TUBE_R + 0.004,
                8,
                IRON,
            ),
        );
    }
    kit::finish(p.mesh(BODY));

    // The chamber's glass.
    p.add(
        GLASS,
        tube(
            Vec3::new(0.0, BORE_Y, CHAMBER_Z.0 - 0.011),
            Vec3::new(0.0, BORE_Y, CHAMBER_Z.1 + 0.011),
            CHAMBER_R,
            CHAMBER_R,
            10,
            Vec4::new(1.0, 0.95, 0.85, 1.0),
        ),
    );
    p.color(GLASS, Vec4::new(1.0, 0.94, 0.82, 0.32));

    // The wick's flame: a teardrop standing on the stub.
    let base = Vec3::from(FLAME_BASE);
    p.add(
        WICK,
        kit::crystal(
            base + Vec3::Y * 0.012,
            Vec3::Y,
            (0.012, 0.03),
            0.012,
            6,
            FLAME,
        ),
    );
    p.add(WICK, gem(base + Vec3::Y * 0.009, 0.006, 0, Vec4::ONE));

    // Glow: inlay lines along the receiver's flanks, a ring round the chamber's front collar
    // and the stock's gem.
    let inlay = limb(
        Vec3::new(0.0235, BORE_Y + 0.012, 0.03),
        Vec3::new(0.0235, BORE_Y + 0.012, -0.09),
        Vec2::new(0.0015, 0.0025),
        Vec2::new(0.0015, 0.0025),
        Vec4::ONE,
    );
    add_mirrored(p.mesh(GLOW), &inlay);
    p.add(
        GLOW,
        kit::band(
            Vec3::new(0.0, BORE_Y, CHAMBER_Z.1 - 0.034),
            Vec3::Z,
            0.0195,
            0.005,
            0.004,
            8,
            Vec4::ONE,
        ),
    );
    let stud = gem(Vec3::new(0.02, 0.03, 0.18), 0.006, 0, Vec4::ONE);
    add_mirrored(p.mesh(GLOW), &stud);

    // The focus ring: a knurled black band with a brass lever to turn it.
    p.add(
        RING,
        tube(
            Vec3::new(0.0, BORE_Y, RING_Z + 0.018),
            Vec3::new(0.0, BORE_Y, RING_Z - 0.018),
            TUBE_R + 0.007,
            TUBE_R + 0.007,
            10,
            BLACK_STEEL,
        ),
    );
    for k in 0..5 {
        let z = RING_Z + 0.012 - k as f32 * 0.006;
        p.add(
            RING,
            kit::band(
                Vec3::new(0.0, BORE_Y, z),
                Vec3::Z,
                TUBE_R + 0.008,
                0.002,
                0.002,
                10,
                IRON,
            ),
        );
    }
    p.add(
        RING,
        limb(
            Vec3::new(TUBE_R + 0.006, BORE_Y, RING_Z),
            Vec3::new(TUBE_R + 0.028, BORE_Y + 0.004, RING_Z),
            Vec2::new(0.004, 0.006),
            Vec2::new(0.0035, 0.005),
            BRASS,
        ),
    );
    p.add(
        RING,
        gem(
            Vec3::new(TUBE_R + 0.03, BORE_Y + 0.004, RING_Z),
            0.0055,
            0,
            BRASS,
        ),
    );
    kit::finish(p.mesh(RING));

    // The sleeve: a wider tube telescoped over the end of the pole, flaring into the lens bell,
    // and the lamplighter's hook curling over it.
    let s0 = SLEEVE_Z;
    p.add(
        SLEEVE,
        kit::smooth_tube(
            Vec3::new(0.0, BORE_Y, s0),
            Vec3::new(0.0, BORE_Y, -0.86),
            TUBE_R + 0.004,
            TUBE_R + 0.004,
            8,
            BRASS,
        ),
    );
    p.add(
        SLEEVE,
        tube(
            Vec3::new(0.0, BORE_Y, s0 + 0.004),
            Vec3::new(0.0, BORE_Y, s0 - 0.012),
            TUBE_R + 0.007,
            TUBE_R + 0.007,
            8,
            IRON,
        ),
    );
    let (side, depth) = kit::frame_for(Vec3::NEG_Z);
    let bell = |z: f32, r: f32| {
        kit::ngon_ring(
            Vec3::new(0.0, BORE_Y, z),
            side,
            depth,
            Vec2::splat(r),
            10,
            0.0,
        )
    };
    p.add(
        SLEEVE,
        kit::loft(
            &[
                bell(-0.855, TUBE_R + 0.004),
                bell(-0.885, LENS_R + 0.001),
                bell(-0.905, LENS_R + 0.006),
                bell(LENS_Z + 0.002, LENS_R + 0.007),
            ],
            BRASS,
            false,
            (false, true),
            kit::Uv::PerFace,
        ),
    );
    let hook: Vec<Vec3> = HOOK_PATH.iter().map(|q| Vec3::from(*q)).collect();
    for (i, w) in hook.windows(2).enumerate() {
        let r = 0.0088 - i as f32 * 0.0005;
        p.add(
            SLEEVE,
            kit::limb_oriented(
                w[0],
                w[1],
                Vec3::X,
                Vec2::new(r * 0.8, r),
                Vec2::new((r - 0.0005) * 0.8, r - 0.0005),
                OLD_BRASS,
            ),
        );
    }
    p.add(SLEEVE, gem(hook[hook.len() - 1], 0.0078, 0, BRASS));
    kit::finish(p.mesh(SLEEVE));

    // The lens: a glass disc lit by the wick behind it.
    p.add(
        LENS,
        tube(
            Vec3::new(0.0, BORE_Y, LENS_Z + 0.003),
            Vec3::new(0.0, BORE_Y, LENS_Z - 0.003),
            LENS_R,
            LENS_R,
            10,
            LENS_GLASS,
        ),
    );
    p.color(LENS, Vec4::new(1.0, 1.0, 1.0, 0.8));
    // The bell's lip ring.
    p.add(
        SLEEVE_GLOW,
        kit::band(
            Vec3::new(0.0, BORE_Y, -0.898),
            Vec3::Z,
            LENS_R + 0.006,
            0.004,
            0.003,
            10,
            Vec4::ONE,
        ),
    );

    p.texture(BODY, kit::tex_plate());
    p.texture(STOCK, kit::tex_leather());
    p.texture(GLASS, kit::tex_glow());
    p.texture(WICK, kit::tex_glow());
    p.texture(GLOW, kit::tex_glow());
    p.texture(RING, kit::tex_grip());
    p.texture(SLEEVE, kit::tex_plate());
    p.texture(LENS, kit::tex_glow());
    p.texture(SLEEVE_GLOW, kit::tex_glow());
    p.finish()
}

/// First-person placement: held low on the right like the long guns and turned a little out,
/// so the long tube runs in towards the centre without its lens reaching the crosshair; the
/// chamber's flame sits in plain view. Focusing settles it lower and further out
/// ([`FOCUS_SHIFT`], [`view_rotation`]) until, at full focus, only its tube and lens reach into
/// the scope's aperture, low in its right corner, the rest under the vignette: the sight stays
/// clear.
pub const VIEW_OFFSET: Vec3 = Vec3::new(0.13, -0.128, -0.3);
pub const FOCUS_SHIFT: Vec3 = Vec3::new(0.06, -0.075, 0.02);

/// The rifle's view rotation at `focus` 0..1 (on top of the common one): turned out a little
/// (yaw right), more and dipped as it settles.
pub fn view_rotation(focus: f32) -> Quat {
    let f = smoothstep(unit(focus));
    Quat::from_rotation_y(-0.07 - 0.07 * f) * Quat::from_rotation_x(-0.05 * f)
}

/// How far the sleeve stands out (model space, along -Z) at `focus` / `kick`: it runs out as
/// the lens is focused and snaps back past rest on the shot, then settles.
pub fn sleeve_offset(focus: f32, kick: f32) -> f32 {
    let snap = smoothstep(unit(kick) * 1.5);
    FOCUS_TRAVEL * smoothstep(unit(focus)) * (1.0 - snap) - 0.012 * snap
}

/// Focus: the ring turns, the sleeve runs out, the wick's flame grows and brightens with the
/// lens and strobes white at full focus; the shot snaps the sleeve back and flares the lens.
pub(super) fn pose(h: &HeldWeapon, glow: &mut f32, color: &mut Vec3) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let focus = unit(h.draw);
    let kick = unit(h.kick);
    let eased = smoothstep(focus);
    pose.rotate(RING, Quat::from_rotation_z(-1.9 * eased));
    pose.set(
        SLEEVE,
        Mat4::from_translation(Vec3::new(0.0, 0.0, -sleeve_offset(focus, kick))),
    );
    let flicker = 0.08 * wave(h.time, 7.0) * wave(h.time, 2.9);
    let full = focus >= 0.98;
    let strobe = if full {
        0.6 * (h.time * 11.0).sin().abs() as f32
    } else {
        0.0
    };
    // The flame stretches up as it is fed, and gutters flat for a moment after the shot.
    let height = (1.0 + 0.55 * eased + 0.5 * flicker - 0.35 * kick).max(0.4);
    pose.set(
        WICK,
        Mat4::from_scale(Vec3::new(1.0 + 0.15 * eased, height, 1.0 + 0.15 * eased)),
    );
    let heat = 0.95 + 1.3 * eased + strobe + 1.4 * kick + flicker;
    pose.core = Vec4::new(
        heat,
        heat * (1.0 + 0.1 * eased),
        heat * (1.0 + 0.3 * eased),
        1.0,
    );
    *glow += 0.9 * eased + 0.4 * strobe + kick * 0.9;
    *color += Vec3::splat(0.25 * focus * focus + if full { 0.15 } else { 0.0 });
    pose
}
