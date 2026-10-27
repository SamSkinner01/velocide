//! Trimmer: hooked wick-shears whose pivot pin knocks out, so they are carried as two blades,
//! one per hand. Each half is a long, straight blued-steel blade rising from a brass pivot boss,
//! bent down at the end into the trimmer's angled jaw (the hook that catches a wick), its honed
//! edge along the underside and the inside of the jaw. Under the boss: a leather-wrapped handle
//! and the shears' brass finger loop, closed round the knuckles like a guard. The right half
//! carries the snuffer box on its back (the little tray real wick-shears catch the trimmed wick
//! in), a stub of wick still smouldering in it.
//!
//! Model space is the right half's: its grip at the origin, the blade along -Z. The left half
//! is the mirror image, built beside it ([`LEFT_GRIP`]) so the bind pose shows the closed pair
//! (the loadout preview, a dropped weapon); in use each half is posed into its own hand
//! (`super::melee`). The edges (`Glow`) take the selected ability's colour.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{unit, wave};
use crate::models::PartSet;
use crate::models::kit::{
    self, BLACK_STEEL, BRASS, DARK_LEATHER, LEATHER, block, gem, limb, rgb, tube,
};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint, static_part};

pub const BLADE_R: usize = 0;
pub const EDGE_R: usize = 1;
pub const WICK: usize = 2;
pub const BLADE_L: usize = 3;
pub const EDGE_L: usize = 4;

/// The left half's grip in the bind pose (the closed pair lie side by side).
pub const LEFT_GRIP: [f32; 3] = [-0.022, 0.0, 0.0];

pub const JOINTS: [JointDef; 5] = [
    static_part("blade_r", Shade::Lit),
    joint("edge_r", BLADE_R, [0.0, 0.0, 0.0], Shade::Glow),
    joint("wick", BLADE_R, [0.0, 0.0, 0.0], Shade::Core),
    joint("blade_l", ROOT, LEFT_GRIP, Shade::Lit),
    joint("edge_l", BLADE_L, LEFT_GRIP, Shade::Glow),
];

/// Where the blade bites: the inside of the hook (the "muzzle" of a melee weapon: slash trails
/// and hit sparks start here).
pub const MUZZLE: Vec3 = Vec3::new(0.0, 0.07, -0.43);
/// The middle of a blade (a hurled blade spins about it).
pub const BLADE_CENTER: Vec3 = Vec3::new(0.0, 0.07, -0.24);

/// Blued blade steel and the brighter honed bevel.
const BLUED: Vec4 = Vec4::new(0.5, 0.55, 0.66, 1.0);
const BEVEL: Vec4 = Vec4::new(0.88, 0.9, 0.94, 1.0);
/// The smouldering wick stub.
const EMBER: Vec4 = Vec4::new(1.0, 0.52, 0.16, 1.0);

/// The blade's spine (back of the blade) from the boss to the hook's point, and the blade's
/// depth at each point (measured towards the inside of the curve).
const SPINE: [([f32; 3], f32); 8] = [
    ([0.0, 0.04, -0.02], 0.028),
    ([0.0, 0.07, -0.1], 0.04),
    ([0.0, 0.095, -0.2], 0.045),
    ([0.0, 0.112, -0.3], 0.043),
    ([0.0, 0.12, -0.37], 0.04),
    ([0.0, 0.1, -0.43], 0.033),
    ([0.0, 0.06, -0.465], 0.024),
    ([0.0, 0.012, -0.48], 0.0),
];

/// Spine points with the inward normals (towards the edge) and depths.
fn spine() -> Vec<(Vec3, Vec3, f32)> {
    let pts: Vec<Vec3> = SPINE.iter().map(|(p, _)| Vec3::from(*p)).collect();
    pts.iter()
        .enumerate()
        .map(|(i, p)| {
            let prev = pts[i.saturating_sub(1)];
            let next = pts[(i + 1).min(pts.len() - 1)];
            let t = (next - prev).normalize_or(Vec3::NEG_Z);
            // Perpendicular in the blade's plane, on the inside of the curve (downwards along
            // the straight part, backwards at the hook).
            let n = Vec3::new(0.0, t.z, -t.y);
            (*p, n, SPINE[i].1)
        })
        .collect()
}

/// One half's blade (right-handed), built in the plane x = 0: steel body, a bright bevel
/// strip just inside the edge, and the edge itself (returned separately: it glows).
fn blade() -> (MeshBuilder, MeshBuilder) {
    let s = spine();
    let mut steel = MeshBuilder::new();
    let mut edge = MeshBuilder::new();
    let thick = 0.009;
    for w in s.windows(2) {
        let (a, na, wa) = w[0];
        let (b, nb, wb) = w[1];
        // Body: spine to 75 % of the depth; the bevel the rest; the glowing edge a sliver
        // beyond the steel.
        let body = |p: Vec3, n: Vec3, d: f32| p + n * d * 0.72;
        let quad = [a, b, body(b, nb, wb), body(a, na, wa)];
        kit::add(&mut steel, kit::plate(&dedup(&quad), thick, BLUED));
        let bevel = [body(a, na, wa), body(b, nb, wb), b + nb * wb, a + na * wa];
        kit::add(&mut steel, kit::plate(&dedup(&bevel), thick * 0.55, BEVEL));
        let line = [
            a + na * wa * 0.9,
            b + nb * wb * 0.9,
            b + nb * (wb + 0.007),
            a + na * (wa + 0.007),
        ];
        kit::add(
            &mut edge,
            kit::plate(&dedup(&line), thick * 0.35, Vec4::ONE),
        );
    }
    // A thickened spine ridge along the back.
    for w in s.windows(2).take(6) {
        kit::add(
            &mut steel,
            limb(
                w[0].0,
                w[1].0,
                Vec2::new(0.0075, 0.006),
                Vec2::new(0.0075, 0.006),
                kit::GUNMETAL,
            ),
        );
    }
    (steel, edge)
}

