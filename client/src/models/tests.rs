//! Model-wide tests: every model builds valid meshes matching its part table, glTF round
//! trips keep the part names, pivots sit on their geometry, and every pose function produces
//! finite transforms.

use engine::gfx::{MeshId, TextureId};
use engine::glam::{Mat4, Vec2, Vec3, Vec4};

use super::enemies::{EnemyAnim, EnemyKind, EnemyState};
use super::kit::tests::assert_valid;
use super::library::match_parts;
use super::rig::{Pose, Rig, Tint, validate_joints};
use super::shrike::PlayerAnim;
use super::weapons::{
    self, Ability, DrumTurn, HeldWeapon, ViewmodelAnim, WeaponKind, WeaponLook, ability_color,
};
use super::{ModelId, generate_all, load_parts, props, shrike};

fn temp_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("velocide-models-{}-{name}", std::process::id()))
}

/// A rig with dummy GPU handles (pose / draw logic needs no renderer).
fn dummy_rig(id: ModelId) -> Rig {
    Rig::new(id.joints(), |i| (MeshId(i as u32), TextureId(0), Vec4::ONE))
}

fn assert_pose_finite(id: ModelId, pose: &Pose, what: &str) {
    assert_eq!(
        pose.locals.len(),
        id.joints().len(),
        "{id:?} {what}: pose size"
    );
    assert!(pose.is_finite(), "{id:?} {what}: non-finite pose");
    let rig = dummy_rig(id);
    let mut world = Vec::new();
    rig.world_transforms(
        pose,
        Mat4::from_translation(Vec3::new(3.0, 1.0, -2.0)),
        &mut world,
    );
    assert!(
        world.iter().all(|m| m.is_finite()),
        "{id:?} {what}: non-finite world transform"
    );
    let mut out = Vec::new();
    rig.draw(pose, Mat4::IDENTITY, &Tint::default(), &mut out);
    assert!(
        out.iter()
            .all(|d| d.transform.is_finite() && d.tint.is_finite()),
        "{id:?} {what}: bad draw item"
    );
}

#[test]
fn every_model_builds_valid_meshes() {
    for id in ModelId::ALL {
        let joints = id.joints();
        validate_joints(joints).unwrap_or_else(|e| panic!("{id:?}: {e}"));
        let parts = id.build();
        let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
        let expected: Vec<&str> = joints.iter().map(|j| j.name).collect();
        assert_eq!(names, expected, "{id:?}: part names / order");
        let mut triangles = 0;
        for part in &parts {
            assert_valid(&part.mesh, &format!("{id:?}/{}", part.name));
            triangles += part.mesh.indices.len() / 3;
            if let Some(t) = &part.texture {
                assert!(
                    t.width <= 64 && t.height <= 64,
                    "{id:?}/{}: texture too big",
                    part.name
                );
                assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize);
            }
            assert!(part.base_color.is_finite());
        }
        // Low-poly budget.
        assert!(triangles < 6000, "{id:?}: {triangles} triangles");
        match_parts(id, &parts).unwrap_or_else(|e| panic!("{e:#}"));
    }
}

#[test]
fn every_model_has_a_unique_file() {
    let mut names: Vec<&str> = ModelId::ALL.iter().map(|m| m.file_name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ModelId::ALL.len());
    for (i, id) in ModelId::ALL.iter().enumerate() {
        assert_eq!(id.index(), i, "ModelId::ALL must follow declaration order");
    }
}

