//! Every weapon and fire mode against Husks, weak points, cover, explosions/self damage,
//! friendly fire, dash i-frames, hard damage and blood healing — on the SimWorld directly
//! (deterministic, no networking). Network-level tests live in `server/tests/combat_net.rs`.

mod combat_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::LevelKind;
use shared::protocol::NetId;
use shared::sim::{
    Attack, Blocked, DamageFlags, DamageInfo, HitTarget, Owner, PlayerId, ProjectileKind, SimEvent,
};

const LANCET: u8 = 0;
const THRESHER: u8 = 1;
const RIVETER: u8 = 2;
const CINDERBOX: u8 = 3;

/// Player 0 at the origin looking down -Z (ignored by enemies), husks in front of it.
fn range(husks: &[Vec3]) -> (Arena, PlayerId, Vec<NetId>) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let ids = husks.iter().map(|pos| a.husk(*pos, PI)).collect();
    a.ticks(10); // settle on the floor, fill the history
    (a, p, ids)
}

fn husk_damage(events: &[SimEvent], id: NetId) -> f32 {
    damage_to(events, HitTarget::Entity(id))
}

#[test]
fn lancet_body_shots_kill_a_husk_in_two() {
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -15.0)]);
    let body = a.husk_body(h[0]);
    a.aim(p, body);
    a.set_buttons(p, Buttons::FIRE);
    let first = a.tick();
    assert_eq!(husk_damage(&first, h[0]), 22.0);
    assert!(first.iter().any(
        |e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Lancet && s.ends.len() == 1)
    ));
    // Automatic at 0.3 s: the second shot comes 18 ticks later and kills (40 HP).
    let rest = a.ticks(18);
    assert!(!a.enemy_alive(h[0]));
    let k = kills(&rest);
    assert_eq!(k.len(), 1);
    assert_eq!(k[0].killer, Owner::Player(p));
    assert_eq!(k[0].attack, Attack::Lancet);
    assert_eq!(k[0].victim, HitTarget::Entity(h[0]));
    assert!(!k[0].flags.contains(DamageFlags::HEADSHOT));
}

#[test]
fn headshot_doubles_damage() {
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -15.0)]);
    let head = a.husk_head(h[0]);
    a.aim(p, head);
    let ev = a.tap(p, Buttons::FIRE, 1);
    // 22 x 2 = 44 >= 40: one-shot kill, flagged as a headshot on a weak point.
    let dealt: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) if d.target == HitTarget::Entity(h[0]) => Some(*d),
            _ => None,
        })
        .collect();
    assert_eq!(dealt.len(), 1);
    assert_eq!(
        dealt[0].info.amount, 22.0,
        "base damage is reported unmultiplied"
    );
    assert!(
        dealt[0]
            .info
            .flags
            .contains(DamageFlags::HEADSHOT | DamageFlags::WEAK_POINT)
    );
    assert!(dealt[0].killed);
    let k = kills(&ev);
    assert!(k[0].flags.contains(DamageFlags::HEADSHOT));
    // Death burst for healing.
    assert!(a.world.blood.values().any(|b| b.big));
}

#[test]
fn walls_block_hitscan() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-3.0, 0.0, -8.0], [3.0, 4.0, -7.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let h = a.husk(Vec3::new(0.0, 0.0, -15.0), PI);
    a.ticks(10);
    let body = a.husk_body(h);
    a.aim(p, body);
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(60);
    assert_eq!(husk_damage(&ev, h), 0.0);
    // The tracer ends on the wall.
    let end = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Shot(s) => s.ends.first().copied(),
            _ => None,
        })
        .unwrap();
    assert!((end.z + 7.0).abs() < 0.05, "tracer end {end}");
    // Explosions don't go through walls either.
    a.select(p, CINDERBOX, 0);
    let ev = a.ticks(60);
    assert_eq!(husk_damage(&ev, h), 0.0);
    assert!(ev.iter().any(|e| matches!(e, SimEvent::Explosion { .. })));
}

