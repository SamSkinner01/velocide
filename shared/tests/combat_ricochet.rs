//! Thorns and ricochets: throwing and planting, hitting targets (drop and lie), lifetime,
//! Quill shots bouncing off thorns to weak points, chains, branded targets first, PvP targets,
//! which modes ricochet, and hitting a flying thorn as a lagging player saw it.

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::LevelKind;
use shared::protocol::{NetFrame, NetId, thorn_flags};
use shared::sim::projectiles::replicated_position;
use shared::sim::{Attack, DamageFlags, HitTarget, PlayerId, SimEvent, ThornState};
use weapon_support::*;

fn setup() -> (Arena, PlayerId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    (a, p)
}

/// A thorn planted in mid-air at `pos` (as if stuck in something there).
fn thorn_at(a: &mut Arena, owner: PlayerId, pos: Vec3) -> NetId {
    let t = a.tuning.clone();
    let id = a.world.spawn_thorn(&t, owner, pos, Vec3::ZERO).expect("thorn");
    let thorn = a.world.thorns.get_mut(&id).unwrap();
    thorn.position = pos;
    thorn.state = ThornState::Planted;
    thorn.expire_tick += 6000;
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

fn thorn_entity(a: &Arena, id: NetId) -> shared::protocol::EntityState {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).expect("replicated")
}

#[test]
fn quill_ricochets_off_a_thorn_into_a_weak_point() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, 70.0);
    let thorn = thorn_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    // Aim 0.4 m beside the thorn: within the 0.6 m ricochet radius.
    a.aim(p, Vec3::new(0.4, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(!a.world.thorns.contains_key(&thorn), "thorn shattered");
    assert!(ev.iter().any(|e| matches!(e, SimEvent::ThornShattered { id, .. } if *id == thorn)));
    let paths = ricochet_paths(&ev);
    assert_eq!(paths.len(), 1);
    assert!((paths[0][0] - Vec3::new(0.0, 1.6, -6.0)).length() < 1e-3);
    // 24 x 1.5 (bounce) x 2 (head) = 72 > 70: dead, with the bounce on record.
    let hit = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(dm) if dm.target == HitTarget::Entity(d) => Some(*dm),
            _ => None,
        })
        .expect("ricochet hit the dummy");
    assert!((hit.info.amount - 36.0).abs() < 1e-3, "{}", hit.info.amount);
    assert!(hit.info.flags.contains(DamageFlags::RICOCHET | DamageFlags::HEADSHOT));
    assert_eq!(hit.info.ricochets, 1);
    assert_eq!(kills(&ev)[0].ricochets, 1);
    // The tracer's first leg ends at the thorn.
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
fn thorns_chain_for_bonus_damage() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(10.0, 0.0, -20.0), 0.0, 1000.0);
    thorn_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    thorn_at(&mut a, p, Vec3::new(8.0, 3.0, -8.0));
    a.ticks(5);
    a.aim(p, Vec3::new(0.0, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(a.world.thorns.is_empty(), "both thorns used");
    let paths = ricochet_paths(&ev);
    assert_eq!(paths[0].len(), 3, "thorn, thorn, target: {:?}", paths[0]);
    // 24 x 1.5^2 x 2 = 108.
    assert!((damage_to(&ev, HitTarget::Entity(d)) - 108.0).abs() < 1e-3);
}

#[test]
fn ricochets_pick_branded_targets_first() {
    let (mut a, p) = setup();
    let near = dummy(&mut a, Vec3::new(5.0, 0.0, -6.0), 0.0, 1000.0);
    let far = dummy(&mut a, Vec3::new(-14.0, 0.0, -6.0), 0.0, 1000.0);
    thorn_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    let t = a.tuning.clone();
    let at = dummy_body(&a, far);
    a.world.brand_target(&t, p, HitTarget::Entity(far), at);
    a.aim(p, Vec3::new(0.0, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(damage_to(&ev, HitTarget::Entity(near)), 0.0);
    // 24 x 1.5 x 2 (head) x 1.35 (brand).
    assert!((damage_to(&ev, HitTarget::Entity(far)) - 97.2).abs() < 1e-3);
}

#[test]
fn brand_and_splinter_ricochet_but_blast_and_arc_do_not() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, 100_000.0);
    let behind = dummy(&mut a, Vec3::new(10.6, 0.0, -6.0), -PI / 2.0, 100_000.0);
    let thorn = thorn_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    a.aim(p, Vec3::new(0.0, 1.6, -6.0));
    // Bellows pellets and Arclash zaps pass the thorn by.
    for weapon in [1, 2] {
        a.select(p, weapon, 0);
        let ev = a.tap(p, Buttons::FIRE, 1);
        assert!(ricochet_paths(&ev).is_empty());
        assert!(a.world.thorns.contains_key(&thorn));
    }
    // A Brand bounces and brands what it lands on.
    a.select(p, 0, 0);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert_eq!(ricochet_paths(&ev).len(), 1);
    assert!(a.world.is_branded(HitTarget::Entity(d)));
    // A Splinter (after the Quill's alt cooldown) bounces, then bursts from the target into
    // the one behind it.
    a.ticks(150);
    thorn_at(&mut a, p, Vec3::new(0.0, 1.6, -6.0));
    a.select(p, 0, 1);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert_eq!(ricochet_paths(&ev).len(), 1);
    assert!(ev.iter().any(|e| matches!(e, SimEvent::Splinters { .. })));
    let main = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(dm) if dm.target == HitTarget::Entity(d) => Some(*dm),
            _ => None,
        })
        .unwrap();
    assert_eq!(main.info.attack, Attack::Splinter);
    assert!(main.info.flags.contains(DamageFlags::RICOCHET | DamageFlags::BRANDED));
    assert!(damage_to(&ev, HitTarget::Entity(behind)) > 0.0, "splinters continue past it");
}

