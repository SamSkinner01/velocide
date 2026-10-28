//! Thorns and relays: throwing and planting, hitting targets (drop and lie), lifetime, shots
//! bouncing off thorns to weak points, chains, branded targets first, PvP targets, hitting a
//! flying thorn as a lagging player saw it, and every attack family relaying: every hitscan
//! weapon's rays (a pellet spread once), projectiles turning (rivets, bombs, arrows; a target
//! in front takes it first), swings and the Whirl, cones and novas, players' blasts.

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::LevelKind;
use shared::loadout::{Loadout, WeaponKind};
use shared::protocol::{NetFrame, NetId, projectile_flags, thorn_flags};
use shared::sim::projectiles::{Explosive, replicated_position};
use shared::sim::{
    Attack, DamageFlags, DamageInfo, HitTarget, Owner, PlayerId, SimEvent, StyleBonus, StyleEvent,
    ThornState,
};
use shared::tuning::Tuning;
use weapon_support::*;

const BIG: f32 = 1.0e5;

fn setup() -> (Arena, PlayerId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    (a, p)
}

/// A thorn planted in mid-air at `pos` (as if stuck in something there).
fn thorn_at(a: &mut Arena, owner: PlayerId, pos: Vec3) -> NetId {
    let t = a.tuning.clone();
    let id = a
        .world
        .spawn_thorn(&t, owner, pos, Vec3::ZERO)
        .expect("thorn");
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

fn projectile_entity(a: &Arena, id: NetId) -> shared::protocol::EntityState {
    thorn_entity(a, id)
}

/// Every relay in `events`: (attack, bounce path, the projectile that turned).
fn relays(events: &[SimEvent]) -> Vec<(Attack, Vec<Vec3>, Option<NetId>)> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Ricochet {
                attack,
                points,
                projectile,
                ..
            } => Some((*attack, points.clone(), *projectile)),
            _ => None,
        })
        .collect()
}

/// The ricochet hits on `target` in `events`.
fn ricochet_hits(events: &[SimEvent], target: HitTarget) -> Vec<DamageInfo> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d)
                if d.target == target && d.info.flags.contains(DamageFlags::RICOCHET) =>
            {
                Some(d.info)
            }
            _ => None,
        })
        .collect()
}

/// Player `p` holds `weapon` with its default abilities (slot 0, ability 0 selected).
fn arm_default(a: &mut Arena, p: PlayerId, weapon: WeaponKind) {
    a.world
        .force_loadout(p, Some(Loadout::single(weapon.default_build())));
    a.select(p, 0, 0);
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
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::ThornShattered { id, .. } if *id == thorn))
    );
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
    assert!(
        hit.info
            .flags
            .contains(DamageFlags::RICOCHET | DamageFlags::HEADSHOT)
    );
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
fn every_ray_ricochets_and_a_pellet_spread_uses_a_thorn_once() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.6, -6.0);
    let c = a.tuning.combat.clone();
    let mult = c.thorn.damage_multiplier;
    // (weapon, its ray's damage, the attack): each ricochets into the head (×2).
    for (weapon, damage) in [
        (WeaponKind::Bellows, c.bellows.pellet_damage),
        (WeaponKind::Thresher, c.thresher.pellet_damage),
        (WeaponKind::Lancet, c.lancet.damage),
        (WeaponKind::Arclash, c.arclash.zap_damage),
        (WeaponKind::Quill, c.quill.damage),
    ] {
        arm_default(&mut a, p, weapon);
        let thorn = thorn_at(&mut a, p, at);
        a.ticks(5);
        a.aim(p, at);
        let ev = a.tap(p, Buttons::FIRE, 1);
        assert!(!a.world.thorns.contains_key(&thorn), "{weapon:?}");
        // A whole pellet spread passes the thorn: the first pellet takes it, once.
        let relayed = relays(&ev);
        assert_eq!(relayed.len(), 1, "{weapon:?}");
        assert_eq!(relayed[0].0, weapon.primary());
        assert_eq!(relayed[0].2, None, "a ray, not a projectile");
        let hits: Vec<_> = ricochet_hits(&ev, HitTarget::Entity(d));
        assert_eq!(hits.len(), 1, "{weapon:?}");
        let hit = hits[0];
        assert!(
            (hit.amount - damage * mult).abs() < 1e-3,
            "{weapon:?}: {}",
            hit.amount
        );
        assert!(
            hit.flags
                .contains(DamageFlags::HITSCAN | DamageFlags::HEADSHOT)
        );
        assert_eq!(hit.ricochets, 1);
        a.ticks(90);
    }
    // A Brand on the Quill bounces and brands what it lands on.
    let thorn = thorn_at(&mut a, p, at);
    a.ticks(5);
    a.select(p, 0, 0);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert!(!a.world.thorns.contains_key(&thorn));
    assert_eq!(relays(&ev).len(), 1);
    assert!(a.world.is_branded(HitTarget::Entity(d)));
}