#[test]
fn thresher_pellets_slug_and_flak() {
    // Pellets: 10 x 9 at 5 m all land.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -5.0)]);
    a.select(p, THRESHER, 0);
    let body = a.husk_body(h[0]);
    a.aim(p, body);
    let before = a.world.enemies[&h[0]].velocity;
    let ev = a.tap(p, Buttons::FIRE, 0);
    let hits = ev
        .iter()
        .filter(|e| matches!(e, SimEvent::Damage(d) if d.target == HitTarget::Entity(h[0])))
        .count();
    assert!(hits >= 5, "{hits} pellets hit");
    assert!(!a.enemy_alive(h[0]), "90 damage kills a husk");
    let shot = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Shot(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(shot.ends.len(), 10);
    assert!(
        a.world.enemies[&h[0]].velocity.z < before.z - 1.0,
        "pellets knock back"
    );

    // Slug: 70 in one hit, recoil on the shooter.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -12.0)]);
    a.select(p, THRESHER, 0);
    let body = a.husk_body(h[0]);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(husk_damage(&ev, h[0]), 40.0, "slug kills (70 > 40 health)");
    assert!(
        a.world.players[&p].movement.velocity.z > 3.0,
        "recoil pushes back (+Z)"
    );

    // Flak: lobbed grenade explodes on contact.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -6.0)]);
    a.select(p, THRESHER, 1);
    let head = a.husk_head(h[0]);
    a.aim(p, head + Vec3::Y * 0.3);
    let mut ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(a.world.projectiles.len(), 1);
    ev.extend(a.ticks(60));
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Explosion {
            attack: Attack::Flak,
            ..
        }
    )));
    assert!(!a.enemy_alive(h[0]));
    assert!(a.health(p) == 100.0, "6 m away is outside the 5 m blast");
}

#[test]
fn riveter_rivets_saw_and_volley() {
    // Rivets: 8 each, 12/s.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -15.0)]);
    a.select(p, RIVETER, 0);
    let body = a.husk_body(h[0]);
    a.aim(p, body);
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(5);
    assert_eq!(a.world.projectiles.len(), 1, "one rivet per 5 ticks");
    let mut ev2 = a.ticks(55);
    ev2.extend(ev);
    let rivet_hits = ev2
        .iter()
        .filter(
            |e| matches!(e, SimEvent::Damage(d) if d.info.attack == Attack::Rivet && d.dealt > 0.0),
        )
        .count();
    assert_eq!(rivet_hits, 5, "40 HP / 8 = 5 rivets");
    assert!(!a.enemy_alive(h[0]));

    // Sawblade pierces a line of husks.
    let (mut a, p, h) = range(&[
        Vec3::new(0.0, 0.0, -6.0),
        Vec3::new(0.0, 0.0, -10.0),
        Vec3::new(0.0, 0.0, -14.0),
    ]);
    a.select(p, RIVETER, 0);
    a.aim(p, Vec3::new(0.0, 1.0, -20.0));
    let mut ev = a.tap(p, Buttons::ALT_FIRE, 0);
    ev.extend(a.ticks(40));
    for id in &h {
        assert!(husk_damage(&ev, *id) >= 10.0, "saw hit {id:?}");
    }

    // Volley: a ring of 8 rivets (12 each) shreds a husk 2.5 m away; the rivets after the
    // killing one fly on through the corpse.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -2.5)]);
    a.select(p, RIVETER, 1);
    let body = a.husk_body(h[0]) + Vec3::Y * 0.2;
    a.aim(p, body);
    let mut ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(a.world.projectiles.len(), 8);
    ev.extend(a.ticks(10));
    let volley = ev
        .iter()
        .filter(|e| matches!(e, SimEvent::Damage(d) if d.info.attack == Attack::Volley))
        .count();
    assert!(volley >= 3, "{volley} volley rivets hit");
    assert!(!a.enemy_alive(h[0]));
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::ProjectileImpact { .. }))
    );
}

