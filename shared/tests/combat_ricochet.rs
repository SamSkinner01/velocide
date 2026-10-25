//! Ricochet prisms: throw, bounce off a prism to the nearest weak point, chains, PvP targets,
//! and hitting a flying prism as a lagging player saw it.

mod combat_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::LevelKind;
use shared::protocol::NetId;
use shared::sim::{Attack, DamageFlags, HitTarget, PlayerId, SimEvent};

fn setup() -> (Arena, PlayerId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    (a, p)
}

/// A prism resting in mid-air at `pos` (as if it had come to rest there).
fn prism_at(a: &mut Arena, owner: PlayerId, pos: Vec3) -> NetId {
    let t = a.tuning.clone();
    let id = a.world.spawn_prism(&t, owner, pos, Vec3::ZERO);
    let prism = a.world.prisms.get_mut(&id).unwrap();
    prism.position = pos;
    prism.resting = true;
    id
}

fn ricochet_paths(events: &[SimEvent]) -> Vec<Vec<Vec3>> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Ricochet { points, .. } => Some(points.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn shot_ricochets_off_a_prism_into_a_husks_head() {
    let (mut a, p) = setup();
    let husk = a.husk(Vec3::new(9.0, 0.0, -6.0), -PI / 2.0);
    let prism = prism_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    // Aim 0.4 m beside the prism: within the 0.6 m ricochet radius.
    a.aim(p, Vec3::new(0.4, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(!a.world.prisms.contains_key(&prism), "prism shattered");
    let paths = ricochet_paths(&ev);
    assert_eq!(paths.len(), 1);
    assert!((paths[0][0] - Vec3::new(0.0, 1.6, -6.0)).length() < 1e-3);
    // 22 x 1.5 (bounce) x 2 (head) = 66 > 40: dead, with the bounce on record.
    let d = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(d) if d.target == HitTarget::Entity(husk) => Some(*d),
            _ => None,
        })
        .expect("ricochet hit the husk");
    assert!((d.info.amount - 33.0).abs() < 1e-3, "{}", d.info.amount);
    assert!(
        d.info
            .flags
            .contains(DamageFlags::RICOCHET | DamageFlags::HEADSHOT)
    );
    assert_eq!(d.info.ricochets, 1);
    let k = kills(&ev);
    assert_eq!(k[0].ricochets, 1);
    // The tracer's first leg ends at the prism.
    let shot = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Shot(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap();
    assert!((shot.ends[0] - Vec3::new(0.0, 1.6, -6.0)).length() < 0.5);
}

#[test]
fn prisms_chain_for_bonus_damage() {
    let (mut a, p) = setup();
    let husk = a.husk(Vec3::new(10.0, 0.0, -20.0), 0.0);
    a.world.enemies.get_mut(&husk).unwrap().health = 500.0;
    prism_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    prism_at(&mut a, p, Vec3::new(8.0, 3.0, -8.0));
    a.ticks(5);
    a.aim(p, Vec3::new(0.0, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(a.world.prisms.is_empty(), "both prisms used");
    let paths = ricochet_paths(&ev);
    assert_eq!(paths[0].len(), 3, "prism, prism, target: {:?}", paths[0]);
    // 22 x 1.5^2 x 2 = 99.
    assert!((damage_to(&ev, HitTarget::Entity(husk)) - 99.0).abs() < 1e-3);
}

#[test]
fn pellets_do_not_ricochet_but_slugs_do() {
    let (mut a, p) = setup();
    let husk = a.husk(Vec3::new(9.0, 0.0, -6.0), -PI / 2.0);
    let prism = prism_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    a.select(p, 1, 0);
    a.aim(p, Vec3::new(0.0, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(ricochet_paths(&ev).is_empty());
    assert!(a.world.prisms.contains_key(&prism));
    a.ticks(60);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert_eq!(ricochet_paths(&ev).len(), 1);
    assert!(
        kills(&ev)
            .iter()
            .any(|k| k.victim == HitTarget::Entity(husk) && k.attack == Attack::Slug)
    );
}

#[test]
fn pvp_ricochet_targets_enemy_players() {
    let mut a = Arena::new(flat_level(LevelKind::Arena, Vec::new()));
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, -6.0), 0.0);
    prism_at(&mut a, p0, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    a.aim(p0, Vec3::new(0.0, 1.6, -6.0));
    let ev = a.tap(p0, Buttons::FIRE, 1);
    // Bounced into p1's head: 22 x 1.5 x 2.
    assert!((damage_to(&ev, HitTarget::Player(p1)) - 66.0).abs() < 1e-3);
}

#[test]
fn thrown_prism_is_hit_where_a_lagging_shooter_saw_it() {
    let (mut a, p) = setup();
    let husk = a.husk(Vec3::new(12.0, 0.0, -10.0), -PI / 2.0);
    a.ticks(5);
    let lag = 12.0;
    a.view_lag.insert(p, lag);
    let before = a.world.players[&p].combat.weapons.prisms(&a.tuning.combat);
    a.tap(p, Buttons::THROW, 0);
    assert_eq!(
        a.world.players[&p].combat.weapons.prisms(&a.tuning.combat),
        before - 1
    );
    let prism = *a.world.prisms.keys().next().expect("thrown");
    a.ticks(20);
    let flying = &a.world.prisms[&prism];
    assert!(!flying.resting, "still in the air");
    // Aim where the prism was 12 ticks ago (what the lagging client displays).
    let seen = flying.trail.at(a.world.tick as f64 + 1.0 - lag).unwrap();
    let now = flying.position;
    assert!(seen.distance(now) > 1.5, "the prism moved a lot since");
    a.aim(p, seen);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(ricochet_paths(&ev).len(), 1, "lag-compensated prism hit");
    assert!(damage_to(&ev, HitTarget::Entity(husk)) > 0.0);
}

#[test]
fn thrown_prisms_bounce_and_rest() {
    let (mut a, p) = setup();
    a.ticks(2);
    a.tap(p, Buttons::THROW, 240);
    // Expired after its 4 s lifetime.
    assert!(a.world.prisms.is_empty());
    a.tap(p, Buttons::THROW, 150);
    let prism = a.world.prisms.values().next().expect("prism");
    assert!(prism.resting, "came to rest");
    assert!(
        prism.position.y < 0.5 && prism.position.y > 0.0,
        "{}",
        prism.position
    );
}