#[test]
fn projectiles_turn_at_a_thorn_onto_a_line_at_the_target() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.6, -6.0);
    arm_default(&mut a, p, WeaponKind::Riveter);
    let thorn = thorn_at(&mut a, p, at);
    a.ticks(5);
    a.aim(p, at);
    let mut ev = a.tap(p, Buttons::FIRE, 0);
    // Flies to the thorn and turns there.
    let mut turned = None;
    for _ in 0..30 {
        if let Some(r) = relays(&ev).first() {
            turned = Some(r.clone());
            break;
        }
        ev = a.tick();
    }
    let (attack, points, projectile) = turned.expect("the rivet reached the thorn");
    assert_eq!(attack, Attack::Riveter);
    assert!(!a.world.thorns.contains_key(&thorn));
    assert!((points[0] - at).length() < 1e-3);
    let id = projectile.expect("a projectile relay");
    // In flight: straight at the dummy's head, no gravity, replicated as RELAYED.
    let e = projectile_entity(&a, id);
    assert!(e.flags & projectile_flags::RELAYED != 0);
    let head = dummy_head(&a, d);
    let rivet = &a.world.projectiles[&id];
    assert_eq!(rivet.ricochets, 1);
    let to_head = (head - rivet.position).normalize();
    assert!(rivet.velocity.normalize().dot(to_head) > 0.999);
    assert!((rivet.velocity.length() - a.tuning.combat.riveter.rivet_speed).abs() < 1.0);
    a.tick();
    if let Some(rivet) = a.world.projectiles.get(&id) {
        let drawn = replicated_position(&e, a.world.tick as f64, 60, &a.tuning.combat);
        // (Within the wire's quantisation.)
        assert!(
            drawn.distance(rivet.position) < 0.02,
            "straight on the wire"
        );
    }
    let ev = a.ticks(20);
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    let c = &a.tuning.combat;
    let want = c.riveter.damage * c.thorn.damage_multiplier;
    assert!((hits[0].amount - want).abs() < 1e-3, "{}", hits[0].amount);
    assert!(hits[0].flags.contains(DamageFlags::PROJECTILE));
    assert_eq!(hits[0].ricochets, 1);
}

