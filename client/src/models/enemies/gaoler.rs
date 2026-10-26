//! Gaoler: the heavy, a 3.7 m iron jailer Mould. A barrel chest of riveted iron plate over wax,
//! a grilled box helm (the grille glows: the head weak point, `Core`), a riveted cell door for
//! a shield on its left arm, a chain flail with a spiked ball in its right hand, and on its back
//! a caged heart-lantern (the big weak point on the unarmoured side, `Core`).
//! Telegraphs (`Glow`): the door's rim burns yellow for the Shield Bash and strobes white-hot
//! in its parry window; the chain glows red for the Chain Sweep; both flare for the Rush.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::MeshBuilder;
use engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};

use super::{
    EnemyAnim, EnemyState, gray, heat, melt, rx, ry, rz, seed_phase, smoothstep, strobe, telegraph,
    walk_legs, wave,
};
use crate::models::PartSet;
use crate::models::kit::{self, add, add_mirrored, ellipsoid, gem, limb, rgb, spike};
use crate::models::rig::{JointDef, Pose, ROOT, Shade, joint};

pub const PELVIS: usize = 0;
pub const TORSO: usize = 1;
pub const LANTERN: usize = 2;
pub const HELM: usize = 3;
pub const GRILLE: usize = 4;
pub const ARM_L: usize = 5;
pub const FOREARM_L: usize = 6;
pub const SHIELD: usize = 7;
pub const RIM: usize = 8;
pub const ARM_R: usize = 9;
pub const FOREARM_R: usize = 10;
pub const CHAIN: usize = 11;
pub const CHAIN_GLOW: usize = 12;
pub const FLAIL: usize = 13;
pub const THIGH_L: usize = 14;
pub const SHIN_L: usize = 15;
pub const THIGH_R: usize = 16;
pub const SHIN_R: usize = 17;

/// Attack indices (assets/enemies/gaoler.ron).
pub const BASH: u8 = 0;
pub const SWEEP: u8 = 1;
pub const RUSH: u8 = 2;

/// Wind-up progress from which the bash's parry window strobe starts (gameplay: last 0.35 s
/// of the 1 s wind-up).
pub const PARRY_WINDOW_FROM: f32 = 0.65;

const SHOULDER: [f32; 3] = [1.0, 2.78, 0.0];
const ELBOW_L: [f32; 3] = [-1.18, 2.08, -0.32];
const HAND_L: [f32; 3] = [-0.98, 1.72, -0.8];
const ELBOW_R: [f32; 3] = [1.2, 2.06, -0.16];
const HAND_R: [f32; 3] = [1.26, 1.5, -0.42];
/// Flail ball centre at rest (hangs below the right hand).
const BALL: [f32; 3] = [1.26, 0.42, -0.42];
const HIP: [f32; 3] = [0.46, 1.3, 0.02];
const KNEE: [f32; 3] = [0.5, 0.72, -0.06];

