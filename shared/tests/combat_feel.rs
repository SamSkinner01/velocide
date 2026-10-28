//! Feel (`docs/feel.md`): player projectiles test *targets* with their radius plus
//! `combat.projectile_hit_pad`, but the level with the plain radius; enemy projectiles never get
//! the pad.

mod combat_support;
mod weapon_support;

use combat_support::*;
use glam::Vec3;
use shared::protocol::NetId;
use shared::sim::projectiles::projectile_target_pad;
use shared::sim::{HitTarget, Owner, ProjectileKind, SimEvent};
use weapon_support::*;

/// A player's rivet flying along +Z at height 1 m, `lateral` metres to the side of a dummy
/// standing at the origin. Returns the damage the dummy took.
fn rivet_past_dummy(pad: f32, lateral: f32) -> f32 {
    let mut a = Arena::coop();
    a.tuning.combat.projectile_hit_pad = pad;
    let p = a.player(0, Vec3::new(20.0, 0.0, 20.0), 0.0);
    let target = dummy(&mut a, Vec3::ZERO, 0.0, 500.0);
    let speed = a.tuning.combat.riveter.rivet_speed;
    let id = a.world.spawn_projectile(
        &a.tuning,
        ProjectileKind::Rivet,
        Owner::Player(p),
        Vec3::new(lateral, 1.0, -8.0),
        Vec3::Z * speed,
    );
    let id = id.expect("the rivet spawns");
    // (Weapons set a rivet's damage when they fire it.)
    if let Some(r) = a.world.projectiles.get_mut(&id) {
        r.damage = 7.0;
    }
    let events = a.ticks(30);
    damage_to(&events, HitTarget::Entity(target))
}

/// How far from the dummy's axis a rivet still touches it without the pad: body radius + the
/// hit assist's body pad + the rivet's radius.
fn plain_reach() -> f32 {
    let t = shared::tuning::Tuning::default();
    DUMMY_RADIUS + t.combat.hit_assist.enemy_body_pad + t.combat.riveter.rivet_radius
}

#[test]
fn player_projectiles_hit_targets_with_the_pad() {
    let pad = shared::tuning::Tuning::default().combat.projectile_hit_pad;
    assert!(pad >= 0.2, "the shipped pad is {pad}");
    // A graze 0.1 m outside the plain reach: a hit with the pad, a miss without it.
    let graze = plain_reach() + 0.1;
    assert!(rivet_past_dummy(pad, graze) > 0.0, "a graze counts");
    assert_eq!(rivet_past_dummy(0.0, graze), 0.0, "no pad, no hit");
    // Beyond the padded reach it still misses.
    assert_eq!(rivet_past_dummy(pad, plain_reach() + pad + 0.1), 0.0);
}

#[test]
fn the_pad_is_for_targets_not_the_level() {
    // A wall whose face is at x = 0.5; a rivet flies along it 0.1 m clear of its plain radius,
    // well inside the padded radius.
    let wall = brush([0.5, 0.0, -6.0], [1.5, 3.0, 6.0]);
    let mut a = Arena::new(flat_level(shared::level::LevelKind::Campaign, vec![wall]));
    let p = a.player(0, Vec3::new(-20.0, 0.0, 20.0), 0.0);
    let t = &a.tuning.combat;
    let x = 0.5 - t.riveter.rivet_radius - 0.1;
    let speed = t.riveter.rivet_speed;
    let id = a
        .world
        .spawn_projectile(
            &a.tuning,
            ProjectileKind::Rivet,
            Owner::Player(p),
            Vec3::new(x, 1.5, -8.0),
            Vec3::Z * speed,
        )
        .expect("the rivet spawns");
    // Half way along the wall it is still flying.
    let events = a.ticks(6);
    let impacts = events
        .iter()
        .filter(|e| matches!(e, SimEvent::ProjectileImpact { .. }))
        .count();
    assert_eq!(impacts, 0, "{events:?}");
    let rivet = &a.world.projectiles[&id];
    assert!(
        rivet.position.z > 0.0,
        "passed the middle: {:?}",
        rivet.position
    );
}

#[test]
fn enemy_projectiles_get_no_pad() {
    let t = shared::tuning::Tuning::default();
    assert_eq!(
        projectile_target_pad(&t.combat, Owner::Enemy(NetId(7))),
        0.0
    );
    assert_eq!(projectile_target_pad(&t.combat, Owner::World), 0.0);
    assert_eq!(
        projectile_target_pad(&t.combat, Owner::Player(shared::sim::PlayerId(1))),
        t.combat.projectile_hit_pad
    );

    // A needle flying past a player 0.1 m outside its plain reach (player radius + needle
    // radius) never touches them, although the pad would have reached.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let shooter = dummy(&mut a, Vec3::new(30.0, 0.0, 30.0), 0.0, 500.0);
    let radius = a.tuning.movement.radius + a.tuning.combat.enemy_projectiles.needle_radius;
    let id = a.world.spawn_projectile(
        &a.tuning,
        ProjectileKind::Needle,
        Owner::Enemy(shooter),
        Vec3::new(radius + 0.1, 1.0, -6.0),
        Vec3::Z * 22.0,
    );
    assert!(id.is_some());
    a.ticks(90);
    assert_eq!(a.health(p), a.tuning.combat.max_health);
}