#[test]
fn a_relayed_bomb_keeps_its_blast_and_an_arrow_its_draw() {
    // The Tumbler's bomb: it goes off at the dummy (no more bouncing), × the multiplier.
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(8.0, 0.0, -4.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.5, -4.0);
    arm_default(&mut a, p, WeaponKind::Tumbler);
    thorn_at(&mut a, p, at);
    a.ticks(5);
    a.aim(p, at);
    // (Where it stood: the blast throws it.)
    let body = dummy_body(&a, d);
    let ev = a.tap(p, Buttons::FIRE, 60);
    let turned = relays(&ev);
    assert_eq!(turned.len(), 1);
    assert_eq!(turned[0].0, Attack::Tumble);
    let blast = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Explosion {
                position, attack, ..
            } if *attack == Attack::Tumble => Some(*position),
            _ => None,
        })
        .expect("it exploded");
    assert!(blast.distance(body) < 1.5, "at the dummy: {blast}");
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    let c = &a.tuning.combat;
    let full = c.tumbler.bomb_damage * c.thorn.damage_multiplier;
    assert!(
        hits[0].amount > full * 0.9 && hits[0].amount <= full + 1e-3,
        "{}",
        hits[0].amount
    );
    assert!(
        hits[0]
            .flags
            .contains(DamageFlags::EXPLOSIVE | DamageFlags::DIRECT)
    );

    // The Longwick: a full-draw arrow turns and hits the head with its draw's damage.
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.6, -6.0);
    arm_default(&mut a, p, WeaponKind::Longwick);
    thorn_at(&mut a, p, at);
    a.ticks(5);
    a.aim(p, at);
    a.set_buttons(p, Buttons::FIRE);
    a.ticks(50);
    a.set_buttons(p, Buttons::NONE);
    let ev = a.ticks(40);
    assert_eq!(relays(&ev).len(), 1);
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    let c = &a.tuning.combat;
    let want = c.longwick.max_damage * c.thorn.damage_multiplier;
    assert!((hits[0].amount - want).abs() < 1e-2, "{}", hits[0].amount);
}

#[test]
fn a_hurled_blade_turns_at_a_thorn_on_its_way_out() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.6, -6.0);
    arm_default(&mut a, p, WeaponKind::Trimmer);
    let thorn = thorn_at(&mut a, p, at);
    a.ticks(5);
    a.aim(p, at);
    a.select(p, 0, 1);
    let ev = a.tap(p, Buttons::ALT_FIRE, 60);
    assert!(!a.world.thorns.contains_key(&thorn));
    let turned = relays(&ev);
    assert_eq!(turned.len(), 1);
    assert_eq!(turned[0].0, Attack::Hurl);
    assert!(turned[0].2.is_some(), "the blade turned");
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert!(!hits.is_empty());
    let c = &a.tuning.combat;
    let want =
        WeaponKind::Trimmer.primary_damage(c) * c.hurl.damage_scale * c.thorn.damage_multiplier;
    assert!((hits[0].amount - want).abs() < 1e-3, "{}", hits[0].amount);
    assert_eq!(hits[0].ricochets, 1);
}

#[test]
fn a_lagging_shooters_projectile_relays_in_its_made_up_flight() {
    // 12 ticks behind: the rivet makes up ~18 m of flight on its first step, through the thorn
    // 6 m out and on to the dummy it turns at.
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.6, -6.0);
    arm_default(&mut a, p, WeaponKind::Riveter);
    let thorn = thorn_at(&mut a, p, at);
    a.ticks(20);
    a.view_lag.insert(p, 12.0);
    a.aim(p, at);
    let ev = a.tap(p, Buttons::FIRE, 0);
    assert!(
        !a.world.thorns.contains_key(&thorn),
        "relayed on its first step"
    );
    assert_eq!(relays(&ev).len(), 1);
    let ev: Vec<SimEvent> = ev.into_iter().chain(a.ticks(10)).collect();
    assert_eq!(ricochet_hits(&ev, HitTarget::Entity(d)).len(), 1);
}

#[test]
fn a_target_before_the_thorn_takes_the_projectile() {
    let (mut a, p) = setup();
    let front = dummy(&mut a, Vec3::new(0.0, 0.0, -5.0), 0.0, BIG);
    let side = dummy(&mut a, Vec3::new(9.0, 0.0, -6.0), -PI / 2.0, BIG);
    let at = Vec3::new(0.0, 1.0, -6.0);
    arm_default(&mut a, p, WeaponKind::Riveter);
    let thorn = thorn_at(&mut a, p, at);
    a.ticks(5);
    a.aim(p, dummy_body(&a, front));
    let ev = a.tap(p, Buttons::FIRE, 20);
    assert!(relays(&ev).is_empty());
    assert!(a.world.thorns.contains_key(&thorn));
    assert!(damage_to(&ev, HitTarget::Entity(front)) > 0.0);
    assert_eq!(damage_to(&ev, HitTarget::Entity(side)), 0.0);
}