#[test]
fn pvp_ricochet_targets_enemy_players() {
    let mut a = Arena::new(flat_level(LevelKind::Arena, Vec::new()));
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(10.0, 0.0, -6.0), 0.0);
    thorn_at(&mut a, p0, Vec3::new(0.0, 1.6, -6.0));
    a.ticks(5);
    a.aim(p0, Vec3::new(0.0, 1.6, -6.0));
    let ev = a.tap(p0, Buttons::FIRE, 1);
    // Bounced into p1's head: 24 x 1.5 x 2.
    assert!((damage_to(&ev, HitTarget::Player(p1)) - 72.0).abs() < 1e-3);
}

#[test]
fn flying_thorn_is_hit_where_a_lagging_shooter_saw_it() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(12.0, 0.0, -10.0), -PI / 2.0, 1000.0);
    a.ticks(5);
    let lag = 12.0;
    a.view_lag.insert(p, lag);
    let before = a.world.players[&p].combat.weapons.thorns(&a.tuning.combat);
    a.held.get_mut(&p).unwrap().pitch = 0.5;
    a.tap(p, Buttons::THROW, 0);
    assert_eq!(
        a.world.players[&p].combat.weapons.thorns(&a.tuning.combat),
        before - 1
    );
    let thorn = *a.world.thorns.keys().next().expect("thrown");
    a.ticks(20);
    let flying = &a.world.thorns[&thorn];
    assert_eq!(flying.state, ThornState::Flying, "still in the air");
    // Aim where the thorn was 12 ticks ago (what the lagging client displays).
    let seen = flying.trail.at(a.world.tick as f64 + 1.0 - lag).unwrap();
    let now = flying.position;
    assert!(seen.distance(now) > 5.0, "the thorn moved a lot since");
    a.aim(p, seen);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(ricochet_paths(&ev).len(), 1, "lag-compensated thorn hit");
    assert!(damage_to(&ev, HitTarget::Entity(d)) > 0.0);
}