/// Drops repeated points (the hook's point closes a quad to a triangle).
fn dedup(points: &[Vec3]) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::with_capacity(points.len());
    for p in points {
        if out.iter().all(|q| q.distance(*p) > 1e-4) {
            out.push(*p);
        }
    }
    out
}

/// The grip under the boss: handle, pommel, the finger loop and the boss itself.
fn hilt(m: &mut MeshBuilder, loop_radius: f32) {
    // The pivot boss: a brass rivet through the blade's root.
    kit::add(
        m,
        tube(
            Vec3::new(-0.013, 0.036, -0.014),
            Vec3::new(0.013, 0.036, -0.014),
            0.024,
            0.024,
            8,
            BRASS,
        ),
    );
    kit::add(
        m,
        gem(Vec3::new(0.014, 0.036, -0.014), 0.009, 0, BLACK_STEEL),
    );
    // Leather-wrapped handle down the grip (raked back like a pistol grip) and the pommel.
    kit::add(
        m,
        limb(
            Vec3::new(0.0, 0.03, -0.006),
            Vec3::new(0.0, -0.112, 0.036),
            Vec2::new(0.015, 0.019),
            Vec2::new(0.017, 0.021),
            DARK_LEATHER,
        ),
    );
    for y in [-0.01, -0.05, -0.09] {
        let z = -y * 0.3;
        kit::add(
            m,
            limb(
                Vec3::new(0.0, y + 0.004, z - 0.001),
                Vec3::new(0.0, y - 0.004, z + 0.001),
                Vec2::new(0.018, 0.022),
                Vec2::new(0.018, 0.022),
                LEATHER,
            ),
        );
    }
    kit::add(m, gem(Vec3::new(0.0, -0.122, 0.04), 0.021, 0, BRASS));
    // The shears' finger loop, closed round the knuckles: a brass bow from the pommel to the
    // boss in front of the fingers.
    kit::add(
        m,
        kit::band(
            Vec3::new(0.0, -0.042, -0.03),
            Vec3::X,
            loop_radius,
            0.013,
            0.009,
            10,
            BRASS,
        ),
    );
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let (steel, edge) = blade();

    // Right half: blade, hilt, the snuffer box riding on the spine with its wick stub.
    let mut right = steel.clone();
    hilt(&mut right, 0.064);
    let s = spine();
    let (box_at, down, _) = s[3];
    let along = (s[4].0 - s[2].0).normalize_or(Vec3::NEG_Z);
    let tilt = Quat::from_rotation_arc(Vec3::NEG_Z, along);
    let box_c = box_at - down * 0.018;
    kit::add(
        &mut right,
        block(box_c, Vec3::new(0.014, 0.014, 0.036), tilt, BRASS),
    );
    // The box's open top (dark) and a hinge pin across its back end.
    kit::add(
        &mut right,
        block(
            box_c - down * 0.012,
            Vec3::new(0.01, 0.004, 0.03),
            tilt,
            rgb(0x3a2a12),
        ),
    );
    kit::add(
        &mut right,
        tube(
            box_c + tilt * Vec3::new(-0.017, 0.012, 0.034),
            box_c + tilt * Vec3::new(0.017, 0.012, 0.034),
            0.005,
            0.005,
            6,
            BLACK_STEEL,
        ),
    );
    p.add(BLADE_R, right);
    kit::finish(p.mesh(BLADE_R));
    p.add(EDGE_R, edge.clone());
    // The wick stub smouldering in the box.
    p.add(
        WICK,
        kit::spike(
            box_c - down * 0.014 + along * 0.01,
            box_c - down * 0.04 + along * 0.016,
            0.006,
            5,
            EMBER,
        ),
    );
    p.add(
        WICK,
        gem(box_c - down * 0.016 + along * 0.01, 0.009, 0, EMBER),
    );

    // Left half: the mirror image (no box), beside the right one.
    let offset = engine::glam::Mat4::from_translation(Vec3::from(LEFT_GRIP));
    let mut left = steel;
    hilt(&mut left, 0.058);
    let mut mirrored = kit::mirrored_x(&left);
    mirrored.transform(offset);
    p.add(BLADE_L, mirrored);
    kit::finish(p.mesh(BLADE_L));
    let mut edge_l = kit::mirrored_x(&edge);
    edge_l.transform(offset);
    p.add(EDGE_L, edge_l);

    p.texture(BLADE_R, kit::tex_plate());
    p.texture(BLADE_L, kit::tex_plate());
    p.texture(EDGE_R, kit::tex_glow());
    p.texture(EDGE_L, kit::tex_glow());
    p.texture(WICK, kit::tex_glow());
    p.finish()
}

/// The edges shine brighter on a swing (`kick` 0..1); the wick stub breathes.
pub(super) fn pose(kick: f32, time: f64, glow: &mut f32) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    *glow += unit(kick) * 0.9;
    let heat = 1.1 + 0.15 * wave(time, 1.7) + 0.08 * wave(time, 6.1) * wave(time, 0.9);
    pose.core = Vec4::new(heat, heat, heat, 1.0);
    pose
}
