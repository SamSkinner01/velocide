//! Gnawer: fodder melee, a pack crawler (~2 m to the top of its hump). A hunched Mould that
//! runs on its oversized fists: grey wax-flesh over a brass-caged ribcage, slag spines down its
//! back, short digitigrade hind legs, a small low cranium with two ember eyes and a huge
//! under-slung jaw of brass teeth. The whole head (jaw included) is the weak point.
//! Telegraph (`Glow`): the throat and the gums burn orange as the jaw unhinges (bite) or as it
//! coils down for a pounce.

use engine::assets::gltf::ModelPart;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, ry, rz, seed_phase, smoothstep, telegraph, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, add_mirrored, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const CHEST: usize = 1;
pub const HEAD: usize = 2;
pub const JAW: usize = 3;
pub const THROAT: usize = 4;
pub const EYES: usize = 5;
pub const UPPER_ARM_L: usize = 6;
pub const FOREARM_L: usize = 7;
pub const FIST_L: usize = 8;
pub const UPPER_ARM_R: usize = 9;
pub const FOREARM_R: usize = 10;
pub const FIST_R: usize = 11;
pub const THIGH_L: usize = 12;
pub const SHIN_L: usize = 13;
pub const THIGH_R: usize = 14;
pub const SHIN_R: usize = 15;

/// Attack indices (assets/enemies/gnawer.ron).
pub const BITE: u8 = 0;
pub const POUNCE: u8 = 1;