#[test]
fn cinderbox_rocket_sticky_and_mortar() {
    // Rocket: 70 at the centre.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -15.0)]);
    a.select(p, CINDERBOX, 0);
    let body = a.husk_body(h[0]);
    a.aim(p, body);
    let mut ev = a.tap(p, Buttons::FIRE, 0);
    ev.extend(a.ticks(30));
    assert!(!a.enemy_alive(h[0]));
    let d = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(d) if d.target == HitTarget::Entity(h[0]) => Some(*d),
            _ => None,
        })
        .unwrap();
    assert!(
        d.info
            .flags
            .contains(DamageFlags::EXPLOSIVE | DamageFlags::DIRECT)
    );
    assert!(d.info.amount > 60.0, "{}", d.info.amount);
    assert_eq!(a.health(p), 100.0);

    // Sticky: sticks to the husk, detonates on the next alt press.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -8.0)]);
    a.select(p, CINDERBOX, 0);
    let head = a.husk_head(h[0]);
    a.aim(p, head);
    a.tap(p, Buttons::ALT_FIRE, 40);
    let sticky = a.world.projectiles.values().next().expect("sticky");
    assert_eq!(sticky.kind, ProjectileKind::Sticky);
    assert_eq!(
        sticky.stuck.and_then(|s| s.to),
        Some(HitTarget::Entity(h[0])),
        "stuck to the husk"
    );
    assert_eq!(a.world.players[&p].combat.weapons.stickies, 1);
    assert!(a.enemy_alive(h[0]), "stickies don't explode on contact");
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::StickiesDetonated { count: 1, .. }))
    );
    assert!(!a.enemy_alive(h[0]));
    assert_eq!(a.world.players[&p].combat.weapons.stickies, 0);

    // Mortar: a level aim lobs it ~20 m.
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -20.0)]);
    a.select(p, CINDERBOX, 1);
    a.aim(p, Vec3::new(0.0, 1.6, -30.0));
    let mut ev = a.tap(p, Buttons::ALT_FIRE, 0);
    ev.extend(a.ticks(90));
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Explosion {
            attack: Attack::Mortar,
            ..
        }
    )));
    assert!(!a.enemy_alive(h[0]));
}

#[test]
fn lancet_overcharge_pierces_and_fan_spreads() {
    let (mut a, p, h) = range(&[
        Vec3::new(0.0, 0.0, -10.0),
        Vec3::new(0.0, 0.0, -14.0),
        Vec3::new(0.0, 0.0, -18.0),
    ]);
    a.aim(p, Vec3::new(0.0, 0.9, -30.0));
    // Hold alt for the full 0.8 s, release: a 75 damage beam through all three.
    a.set_buttons(p, Buttons::ALT_FIRE);
    let held = a.ticks(60);
    assert!(held.iter().all(|e| !matches!(e, SimEvent::Shot(_))));
    assert!(a.world.players[&p].combat.weapons.charge > 0);
    let ev = a.tap(p, Buttons::NONE, 1);
    for id in &h {
        assert!(!a.enemy_alive(*id), "beam pierced {id:?}");
    }
    let pierced = ev
        .iter()
        .filter(|e| matches!(e, SimEvent::Damage(d) if d.info.flags.contains(DamageFlags::PIERCED)))
        .count();
    assert_eq!(pierced, 2);

    // Fan: three shots at -6, 0, +6 degrees, 15 each.
    let yaw6 = 6f32.to_radians();
    let at = |deg: f32| Vec3::new(-(deg.sin()) * 12.0, 0.0, -(deg.cos()) * 12.0);
    let (mut a, p, h) = range(&[at(-yaw6), at(0.0), at(yaw6)]);
    a.select(p, LANCET, 1);
    a.aim(p, Vec3::new(0.0, 0.9, -12.0));
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    for id in &h {
        assert_eq!(husk_damage(&ev, *id), 15.0, "fan shot hit {id:?}");
    }
}