#[test]
fn swings_bat_a_thorn_in_their_arc_at_the_swing_damage() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(10.0, 0.0, 0.0), PI / 2.0, BIG);
    let c = a.tuning.combat.clone();
    let mult = c.thorn.damage_multiplier;
    // The Trimmer's slash.
    arm_default(&mut a, p, WeaponKind::Trimmer);
    let thorn = thorn_at(&mut a, p, Vec3::new(0.0, 1.4, -2.0));
    a.ticks(5);
    a.aim(p, Vec3::new(0.0, 1.4, -2.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(!a.world.thorns.contains_key(&thorn), "batted");
    let relayed = relays(&ev);
    assert_eq!(relayed.len(), 1);
    assert_eq!(relayed[0].0, Attack::Trimmer);
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    assert!((hits[0].amount - c.trimmer.damage * mult).abs() < 1e-3);
    assert_eq!(hits[0].attack, Attack::Trimmer);
    assert_eq!(hits[0].ricochets, 1);
    // Style counts it as a ricochet.
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Style(StyleEvent::Bonus {
            bonus: StyleBonus::Ricochet,
            ..
        })
    )));
    // Out of reach or behind: left alone.
    a.ticks(60);
    let far = thorn_at(&mut a, p, Vec3::new(0.0, 1.4, -5.0));
    let behind = thorn_at(&mut a, p, Vec3::new(0.0, 1.4, 1.5));
    a.ticks(5);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(relays(&ev).is_empty());
    assert!(a.world.thorns.contains_key(&far) && a.world.thorns.contains_key(&behind));

    // The Tamper's Whirl: everything around, thorns too.
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -15.0), 0.0, BIG);
    arm_default(&mut a, p, WeaponKind::Tamper);
    let thorn = thorn_at(&mut a, p, Vec3::new(2.0, 1.0, 1.0));
    a.ticks(5);
    a.select(p, 0, 0);
    let ev = a.tap(p, Buttons::ALT_FIRE, 2);
    assert!(!a.world.thorns.contains_key(&thorn));
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    let whirl = WeaponKind::Tamper.primary_damage(&c) * c.whirl.damage_scale;
    assert!((hits[0].amount - whirl * mult).abs() < 1e-3);
    assert_eq!(hits[0].attack, Attack::Whirl);
}

#[test]
fn cones_and_novas_fling_a_thorn_at_their_per_hit_damage() {
    let c = Tuning::default().combat;
    let mult = c.thorn.damage_multiplier;
    // The Kiln's spray and the Bellows' Gust: a thorn on the aim inside the cone.
    for (weapon, alt, attack, damage) in [
        (WeaponKind::Kiln, false, Attack::Kiln, c.kiln.damage),
        (
            WeaponKind::Bellows,
            true,
            Attack::Gust,
            c.bellows.gust_damage,
        ),
    ] {
        let (mut a, p) = setup();
        let d = dummy(&mut a, Vec3::new(10.0, 0.0, 0.0), PI / 2.0, BIG);
        arm_default(&mut a, p, weapon);
        let at = Vec3::new(0.0, 1.6, -4.0);
        let thorn = thorn_at(&mut a, p, at);
        a.ticks(5);
        a.aim(p, at);
        a.select(p, 0, 0);
        let button = if alt {
            Buttons::ALT_FIRE
        } else {
            Buttons::FIRE
        };
        let ev = a.tap(p, button, 1);
        assert!(!a.world.thorns.contains_key(&thorn), "{weapon:?}");
        let relayed = relays(&ev);
        assert_eq!(relayed.len(), 1, "{weapon:?}");
        assert_eq!(relayed[0].0, attack);
        let hits = ricochet_hits(&ev, HitTarget::Entity(d));
        assert_eq!(hits.len(), 1, "{weapon:?}");
        assert!((hits[0].amount - damage * mult).abs() < 1e-3, "{weapon:?}");
    }

    // The Arclash's Discharge, fully charged: a thorn beside the shooter.
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -20.0), 0.0, BIG);
    arm_default(&mut a, p, WeaponKind::Arclash);
    let thorn = thorn_at(&mut a, p, Vec3::new(3.0, 1.0, 0.0));
    a.ticks(5);
    a.select(p, 0, 1);
    a.set_buttons(p, Buttons::ALT_FIRE);
    a.ticks(75);
    a.set_buttons(p, Buttons::NONE);
    let ev = a.ticks(2);
    assert!(!a.world.thorns.contains_key(&thorn));
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].attack, Attack::Discharge);
    let full = c.arclash.discharge_max_damage * mult;
    assert!((hits[0].amount - full).abs() < 1e-3, "{}", hits[0].amount);
}