const SHOULDER: [f32; 3] = [0.42, 1.48, -0.12];
const ELBOW: [f32; 3] = [0.55, 0.95, -0.32];
const WRIST: [f32; 3] = [0.56, 0.36, -0.5];
const HIP: [f32; 3] = [0.24, 0.92, 0.42];
const KNEE: [f32; 3] = [0.28, 0.52, 0.18];
const NECK: [f32; 3] = [0.0, 1.5, -0.2];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 16] = [
    joint("pelvis", ROOT, [0.0, 0.95, 0.35], Shade::Lit),
    joint("chest", PELVIS, [0.0, 1.15, 0.1], Shade::Lit),
    joint("head", CHEST, NECK, Shade::Lit),
    joint("jaw", HEAD, [0.0, 1.58, -0.24], Shade::Lit),
    joint("throat", HEAD, NECK, Shade::Glow),
    joint("eyes", HEAD, NECK, Shade::Core),
    joint("upper_arm_l", CHEST, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", UPPER_ARM_L, neg_x(ELBOW), Shade::Lit),
    joint("fist_l", FOREARM_L, neg_x(WRIST), Shade::Lit),
    joint("upper_arm_r", CHEST, SHOULDER, Shade::Lit),
    joint("forearm_r", UPPER_ARM_R, ELBOW, Shade::Lit),
    joint("fist_r", FOREARM_R, WRIST, Shade::Lit),
    joint("thigh_l", PELVIS, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", PELVIS, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wax = rgb(0xcfc8b8);
    let wax_dark = rgb(0x9a9284);
    let slag = rgb(0x3d3733);
    let brass = rgb(0xc89a42);
    let ember = Vec4::new(1.0, 0.5, 0.12, 1.0);
    let gum = Vec4::new(1.0, 0.32, 0.08, 1.0);

    // Haunches: a heavy rump between the hind legs, a slag tail stub.
    p.add(
        PELVIS,
        ellipsoid(
            Vec3::new(0.0, 0.98, 0.4),
            Vec3::new(0.3, 0.26, 0.3),
            Quat::IDENTITY,
            4,
            7,
            wax,
            true,
        ),
    );
    p.add(
        PELVIS,
        spike(
            Vec3::new(0.0, 1.05, 0.62),
            Vec3::new(0.0, 0.85, 0.95),
            0.07,
            5,
            slag,
        ),
    );

    // Torso: a hump rising from the hips to massive shoulders, brass cage hoops over the
    // ribs, slag spines along the back, wax drips under the belly.
    let ring = |c: Vec3, r: Vec2| kit::ngon_ring(c, Vec3::X, Vec3::Y, r, 8, 0.0);
    p.add(
        CHEST,
        kit::loft(
            &[
                ring(Vec3::new(0.0, 1.0, 0.42), Vec2::new(0.24, 0.22)),
                ring(Vec3::new(0.0, 1.12, 0.18), Vec2::new(0.34, 0.33)),
                ring(Vec3::new(0.0, 1.28, -0.05), Vec2::new(0.46, 0.4)),
                ring(Vec3::new(0.0, 1.38, -0.2), Vec2::new(0.42, 0.34)),
                ring(Vec3::new(0.0, 1.45, -0.3), Vec2::new(0.22, 0.2)),
            ],
            wax,
            true,
            (true, true),
            kit::Uv::Density(2.0),
        ),
    );
    for (i, (y, z, rx_, ry_)) in [
        (1.2, 0.12, 0.38, 0.36),
        (1.27, -0.02, 0.48, 0.42),
        (1.32, -0.15, 0.46, 0.38),
    ]
    .into_iter()
    .enumerate()
    {
        let c = Vec3::new(0.0, y, z);
        let r = Vec2::new(rx_ + 0.03, ry_ + 0.03);
        p.add(
            CHEST,
            kit::band(c, Vec3::new(0.0, 0.25, 1.0), r.x, 0.06, 0.03, 10, brass),
        );
        // Wax drips hanging off the cage.
        p.add(
            CHEST,
            spike(
                c + Vec3::new(0.12 * (i as f32 - 1.0), -r.y + 0.02, 0.0),
                c + Vec3::new(0.12 * (i as f32 - 1.0), -r.y - 0.16, 0.02),
                0.035,
                4,
                wax_dark,
            ),
        );
    }
    for (i, z) in [0.38, 0.2, 0.02, -0.14].into_iter().enumerate() {
        let y = 1.22 + 0.11 * i as f32 + if i == 3 { -0.02 } else { 0.0 };
        let base = Vec3::new(0.0, y + 0.18, z);
        p.add(
            CHEST,
            spike(base, base + Vec3::new(0.0, 0.26, 0.12), 0.07, 4, slag),
        );
    }

    // Head: a small low cranium over the neck, the huge jaw below it.
    p.add(
        HEAD,
        limb(
            v(NECK),
            Vec3::new(0.0, 1.66, -0.36),
            Vec2::new(0.17, 0.15),
            Vec2::new(0.2, 0.18),
            wax,
        ),
    );
    p.add(
        HEAD,
        ellipsoid(
            Vec3::new(0.0, 1.78, -0.42),
            Vec3::new(0.27, 0.17, 0.27),
            rx(-0.25),
            4,
            8,
            wax,
            true,
        ),
    );
    // Brow ridge and a slag crest.
    p.add(
        HEAD,
        limb(
            Vec3::new(-0.24, 1.82, -0.6),
            Vec3::new(0.24, 1.82, -0.6),
            Vec2::new(0.05, 0.05),
            Vec2::new(0.05, 0.05),
            wax_dark,
        ),
    );
    p.add(
        HEAD,
        spike(
            Vec3::new(0.0, 1.9, -0.34),
            Vec3::new(0.0, 2.02, -0.08),
            0.06,
            4,
            slag,
        ),
    );
    // Upper fangs hanging from the snout.
    for x in [-0.17, -0.06, 0.06, 0.17] {
        p.add(
            HEAD,
            spike(
                Vec3::new(x, 1.66, -0.64),
                Vec3::new(x * 0.95, 1.5, -0.68),
                0.032,
                4,
                brass,
            ),
        );
    }
    // Eyes: two ember slits under the brow.
    let eye = kit::block(
        Vec3::new(0.12, 1.76, -0.665),
        Vec3::new(0.06, 0.022, 0.02),
        rz(-0.3),
        ember,
    );
    add_mirrored(p.mesh(EYES), &eye);
    // Throat / gums: the inside of the mouth that burns orange when the jaw opens.
    p.add(
        THROAT,
        ellipsoid(
            Vec3::new(0.0, 1.52, -0.44),
            Vec3::new(0.2, 0.09, 0.24),
            Quat::IDENTITY,
            3,
            7,
            gum,
            true,
        ),
    );
    p.add(
        THROAT,
        limb(
            Vec3::new(-0.2, 1.64, -0.6),
            Vec3::new(0.2, 1.64, -0.6),
            Vec2::new(0.035, 0.02),
            Vec2::new(0.035, 0.02),
            gum,
        ),
    );

    // The jaw: a wide brass-toothed scoop hinged under the ears.
    let jaw_ring = |z: f32, y: f32, w: f32, d: f32| {
        kit::rect_ring(Vec3::new(0.0, y, z), Vec3::X, Vec3::Y, Vec2::new(w, d))
    };
    p.add(
        JAW,
        kit::loft(
            &[
                jaw_ring(-0.22, 1.56, 0.26, 0.1),
                jaw_ring(-0.48, 1.36, 0.36, 0.15),
                jaw_ring(-0.76, 1.3, 0.33, 0.13),
                jaw_ring(-0.9, 1.38, 0.2, 0.09),
            ],
            wax_dark,
            false,
            (true, true),
            kit::Uv::PerFace,
        ),
    );
    for i in 0..5 {
        let f = i as f32 / 4.0;
        let z = -0.5 - 0.36 * f;
        let x = 0.32 - 0.13 * f;
        let tooth = spike(
            Vec3::new(x, 1.46 - 0.04 * f, z),
            Vec3::new(x * 1.03, 1.7 - 0.06 * f, z - 0.03),
            0.045,
            4,
            brass,
        );
        add_mirrored(p.mesh(JAW), &tooth);
    }
    // Brass chin plate with a pair of tusks.
    p.add(
        JAW,
        kit::block(
            Vec3::new(0.0, 1.25, -0.78),
            Vec3::new(0.22, 0.05, 0.11),
            Quat::IDENTITY,
            brass,
        ),
    );
    let tusk = spike(
        Vec3::new(0.2, 1.4, -0.88),
        Vec3::new(0.27, 1.78, -0.95),
        0.05,
        5,
        brass,
    );
    add_mirrored(p.mesh(JAW), &tusk);

    // Arms (right built, left mirrored): bulging wax shoulders, forearm bands, huge fists
    // walked on the knuckles.
    let (s, e, w) = (v(SHOULDER), v(ELBOW), v(WRIST));
    let mut upper = kit::smooth_tube(s, e, 0.17, 0.12, 7, wax);
    add(&mut upper, gem(s, 0.19, 1, wax));
    // Gorilla forearms: thicker towards the fist.
    let mut fore = kit::smooth_tube(e, w, 0.12, 0.15, 7, wax);
    add(
        &mut fore,
        kit::band(e.lerp(w, 0.55), w - e, 0.15, 0.08, 0.03, 8, brass),
    );
    add(&mut fore, gem(e, 0.12, 0, wax_dark));
    // The fist, walked on its knuckles: a rounded mass, four curled fingers whose knuckles
    // take the weight, a thumb, brass studs.
    let mut fist = ellipsoid(
        Vec3::new(0.57, 0.24, -0.5),
        Vec3::new(0.17, 0.17, 0.16),
        ry(0.1),
        3,
        7,
        wax,
        true,
    );
    for k in 0..4 {
        let x = 0.45 + 0.08 * k as f32;
        let knuckle = Vec3::new(x, 0.1, -0.6 - 0.02 * (k == 1 || k == 2) as u8 as f32);
        add(
            &mut fist,
            ellipsoid(
                knuckle,
                Vec3::new(0.045, 0.08, 0.07),
                Quat::IDENTITY,
                2,
                6,
                wax_dark,
                true,
            ),
        );
        add(
            &mut fist,
            gem(knuckle + Vec3::new(0.0, 0.03, -0.065), 0.03, 0, brass),
        );
    }
    add(
        &mut fist,
        ellipsoid(
            Vec3::new(0.42, 0.22, -0.58),
            Vec3::new(0.05, 0.05, 0.1),
            ry(0.6),
            2,
            6,
            wax,
            true,
        ),
    );
    add(
        &mut fist,
        kit::limb(
            w,
            Vec3::new(0.57, 0.3, -0.5),
            Vec2::splat(0.12),
            Vec2::splat(0.14),
            wax,
        ),
    );
    // Hind legs: short and digitigrade.
    let (hip, knee) = (v(HIP), v(KNEE));
    let hock = Vec3::new(0.3, 0.22, 0.46);
    let mut thigh = kit::smooth_tube(hip, knee, 0.15, 0.1, 7, wax);
    add(&mut thigh, gem(knee, 0.1, 0, wax_dark));
    let mut shin = limb(knee, hock, Vec2::splat(0.08), Vec2::splat(0.06), wax);
    add(
        &mut shin,
        limb(
            hock,
            Vec3::new(0.3, 0.04, 0.3),
            Vec2::splat(0.06),
            Vec2::new(0.08, 0.05),
            wax_dark,
        ),
    );
    add(
        &mut shin,
        spike(
            Vec3::new(0.3, 0.05, 0.27),
            Vec3::new(0.31, 0.01, 0.12),
            0.04,
            4,
            slag,
        ),
    );

    for (right, left, mesh) in [
        (UPPER_ARM_R, UPPER_ARM_L, upper),
        (FOREARM_R, FOREARM_L, fore),
        (FIST_R, FIST_L, fist),
        (THIGH_R, THIGH_L, thigh),
        (SHIN_R, SHIN_L, shin),
    ] {
        p.add(left, kit::mirrored_x(&mesh));
        p.add(right, mesh);
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(31 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else {
            kit::tex_wax()
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Limb angles of one side: shoulder swing (+ = forward), spread, elbow, wrist; hip / knee.
#[derive(Clone, Copy)]
struct Side {
    arm: f32,
    spread: f32,
    elbow: f32,
    wrist: f32,
    hip: f32,
    knee: f32,
}

const REST: Side = Side {
    arm: 0.0,
    spread: 0.0,
    elbow: 0.0,
    wrist: 0.0,
    hip: 0.0,
    knee: 0.0,
};

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 3);
    let glow = telegraph(a.state, t, 0.65);
    pose.glow = gray(glow);
    pose.core = gray(heat(a.state, glow));

    let mut pelvis_off = Vec3::ZERO;
    let mut pelvis_rot = Quat::IDENTITY;
    let mut chest: f32;
    let mut head = 0.1 * wave(t, 0.4, off);
    let mut head_yaw = 0.0f32;
    let mut jaw = -0.08 - 0.06 * (wave(t, 1.1, off) * 0.5 + 0.5);
    let mut sides = [REST; 2];
    let mut squash = 1.0f32;

    let k = (a.speed / 8.0).clamp(0.0, 1.0);
    if a.speed > 0.2 {
        // Knuckle gallop: the fists strike together-ish, the hind legs push off half a cycle
        // later; the body rocks over the shoulders.
        let phase = a.cycle * std::f32::consts::TAU;
        let (s, c) = phase.sin_cos();
        let amp = 0.25 + 0.4 * k;
        for (i, side) in sides.iter_mut().enumerate() {
            let lag = if i == 0 { 0.0 } else { 0.5 };
            let (sa, ca) = (phase + lag).sin_cos();
            side.arm = amp * sa;
            side.elbow = -0.3 * amp * (1.0 + ca).max(0.0);
            side.wrist = -0.4 * amp * sa;
            side.hip = -amp * sa;
            side.knee = 0.5 * amp * (1.0 - ca).max(0.0);
        }
        chest = 0.12 * k * c;
        pelvis_off.y = 0.06 * k * (2.0 * phase).sin().abs() - 0.04 * k;
        pelvis_rot = rx(-0.08 * k * s);
        head -= 0.15 * k;
        jaw -= 0.1 * k;
    } else {
        // Idle: heaving breath, sniffing head turns.
        chest = 0.04 * wave(t, 0.5, off);
        let slot = (t * 0.45 + off as f64).floor() as i32;
        let look = engine::assets::procedural::rand01(slot, a.seed as i32, 21) * 2.0 - 1.0;
        head_yaw = 0.4 * look * smoothstep(((t * 0.45 + off as f64).fract() as f32) * 5.0);
    }
    // Lean into the turn / strafe.
    pelvis_rot *= rz(-0.18 * a.strafe.clamp(-1.0, 1.0));

    let pounce = a.attack == super::gnawer::POUNCE;
    match a.state {
        EnemyState::Windup(p) if pounce => {
            // Coils down low, fists spread, rump up, jaw parted.
            let e = smoothstep(p / 0.7);
            pelvis_off += Vec3::new(0.0, -0.18 * e, 0.08 * e);
            pelvis_rot *= rx(-0.15 * e);
            chest -= 0.25 * e;
            head += 0.35 * e;
            jaw -= 0.35 * e;
            for side in &mut sides {
                side.arm = -0.3 * e;
                side.spread = 0.3 * e;
                side.elbow = -0.9 * e;
                side.wrist = 0.4 * e;
                side.hip = 0.5 * e;
                side.knee = 0.9 * e;
            }
            // A quiver before it springs.
            pelvis_off.x += 0.015 * e * wave(t, 14.0, off);
        }
        EnemyState::Attack(p) if pounce => {
            // Airborne: stretched out, fists reaching, hind legs trailing, jaw wide.
            let e = smoothstep(p / 0.25);
            let land = smoothstep((p - 0.6) / 0.4);
            chest += 0.15 * e - 0.1 * land;
            head += 0.25 * e;
            jaw -= 0.75 * e * (1.0 - land);
            pelvis_rot *= rx(0.12 * e * (1.0 - land));
            for side in &mut sides {
                side.arm = 1.25 * e * (1.0 - land);
                side.spread = 0.25 * e;
                side.elbow = -0.2 * e;
                side.wrist = -0.6 * e;
                side.hip = -0.7 * e * (1.0 - land);
                side.knee = 0.3 * e;
            }
        }
        EnemyState::Windup(p) => {
            // Bite: rears the head, the jaw unhinges, the throat lights up.
            let e = smoothstep(p / 0.8);
            chest += 0.18 * e;
            head += 0.45 * e;
            jaw -= 0.95 * e;
            pelvis_off.y -= 0.05 * e;
            for side in &mut sides {
                side.elbow -= 0.25 * e;
                side.spread += 0.12 * e;
            }
            jaw += 0.05 * e * wave(t, 18.0, off);
        }
        EnemyState::Attack(p) => {
            // Snap: the head drives down and forward, the jaw slams shut.
            let s = smoothstep(p / 0.4);
            chest += 0.18 * (1.0 - s) - 0.12 * s;
            head += 0.45 * (1.0 - s) - 0.35 * s;
            jaw = -1.03 * (1.0 - s) - 0.02;
            pelvis_off += Vec3::new(0.0, -0.05, -0.22 * s);
            for side in &mut sides {
                side.arm += 0.2 * s;
            }
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            chest -= 0.12 * s;
            head -= 0.3 * s;
            pelvis_off.z -= 0.22 * s;
        }
        EnemyState::Stunned => {
            let w = wave(t, 1.2, off);
            chest = -0.25;
            head = -0.5 + 0.1 * w;
            head_yaw = 0.3 * w;
            jaw = -0.5;
            pelvis_off.y = -0.15;
            pelvis_rot = rz(0.1 * w);
            for side in &mut sides {
                side.elbow = -0.5;
                side.spread = 0.2;
                side.knee = 0.6;
                side.hip = 0.4;
            }
        }
        EnemyState::Dying(p) => {
            // Slumps, splits and melts: the legs give, the body sags into a puddle, the head
            // lolls off to one side, the arms fold under it.
            let (e, m, sq) = melt(p);
            squash = sq;
            pelvis_off = Vec3::new(0.0, -0.6 * e, 0.0);
            pelvis_rot = rz(0.2 * e) * rx(0.1 * e);
            chest = -0.3 * e;
            head = -0.5 * e - 0.3 * m;
            head_yaw = 0.6 * e;
            jaw = -0.12 - 0.7 * e;
            for (i, side) in sides.iter_mut().enumerate() {
                let sgn = if i == 0 { 1.0 } else { 0.8 };
                side.arm = 0.5 * e;
                side.spread = 0.6 * e * sgn;
                side.elbow = -1.4 * e;
                side.wrist = 0.8 * e;
                side.hip = 0.9 * e;
                side.knee = 1.4 * e;
            }
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    // Melting: the body squashes towards the floor and spreads a little.
    let spread = 1.0 + 0.15 * (1.0 - squash);
    pose.set(
        PELVIS,
        Mat4::from_scale_rotation_translation(
            Vec3::new(spread, squash, spread),
            pelvis_rot,
            pelvis_off,
        ),
    );
    pose.rotate(CHEST, rx(chest));
    pose.rotate(
        HEAD,
        ry(head_yaw) * rx(head + a.aim_pitch.clamp(-0.5, 0.5) * 0.5),
    );
    pose.rotate(JAW, rx(jaw));
    for (i, (upper, fore, fist, thigh, shin, sgn)) in [
        (UPPER_ARM_L, FOREARM_L, FIST_L, THIGH_L, SHIN_L, -1.0f32),
        (UPPER_ARM_R, FOREARM_R, FIST_R, THIGH_R, SHIN_R, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let s = sides[i];
        pose.rotate(upper, rz(s.spread * sgn) * rx(s.arm));
        pose.rotate(fore, rx(s.elbow));
        pose.rotate(fist, rx(s.wrist));
        pose.rotate(thigh, rx(s.hip));
        pose.rotate(shin, rx(s.knee));
    }
    pose
}