#[test]
fn thrown_thorns_plant_in_walls_and_expire() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-5.0, 0.0, -9.0], [5.0, 5.0, -8.0])],
    ));
    let p = a.player(0, Vec3::new(0.0, 0.0, 0.0), 0.0);
    a.ticks(2);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    let ev = a.tap(p, Buttons::THROW, 0);
    assert!(ev.iter().any(|e| matches!(e, SimEvent::ThornThrown { player, .. } if *player == p)));
    let mut planted = None;
    for _ in 0..30 {
        for e in a.tick() {
            if let SimEvent::ThornPlanted { id, position } = e {
                planted = Some((id, position));
            }
        }
        if planted.is_some() {
            break;
        }
    }
    let planted = planted.expect("planted");
    let th = &a.world.thorns[&planted.0];
    assert_eq!(th.state, ThornState::Planted);
    assert!((th.position.z + 8.0).abs() < 0.15, "in the wall face: {}", th.position);
    assert!(th.position.y > 1.0, "nearly straight: {}", th.position);
    assert!(th.facing.dot(Vec3::NEG_Z) > 0.95, "along its flight: {}", th.facing);
    let e = thorn_entity(&a, planted.0);
    assert_eq!(e.kind, shared::protocol::kind::THORN);
    assert!(e.flags & thorn_flags::PLANTED != 0 && e.flags & thorn_flags::LYING == 0);
    assert!((e.velocity - th.facing).length() < 0.05, "orientation");
    let pos = replicated_position(&e, a.world.tick as f64 + 100.0, 60, &a.tuning.combat);
    assert!(pos.distance(th.position) < 0.01, "planted: no trajectory");
    // 8 s once planted.
    a.ticks(479);
    assert!(a.world.thorns.contains_key(&planted.0));
    let ev = a.ticks(1);
    assert!(!a.world.thorns.contains_key(&planted.0));
    assert!(ev.iter().any(|e| matches!(e, SimEvent::ThornExpired { id } if *id == planted.0)));
}

#[test]
fn a_thorn_that_never_lands_fizzles() {
    let (mut a, p) = setup();
    a.ticks(2);
    a.held.get_mut(&p).unwrap().pitch = 1.55;
    a.tap(p, Buttons::THROW, 175);
    assert_eq!(a.world.thorns.len(), 1, "still flying at 2.9 s");
    let ev = a.ticks(10);
    assert!(a.world.thorns.is_empty());
    assert!(ev.iter().any(|e| matches!(e, SimEvent::ThornExpired { .. })));
}

#[test]
fn thorn_hits_a_target_then_lies_on_the_floor_live() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), PI, 1000.0);
    let other = dummy(&mut a, Vec3::new(6.0, 0.0, -14.0), PI, 1000.0);
    a.ticks(5);
    let body = dummy_body(&a, d);
    a.aim(p, body + Vec3::Y * 0.3);
    let ev = a.tap(p, Buttons::THROW, 60);
    let hit = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(dm) if dm.target == HitTarget::Entity(d) => Some(*dm),
            _ => None,
        })
        .expect("thorn hit");
    assert_eq!(hit.dealt, 8.0);
    assert_eq!(hit.info.attack, Attack::Thorn);
    assert!(hit.info.flags.contains(DamageFlags::PROJECTILE));
    let (&id, th) = a.world.thorns.iter().next().expect("still live");
    assert_eq!(th.state, ThornState::Lying);
    assert!(th.position.y < 0.3, "on the floor: {}", th.position);
    assert!(th.position.z > -6.0, "dropped in front of the target: {}", th.position);
    let e = thorn_entity(&a, id);
    assert!(e.flags & (thorn_flags::PLANTED | thorn_flags::LYING) == thorn_flags::PLANTED | thorn_flags::LYING);
    // A lying thorn still relays shots.
    let lying = th.position;
    a.aim(p, lying);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(ricochet_paths(&ev).len(), 1);
    assert!(damage_to(&ev, HitTarget::Entity(d)) + damage_to(&ev, HitTarget::Entity(other)) > 0.0);
}

#[test]
fn thorn_hits_use_hit_assist() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), PI, 1000.0);
    a.ticks(5);
    // 0.12 m past the body (plus the thorn's own radius): caught by the 0.2 m pad.
    let r = a.tuning.combat.thorn.radius;
    let body = dummy_body(&a, d);
    a.aim(p, body + Vec3::new(DUMMY_RADIUS + r + 0.12, 0.6, 0.0));
    let ev = a.tap(p, Buttons::THROW, 30);
    assert_eq!(damage_to(&ev, HitTarget::Entity(d)), 8.0);
}