const fn neg_x(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

pub const JOINTS: [JointDef; 18] = [
    joint("pelvis", ROOT, [0.0, 1.35, 0.0], Shade::Lit),
    joint("torso", PELVIS, [0.0, 1.6, 0.0], Shade::Lit),
    joint("lantern", TORSO, [0.0, 2.2, 0.95], Shade::Core),
    joint("helm", TORSO, [0.0, 3.02, -0.22], Shade::Lit),
    joint("grille", HELM, [0.0, 3.02, -0.22], Shade::Core),
    joint("arm_l", TORSO, neg_x(SHOULDER), Shade::Lit),
    joint("forearm_l", ARM_L, ELBOW_L, Shade::Lit),
    joint("shield", FOREARM_L, HAND_L, Shade::Lit),
    joint("rim", SHIELD, HAND_L, Shade::Glow),
    joint("arm_r", TORSO, SHOULDER, Shade::Lit),
    joint("forearm_r", ARM_R, ELBOW_R, Shade::Lit),
    joint("chain", FOREARM_R, HAND_R, Shade::Lit),
    joint("chain_glow", CHAIN, HAND_R, Shade::Glow),
    joint("flail", CHAIN, BALL, Shade::Lit),
    joint("thigh_l", PELVIS, neg_x(HIP), Shade::Lit),
    joint("shin_l", THIGH_L, neg_x(KNEE), Shade::Lit),
    joint("thigh_r", PELVIS, HIP, Shade::Lit),
    joint("shin_r", THIGH_R, KNEE, Shade::Lit),
];

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// Riveted plate: a box plus a row of rivet heads along its top edge.
fn plate(center: Vec3, half: Vec3, rot: Quat, color: Vec4, rivet: Vec4) -> MeshBuilder {
    let mut m = kit::block(center, half, rot, color);
    for k in 0..4 {
        let x = -half.x + (k as f32 + 0.5) * half.x * 0.5;
        add(
            &mut m,
            gem(
                center + rot * Vec3::new(x, half.y * 0.75, -half.z),
                0.03,
                0,
                rivet,
            ),
        );
    }
    m
}

pub fn build() -> Vec<ModelPart> {
    let mut p = PartSet::new(&JOINTS);
    let iron = rgb(0x56524d);
    let iron_dark = rgb(0x302d2b);
    let wax = rgb(0xc8c1b2);
    let brass = rgb(0xc4973f);
    let rim = Vec4::new(1.0, 0.82, 0.18, 1.0);
    let hot = Vec4::new(1.0, 0.25, 0.08, 1.0);
    let lantern = Vec4::new(1.0, 0.72, 0.25, 1.0);
    let grille = Vec4::new(1.0, 0.55, 0.15, 1.0);

    // Pelvis: an iron belt and a skirt of hanging plates.
    p.add(
        PELVIS,
        kit::block(
            Vec3::new(0.0, 1.42, 0.0),
            Vec3::new(0.62, 0.16, 0.45),
            Quat::IDENTITY,
            iron_dark,
        ),
    );
    for k in 0..5 {
        let x = -0.5 + 0.25 * k as f32;
        p.add(
            PELVIS,
            kit::block(
                Vec3::new(x, 1.12, -0.42),
                Vec3::new(0.11, 0.2, 0.03),
                rx(-0.12),
                iron,
            ),
        );
    }

    // Torso: a barrel of iron plate, wax bulging at the seams, a heavy collar.
    let ring = |y: f32, w: f32, d: f32| {
        kit::chamfer_ring(
            Vec3::new(0.0, y, 0.0),
            Vec3::X,
            Vec3::Z,
            Vec2::new(w, d),
            0.18,
        )
    };
    p.add(
        TORSO,
        kit::loft(
            &[
                ring(1.58, 0.6, 0.44),
                ring(2.1, 0.86, 0.56),
                ring(2.65, 0.98, 0.58),
                ring(2.98, 0.72, 0.46),
            ],
            wax,
            false,
            (true, true),
            kit::Uv::Density(1.5),
        ),
    );
    for (y, w) in [(1.95, 0.84), (2.4, 0.95), (2.78, 0.9)] {
        let front = plate(
            Vec3::new(0.0, y, -0.56),
            Vec3::new(w * 0.8, 0.18, 0.05),
            Quat::IDENTITY,
            iron,
            brass,
        );
        add(p.mesh(TORSO), front);
    }
    for side in [-1.0f32, 1.0] {
        p.add(
            TORSO,
            kit::block(
                Vec3::new(side * 0.94, 2.35, 0.0),
                Vec3::new(0.06, 0.45, 0.42),
                rz(side * 0.12),
                iron,
            ),
        );
    }
    p.add(
        TORSO,
        kit::block(
            Vec3::new(0.0, 3.0, -0.05),
            Vec3::new(0.6, 0.1, 0.42),
            Quat::IDENTITY,
            iron_dark,
        ),
    );
    // Pauldrons: big round iron shells.
    let pauldron = ellipsoid(
        Vec3::new(1.0, 2.88, 0.0),
        Vec3::new(0.38, 0.26, 0.4),
        rz(-0.3),
        3,
        8,
        iron,
        false,
    );
    add_mirrored(p.mesh(TORSO), &pauldron);
    // The lantern cage on its back: brackets, rings and bars of brass around the heart glow.
    let lc = Vec3::new(0.0, 2.2, 0.95);
    for x in [-0.3, 0.3] {
        p.add(
            TORSO,
            limb(
                Vec3::new(x, 2.5, 0.45),
                Vec3::new(x * 0.8, 2.6, 0.95),
                Vec2::splat(0.05),
                Vec2::splat(0.04),
                brass,
            ),
        );
        p.add(
            TORSO,
            limb(
                Vec3::new(x, 1.9, 0.45),
                Vec3::new(x * 0.8, 1.8, 0.95),
                Vec2::splat(0.05),
                Vec2::splat(0.04),
                brass,
            ),
        );
    }
    p.add(
        TORSO,
        kit::band(lc + Vec3::Y * 0.42, Vec3::Y, 0.42, 0.07, 0.05, 10, brass),
    );
    p.add(
        TORSO,
        kit::band(lc - Vec3::Y * 0.42, Vec3::Y, 0.42, 0.07, 0.05, 10, brass),
    );
    for k in 0..7 {
        let a = k as f32 / 7.0 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        p.add(
            TORSO,
            limb(
                lc + Vec3::new(c * 0.44, -0.45, s * 0.44),
                lc + Vec3::new(c * 0.44, 0.45, s * 0.44),
                Vec2::splat(0.025),
                Vec2::splat(0.025),
                brass,
            ),
        );
    }
    p.add(
        TORSO,
        spike(lc + Vec3::Y * 0.46, lc + Vec3::Y * 0.75, 0.12, 6, iron_dark),
    );
    p.add(
        LANTERN,
        ellipsoid(
            lc,
            Vec3::new(0.34, 0.38, 0.34),
            Quat::IDENTITY,
            4,
            8,
            lantern,
            false,
        ),
    );

    // The helm: an iron box with a crest and a barred grille; the heat glows behind the bars.
    let hc = Vec3::new(0.0, 3.3, -0.3);
    p.add(
        HELM,
        kit::block(
            Vec3::new(0.0, 3.1, -0.18),
            Vec3::new(0.24, 0.12, 0.22),
            Quat::IDENTITY,
            iron_dark,
        ),
    );
    p.add(
        HELM,
        kit::block(
            hc + Vec3::new(0.0, 0.26, 0.05),
            Vec3::new(0.34, 0.06, 0.32),
            Quat::IDENTITY,
            iron,
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            HELM,
            kit::block(
                hc + Vec3::new(side * 0.31, 0.0, 0.05),
                Vec3::new(0.04, 0.3, 0.32),
                Quat::IDENTITY,
                iron,
            ),
        );
    }
    p.add(
        HELM,
        kit::block(
            hc + Vec3::new(0.0, 0.0, 0.34),
            Vec3::new(0.3, 0.3, 0.04),
            Quat::IDENTITY,
            iron,
        ),
    );
    p.add(
        HELM,
        kit::block(
            hc + Vec3::new(0.0, -0.27, 0.05),
            Vec3::new(0.3, 0.04, 0.3),
            Quat::IDENTITY,
            iron,
        ),
    );
    for k in 0..5 {
        let x = -0.24 + 0.12 * k as f32;
        p.add(
            HELM,
            kit::block(
                hc + Vec3::new(x, 0.0, -0.28),
                Vec3::new(0.022, 0.26, 0.022),
                Quat::IDENTITY,
                iron_dark,
            ),
        );
    }
    p.add(
        HELM,
        kit::block(
            hc + Vec3::new(0.0, 0.08, -0.28),
            Vec3::new(0.3, 0.025, 0.025),
            Quat::IDENTITY,
            iron_dark,
        ),
    );
    p.add(
        HELM,
        limb(
            hc + Vec3::new(0.0, 0.32, -0.22),
            hc + Vec3::new(0.0, 0.4, 0.3),
            Vec2::new(0.04, 0.08),
            Vec2::new(0.03, 0.03),
            brass,
        ),
    );
    p.add(
        GRILLE,
        kit::block(
            hc + Vec3::new(0.0, 0.0, -0.18),
            Vec3::new(0.27, 0.25, 0.08),
            Quat::IDENTITY,
            grille,
        ),
    );

    // Arms: thick wax upper arms, iron vambraces, gauntlets.
    let s = v(SHOULDER);
    let upper_r = kit::smooth_tube(s, v(ELBOW_R), 0.26, 0.2, 7, wax);
    let upper_l = kit::smooth_tube(v(neg_x(SHOULDER)), v(ELBOW_L), 0.26, 0.2, 7, wax);
    p.add(ARM_R, upper_r);
    p.add(ARM_L, upper_l);
    let mut fore_r = limb(
        v(ELBOW_R),
        v(HAND_R),
        Vec2::splat(0.2),
        Vec2::splat(0.17),
        iron,
    );
    add(&mut fore_r, gem(v(HAND_R), 0.17, 0, iron_dark));
    p.add(FOREARM_R, fore_r);
    let mut fore_l = limb(
        v(ELBOW_L),
        v(HAND_L),
        Vec2::splat(0.2),
        Vec2::splat(0.17),
        iron,
    );
    add(&mut fore_l, gem(v(HAND_L), 0.17, 0, iron_dark));
    p.add(FOREARM_L, fore_l);

    // The shield: a riveted cell door, barred window, hinge straps and a lock plate. It stands
    // in front of the left half of the body.
    let dc = Vec3::new(-0.62, 1.82, -1.02);
    let (dw, dh, dt) = (0.68, 1.12, 0.06);
    // Door leaf around the window (four frames), so the bars read against the dark gap.
    p.add(
        SHIELD,
        kit::block(
            dc + Vec3::new(0.0, -0.4, 0.0),
            Vec3::new(dw, 0.72, dt),
            Quat::IDENTITY,
            iron,
        ),
    );
    p.add(
        SHIELD,
        kit::block(
            dc + Vec3::new(0.0, 0.98, 0.0),
            Vec3::new(dw, 0.14, dt),
            Quat::IDENTITY,
            iron,
        ),
    );
    for side in [-1.0f32, 1.0] {
        p.add(
            SHIELD,
            kit::block(
                dc + Vec3::new(side * (dw - 0.13), 0.6, 0.0),
                Vec3::new(0.13, 0.26, dt),
                Quat::IDENTITY,
                iron,
            ),
        );
    }
    for k in 0..5 {
        let x = -0.4 + 0.2 * k as f32;
        p.add(
            SHIELD,
            kit::block(
                dc + Vec3::new(x, 0.6, -0.01),
                Vec3::new(0.03, 0.27, 0.03),
                Quat::IDENTITY,
                iron_dark,
            ),
        );
    }
    for y in [-0.85, -0.1, 0.9] {
        p.add(
            SHIELD,
            kit::block(
                dc + Vec3::new(-dw * 0.45, y, -dt - 0.015),
                Vec3::new(dw * 0.55, 0.05, 0.015),
                Quat::IDENTITY,
                iron_dark,
            ),
        );
        for k in 0..4 {
            p.add(
                SHIELD,
                gem(
                    dc + Vec3::new(-dw + 0.1 + 0.18 * k as f32, y, -dt - 0.03),
                    0.028,
                    0,
                    brass,
                ),
            );
        }
    }
    p.add(
        SHIELD,
        kit::block(
            dc + Vec3::new(dw - 0.18, -0.2, -dt - 0.02),
            Vec3::new(0.1, 0.14, 0.02),
            Quat::IDENTITY,
            brass,
        ),
    );
    // Grip from the gauntlet to the door's back.
    p.add(
        SHIELD,
        limb(
            v(HAND_L),
            dc + Vec3::new(0.0, -0.1, dt),
            Vec2::splat(0.07),
            Vec2::splat(0.07),
            iron_dark,
        ),
    );
    // The rim: a frame around the door's face that burns when it bashes.
    let rim_bar = |c: Vec3, h: Vec3| kit::block(dc + c, h, Quat::IDENTITY, rim);
    for side in [-1.0f32, 1.0] {
        p.add(
            RIM,
            rim_bar(
                Vec3::new(side * (dw + 0.02), -0.0, -0.01),
                Vec3::new(0.035, dh, dt + 0.015),
            ),
        );
        p.add(
            RIM,
            rim_bar(
                Vec3::new(0.0, side * (dh + 0.02), -0.01),
                Vec3::new(dw + 0.05, 0.035, dt + 0.015),
            ),
        );
    }

    // The chain flail: links from the gauntlet down to a spiked iron ball.
    let hand = v(HAND_R);
    let ball = v(BALL);
    let links = 8;
    let mut glow = MeshBuilder::new();
    for k in 0..links {
        let f0 = k as f32 / links as f32;
        let f1 = (k as f32 + 0.85) / links as f32;
        let a = hand.lerp(ball + Vec3::Y * 0.3, f0);
        let b = hand.lerp(ball + Vec3::Y * 0.3, f1);
        let side = if k % 2 == 0 { Vec3::X } else { Vec3::Z };
        p.add(
            CHAIN,
            kit::limb_oriented(
                a,
                b,
                side,
                Vec2::new(0.06, 0.02),
                Vec2::new(0.06, 0.02),
                iron_dark,
            ),
        );
        add(
            &mut glow,
            kit::limb_oriented(
                a,
                b,
                side,
                Vec2::new(0.035, 0.03),
                Vec2::new(0.035, 0.03),
                hot,
            ),
        );
    }
    p.add(CHAIN_GLOW, glow);
    p.add(
        FLAIL,
        ellipsoid(
            ball,
            Vec3::splat(0.3),
            Quat::IDENTITY,
            4,
            8,
            iron_dark,
            false,
        ),
    );
    for k in 0..10 {
        let d = engine::assets::procedural::rand01(k, 3, 0x9a0) * std::f32::consts::TAU;
        let h = engine::assets::procedural::rand01(k, 5, 0x9a0) * 2.0 - 1.0;
        let dir = Vec3::new(
            d.cos() * (1.0 - h * h).sqrt(),
            h,
            d.sin() * (1.0 - h * h).sqrt(),
        );
        p.add(
            FLAIL,
            spike(ball + dir * 0.26, ball + dir * 0.5, 0.06, 4, iron),
        );
    }

    // Legs: thick wax thighs, iron greaves, heavy boots.
    let (hip, knee) = (v(HIP), v(KNEE));
    let ankle = Vec3::new(0.5, 0.2, 0.0);
    let thigh = kit::smooth_tube(hip, knee, 0.3, 0.24, 7, wax);
    let mut shin = limb(knee, ankle, Vec2::splat(0.24), Vec2::splat(0.2), iron);
    add(
        &mut shin,
        kit::block(
            Vec3::new(0.5, 0.1, -0.12),
            Vec3::new(0.24, 0.1, 0.36),
            Quat::IDENTITY,
            iron_dark,
        ),
    );
    add(
        &mut shin,
        gem(knee + Vec3::new(0.0, 0.0, -0.18), 0.12, 0, brass),
    );
    p.add(THIGH_L, kit::mirrored_x(&thigh));
    p.add(THIGH_R, thigh);
    p.add(SHIN_L, kit::mirrored_x(&shin));
    p.add(SHIN_R, shin);

    p.for_each_mesh(|i, m| {
        if !JOINTS[i].shade.is_emissive() {
            kit::finish(m);
            m.jitter_colors(71 + i as u32, 0.05);
        }
    });
    for (i, j) in JOINTS.iter().enumerate() {
        let tex = if j.shade.is_emissive() {
            kit::tex_glow()
        } else if matches!(i, ARM_L | ARM_R | THIGH_L | THIGH_R) {
            kit::tex_wax()
        } else {
            kit::tex_grime()
        };
        p.texture(i, tex);
    }
    p.finish()
}

/// Chain rotation relative to the forearm (world rotation `parent`) pointing the chain out
/// horizontally at yaw `spin`, drooping by `droop` radians.
fn chain_out(parent: Quat, spin: f32, droop: f32) -> Quat {
    parent.inverse() * ry(spin) * rz(std::f32::consts::FRAC_PI_2 - droop)
}

pub fn pose(a: &EnemyAnim) -> Pose {
    let mut pose = Pose::rest(JOINTS.len());
    let t = a.time;
    let off = seed_phase(a.seed, 11);
    let bash = a.attack == BASH;
    let sweep = a.attack == SWEEP;
    let rush = a.attack == RUSH;
    let flicker = if bash { PARRY_WINDOW_FROM } else { 0.75 };
    let glow = telegraph(a.state, t, flicker);
    pose.glow = gray(glow);
    pose.core = gray(heat(a.state, glow) * (1.0 + 0.1 * wave(t, 1.3, off)));

    let k = (a.speed / 4.5).clamp(0.0, 1.0);
    let mut pelvis_off = Vec3::ZERO;
    let mut pelvis_rot = rz(-0.06 * a.strafe.clamp(-1.0, 1.0));
    let mut torso = rx(0.02 * wave(t, 0.35, off));
    let mut helm = rx(0.03 * wave(t, 0.5, off + 0.4));
    let mut arm_l = Quat::IDENTITY;
    // The door stays upright facing `door_yaw` (the forearm counter-rotates whatever the arm
    // and body do), unless a pose sets `door` itself (knocked aside, falling).
    let mut door_yaw = 0.0f32;
    let mut door: Option<Quat> = None;
    let mut arm_r = rz(0.05);
    let fore_r = Quat::IDENTITY;
    let mut chain =
        rx(0.12 * wave(t, 0.5, off) + 0.25 * k * (a.cycle * std::f32::consts::TAU).sin());
    let mut whirl: Option<(f32, f32)> = None;
    let mut legs = [(0.0f32, 0.0f32); 2];
    let mut squash = 1.0f32;
    // Which telegraph glows: the rim (bash), the chain (sweep), both (rush, idle).
    let (mut rim_on, mut chain_on) = (true, true);

    if a.speed > 0.2 {
        let amp = 0.2 + 0.25 * k;
        legs = walk_legs(a.cycle, amp, 1.1);
        let phase = a.cycle * std::f32::consts::TAU;
        pelvis_off.y = -0.05 * (2.0 * phase).cos().abs();
        pelvis_rot *= rz(0.05 * phase.sin()) * ry(0.06 * phase.sin());
        door_yaw += 0.06 * phase.sin();
        torso *= rx(0.06 * k);
        // The door swings a little with the stride.
        arm_l = rx(0.12 * phase.sin());
    }

    match a.state {
        EnemyState::Windup(p) if bash => {
            // Draws the door back, twisting away; the rim burns, then strobes white-hot.
            let e = smoothstep(p / 0.6);
            chain_on = false;
            torso *= ry(0.45 * e) * rx(-0.08 * e);
            door_yaw += 0.45 * e + 0.25 * e;
            arm_l = rx(-0.55 * e) * rz(-0.25 * e);
            pelvis_off.y -= 0.12 * e;
            legs = [(0.25 * e, -0.3 * e), (-0.35 * e, -0.2 * e)];
            if p >= PARRY_WINDOW_FROM {
                // Parry window: alternate yellow and white-hot.
                let w = 3.6 * strobe(t, 12.0);
                pose.glow = Vec4::new(glow.max(w), glow.max(w), glow.max(w * 0.9), 1.0);
            }
        }
        EnemyState::Attack(p) if bash => {
            // Slams the door forward.
            let s = smoothstep(p / 0.35);
            chain_on = false;
            let twist = 0.45 * (1.0 - s) - 0.35 * s;
            torso *= ry(twist) * rx(0.15 * s);
            door_yaw += twist;
            arm_l = rx(-0.55 + 1.25 * s) * rz(-0.25 + 0.2 * s);
            pelvis_off += Vec3::new(0.0, -0.12, -0.4 * s);
            legs = [(0.6 * s, -0.4 * s), (-0.45 * s, -0.1)];
        }
        EnemyState::Windup(p) if sweep => {
            // Crouches and swings the chain out low; it glows red as it gathers speed.
            let e = smoothstep(p / 0.5);
            rim_on = false;
            pelvis_off.y -= 0.35 * e;
            torso *= rx(0.25 * e) * ry(-0.3 * e);
            door_yaw -= 0.3 * e;
            arm_r = rz(1.2 * e) * rx(0.15 * e);
            arm_l = rx(0.3 * e);
            legs = [(0.55 * e, -0.9 * e), (0.55 * e, -0.9 * e)];
            let turns = 0.5 * p + 1.2 * p * p;
            whirl = Some((turns * std::f32::consts::TAU, 1.2 - 0.7 * e));
        }
        EnemyState::Attack(p) if sweep => {
            // The 300 degree sweep: the whole body turns with the chain at knee height.
            let s = smoothstep(p);
            rim_on = false;
            pelvis_off.y -= 0.35;
            pelvis_rot *= ry(-5.2 * s);
            door_yaw -= 5.2 * s;
            torso *= rx(0.25);
            arm_r = rz(1.25) * rx(0.15);
            arm_l = rx(0.3);
            legs = [(0.55, -0.9), (0.55, -0.9)];
            // The chain trails a little behind the turning body, out at knee height.
            whirl = Some((-5.2 * s + 0.35, 0.45));
        }
        EnemyState::Windup(p) if rush => {
            // Hunkers behind the door, pawing the floor.
            let e = smoothstep(p / 0.6);
            pelvis_off.y -= 0.25 * e;
            torso *= rx(0.45 * e);
            helm *= rx(-0.3 * e);
            arm_l = rx(0.75 * e) * rz(0.3 * e);
            legs = [
                (0.7 * e, -0.9 * e),
                (-0.3 * e + 0.15 * wave(t, 4.0, off) * e, -0.5 * e),
            ];
        }
        EnemyState::Attack(_) if rush => {
            // Charging behind the shield.
            let phase = (t * 3.5) as f32 * std::f32::consts::TAU;
            pelvis_off.y -= 0.2;
            torso *= rx(0.5);
            helm *= rx(-0.35);
            arm_l = rx(0.85) * rz(0.3);
            legs = [
                (0.7 * phase.sin(), -0.6 - 0.4 * phase.cos().max(0.0)),
                (-0.7 * phase.sin(), -0.6 - 0.4 * (-phase.cos()).max(0.0)),
            ];
        }
        EnemyState::Recover(p) => {
            let s = 1.0 - smoothstep(p);
            pelvis_off.y -= 0.2 * s;
            torso *= rx(0.15 * s);
            if sweep {
                rim_on = false;
                arm_r = rz(1.2 * s);
                whirl = (s > 0.4).then_some((-0.6 - 1.5 * p, 1.1 + 0.4 * p));
            } else {
                chain_on = !bash;
                arm_l = rx(0.6 * s);
                door_yaw -= 0.35 * s * bash as u8 as f32;
            }
        }
        EnemyState::Stunned => {
            // Parried: the door is knocked aside, it reels.
            let w = wave(t, 0.8, off);
            rim_on = false;
            chain_on = false;
            torso = rx(-0.25) * ry(-0.4 + 0.1 * w) * rz(0.08 * w);
            helm = rx(-0.4) * rz(0.25 * w);
            arm_l = rz(-1.25) * rx(-0.4);
            // The door knocked open to the side like a gate.
            door = Some(ry(-1.3 + 0.1 * w) * rz(-0.25));
            arm_r = rz(0.5);
            pelvis_off.y = -0.15;
            legs = [(0.3, -0.5), (-0.2, -0.3)];
        }
        EnemyState::Dying(p) => {
            // Drops to its knees, the door falls, it sags and melts; the lantern gutters.
            let (e, m, sq) = melt(p);
            squash = sq;
            rim_on = false;
            chain_on = false;
            pelvis_off = Vec3::new(0.0, -0.75 * e, 0.0);
            pelvis_rot = rx(0.2 * e) * rz(0.12 * m);
            torso = rx(0.5 * e) * rz(-0.2 * m);
            helm = rx(0.7 * e) * rz(0.4 * m);
            arm_l = rz(-0.9 * e) * rx(0.3 * e);
            // The door topples flat beside it.
            door = Some(rz(-1.45 * e) * ry(-0.3 * e));
            arm_r = rz(0.8 * e);
            legs = [(1.4 * e, -2.0 * e), (1.2 * e, -1.9 * e)];
        }
        EnemyState::Idle | EnemyState::Walk | EnemyState::Latched => {}
        // Telegraphs of other attacks fall back on the bash.
        EnemyState::Windup(_) | EnemyState::Attack(_) => {}
    }
    if !rim_on {
        pose.hide(RIM);
    }
    if !chain_on {
        pose.hide(CHAIN_GLOW);
    }

    pose.set(
        PELVIS,
        Mat4::from_scale_rotation_translation(
            Vec3::new(
                1.0 + 0.3 * (1.0 - squash),
                squash,
                1.0 + 0.3 * (1.0 - squash),
            ),
            pelvis_rot,
            pelvis_off,
        ),
    );
    pose.rotate(TORSO, torso);
    pose.rotate(HELM, helm);
    pose.rotate(ARM_L, arm_l);
    let door = door.unwrap_or_else(|| ry(door_yaw));
    pose.rotate(FOREARM_L, (pelvis_rot * torso * arm_l).inverse() * door);
    pose.rotate(ARM_R, arm_r);
    pose.rotate(FOREARM_R, fore_r);
    if let Some((spin, droop)) = whirl {
        let parent = pelvis_rot * torso * arm_r * fore_r;
        chain = chain_out(parent, spin, droop);
    }
    pose.rotate(CHAIN, chain);
    for (i, (thigh, shin)) in [(THIGH_L, SHIN_L), (THIGH_R, SHIN_R)]
        .into_iter()
        .enumerate()
    {
        pose.rotate(thigh, rx(legs[i].0));
        pose.rotate(shin, rx(legs[i].1));
    }
    pose
}