#[test]
fn gltf_round_trip_keeps_parts() {
    let dir = temp_dir("roundtrip");
    generate_all(&dir).expect("generate");
    for id in ModelId::ALL {
        let built = id.build();
        let loaded = load_parts(id, &dir).unwrap_or_else(|e| panic!("{id:?}: {e:#}"));
        let names: Vec<&str> = loaded.iter().map(|p| p.name.as_str()).collect();
        let expected: Vec<&str> = id.joints().iter().map(|j| j.name).collect();
        assert_eq!(names, expected, "{id:?}: loaded part names");
        for (a, b) in loaded.iter().zip(&built) {
            assert_eq!(
                a.mesh.indices.len(),
                b.mesh.indices.len(),
                "{id:?}/{}",
                a.name
            );
            assert_eq!(
                a.mesh.vertices.len(),
                b.mesh.vertices.len(),
                "{id:?}/{}",
                a.name
            );
            assert_eq!(
                a.texture.is_some(),
                b.texture.is_some(),
                "{id:?}/{}",
                a.name
            );
            assert_eq!(a.base_color, b.base_color, "{id:?}/{}", a.name);
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn broken_file_is_rejected() {
    let dir = temp_dir("broken");
    std::fs::create_dir_all(&dir).unwrap();
    // A valid glTF of the wrong model: part names don't match.
    engine::assets::gltf::write_parts(
        &dir.join(ModelId::Gnawer.file_name()),
        &ModelId::Bomb.build(),
    )
    .unwrap();
    assert!(load_parts(ModelId::Gnawer, &dir).is_err());
    std::fs::write(dir.join(ModelId::Harrier.file_name()), "not json").unwrap();
    assert!(load_parts(ModelId::Harrier, &dir).is_err());
    assert!(load_parts(ModelId::Door, &dir).is_err(), "missing file");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn joint_pivots_sit_on_their_geometry() {
    // A part that rotates around a pivot far from its mesh would swing detached.
    for id in ModelId::ALL {
        let joints = id.joints();
        let parts = id.build();
        for (j, part) in joints.iter().zip(&parts) {
            let Some(parent) = j.parent else { continue };
            if joints[parent].pivot.distance(j.pivot) < 1e-4 {
                continue; // shares the parent's frame (decals, glow overlays)
            }
            let (lo, hi) = engine::assets::meshgen::bounds(&part.mesh.vertices);
            let gap = (lo - j.pivot).max(j.pivot - hi).max(Vec3::ZERO).length();
            assert!(
                gap < 0.06,
                "{id:?}/{}: pivot {} is {gap:.3} m from its mesh",
                j.name,
                j.pivot
            );
        }
    }
}

fn player_variants() -> Vec<(&'static str, PlayerAnim)> {
    let base = PlayerAnim {
        on_ground: true,
        time: 12345.678,
        ..Default::default()
    };
    let mut v = vec![
        ("idle", base),
        (
            "run",
            PlayerAnim {
                velocity: Vec3::new(3.0, 0.0, -12.0),
                cycle: 0.3,
                ..base
            },
        ),
        (
            "back",
            PlayerAnim {
                velocity: Vec3::new(0.0, 0.0, 9.0),
                cycle: 0.7,
                yaw: 0.4,
                ..base
            },
        ),
        (
            "strafe",
            PlayerAnim {
                velocity: Vec3::new(-11.0, 0.0, 0.0),
                cycle: 0.1,
                ..base
            },
        ),
        (
            "air",
            PlayerAnim {
                on_ground: false,
                velocity: Vec3::new(0.0, 8.0, 3.0),
                ..base
            },
        ),
        (
            "slide",
            PlayerAnim {
                sliding: true,
                velocity: Vec3::new(0.0, 0.0, -18.0),
                ..base
            },
        ),
        (
            "dash",
            PlayerAnim {
                dashing: true,
                velocity: Vec3::new(30.0, 0.0, 0.0),
                ..base
            },
        ),
        (
            "slam",
            PlayerAnim {
                slamming: true,
                on_ground: false,
                velocity: Vec3::new(0.0, -40.0, 0.0),
                ..base
            },
        ),
        (
            "wall",
            PlayerAnim {
                wall_cling: Some(Vec3::X),
                on_ground: false,
                ..base
            },
        ),
        (
            "wall_r",
            PlayerAnim {
                wall_cling: Some(Vec3::NEG_X),
                on_ground: false,
                ..base
            },
        ),
        (
            "punch",
            PlayerAnim {
                punching: 0.2,
                firing: 1.0,
                pitch: 1.5,
                ..base
            },
        ),
        (
            "aim_down",
            PlayerAnim {
                pitch: -1.5,
                weapon: 3,
                ..base
            },
        ),
        (
            "downed",
            PlayerAnim {
                downed: true,
                ..base
            },
        ),
        ("dead", PlayerAnim { dead: true, ..base }),
        (
            "bad_weapon",
            PlayerAnim {
                weapon: 200,
                ..base
            },
        ),
    ];
    for i in 0..16 {
        v.push((
            "cycle",
            PlayerAnim {
                velocity: Vec3::new(0.0, 0.0, -7.0),
                cycle: i as f32 / 16.0,
                ..base
            },
        ));
    }
    v
}

#[test]
fn player_poses_are_finite() {
    for (name, anim) in player_variants() {
        let pose = shrike::pose(&anim);
        assert_pose_finite(ModelId::Shrike, &pose, name);
        let w = weapons::weapon_kind(anim.weapon);
        assert_pose_finite(
            w.model(),
            &weapons::third_person_pose(w, anim.firing, anim.time),
            name,
        );
    }
}

#[test]
fn shrike_aims_its_weapon_along_the_look_direction() {
    // With no locomotion the gun barrel should point roughly along the aim (yaw + pitch).
    let rig = dummy_rig(ModelId::Shrike);
    for pitch in [-0.6f32, 0.0, 0.6] {
        for weapon in 0..WeaponKind::COUNT as u8 {
            let anim = PlayerAnim {
                on_ground: true,
                pitch,
                weapon,
                yaw: 0.7,
                ..Default::default()
            };
            let pose = shrike::pose(&anim);
            let root = shrike::root_transform(&anim, Vec3::ZERO);
            let gun = rig.part_transform(&pose, root, shrike::HAND_R) * shrike::grip_transform();
            let dir = gun.transform_vector3(Vec3::NEG_Z).normalize();
            let look =
                engine::gfx::Camera::rotation_from_angles(anim.yaw, pitch, 0.0) * Vec3::NEG_Z;
            assert!(
                dir.dot(look) > 0.85,
                "weapon {weapon} pitch {pitch}: barrel {dir} vs look {look}"
            );
        }
    }
}

#[test]
fn feet_stay_near_the_ground_when_standing_and_running() {
    let rig = dummy_rig(ModelId::Shrike);
    for (name, anim) in player_variants() {
        let special = anim.sliding || anim.dashing || anim.slamming || anim.wall_cling.is_some();
        if !anim.on_ground || anim.dead || anim.downed || special {
            continue;
        }
        let pose = shrike::pose(&anim);
        let lowest = [shrike::FOOT_L, shrike::FOOT_R]
            .iter()
            .map(|&f| {
                rig.part_transform(&pose, Mat4::IDENTITY, f)
                    .transform_point3(Vec3::ZERO)
                    .y
            })
            .fold(f32::MAX, f32::min);
        assert!(
            (-0.08..0.35).contains(&lowest),
            "{name}: lowest ankle at {lowest}"
        );
    }
}

#[test]
fn enemy_poses_are_finite() {
    let states = [
        EnemyState::Idle,
        EnemyState::Walk,
        EnemyState::Windup(0.0),
        EnemyState::Windup(0.5),
        EnemyState::Windup(1.0),
        EnemyState::Windup(7.0),
        EnemyState::Attack(0.0),
        EnemyState::Attack(0.3),
        EnemyState::Attack(1.0),
        EnemyState::Recover(0.5),
        EnemyState::Stunned,
        EnemyState::Dying(0.0),
        EnemyState::Dying(0.2),
        EnemyState::Dying(0.6),
        EnemyState::Dying(1.0),
        EnemyState::Dying(-1.0),
        EnemyState::Crouch(0.0),
        EnemyState::Crouch(0.7),
        EnemyState::Crouch(3.0),
        EnemyState::Airborne(0.0),
        EnemyState::Airborne(0.6),
        EnemyState::Melt(0.0),
        EnemyState::Melt(0.5),
        EnemyState::Melt(1.0),
        EnemyState::Reform(0.0),
        EnemyState::Reform(0.6),
        EnemyState::Reform(1.0),
    ];
    for kind in EnemyKind::ALL {
        for state in states {
            for attack in 0..4u8 {
                for (speed, strafe, time) in
                    [(0.0, 0.0, 0.0), (6.0, 1.0, 98765.4321), (30.0, -3.0, 1.5)]
                {
                    let anim = EnemyAnim {
                        state,
                        attack,
                        speed,
                        strafe,
                        time,
                        cycle: kind.advance_cycle(0.9, speed, 0.25),
                        aim_pitch: 0.4,
                        aim_locked: attack == 1,
                        seed: 99,
                        ..EnemyAnim::new(kind)
                    };
                    let pose = super::enemies::pose(&anim);
                    assert_pose_finite(kind.model(), &pose, &format!("{state:?} attack {attack}"));
                }
            }
        }
    }
}

/// The getting-there states (docs/enemies.md): a leap's crouch squashes every Mould down
/// without lighting its attack telegraph, the flight stretches it, a melt sinks it into its
/// puddle (flat at the bottom, as the server's sinking hit shapes), a re-form brings it back
/// up, and the puddle is gone at both ends of the move between them.
#[test]
fn getting_there_states_squash_sink_and_rise() {
    use super::enemies::{EnemyState as S, body_transform, melt_depth};
    use crate::ingame::combat::enemy_fx::puddle_radius;
    for kind in EnemyKind::ALL {
        // The top of the drawn model.
        let top = kind.height();
        let height = |a: &EnemyAnim| body_transform(a).transform_point3(Vec3::Y * top).y;
        let at = |state| EnemyAnim {
            state,
            ..EnemyAnim::new(kind)
        };
        assert!(height(&at(S::Crouch(1.0))) < top * 0.9, "{kind:?} crouches");
        assert!(
            height(&at(S::Airborne(0.0))) > top * 1.05,
            "{kind:?} stretches"
        );
        assert!(
            height(&at(S::Melt(1.0))) < 0.0,
            "{kind:?} sank into its puddle"
        );
        assert!(
            height(&at(S::Melt(0.4))) > top * 0.3,
            "{kind:?} still shows"
        );
        assert!(
            (height(&at(S::Reform(1.0))) - top).abs() < 1e-3,
            "{kind:?} stands"
        );
        assert!(melt_depth(S::Melt(0.4)) < melt_depth(S::Melt(0.7)));
        assert!(melt_depth(S::Reform(0.4)) > melt_depth(S::Reform(0.7)));
        // A crouch is no telegraph: it glows like walking.
        let calm = super::enemies::pose(&at(S::Walk)).glow;
        let crouch = super::enemies::pose(&at(S::Crouch(0.9))).glow;
        assert!((calm - crouch).abs().max_element() < 0.2, "{kind:?}");
    }
    for melt in [true, false] {
        assert_eq!(puddle_radius(melt, 0.0, 1.0), 0.0);
        assert!(puddle_radius(melt, 1.0, 1.0) < 1e-3);
        assert!(puddle_radius(melt, 0.5, 1.0) > 1.0);
    }
}

/// The models are built at the contract sizes (docs/roster.md) and drawn `model_scale` bigger
/// (docs/enemies.md "Sizes and hit shapes"): as drawn, roughly as tall as their hit shapes,
/// the Harrier's wingspan, the weak points inside visible geometry, and the hit shapes cover
/// the drawn model (with the hit assist's pads, nearly all of it).
#[test]
fn enemy_models_match_their_definitions() {
    use shared::sim::{HitPads, HitTarget};
    let defs = shared::sim::EnemyRegistry::builtin();
    let assist = shared::tuning::HitAssistTuning::default();
    let pads = HitPads {
        body: assist.enemy_body_pad,
        weak: assist.enemy_weak_pad,
    };
    for kind in EnemyKind::ALL {
        let name = format!("{kind:?}").to_ascii_lowercase();
        let def = defs
            .get(&name)
            .unwrap_or_else(|| panic!("no definition for {name}"));
        assert_eq!(EnemyKind::from_name(&name), Some(kind));
        assert_eq!(kind.scale(), def.draw_scale(), "{kind:?}");
        // Model space as drawn: scaled about the origin (the feet; the Harrier: its hover
        // centre), then lifted onto the feet.
        let (scale, lift) = (kind.scale(), kind.root_lift());
        let verts: Vec<Vec3> = kind
            .model()
            .build()
            .iter()
            .flat_map(|p| p.mesh.vertices.iter())
            .map(|v| Vec3::from(v.position) * scale + Vec3::Y * lift)
            .collect();
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &verts {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        let size = hi - lo;
        let hitbox = def.hitbox(
            HitTarget::Entity(shared::protocol::NetId(1)),
            Vec3::ZERO,
            0.0,
        );
        let shapes_top = hitbox
            .shapes
            .iter()
            .map(|s| s.a.y.max(s.b.y) + s.radius)
            .fold(f32::NEG_INFINITY, f32::max);
        let covered = |pads: HitPads| {
            let inside = verts
                .iter()
                .filter(|v| {
                    hitbox
                        .distance_to_point_padded(**v, pads)
                        .is_some_and(|d| d.0 <= 0.0)
                })
                .count();
            inside as f32 / verts.len().max(1) as f32
        };
        let (plain, padded) = (covered(HitPads::NONE), covered(pads));
        println!(
            "{kind:?} x{scale}: {size} (feet {:.2} .. top {:.2}), hit shapes to {shapes_top:.2}, \
             covered {plain:.2} / padded {padded:.2}, capsule {} x {}",
            lo.y, hi.y, def.radius, def.height
        );
        assert!(
            size.max_element() >= 2.3,
            "{kind:?}: drawn at least ~2.3 m in its longest dimension"
        );
        if kind == EnemyKind::Harrier {
            assert!(size.x >= 3.5, "a 3.9 m wingspan as drawn: {}", size.x);
        } else {
            assert!(lo.y.abs() < 0.12, "{kind:?} stands on its feet: {}", lo.y);
            assert!(
                (hi.y - shapes_top).abs() < 0.45,
                "{kind:?}: drawn top {} vs hit shapes {shapes_top}",
                hi.y
            );
        }
        for w in &def.weak_points {
            assert!(
                w.offset.cmpge(lo - Vec3::splat(w.radius * 0.5)).all()
                    && w.offset.cmple(hi + Vec3::splat(w.radius * 0.5)).all(),
                "{kind:?} weak point {} at {} outside the drawn model {lo}..{hi}",
                w.name,
                w.offset
            );
        }
        // Hit shapes larger than the model: most of it inside the plain shapes, nearly all of
        // it inside the padded ones player attacks test.
        assert!(plain >= 0.55, "{kind:?}: plain hit shapes cover {plain:.2}");
        assert!(
            padded >= 0.9,
            "{kind:?}: padded hit shapes cover {padded:.2}"
        );
    }
}

#[test]
fn telegraphs_brighten_during_windup() {
    for kind in EnemyKind::ALL {
        let at = |state| {
            let anim = EnemyAnim {
                state,
                time: 3.0,
                ..EnemyAnim::new(kind)
            };
            let g = super::enemies::pose(&anim).glow;
            g.x + g.y + g.z
        };
        let idle = at(EnemyState::Idle);
        let windup = at(EnemyState::Windup(0.6));
        assert!(
            windup > idle * 2.0,
            "{kind:?}: wind-up glow {windup} vs idle {idle}"
        );
        assert!(
            at(EnemyState::Dying(1.0)) < idle,
            "{kind:?}: dead enemies go dark"
        );
    }
}

#[test]
fn gaoler_strobes_in_the_bash_parry_window() {
    let glow = |p: f32, t: f64| {
        let anim = EnemyAnim {
            state: EnemyState::Windup(p),
            attack: super::enemies::gaoler::BASH,
            time: t,
            ..EnemyAnim::new(EnemyKind::Gaoler)
        };
        super::enemies::pose(&anim).glow
    };
    // Within the window the colour alternates between yellow and white-hot over time.
    let samples: Vec<Vec4> = (0..8).map(|i| glow(0.9, i as f64 / 72.0)).collect();
    assert!(samples.iter().any(|g| g.z > 3.0), "white-hot phase");
    assert!(samples.iter().any(|g| g.z < 3.0), "yellow phase");
    // Before the window there is no white-hot phase.
    assert!((0..8).all(|i| glow(0.3, i as f64 / 72.0).z < 3.0));
    // The bash lights the door's rim, the sweep the chain (the other one is hidden).
    use super::enemies::gaoler::{CHAIN_GLOW, RIM, SWEEP};
    let pose = |attack| {
        super::enemies::pose(&EnemyAnim {
            state: EnemyState::Windup(0.5),
            attack,
            ..EnemyAnim::new(EnemyKind::Gaoler)
        })
    };
    let bash = pose(super::enemies::gaoler::BASH);
    assert!(!bash.is_hidden(RIM) && bash.is_hidden(CHAIN_GLOW));
    let sweep = pose(SWEEP);
    assert!(sweep.is_hidden(RIM) && !sweep.is_hidden(CHAIN_GLOW));
}

#[test]
fn heron_lens_flares_once_the_aim_locks() {
    let glow = |locked: bool| {
        let anim = EnemyAnim {
            state: EnemyState::Windup(0.9),
            attack: super::enemies::heron::NEEDLE,
            aim_locked: locked,
            time: 2.0,
            ..EnemyAnim::new(EnemyKind::Heron)
        };
        super::enemies::pose(&anim).glow.x
    };
    assert!(
        glow(true) > glow(false) * 1.2,
        "{} vs {}",
        glow(true),
        glow(false)
    );
}

#[test]
fn viewmodels_are_finite_and_framed() {
    let base = ViewmodelAnim {
        switch: 1.0,
        time: 4321.0,
        ..Default::default()
    };
    for weapon in WeaponKind::ALL {
        let ability = weapon.default_build().abilities[1];
        for anim in [
            ViewmodelAnim { weapon, ..base },
            ViewmodelAnim {
                weapon,
                ability,
                fire_kick: 1.0,
                alt_charge: 1.0,
                draw: 1.0,
                salvo: Some(0.09),
                spray: 1.0,
                ..base
            },
            ViewmodelAnim {
                weapon,
                punch: 0.25,
                bob_amount: 1.0,
                bob_phase: 2.0,
                sway: Vec2::new(9.0, -9.0),
                draw: 0.5,
                ..base
            },
            ViewmodelAnim {
                weapon,
                throw: 0.4,
                spin: 1e4,
                salvo: Some(f32::INFINITY),
                draw: f32::NAN,
                ..base
            },
            ViewmodelAnim {
                weapon,
                switch: 0.0,
                ..base
            },
        ] {
            let gun = weapons::viewmodel_transform(&anim);
            assert!(gun.is_finite(), "{weapon:?}");
            let pose = weapons::weapon_pose(&anim.held());
            assert_pose_finite(weapon.model(), &pose, "viewmodel");
        }
        // At rest: grip on the lower right, muzzle in front of the camera and inside the
        // 60 degree viewmodel frustum.
        let gun = weapons::viewmodel_transform(&ViewmodelAnim { weapon, ..base });
        let grip = gun.transform_point3(Vec3::ZERO);
        let muzzle = gun.transform_point3(weapon.muzzle());
        // (The bow is held on the lower left: `the_bow_never_covers_the_centre`.)
        let side = if weapon == WeaponKind::Longwick {
            -grip.x
        } else {
            grip.x
        };
        assert!(
            side > 0.05 && grip.y < -0.05 && grip.z < -0.1,
            "{weapon:?}: grip {grip}"
        );
        assert!(muzzle.z < grip.z, "{weapon:?}: muzzle behind grip");
        let tan = (30f32).to_radians().tan();
        assert!(
            muzzle.y / -muzzle.z > -tan && muzzle.x / -muzzle.z < tan * 16.0 / 9.0,
            "{weapon:?}: muzzle off screen {muzzle}"
        );
        // Just switched: lowered out of view.
        let lowered = weapons::viewmodel_transform(&ViewmodelAnim {
            weapon,
            switch: 0.0,
            ..base
        });
        assert!(lowered.transform_point3(weapon.muzzle()).y < muzzle.y - 0.1);
    }
}

#[test]
fn the_bow_never_covers_the_centre() {
    // Every point of the riser, the flexed limbs, the strings and the nocked arrow, at every
    // draw (and through the release), projected into the 62 degree viewmodel frustum, stays
    // out of a box around the crosshair (about 70 px either way at 720p); at full draw the
    // arrow points at the crosshair from below.
    let tan = (31f32).to_radians().tan();
    let half = 0.12;
    for step in 0..=10 {
        for kick in [0.0, 0.5, 1.0] {
            let draw = step as f32 / 10.0;
            let anim = ViewmodelAnim {
                weapon: WeaponKind::Longwick,
                switch: 1.0,
                draw,
                fire_kick: kick,
                time: 12.5,
                ..Default::default()
            };
            let gun = weapons::viewmodel_transform(&anim);
            for p in weapons::longwick::outline_points(draw, kick, anim.time) {
                let v = gun.transform_point3(p);
                assert!(v.z < -0.05, "behind the camera: {v} (draw {draw})");
                let s = Vec2::new(v.x, v.y) / -v.z;
                assert!(
                    s.x.abs() > half || s.y.abs() > half,
                    "draw {draw} kick {kick}: {p} covers the crosshair at {s}"
                );
                assert!(
                    s.y < tan * 0.8,
                    "draw {draw}: {p} near the top of the screen"
                );
            }
        }
    }
    let full = ViewmodelAnim {
        weapon: WeaponKind::Longwick,
        switch: 1.0,
        draw: 1.0,
        time: 12.5,
        ..Default::default()
    };
    let gun = weapons::viewmodel_transform(&full);
    let nock = gun.transform_point3(weapons::longwick::nock_position(1.0, 0.0, 12.5));
    let bow_head = weapons::longwick::MUZZLE + Vec3::Z * weapons::longwick::DRAW_LENGTH;
    let head = gun.transform_point3(bow_head);
    let (n, h) = (nock.truncate() / -nock.z, head.truncate() / -head.z);
    assert!(
        h.y < 0.0 && h.y > n.y,
        "the arrow rises towards the crosshair: {n} -> {h}"
    );
    // Extended, the arrow's line passes within a few pixels of the crosshair.
    let along = (h - n).normalize();
    let miss = (Vec2::ZERO - n).perp_dot(along).abs();
    assert!(miss < 0.06, "the arrow points {miss} off the crosshair");
}

#[test]
fn drum_turns_one_chamber_per_shot_with_overshoot() {
    let mut drum = DrumTurn::default();
    drum.kick();
    let mut peak = 0.0f32;
    for _ in 0..60 {
        drum.update(1.0 / 60.0);
        peak = peak.max(drum.angle());
    }
    assert!(
        (drum.angle() - DrumTurn::STEP).abs() < 1e-3,
        "{}",
        drum.angle()
    );
    assert!(peak > DrumTurn::STEP * 1.02, "a little overshoot: {peak}");
    // Many shots keep the numbers bounded and every rest angle a whole chamber.
    for _ in 0..500 {
        drum.kick();
        drum.update(0.5);
    }
    let chambers = drum.angle() / DrumTurn::STEP;
    assert!(drum.angle().is_finite() && drum.angle().abs() < 200.0);
    assert!((chambers - chambers.round()).abs() < 1e-2, "{chambers}");
    // Huge or zero time steps are harmless.
    drum.update(f32::MAX);
    drum.update(0.0);
    assert!(drum.angle().is_finite());
}

fn held(weapon: WeaponKind) -> HeldWeapon {
    HeldWeapon {
        weapon,
        ability: weapon.default_build().abilities[0],
        time: 1.0,
        ..HeldWeapon::default()
    }
}

#[test]
fn weapon_mechanisms_move() {
    use super::weapons::{
        arclash, bellows, cinderbox, kiln, lancet, longwick, quill, riveter, thresher, tumbler,
    };
    let moved = |pose: &Pose, part: usize| !pose.locals[part].abs_diff_eq(Mat4::IDENTITY, 1e-5);
    let pose = |h: HeldWeapon| weapons::weapon_pose(&h);
    // Quill: the crest folds front to back on the shot and stands at rest.
    let rest = pose(held(WeaponKind::Quill));
    let kick = pose(HeldWeapon {
        kick: 1.0,
        ..held(WeaponKind::Quill)
    });
    for i in 0..quill::CREST_COUNT {
        assert!(
            !moved(&rest, quill::QUILL + i) && moved(&kick, quill::QUILL + i),
            "quill {i}"
        );
    }
    // Bellows: the pump closes the board half way through the recoil, open at rest.
    let pump = pose(HeldWeapon {
        kick: 0.5,
        ..held(WeaponKind::Bellows)
    });
    let open = pose(held(WeaponKind::Bellows));
    assert!(moved(&pump, bellows::BOARD) && moved(&pump, bellows::PLEATS));
    assert!(!moved(&open, bellows::BOARD));
    // Arclash: arcs only while charging (or on a strong shot); the glow whitens with charge.
    let discharge = HeldWeapon {
        ability: Ability::Discharge,
        ..held(WeaponKind::Arclash)
    };
    let calm = pose(discharge);
    assert!(calm.is_hidden(arclash::ARCS));
    let arcing = (0..8)
        .map(|i| {
            pose(HeldWeapon {
                charge: 1.0,
                time: i as f64 / 50.0,
                ..discharge
            })
        })
        .filter(|p| !p.is_hidden(arclash::ARCS))
        .count();
    assert!(arcing > 0);
    let full = pose(HeldWeapon {
        charge: 1.0,
        ..discharge
    });
    assert!(full.glow.x > calm.glow.x * 2.0);
    // Tumbler and Thresher: the drum follows the spin angle.
    let turned = HeldWeapon {
        spin: DrumTurn::STEP,
        ..HeldWeapon::default()
    };
    assert!(moved(
        &pose(HeldWeapon {
            weapon: WeaponKind::Tumbler,
            ..turned
        }),
        tumbler::DRUM
    ));
    assert!(moved(
        &pose(HeldWeapon {
            weapon: WeaponKind::Thresher,
            ..turned
        }),
        thresher::DRUM
    ));
    // Lancet: the feed sleeve snaps back on the shot.
    assert!(!moved(&pose(held(WeaponKind::Lancet)), lancet::SLIDE));
    assert!(moved(
        &pose(HeldWeapon {
            kick: 0.8,
            ..held(WeaponKind::Lancet)
        }),
        lancet::SLIDE
    ));
    // Riveter: the nose hammers back and a pulse runs down the hose, tank to grip.
    let riveter = |kick| {
        pose(HeldWeapon {
            kick,
            ..held(WeaponKind::Riveter)
        })
    };
    assert!(moved(&riveter(1.0), riveter::NOSE));
    let bulge = |p: &Pose, i: usize| p.locals[riveter::HOSE + i].x_axis.length();
    let early = riveter(0.95);
    let late = riveter(0.8);
    assert!(
        bulge(&early, 3) > bulge(&early, 0) + 0.05,
        "starts at the tank"
    );
    assert!(
        bulge(&late, 0) > bulge(&late, 3) + 0.05,
        "arrives at the tool"
    );
    assert!(riveter::pulse_position(0.65) < 0.0 && riveter::pulse_position(1.0) >= 4.0);
    // Cinderbox: the tubes flare in turn, one kick per rocket; nothing without a salvo.
    let salvo = |s: Option<f32>| {
        pose(HeldWeapon {
            salvo: s,
            ..held(WeaponKind::Cinderbox)
        })
    };
    let idle = salvo(None);
    assert!((0..cinderbox::TUBES).all(|i| idle.is_hidden(cinderbox::FLARE + i)));
    for i in 0..cinderbox::TUBES {
        let p = salvo(Some(i as f32 * cinderbox::SALVO_GAP + 0.01));
        assert!(!p.is_hidden(cinderbox::FLARE + i), "tube {i} flares");
        assert!(cinderbox::salvo_kick(Some(i as f32 * cinderbox::SALVO_GAP)) > 0.9);
    }
    assert!(cinderbox::salvo_kick(None) == 0.0 && cinderbox::salvo_kick(Some(0.9)) < 0.01);
    // Longwick: drawing pulls the arrow back and flexes the limbs; the release hides the
    // arrow; full draw glows hotter.
    let bow = |draw: f32, kick: f32| {
        pose(HeldWeapon {
            draw,
            kick,
            ..held(WeaponKind::Longwick)
        })
    };
    let (slack, drawn) = (bow(0.0, 0.0), bow(1.0, 0.0));
    assert!(!moved(&slack, longwick::LIMB_U) && moved(&drawn, longwick::LIMB_U));
    let arrow_z = |p: &Pose| p.locals[longwick::ARROW].w_axis.z;
    assert!(arrow_z(&drawn) > arrow_z(&slack) + longwick::DRAW_LENGTH * 0.9);
    assert!(bow(0.0, 1.0).is_hidden(longwick::ARROW) && !slack.is_hidden(longwick::ARROW));
    assert!(drawn.glow.x > slack.glow.x * 1.8 && drawn.core.x > slack.core.x);
    // The strings reach the drawn nock from the flexed tips (their meshes end at the rest
    // nock, `MUZZLE.y` high, in joint space relative to the tip).
    let rig = dummy_rig(ModelId::Longwick);
    let nock = longwick::nock_position(1.0, 0.0, 1.0);
    for (part, tip) in [
        (longwick::STRING_U, longwick::TIP_U),
        (longwick::STRING_L, longwick::TIP_L),
    ] {
        let w = rig.part_transform(&drawn, Mat4::IDENTITY, part);
        let rest_nock = longwick::nock_position(0.0, 0.0, 1.0);
        let end = w.transform_point3(rest_nock - Vec3::from(tip));
        assert!(
            end.distance(nock) < 0.01,
            "string {part} ends at {end}, nock {nock}"
        );
    }
    // Kiln: spraying rattles the damper and stokes the coals.
    let kiln_pose = |spray: f32, time: f64| {
        pose(HeldWeapon {
            spray,
            time,
            ..held(WeaponKind::Kiln)
        })
    };
    let roaring = (0..10).any(|i| moved(&kiln_pose(1.0, 0.3 + i as f64 * 0.013), kiln::DAMPER));
    assert!(roaring);
    assert!(kiln_pose(1.0, 1.0).core.x > kiln_pose(0.0, 1.0).core.x + 0.5);
    // Ability colours: every ability has its own, and they drive the glow.
    let mut colours: Vec<[u32; 3]> = Ability::ALL
        .iter()
        .map(|a| ability_color(*a))
        .map(|c| [c.x, c.y, c.z].map(|v| (v * 100.0) as u32))
        .collect();
    colours.sort_unstable();
    colours.dedup();
    assert_eq!(colours.len(), Ability::COUNT);
    let a = pose(HeldWeapon {
        ability: Ability::Bolas,
        ..held(WeaponKind::Thresher)
    });
    let b = pose(HeldWeapon {
        ability: Ability::Flak,
        ..held(WeaponKind::Thresher)
    });
    assert!(a.glow.distance(b.glow) > 0.2);
}

#[test]
fn shrike_wick_and_coat() {
    let base = PlayerAnim {
        on_ground: true,
        time: 7.5,
        ..Default::default()
    };
    // The wick burns (and flickers) while alive, is snuffed on death and gutters downed.
    let alive = shrike::pose(&base);
    assert!(!alive.is_hidden(shrike::WICK) && alive.glow.x > 0.5);
    let dead = shrike::pose(&PlayerAnim { dead: true, ..base });
    assert!(dead.is_hidden(shrike::WICK) && dead.glow.x < 0.1);
    let downed = shrike::pose(&PlayerAnim {
        downed: true,
        ..base
    });
    assert!(!downed.is_hidden(shrike::WICK) && downed.glow.x < alive.glow.x);
    let glows: Vec<f32> = (0..10)
        .map(|i| {
            shrike::pose(&PlayerAnim {
                time: i as f64 * 0.037,
                ..base
            })
            .glow
            .x
        })
        .collect();
    assert!(glows.iter().any(|g| (g - glows[0]).abs() > 0.02), "flicker");
    // Running streams the coat tails back: their hems end up behind the knees.
    let rig = dummy_rig(ModelId::Shrike);
    let hem = |anim: &PlayerAnim| {
        let pose = shrike::pose(anim);
        rig.part_transform(&pose, Mat4::IDENTITY, shrike::COAT_R)
            .transform_point3(Vec3::new(0.03, -0.62, 0.035))
    };
    let still = hem(&base);
    let running = hem(&PlayerAnim {
        velocity: Vec3::new(0.0, 0.0, -14.0),
        cycle: 0.3,
        ..base
    });
    assert!(
        running.z > still.z + 0.15,
        "still {still}, running {running}"
    );
}

#[test]
fn prop_poses_are_finite() {
    for t in [0.0, 1.25, 1e6] {
        assert_pose_finite(ModelId::Door, &props::door_pose(0.5, false), "door");
        assert_pose_finite(ModelId::Door, &props::door_pose(2.0, true), "door clamp");
        assert_pose_finite(
            ModelId::Checkpoint,
            &props::checkpoint_pose(true, t),
            "checkpoint",
        );
        assert_pose_finite(ModelId::JumpPad, &props::jump_pad_pose(t), "jump pad");
        assert_pose_finite(ModelId::ExitGate, &props::exit_gate_pose(t), "exit gate");
    }
}

#[test]
fn lookups() {
    assert_eq!(weapons::weapon_kind(2), WeaponKind::Arclash);
    assert_eq!(weapons::weapon_kind(9), WeaponKind::Kiln);
    assert_eq!(weapons::weapon_kind(200), WeaponKind::Quill);
    assert!(!WeaponKind::Quill.two_handed() && WeaponKind::Tumbler.two_handed());
    assert!(!WeaponKind::Lancet.two_handed() && WeaponKind::Longwick.two_handed());
    assert!(WeaponKind::Lamplighter.two_handed());
    // Every weapon has its own model, and the default builds' glows are the ability colours.
    let mut models: Vec<ModelId> = WeaponKind::ALL.iter().map(|w| w.model()).collect();
    models.sort_unstable();
    models.dedup();
    assert_eq!(models.len(), WeaponKind::COUNT);
    assert_eq!(WeaponKind::Lamplighter.model(), ModelId::Lamplighter);
    assert_eq!(WeaponKind::Trimmer.model(), ModelId::Trimmer);
    assert_eq!(WeaponKind::Tamper.model(), ModelId::Tamper);
    assert_eq!(
        WeaponKind::Tumbler.glow_color(1),
        ability_color(Ability::Gravewell)
    );
    assert_eq!(EnemyKind::from_name("Gaoler"), Some(EnemyKind::Gaoler));
    assert_eq!(EnemyKind::from_name("nope"), None);
    let c = PlayerAnim::advance_cycle(0.95, Vec3::new(0.0, 0.0, -100.0), 0.1);
    assert!((0.0..1.0).contains(&c));
}