#[test]
fn rocket_jump_self_damage_and_knockback() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.ticks(5);
    a.select(p, CINDERBOX, 0);
    a.held.get_mut(&p).unwrap().pitch = -1.5;
    let ev = a.tap(p, Buttons::FIRE | Buttons::JUMP, 6);
    let d = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(d) if d.target == HitTarget::Player(p) => Some(*d),
            _ => None,
        })
        .expect("own rocket hurts");
    assert!(
        d.info
            .flags
            .contains(DamageFlags::SELF | DamageFlags::EXPLOSIVE)
    );
    // 35 % of up to 70.
    assert!(
        d.dealt > 10.0 && d.dealt <= 70.0 * 0.35 + 1e-3,
        "{}",
        d.dealt
    );
    assert!(
        d.info.knockback.y > 15.0,
        "knockback {:?}",
        d.info.knockback
    );
    let m = a.world.players[&p].movement;
    assert!(!m.on_ground);
    assert!(m.position.y > 1.0 && m.velocity.y > 10.0, "{m:?}");
}

#[test]
fn no_friendly_fire_in_coop_but_knockback_applies() {
    let mut a = Arena::coop();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -6.0), PI);
    a.ticks(5);
    let target = a.world.players[&p1].movement.position + Vec3::Y;
    a.aim(p0, target);
    // Hitscan passes through teammates.
    let ev = a.tap(p0, Buttons::FIRE, 2);
    assert_eq!(damage_to(&ev, HitTarget::Player(p1)), 0.0);
    // A rocket at their feet: no damage, but they fly.
    a.select(p0, CINDERBOX, 0);
    let feet = a.world.players[&p1].movement.position + Vec3::new(0.0, 0.05, 1.0);
    a.aim(p0, feet);
    a.ticks(30);
    let ev = a.tap(p0, Buttons::FIRE, 20);
    assert_eq!(a.health(p1), 100.0);
    assert!(ev.iter().any(|e| matches!(e, SimEvent::Damage(d)
        if d.target == HitTarget::Player(p1) && d.blocked == Some(Blocked::FriendlyFire)
            && d.info.knockback.length() > 5.0)));

    // Same thing in PvP hurts.
    let mut a = Arena::pvp();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -6.0), PI);
    a.ticks(5);
    let head = a.world.players[&p1]
        .movement
        .eye_position(&a.tuning.movement);
    a.aim(p0, head);
    let ev = a.tap(p0, Buttons::FIRE, 1);
    assert_eq!(
        damage_to(&ev, HitTarget::Player(p1)),
        44.0,
        "PvP headshot x2"
    );
    // PvP damage spawns blood the attacker can heal from.
    assert!(!a.world.blood.is_empty());
}

#[test]
fn dash_iframes_block_damage() {
    let mut a = Arena::pvp();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -10.0), 0.0);
    a.ticks(5);
    // p1 dashes sideways; p0 fires at where p1 is during the i-frames.
    a.held.get_mut(&p1).unwrap().right = 127;
    a.set_buttons(p1, Buttons::DASH);
    a.tick();
    a.set_buttons(p1, Buttons::NONE);
    assert!(a.world.players[&p1].movement.is_invulnerable());
    let chest = a.world.players[&p1].movement.position + Vec3::Y;
    a.aim(p0, chest);
    a.view_lag.insert(p0, 0.0);
    a.use_history = false;
    let ev = a.tap(p0, Buttons::FIRE, 0);
    assert_eq!(a.health(p1), 100.0);
    assert!(ev.iter().any(|e| matches!(e, SimEvent::Damage(d)
        if d.target == HitTarget::Player(p1) && d.blocked == Some(Blocked::Invulnerable))));
    // After the i-frames it hurts.
    a.held.get_mut(&p1).unwrap().right = 0;
    a.ticks(30);
    let chest = a.world.players[&p1].movement.position + Vec3::Y;
    a.aim(p0, chest);
    a.tap(p0, Buttons::FIRE, 0);
    assert_eq!(a.health(p1), 78.0);
}