#[test]
fn a_players_blast_flings_its_thorns_at_the_relay_damage() {
    let (mut a, p) = setup();
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -20.0), 0.0, BIG);
    let near = thorn_at(&mut a, p, Vec3::new(5.0, 1.0, -5.0));
    let chained = thorn_at(&mut a, p, Vec3::new(-4.0, 3.0, -12.0));
    a.ticks(5);
    let t = a.tuning.clone();
    let spec = Explosive {
        radius: 3.0,
        damage: 50.0,
        knockback: 0.0,
    };
    let center = Vec3::new(6.5, 0.5, -5.0);
    // An enemy's blast leaves it alone.
    a.world.explode(
        &t,
        center,
        spec,
        Owner::Enemy(d),
        Attack::Enemy { kind: 0, attack: 0 },
        DamageFlags::NONE,
        None,
        None,
        Owner::Enemy(d),
    );
    assert!(a.world.thorns.contains_key(&near));
    // A player's: the thorn in it is flung on, chaining through the other thorn first.
    a.world.explode(
        &t,
        center,
        spec,
        Owner::Player(p),
        Attack::Breach,
        DamageFlags::NONE,
        None,
        None,
        Owner::Player(p),
    );
    assert!(a.world.thorns.is_empty(), "{:?}", a.world.thorns.keys());
    let ev = a.tick();
    let paths = relays(&ev);
    assert_eq!(paths.len(), 1, "one fling, the chain on its path");
    assert_eq!(paths[0].1.len(), 3, "thorn, thorn, target");
    let _ = chained;
    let hits = ricochet_hits(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    let c = &t.combat;
    let want = c.thorn.relay_damage * c.thorn.damage_multiplier.powi(2);
    assert!((hits[0].amount - want).abs() < 1e-3, "{}", hits[0].amount);
    assert_eq!(hits[0].ricochets, 2);
    assert_eq!(hits[0].attack, Attack::Breach);
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
    let before = a.world.players[&p]
        .combat
        .weapons
        .knack_charges(&a.tuning.combat);
    a.held.get_mut(&p).unwrap().pitch = 0.5;
    a.tap(p, Buttons::THROW, 0);
    assert_eq!(
        a.world.players[&p]
            .combat
            .weapons
            .knack_charges(&a.tuning.combat),
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
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::ThornThrown { player, .. } if *player == p))
    );
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
    assert!(
        (th.position.z + 8.0).abs() < 0.15,
        "in the wall face: {}",
        th.position
    );
    assert!(th.position.y > 1.0, "nearly straight: {}", th.position);
    assert!(
        th.facing.dot(Vec3::NEG_Z) > 0.95,
        "along its flight: {}",
        th.facing
    );
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
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::ThornExpired { id } if *id == planted.0))
    );
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
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::ThornExpired { .. }))
    );
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
    assert!(
        th.position.z > -6.0,
        "dropped in front of the target: {}",
        th.position
    );
    let e = thorn_entity(&a, id);
    assert!(
        e.flags & (thorn_flags::PLANTED | thorn_flags::LYING)
            == thorn_flags::PLANTED | thorn_flags::LYING
    );
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
