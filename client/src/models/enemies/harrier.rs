//! Harrier: a ragged carrion flyer (3 m wingspan). Wax-skin wings stretched over brass ribs
//! with torn trailing edges, a gaunt keeled body, a long neck and a skull with a hooked brass
//! beak, dangling talons, a slag tail of tatters and a glowing heart sac under the chest (the
//! weak point, `Core`). Telegraph (`Glow`): it rears up, folds its wings back and its beak and
//! eyes burn red before it dives.
//!
//! Root = hover centre ([`HOVER_LIFT`] above the replicated feet); the pose adds the flap bob
//! and pitches the body down its dive (`EnemyAnim::aim_pitch`).

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, rx, ry, rz, seed_phase, smoothstep, telegraph, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

/// Model origin above the feet (half the 1.3 m capsule).
pub const HOVER_LIFT: f32 = 0.65;

pub const BODY: usize = 0;
pub const NECK: usize = 1;
pub const HEAD: usize = 2;
pub const GLARE: usize = 3;
pub const HEART: usize = 4;
pub const WING_L: usize = 5;
pub const TIP_L: usize = 6;
pub const WING_R: usize = 7;
pub const TIP_R: usize = 8;
pub const LEGS: usize = 9;
pub const TAIL: usize = 10;

const SHOULDER: [f32; 3] = [0.22, 0.12, -0.12];
const WRIST: [f32; 3] = [0.95, 0.18, -0.06];
const HEAD_AT: [f32; 3] = [0.0, 0.24, -0.74];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 11] = [
    joint("body", ROOT, [0.0, 0.0, 0.0], Shade::Lit),
    joint("neck", BODY, [0.0, 0.12, -0.36], Shade::Lit),
    joint("head", NECK, HEAD_AT, Shade::Lit),
    joint("glare", HEAD, HEAD_AT, Shade::Glow),
    joint("heart", BODY, [0.0, -0.3, -0.15], Shade::Core),
    joint("wing_l", BODY, neg_x(SHOULDER), Shade::Lit),
    joint("tip_l", WING_L, neg_x(WRIST), Shade::Lit),
    joint("wing_r", BODY, SHOULDER, Shade::Lit),
    joint("tip_r", WING_R, WRIST, Shade::Lit),
    joint("legs", BODY, [0.0, -0.18, 0.12], Shade::Lit),
    joint("tail", BODY, [0.0, 0.02, 0.4], Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// A thin membrane panel (quad, 3 or 4 points, convex) of wax-skin.
fn membrane(points: &[Vec3], color: Vec4) -> MeshBuilder {
    kit::plate(points, 0.02, color)
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let wax = rgb(0xd8cfbc);
    let skin = rgb(0xbfae93);
    let slag = rgb(0x3a3431);
    let brass = rgb(0xc59a44);
    let bone = rgb(0xe3d7bd);
    let red = Vec4::new(1.0, 0.16, 0.06, 1.0);
    let heart = Vec4::new(1.0, 0.32, 0.14, 1.0);

    // Body: a gaunt keel, brass ribs showing through the wax, a hunch over the shoulders.
    let ring =
        |z: f32, y: f32, r: Vec2| kit::ngon_ring(Vec3::new(0.0, y, z), Vec3::X, Vec3::Y, r, 7, 0.0);
    p.add(
        BODY,
        kit::loft(
            &[
                ring(-0.44, 0.08, Vec2::new(0.1, 0.1)),
                ring(-0.25, 0.06, Vec2::new(0.22, 0.2)),
                ring(0.05, 0.02, Vec2::new(0.24, 0.22)),
                ring(0.3, 0.02, Vec2::new(0.15, 0.13)),
                ring(0.45, 0.04, Vec2::new(0.06, 0.06)),
            ],
            wax,
            true,
            (true, true),
            kit::Uv::Density(3.0),
        ),
    );
    for z in [-0.18, -0.04, 0.1] {
        p.add(
            BODY,
            kit::band(
                Vec3::new(0.0, 0.03, z),
                Vec3::Z,
                0.22,
                0.035,
                0.025,
                8,
                brass,
            ),
        );
    }
    // A keel ridge under the chest that cradles the heart, and a slag spine.
    p.add(
        BODY,
        limb(
            Vec3::new(0.0, -0.18, -0.3),
            Vec3::new(0.0, -0.16, 0.1),
            Vec2::new(0.03, 0.04),
            Vec2::new(0.03, 0.03),
            brass,
        ),
    );
    for k in 0..4 {
        let z = -0.25 + 0.18 * k as f32;
        p.add(
            BODY,
            spike(
                Vec3::new(0.0, 0.2, z),
                Vec3::new(0.0, 0.34, z + 0.1),
                0.04,
                4,
                slag,
            ),
        );
    }

    // Neck: a long curve of wax and brass collars.
    p.add(
        NECK,
        kit::smooth_tube(
            Vec3::new(0.0, 0.12, -0.36),
            Vec3::new(0.0, 0.2, -0.66),
            0.09,
            0.07,
            6,
            skin,
        ),
    );
    p.add(
        NECK,
        kit::band(
            Vec3::new(0.0, 0.16, -0.5),
            Vec3::new(0.0, 0.25, -1.0),
            0.085,
            0.04,
            0.02,
            7,
            brass,
        ),
    );

    // Head: a bone skull with a long hooked brass beak and deep eye sockets.
    let h = v(HEAD_AT);
    p.add(
        HEAD,
        ellipsoid(
            h + Vec3::new(0.0, 0.02, -0.04),
            Vec3::new(0.1, 0.1, 0.14),
            Quat::IDENTITY,
            3,
            7,
            bone,
            true,
        ),
    );
    p.add(
        HEAD,
        kit::loft(
            &[
                kit::ngon_ring(
                    h + Vec3::new(0.0, 0.0, -0.14),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.065, 0.06),
                    5,
                    0.3,
                ),
                kit::ngon_ring(
                    h + Vec3::new(0.0, -0.03, -0.36),
                    Vec3::X,
                    Vec3::Y,
                    Vec2::new(0.035, 0.04),
                    5,
                    0.3,
                ),
                vec![h + Vec3::new(0.0, -0.13, -0.46)],
            ],
            brass,
            false,
            (true, false),
            kit::Uv::PerFace,
        ),
    );
    p.add(
        HEAD,
        spike(
            h + Vec3::new(0.0, 0.08, 0.05),
            h + Vec3::new(0.0, 0.18, 0.22),
            0.04,
            4,
            slag,
        ),
    );
    // The glare: eyes and the beak's cutting edge (red when it means to dive).
    let eye = gem(h + Vec3::new(0.065, 0.04, -0.1), 0.03, 0, red);
    add(p.mesh(GLARE), eye);
    add(
        p.mesh(GLARE),
        gem(h + Vec3::new(-0.065, 0.04, -0.1), 0.03, 0, red),
    );
    p.add(
        GLARE,
        limb(
            h + Vec3::new(0.0, -0.05, -0.16),
            h + Vec3::new(0.0, -0.11, -0.43),
            Vec2::new(0.012, 0.03),
            Vec2::new(0.006, 0.012),
            red,
        ),
    );

    // The heart sac under the chest, cradled by brass straps.
    let hc = Vec3::new(0.0, -0.3, -0.15);
    p.add(
        HEART,
        ellipsoid(
            hc,
            Vec3::new(0.22, 0.2, 0.24),
            Quat::IDENTITY,
            4,
            8,
            heart,
            true,
        ),
    );
    for x in [-0.12, 0.12] {
        p.add(
            BODY,
            limb(
                Vec3::new(x, -0.12, -0.32),
                Vec3::new(x * 1.4, -0.28, -0.15),
                Vec2::splat(0.018),
                Vec2::splat(0.018),
                brass,
            ),
        );
    }

    // Wings (right built, left mirrored): a brass arm rib with wax-skin stretched to the body,
    // then three finger ribs fanning back from the wrist with ragged membrane panels.
    let (s, w) = (v(SHOULDER), v(WRIST));
    let mut inner = limb(s, w, Vec2::splat(0.055), Vec2::splat(0.04), brass);
    add(&mut inner, gem(s, 0.08, 0, brass));
    add(
        &mut inner,
        membrane(
            &[s, w, Vec3::new(0.9, 0.12, 0.42), Vec3::new(0.2, 0.04, 0.42)],
            skin,
        ),
    );
    // Ragged strips hanging off the trailing edge.
    for (x, len) in [(0.35, 0.22), (0.6, 0.3), (0.82, 0.18)] {
        let base = Vec3::new(x, 0.08, 0.42);
        add(
            &mut inner,
            membrane(
                &[
                    base,
                    base + Vec3::new(0.09, 0.0, 0.0),
                    base + Vec3::new(0.05, -0.04, len),
                ],
                skin,
            ),
        );
    }
    let tip = Vec3::new(1.52, 0.14, 0.04);
    let fingers = [tip, Vec3::new(1.44, 0.1, 0.42), Vec3::new(1.14, 0.1, 0.6)];
    let mut outer = MeshBuilder::new();
    for f in fingers {
        add(
            &mut outer,
            limb(w, f, Vec2::splat(0.028), Vec2::splat(0.012), brass),
        );
    }
    add(&mut outer, gem(w, 0.045, 0, brass));
    // Panels between the fingers (and back to the inner wing), each with a torn notch.
    let edge = [
        w + Vec3::new(-0.05, -0.06, 0.48),
        fingers[2],
        fingers[1],
        fingers[0],
    ];
    for k in 0..3 {
        let (a, b) = (edge[k], edge[k + 1]);
        let notch = a.lerp(b, 0.5) + Vec3::new(-0.04, 0.0, -0.07 - 0.03 * k as f32);
        add(&mut outer, membrane(&[w, a, notch], skin));
        add(&mut outer, membrane(&[w, notch, b], skin));
    }
    p.add(WING_L, kit::mirrored_x(&inner));
    p.add(WING_R, inner);
    p.add(TIP_L, kit::mirrored_x(&outer));
    p.add(TIP_R, outer);

    // Dangling legs with brass talons.
    let mut legs = MeshBuilder::new();
    for x in [-0.1f32, 0.1] {
        let hip = Vec3::new(x, -0.18, 0.12);
        let knee = Vec3::new(x * 1.2, -0.42, 0.04);
        let foot = Vec3::new(x * 1.3, -0.62, 0.12);
        add(
            &mut legs,
            limb(hip, knee, Vec2::splat(0.04), Vec2::splat(0.03), skin),
        );
        add(
            &mut legs,
            limb(knee, foot, Vec2::splat(0.03), Vec2::splat(0.022), skin),
        );
        for (dx, dz) in [(-0.04, -0.07), (0.0, -0.09), (0.04, -0.07), (0.0, 0.06)] {
            add(
                &mut legs,
                spike(foot, foot + Vec3::new(dx, -0.07, dz), 0.016, 3, brass),
            );
        }
    }
    p.add(LEGS, legs);
    // Tail: three slag tatters.
    for (x, len) in [(-0.07, 0.4), (0.0, 0.52), (0.07, 0.36)] {
        let base = Vec3::new(x, 0.03, 0.4);
        p.add(
            TAIL,
            membrane(
                &[
                    base + Vec3::new(-0.04, 0.0, 0.0),
                    base + Vec3::new(0.04, 0.0, 0.0),
                    base + Vec3::new(x * 0.5, -0.06, len),
                ],
                slag,
            ),
        );
    }

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(59 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if i == HEAD {
            kit::tex_bone()
        } else {
            kit::tex_wax()
        };
        p.texture(i, tex);
    }
    p.finish()
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 7);
    let glow = telegraph(a.state, t, 0.7);
    pose.glow = gray(glow);
    let mut core = heat(a.state, glow);

    // Slow heavy flaps; the body rises on each down-stroke.
    let mut flap_hz = 1.9f64;
    let mut flap_amp = 0.7f32;
    let mut fold = 0.0f32; // wings swept back (0 = spread)
    let mut tip_fold = 0.0f32;
    let mut body_rot =
        rz(-0.45 * a.strafe.clamp(-1.0, 1.0)) * rx(-0.12 * (a.speed / 12.0).clamp(0.0, 1.0));
    let mut body_off = Vec3::ZERO;
    let mut neck = 0.15 * wave(t, 0.5, off);
    let mut head = 0.1 * wave(t, 0.7, off + 0.2) + a.aim_pitch.clamp(-0.8, 0.8) * 0.4;
    let mut legs = rx(0.25 + 0.1 * wave(t, 0.9, off));
    let mut tail = 0.15 * wave(t, 1.1, off);
    let mut spin = Quat::IDENTITY;

    match a.state {
        EnemyState::Windup(p) => {
            // Rears up, wings drawn back and up, neck coiled, heart and beak burning.
            let e = smoothstep(p / 0.8);
            body_rot *= rx(0.7 * e);
            body_off.y += 0.25 * e;
            fold = 0.9 * e;
            tip_fold = 0.6 * e;
            flap_amp = 0.7 * (1.0 - e) + 0.15 * e;
            flap_hz = 1.9 + 2.0 * e as f64;
            neck += 0.5 * e;
            head += 0.5 * e;
            legs = rx(-0.3 * e);
            core *= 1.0 + 1.2 * e;
        }
        EnemyState::Attack(_) => {
            // Diving: pitched down its line, wings swept tight, talons thrust forward.
            body_rot = rx(a.aim_pitch.clamp(-1.4, 0.6));
            fold = 1.3;
            tip_fold = 1.2;
            flap_amp = 0.05;
            neck = -0.2;
            head = 0.2;
            legs = rx(-1.1);
            tail = 0.0;
            core *= 2.2;
        }
        EnemyState::Recover(p) => {
            // Beats hard to climb back.
            let s = 1.0 - smoothstep(p);
            body_rot *= rx(0.35 * s);
            flap_hz = 3.2;
            flap_amp = 0.95;
            legs = rx(0.6 * s);
        }
        EnemyState::Stunned => {
            // Swatted: tumbling, wings flailing.
            spin = rz(2.0 * wave(t, 0.6, off)) * rx(0.8 * wave(t, 0.45, off));
            flap_hz = 4.5;
            flap_amp = 0.4;
            fold = 0.4 * wave(t, 1.3, off);
            neck = -0.6;
            head = -0.4;
            core *= 0.8;
        }
        EnemyState::Dying(p) => {
            // Crumples: wings fold limp and sag, the body rolls over and sinks.
            let e = smoothstep(p / 0.6);
            spin = rz(1.4 * e) * rx(0.5 * e);
            body_off.y -= 0.25 * e;
            fold = -0.3 * e;
            tip_fold = -0.9 * e;
            flap_amp = 0.0;
            neck = -0.9 * e;
            head = -0.7 * e;
            legs = rx(1.0 * e);
            tail = -0.4 * e;
        }
        EnemyState::Idle | EnemyState::Walk => {}
    }

    // Wings held in a shallow V (they read from the side too), flapping around it.
    let flap = 0.3 * (1.0 - fold.clamp(0.0, 1.0)) + flap_amp * wave(t, flap_hz, off);
    // Body lifts on the down-stroke.
    body_off.y += 0.08 * flap_amp * wave(t, flap_hz, off + 0.25);
    for (wing, tip, sgn) in [(WING_L, TIP_L, -1.0f32), (WING_R, TIP_R, 1.0)] {
        // Flap: rotate about the body's long axis (Z); fold: sweep back about Y and lift.
        pose.rotate(wing, ry(sgn * fold * 0.9) * rz(sgn * (flap + 0.35 * fold)));
        pose.rotate(
            tip,
            ry(sgn * tip_fold * 0.8) * rz(sgn * (0.6 * flap - 0.3 * tip_fold)),
        );
    }
    pose.rotate_move(BODY, spin * body_rot, body_off);
    pose.rotate(NECK, rx(neck));
    pose.rotate(HEAD, rx(head));
    pose.rotate(LEGS, legs);
    pose.rotate(TAIL, rx(tail) * ry(0.2 * wave(t, 0.8, off)));
    pose.core = gray(core);
    pose
}