#[test]
fn hard_damage_caps_healing_then_recovers() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.ticks(2);
    let t = a.tuning.clone();
    let info = DamageInfo::new(40.0, Owner::World, Attack::Dev).at(Vec3::Y, Vec3::NEG_Y);
    let e = a.world.apply_damage(&t, HitTarget::Player(p), info);
    assert_eq!(e.dealt, 40.0);
    assert_eq!(e.hard, 14.0);
    let c = a.world.players[&p].combat;
    assert_eq!(c.health(&t.combat), 60.0);
    assert_eq!(c.effective_max(&t.combat), 86.0);
    // A big blood burst right here: heals only up to the effective max.
    a.world
        .spawn_blood(&t, Vec3::new(0.0, 1.0, 0.0), 100.0, 3.5, 0.6, true);
    let ev = a.ticks(2);
    assert_eq!(a.health(p), 86.0);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Healed { amount, .. } if (*amount - 26.0).abs() < 1e-3))
    );
    // Recovery starts after 1.5 s (90 ticks; 4 have passed) at 25/s.
    a.ticks(80);
    assert_eq!(a.world.players[&p].combat.hard, 14.0, "still waiting");
    a.ticks(20);
    let hard = a.world.players[&p].combat.hard;
    assert!(hard < 14.0 && hard > 0.0, "{hard}");
    a.ticks(40);
    assert_eq!(a.world.players[&p].combat.hard, 0.0);
    assert_eq!(
        a.health(p),
        86.0,
        "recovered hard damage only raises the cap"
    );
}

#[test]
fn blood_burst_heals_the_closest_player() {
    let mut a = Arena::coop();
    let near = a.player(0, Vec3::new(0.0, 0.0, 0.0), 0.0);
    let far = a.player(1, Vec3::new(2.5, 0.0, 0.5), 0.0);
    a.world.players.get_mut(&near).unwrap().notarget = true;
    a.world.players.get_mut(&far).unwrap().notarget = true;
    let h = a.husk(Vec3::new(0.0, 0.0, -1.8), PI);
    a.ticks(10);
    let t = a.tuning.clone();
    for id in [near, far] {
        // 60 damage, 21 of it hard: room for 39 HP of healing.
        let info = DamageInfo::new(60.0, Owner::World, Attack::Dev);
        a.world.apply_damage(&t, HitTarget::Player(id), info);
    }
    // Punch the husk: 25 damage, 100 % of it becomes a heal pool at the hit point.
    let body = a.husk_body(h);
    a.aim(near, body);
    let ev = a.tap(near, Buttons::PUNCH, 3);
    assert_eq!(husk_damage(&ev, h), 25.0);
    assert_eq!(a.health(near), 65.0, "closest player drains the 25 HP pool");
    assert_eq!(a.health(far), 40.0, "the other player gets nothing");
    let heals: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Healed { player, amount, .. } => Some((*player, *amount)),
            _ => None,
        })
        .collect();
    assert_eq!(heals, vec![(near, 25.0)]);
}

#[test]
fn slam_shockwave_hits_nearby_enemies() {
    let (mut a, p, h) = range(&[Vec3::new(3.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -12.0)]);
    a.world.players.get_mut(&p).unwrap().movement.position.y = 12.0;
    a.world.players.get_mut(&p).unwrap().movement.on_ground = false;
    a.tick();
    let ev = a.tap(p, Buttons::CROUCH, 60);
    let slam = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::SlamShockwave { power, .. } => Some(*power),
            _ => None,
        })
        .expect("slam landed");
    assert!(slam > 0.5);
    assert!(husk_damage(&ev, h[0]) > 10.0, "near husk hit");
    assert_eq!(husk_damage(&ev, h[1]), 0.0, "far husk untouched");
}

#[test]
fn dead_players_drop_and_respawn() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.ticks(2);
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
    assert!(a.world.players[&p].combat.dead);
    // Fire does nothing while dead.
    let ev = a.tap(p, Buttons::FIRE, 10);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Shot(_))));
    let ev = a.ticks(120);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Respawned { player, .. } if *player == p))
    );
    let c = a.world.players[&p].combat;
    assert!(!c.dead);
    assert_eq!(c.health(&t.combat), 100.0);
    assert!(c.protection > 0, "spawn protection");
}
