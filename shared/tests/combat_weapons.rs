//! Every weapon and fire mode against target dummies: Quill / Brand / Splinter, Bellows Blast /
//! Gust / Both Barrels, Arclash Arc / Wisp / Discharge, Tumbler bomb / Breach / Gravewell, the
//! fist, hit assist, cover, explosions and self damage, friendly fire, dash i-frames, hard
//! damage and ember bursts, and weapon-state prediction — on the SimWorld directly
//! (deterministic, no networking). Network-level tests live in `server/tests/combat_net.rs`.
//! Thorns and ricochets: `combat_ricochet.rs`.

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::LevelKind;
use shared::protocol::{NetFrame, NetId, enemy_flags, player_flags};
use shared::sim::projectiles::Explosive;
use shared::sim::{
    Attack, Blocked, DamageFlags, DamageInfo, HitTarget, Owner, PlayerId, ProjectileKind, SimEvent,
    StyleBonus, StyleEvent, WeaponState, step_weapons,
};
use weapon_support::*;

const QUILL: u8 = 0;
const BELLOWS: u8 = 1;
const ARCLASH: u8 = 2;
const TUMBLER: u8 = 3;

/// Player 0 at the origin looking down -Z (ignored by enemies), dummies with `health` in front.
fn range(dummies: &[Vec3], health: f32) -> (Arena, PlayerId, Vec<NetId>) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let ids = dummies
        .iter()
        .map(|pos| dummy(&mut a, *pos, PI, health))
        .collect();
    a.ticks(10); // settle on the floor, fill the history
    (a, p, ids)
}

fn dmg(events: &[SimEvent], id: NetId) -> f32 {
    damage_to(events, HitTarget::Entity(id))
}

fn hits_on(events: &[SimEvent], target: HitTarget) -> Vec<shared::sim::DamageEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) if d.target == target => Some(*d),
            _ => None,
        })
        .collect()
}

/// Puts a (knocked away) dummy back at `pos`, at rest.
fn reset_dummy(a: &mut Arena, id: NetId, pos: Vec3) {
    let e = a.world.enemies.get_mut(&id).unwrap();
    e.position = pos;
    e.velocity = Vec3::ZERO;
    e.on_ground = true;
    e.stagger = 0;
}

fn entity_flags(a: &Arena, id: NetId) -> u32 {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).expect("replicated").flags
}

/// An enemy projectile kind (the parryable ones belong to enemies), whatever the roster is.
fn enemy_projectile_kind() -> ProjectileKind {
    ProjectileKind::ALL
        .into_iter()
        .find(|k| k.parryable())
        .expect("enemies have projectiles")
}

// ---- Quill ------------------------------------------------------------------------------

#[test]
fn quill_shots_kill_in_two() {
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -15.0)], 45.0);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body);
    a.set_buttons(p, Buttons::FIRE);
    let first = a.tick();
    assert_eq!(dmg(&first, d[0]), 24.0);
    assert!(
        first.iter().any(
            |e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Quill && s.ends.len() == 1)
        )
    );
    // Automatic at 0.28 s: the second shot comes 17 ticks later and kills (45 HP).
    let rest = a.ticks(16);
    assert!(a.enemy_alive(d[0]), "not before the interval");
    let rest2 = a.ticks(1);
    assert!(!a.enemy_alive(d[0]));
    assert!(kills(&rest).is_empty());
    let k = kills(&rest2);
    assert_eq!(k.len(), 1);
    assert_eq!(k[0].killer, Owner::Player(p));
    assert_eq!(k[0].attack, Attack::Quill);
    assert_eq!(k[0].attack.name(), "Quill");
    assert!(!k[0].flags.contains(DamageFlags::HEADSHOT));
}

#[test]
fn headshot_doubles_damage() {
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -15.0)], 45.0);
    let head = dummy_head(&a, d[0]);
    a.aim(p, head);
    let ev = a.tap(p, Buttons::FIRE, 1);
    let dealt = hits_on(&ev, HitTarget::Entity(d[0]));
    assert_eq!(dealt.len(), 1);
    assert_eq!(
        dealt[0].info.amount, 24.0,
        "base damage is reported unmultiplied"
    );
    assert!(
        dealt[0]
            .info
            .flags
            .contains(DamageFlags::HEADSHOT | DamageFlags::WEAK_POINT)
    );
    assert!(dealt[0].killed, "48 >= 45");
    assert!(kills(&ev)[0].flags.contains(DamageFlags::HEADSHOT));
    // Death burst for healing.
    assert!(a.world.embers.values().any(|b| b.big));
}

#[test]
fn walls_block_hitscan_and_explosions() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-3.0, 0.0, -8.0], [3.0, 4.0, -7.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -15.0), PI, 1000.0);
    a.ticks(10);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(60);
    assert_eq!(dmg(&ev, d), 0.0);
    // The tracer ends on the wall.
    let end = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Shot(s) => s.ends.first().copied(),
            _ => None,
        })
        .unwrap();
    assert!((end.z + 7.0).abs() < 0.05, "tracer end {end}");
    // Bombs don't blow through walls either.
    a.select(p, TUMBLER, 0);
    let ev = a.ticks(120);
    assert_eq!(dmg(&ev, d), 0.0);
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Explosion {
            attack: Attack::Tumble,
            ..
        }
    )));
}

#[test]
fn brand_marks_multiplies_expires_and_moves() {
    let (mut a, p, d) = range(
        &[Vec3::new(-2.0, 0.0, -15.0), Vec3::new(2.0, 0.0, -15.0)],
        1000.0,
    );
    let (da, db) = (d[0], d[1]);
    // Brand A.
    a.select(p, QUILL, 0);
    let body = dummy_body(&a, da);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(dmg(&ev, da), 10.0);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Branded { owner, target, .. }
        if *owner == p && *target == HitTarget::Entity(da)))
    );
    assert!(a.world.is_branded(HitTarget::Entity(da)));
    assert!(
        entity_flags(&a, da) & enemy_flags::BRANDED != 0,
        "replicated"
    );
    assert!(entity_flags(&a, db) & enemy_flags::BRANDED == 0);
    // Everything hits it x1.35: the Quill ...
    let ev = a.tap(p, Buttons::FIRE, 0);
    let hit = hits_on(&ev, HitTarget::Entity(da));
    assert!(
        (hit[0].dealt - 24.0 * 1.35).abs() < 1e-3,
        "{}",
        hit[0].dealt
    );
    assert!(hit[0].info.flags.contains(DamageFlags::BRANDED));
    // ... and anybody else's damage.
    let t = a.tuning.clone();
    let info = DamageInfo::new(10.0, Owner::Player(PlayerId(7)), Attack::Fist);
    let e = a.world.apply_damage(&t, HitTarget::Entity(da), info);
    assert!((e.dealt - 13.5).abs() < 1e-3);
    // Branding B (off cooldown) moves the player's one brand.
    a.ticks(150);
    let body = dummy_body(&a, db);
    a.aim(p, body);
    a.tap(p, Buttons::ALT_FIRE, 0);
    assert!(
        !a.world.is_branded(HitTarget::Entity(da)),
        "one live brand per player"
    );
    assert!(a.world.is_branded(HitTarget::Entity(db)));
    // 5 s later it's gone.
    a.ticks(298);
    assert!(a.world.is_branded(HitTarget::Entity(db)));
    a.ticks(2);
    assert!(!a.world.is_branded(HitTarget::Entity(db)));
    assert!(a.world.brands.is_empty());
    let ev = a.tap(p, Buttons::FIRE, 0);
    assert_eq!(dmg(&ev, db), 24.0);
}

#[test]
fn brand_on_players_in_pvp() {
    let mut a = Arena::pvp();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -10.0), PI);
    a.ticks(5);
    let chest = a.world.players[&p1].movement.position + Vec3::Y;
    a.aim(p0, chest);
    a.select(p0, QUILL, 0);
    a.tap(p0, Buttons::ALT_FIRE, 0);
    assert_eq!(a.health(p1), 90.0);
    assert!(entity_flags(&a, NetId::player(p1)) & player_flags::BRANDED != 0);
    let ev = a.tap(p0, Buttons::FIRE, 0);
    assert!((damage_to(&ev, HitTarget::Player(p1)) - 32.4).abs() < 1e-3);
    // The brand dies with its target.
    let t = a.tuning.clone();
    a.world.kill_player(&t, p1, Attack::Dev);
    a.tick();
    assert!(a.world.brands.is_empty());
}

#[test]
fn splinters_continue_through_the_target() {
    let run = || {
        let (mut a, p, d) = range(
            &[Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, 0.0, -13.5)],
            1000.0,
        );
        a.select(p, QUILL, 1);
        let body = dummy_body(&a, d[0]);
        a.aim(p, body);
        let ev = a.tap(p, Buttons::ALT_FIRE, 0);
        let splinters = ev
            .iter()
            .find_map(|e| match e {
                SimEvent::Splinters { origin, ends, .. } => Some((*origin, ends.clone())),
                _ => None,
            })
            .expect("splinter burst");
        (dmg(&ev, d[0]), dmg(&ev, d[1]), splinters)
    };
    let (front, back, (origin, ends)) = run();
    assert_eq!(
        front, 20.0,
        "the target hit directly takes only the main shot"
    );
    assert_eq!(ends.len(), 6);
    assert!(
        origin.z > -10.0 && origin.z < -9.4,
        "burst on the target: {origin}"
    );
    assert!(
        back >= 9.0 && back % 9.0 == 0.0,
        "splinters hit the one behind: {back}"
    );
    // Every splinter leaves forward, inside the 40 degree cone.
    for end in &ends {
        let dir = (*end - origin).normalize();
        assert!(dir.angle_between(Vec3::NEG_Z).to_degrees() < 21.0, "{end}");
    }
    // Deterministic.
    assert_eq!(run(), (front, back, (origin, ends)));
}

#[test]
fn splinters_reflect_off_walls() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-20.0, 0.0, -10.5], [20.0, 6.0, -10.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    // Around the corner of the reflected shot (the main shot passes 1.6 m away).
    let d = dummy(&mut a, Vec3::new(5.9, 0.0, -8.2), PI, 1000.0);
    a.ticks(10);
    let eye = a.eye(p);
    a.aim(p, Vec3::new(5.0, eye.y, -10.0));
    a.select(p, QUILL, 1);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let (origin, ends) = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Splinters { origin, ends, .. } => Some((*origin, ends.clone())),
            _ => None,
        })
        .expect("burst on the wall");
    assert!((origin.z + 10.0).abs() < 0.1, "{origin}");
    for end in &ends {
        assert!(
            end.z > -10.05,
            "splinters bounce back out of the wall: {end}"
        );
        assert!(end.x > origin.x, "reflected: {end}");
    }
    let hit = dmg(&ev, d);
    assert!(hit >= 9.0 && hit % 9.0 == 0.0, "corner shot: {hit}");
}

// ---- Bellows ----------------------------------------------------------------------------

#[test]
fn bellows_blast_and_both_barrels() {
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -5.0)], 1000.0);
    a.select(p, BELLOWS, 1);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::FIRE, 0);
    let hits = hits_on(&ev, HitTarget::Entity(d[0]));
    assert_eq!(hits.len(), 12, "12 pellets in an 8 degree cone at 5 m");
    assert_eq!(dmg(&ev, d[0]), 96.0);
    assert!(hits.iter().all(|h| h.info.knockback.length() > 1.9));
    assert_eq!(hits[0].info.attack.name(), "Bellows");
    // Both barrels: 24 pellets and a blast-jump recoil; holds the primary too.
    a.ticks(60);
    reset_dummy(&mut a, d[0], Vec3::new(0.0, 0.0, -5.0));
    a.ticks(2);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body);
    let v0 = a.world.players[&p].movement.velocity;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(hits_on(&ev, HitTarget::Entity(d[0])).len(), 24);
    assert_eq!(dmg(&ev, d[0]), 192.0);
    let aim = a.held[&p].look_dir();
    let dv = a.world.players[&p].movement.velocity - v0;
    assert!(dv.dot(-aim) > 7.0, "recoil {dv}");
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(47);
    assert!(
        !ev.iter().any(|e| matches!(e, SimEvent::Shot(_))),
        "primary waits"
    );
    let ev = a.ticks(1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Blast))
    );
}

#[test]
fn gust_knocks_back_in_its_cone_and_recoils() {
    let a50 = 50f32.to_radians();
    let (mut a, p, d) = range(
        &[
            Vec3::new(0.0, 0.0, -4.0),                         // in the cone
            Vec3::new(0.0, 0.0, -9.5),                         // out of range
            Vec3::new(a50.sin() * 4.0, 0.0, -a50.cos() * 4.0), // outside the 60 degree cone
        ],
        1000.0,
    );
    a.select(p, BELLOWS, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    a.held.get_mut(&p).unwrap().yaw = 0.0;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(dmg(&ev, d[0]), 6.0);
    assert_eq!(dmg(&ev, d[1]), 0.0);
    assert_eq!(dmg(&ev, d[2]), 0.0);
    let hit = hits_on(&ev, HitTarget::Entity(d[0]))[0];
    assert!((hit.info.knockback - Vec3::new(0.0, 4.0, -18.0)).length() < 1e-3);
    let v = a.world.enemies[&d[0]].velocity;
    assert!(v.z < -15.0 && v.y > 2.0, "launched along the aim: {v}");
    assert!(
        a.world.players[&p].movement.velocity.z > 8.0,
        "gust-jump recoil {}",
        a.world.players[&p].movement.velocity
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Gust))
    );
}

#[test]
fn gust_deflects_hostile_projectiles() {
    let (mut a, p, d) = range(&[Vec3::new(30.0, 0.0, -30.0)], 1000.0);
    let t = a.tuning.clone();
    let kind = enemy_projectile_kind();
    let eye = a.eye(p);
    let incoming = a
        .world
        .spawn_enemy_projectile(
            &t,
            kind,
            d[0],
            Attack::Enemy {
                kind: 60,
                attack: 0,
            },
            eye + Vec3::new(0.5, 0.0, -3.0),
            Vec3::new(0.0, 0.0, 20.0),
            20.0,
        )
        .unwrap();
    // The player's own bomb in the same place is not deflected.
    let own = a
        .world
        .spawn_projectile(
            &t,
            ProjectileKind::Bomb,
            Owner::Player(p),
            eye + Vec3::new(-0.5, 0.0, -3.0),
            Vec3::new(0.0, 0.0, -1.0),
        )
        .unwrap();
    let speed = a.world.projectiles[&incoming].speed;
    a.select(p, BELLOWS, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    a.held.get_mut(&p).unwrap().yaw = 0.0;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let proj = &a.world.projectiles[&incoming];
    assert_eq!(proj.owner, Owner::Player(p));
    assert!(proj.parried);
    assert_eq!(proj.attack, Attack::Gust);
    assert!((proj.damage - 30.0).abs() < 1e-3, "x1.5: {}", proj.damage);
    assert!((proj.speed - speed * 1.25).abs() < 1e-3);
    assert!(
        proj.velocity.normalize().dot(Vec3::NEG_Z) > 0.999,
        "along the aim"
    );
    assert_eq!(a.world.projectiles[&own].owner, Owner::Player(p));
    assert!(!a.world.projectiles[&own].parried);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Deflected { player, id, .. }
        if *player == p && *id == incoming))
    );
    // DEFLECT style, no parry reward.
    assert!(ev.iter().any(|e| matches!(
        e,
        SimEvent::Style(StyleEvent::Bonus {
            bonus: StyleBonus::Deflect,
            ..
        })
    )));
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Parried(_))));
}

// ---- Arclash ----------------------------------------------------------------------------

#[test]
fn arc_zaps_and_chains_twice() {
    let (mut a, p, d) = range(
        &[
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::new(4.0, 0.0, -10.0),
            Vec3::new(8.0, 0.0, -10.0),
            Vec3::new(12.0, 0.0, -10.0),
        ],
        1000.0,
    );
    a.select(p, ARCLASH, 0);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::FIRE, 0);
    assert_eq!(dmg(&ev, d[0]), 6.0);
    assert_eq!(dmg(&ev, d[1]), 4.5);
    assert_eq!(dmg(&ev, d[2]), 3.375);
    assert_eq!(dmg(&ev, d[3]), 0.0, "two jumps at most");
    assert!(
        hits_on(&ev, HitTarget::Entity(d[1]))[0]
            .info
            .flags
            .contains(DamageFlags::CHAINED)
    );
    let chain = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::ArcChain { points, .. } => Some(points.clone()),
            _ => None,
        })
        .expect("chain event");
    assert_eq!(chain.len(), 3);
    // Held: a zap every 0.1 s.
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(60);
    let zaps = ev
        .iter()
        .filter(|e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Arc))
        .count();
    assert_eq!(zaps, 10);
}

#[test]
fn arc_chains_prefer_branded_and_need_line_of_sight() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([1.5, 0.0, -12.0], [2.0, 4.0, -8.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let first = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), PI, 1000.0);
    let walled = dummy(&mut a, Vec3::new(3.5, 0.0, -10.0), PI, 1000.0); // behind the wall
    let near = dummy(&mut a, Vec3::new(-3.0, 0.0, -10.0), PI, 1000.0);
    let branded = dummy(&mut a, Vec3::new(-1.0, 0.0, -15.5), PI, 1000.0);
    a.ticks(10);
    let t = a.tuning.clone();
    let at = dummy_body(&a, branded);
    a.world.brand_target(&t, p, HitTarget::Entity(branded), at);
    a.select(p, ARCLASH, 0);
    let body = dummy_body(&a, first);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::FIRE, 0);
    assert_eq!(dmg(&ev, walled), 0.0, "no chain through walls");
    // The branded one (farther) first, x1.35 ...
    assert!((dmg(&ev, branded) - 4.5 * 1.35).abs() < 1e-3);
    // ... then the next jump from there.
    assert_eq!(dmg(&ev, near), 3.375);
}

#[test]
fn wisp_drifts_zaps_and_pops() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-10.0, 0.0, -21.0], [10.0, 6.0, -20.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let in_path = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), PI, 1000.0);
    let beside = dummy(&mut a, Vec3::new(3.0, 0.0, -12.0), PI, 1000.0);
    a.ticks(10);
    a.select(p, ARCLASH, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Wisp))
    );
    let wisp = *a.world.projectiles.keys().next().unwrap();
    assert_eq!(a.world.projectiles[&wisp].kind, ProjectileKind::Wisp);
    assert!((a.world.projectiles[&wisp].velocity.length() - 12.0).abs() < 1e-3);
    let ev = a.ticks(150);
    // It passed through the dummy in its path (no pop there) ...
    let pops: Vec<Vec3> = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Explosion {
                position,
                attack: Attack::Wisp,
                ..
            } => Some(*position),
            _ => None,
        })
        .collect();
    assert_eq!(pops.len(), 1);
    assert!(pops[0].z < -19.0, "popped on the wall: {}", pops[0]);
    // ... zapping whatever was within 6 m every 0.2 s (8 each).
    let zaps = ev
        .iter()
        .filter(|e| matches!(e, SimEvent::WispZap { .. }))
        .count();
    assert!(zaps >= 6, "{zaps} zaps");
    let zapped = |id: NetId| {
        hits_on(&ev, HitTarget::Entity(id))
            .iter()
            .filter(|d| !d.info.flags.contains(DamageFlags::EXPLOSIVE))
            .map(|d| d.dealt)
            .sum::<f32>()
    };
    assert!(zapped(in_path) >= 16.0 && zapped(in_path) % 8.0 == 0.0);
    assert!(zapped(beside) >= 8.0 && zapped(beside) % 8.0 == 0.0);
    assert!(!a.world.projectiles.contains_key(&wisp));
}

#[test]
fn discharge_scales_with_charge_and_spares_the_shooter() {
    let (mut a, p, d) = range(
        &[
            Vec3::new(0.0, 0.0, -5.0),
            Vec3::new(7.5, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 9.0),
        ],
        1000.0,
    );
    a.select(p, ARCLASH, 1);
    // Half charge: radius 5.75, 52.5 damage, 18 m/s.
    a.set_buttons(p, Buttons::ALT_FIRE);
    let ev = a.ticks(30);
    assert!(
        !ev.iter().any(|e| matches!(e, SimEvent::Discharge { .. })),
        "charging"
    );
    assert_eq!(a.world.players[&p].combat.weapons.charge, 30);
    a.set_buttons(p, Buttons::NONE);
    let ev = a.tick();
    let radius = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Discharge { radius, charge, .. } => Some((*radius, *charge)),
            _ => None,
        })
        .expect("nova");
    assert!((radius.0 - 5.75).abs() < 1e-3 && (radius.1 - 0.5).abs() < 1e-3);
    assert!((dmg(&ev, d[0]) - 52.5).abs() < 1e-3);
    assert_eq!(dmg(&ev, d[1]), 0.0);
    assert_eq!(dmg(&ev, d[2]), 0.0);
    let kb = hits_on(&ev, HitTarget::Entity(d[0]))[0].info.knockback;
    assert!(kb.z < -15.0 && kb.y > 0.0, "outward: {kb}");
    assert_eq!(a.health(p), 100.0, "never hurts the shooter");
    assert!(hits_on(&ev, HitTarget::Player(p)).is_empty());
    // Full charge after the 2 s cooldown: radius 8, 80 damage.
    a.ticks(120);
    reset_dummy(&mut a, d[0], Vec3::new(0.0, 0.0, -5.0));
    a.set_buttons(p, Buttons::ALT_FIRE);
    a.ticks(90);
    a.set_buttons(p, Buttons::NONE);
    let ev = a.tick();
    assert_eq!(dmg(&ev, d[0]), 80.0);
    assert_eq!(dmg(&ev, d[1]), 80.0, "6.85 m from the padded surface");
    assert_eq!(dmg(&ev, d[2]), 0.0, "8.35 m: out");
    assert_eq!(a.health(p), 100.0);
}

// ---- Tumbler ----------------------------------------------------------------------------

#[test]
fn bombs_bounce_twice_and_explode_on_the_third_contact() {
    let (mut a, p, _) = range(&[], 1000.0);
    let t = a.tuning.clone();
    let bomb = a
        .world
        .spawn_projectile(
            &t,
            ProjectileKind::Bomb,
            Owner::Player(p),
            Vec3::new(0.0, 1.0, -10.0),
            Vec3::new(1.0, -4.0, 0.0),
        )
        .unwrap();
    let mut frame = NetFrame::new();
    a.world.replicate(&t, &mut frame);
    assert_eq!(frame.get(bomb).unwrap().state, 2, "bounces left");
    let mut bounces = 0;
    let mut exploded = None;
    for _ in 0..120 {
        for e in a.tick() {
            match e {
                SimEvent::ProjectileBounced { id, .. } if id == bomb => {
                    bounces += 1;
                    let mut frame = NetFrame::new();
                    a.world.replicate(&t, &mut frame);
                    assert_eq!(frame.get(bomb).unwrap().state, 2 - bounces);
                }
                SimEvent::Explosion {
                    position,
                    attack: Attack::Tumble,
                    ..
                } => exploded = Some((a.world.tick, position)),
                _ => {}
            }
        }
        if exploded.is_some() {
            break;
        }
    }
    assert_eq!(bounces, 2);
    let (_, at) = exploded.expect("exploded");
    assert!(at.y < 1.0, "on the floor: {at}");
}

#[test]
fn bombs_explode_on_hostiles_and_on_their_fuse() {
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -8.0)], 1000.0);
    a.select(p, TUMBLER, 0);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::FIRE, 20);
    let hit = hits_on(&ev, HitTarget::Entity(d[0]));
    assert_eq!(hit.len(), 1);
    assert!(
        hit[0]
            .info
            .flags
            .contains(DamageFlags::EXPLOSIVE | DamageFlags::DIRECT)
    );
    assert!(
        hit[0].dealt > 60.0 && hit[0].dealt <= 65.0,
        "{}",
        hit[0].dealt
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Tumble))
    );
    // Straight up (after the 0.85 s interval): the 1.6 s fuse goes off in the air.
    a.ticks(40);
    a.held.get_mut(&p).unwrap().pitch = 1.5;
    a.tap(p, Buttons::FIRE, 0);
    let fired = a.world.tick;
    let mut boom = None;
    for _ in 0..110 {
        for e in a.tick() {
            if let SimEvent::Explosion {
                position,
                attack: Attack::Tumble,
                ..
            } = e
            {
                boom = Some((a.world.tick, position));
            }
        }
        if boom.is_some() {
            break;
        }
    }
    let (tick, pos) = boom.expect("fuse");
    assert!(pos.y > 15.0, "fuse in the air: {pos}");
    assert_eq!(tick - fired, 96);
}

#[test]
fn breach_blasts_point_blank_and_launches_the_shooter() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-3.0, 0.0, -3.0], [3.0, 4.0, -2.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let behind = dummy(&mut a, Vec3::new(0.0, 0.0, -4.0), PI, 1000.0);
    a.ticks(5);
    a.select(p, TUMBLER, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let (at, radius) = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Explosion {
                position,
                radius,
                attack: Attack::Breach,
                ..
            } => Some((*position, *radius)),
            _ => None,
        })
        .expect("breach");
    assert!((at.z + 1.8).abs() < 0.02, "pulled back from the wall: {at}");
    assert_eq!(radius, 4.0);
    assert_eq!(dmg(&ev, behind), 0.0, "the wall shields it");
    // Self damage (35 %) and full knockback.
    let own = hits_on(&ev, HitTarget::Player(p));
    assert_eq!(own.len(), 1);
    assert!(
        own[0]
            .info
            .flags
            .contains(DamageFlags::SELF | DamageFlags::EXPLOSIVE)
    );
    assert!(own[0].dealt > 5.0 && own[0].dealt <= 70.0 * 0.35 + 1e-3);
    assert!(
        own[0].info.knockback.z > 3.0,
        "pushed back: {}",
        own[0].info.knockback
    );

    // The panic button: aimed at the floor it launches the shooter.
    let (mut a, p, _) = range(&[], 1000.0);
    a.select(p, TUMBLER, 0);
    a.held.get_mut(&p).unwrap().pitch = -1.5;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let own = hits_on(&ev, HitTarget::Player(p));
    assert!((own[0].dealt - 70.0 * 0.35).abs() < 0.5, "{}", own[0].dealt);
    let v = a.world.players[&p].movement.velocity;
    assert!(v.y > 15.0, "breach jump {v}");

    // In the open air it goes off 3 m out (2.8 after the pull-back), hitting what is there.
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -4.5)], 1000.0);
    a.select(p, TUMBLER, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.4;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    let at = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Explosion { position, .. } => Some(*position),
            _ => None,
        })
        .unwrap();
    assert!((at - a.eye(p)).length() > 2.7 && (at - a.eye(p)).length() < 2.9);
    assert!(dmg(&ev, d[0]) > 0.0);
}

#[test]
fn gravewell_lobs_sticks_pulls_and_implodes() {
    let (mut a, p, d) = range(&[Vec3::new(6.0, 0.0, -10.0)], 1000.0);
    // The lob: 20 m/s, 25 degrees above the aim.
    a.select(p, TUMBLER, 1);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Shot(s) if s.attack == Attack::Gravewell))
    );
    let lobbed = a.world.projectiles.values().next().unwrap();
    assert_eq!(lobbed.kind, ProjectileKind::Gravewell);
    let v = lobbed.velocity;
    assert!((v.length() - 20.0).abs() < 0.5);
    assert!((v.y.atan2(-v.z).to_degrees() - 25.0).abs() < 2.0, "{v}");
    a.world.projectiles.clear();

    // One dropped on the floor at (0, 0, -10): it sticks ...
    let t = a.tuning.clone();
    let well = a
        .world
        .spawn_projectile(
            &t,
            ProjectileKind::Gravewell,
            Owner::Player(p),
            Vec3::new(0.0, 0.6, -10.0),
            Vec3::new(0.0, -6.0, 0.0),
        )
        .unwrap();
    let ev = a.ticks(10);
    let stuck = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::GravewellStuck { id, position } if *id == well => Some(*position),
            _ => None,
        })
        .expect("stuck to the floor");
    assert!(a.world.projectiles[&well].stuck.is_some());
    // ... pulls the dummy 6 m away towards it for 1.5 s ...
    let x0 = a.world.enemies[&d[0]].position.x;
    let ev = a.ticks(60);
    let x1 = a.world.enemies[&d[0]].position.x;
    assert!(x1 < x0 - 2.5, "pulled from {x0} to {x1}");
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Explosion { .. })));
    // ... then implodes (80 at the centre, falloff) and is gone.
    let ev = a.ticks(30);
    let at = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Explosion {
                position,
                radius,
                attack: Attack::Gravewell,
                ..
            } => Some((*position, *radius)),
            _ => None,
        })
        .expect("implosion");
    assert!(at.0.distance(stuck) < 0.01 && at.1 == 5.0);
    assert!(dmg(&ev, d[0]) > 40.0, "{}", dmg(&ev, d[0]));
    assert!(!a.world.projectiles.contains_key(&well));
}

#[test]
fn gravewell_pulls_hostile_players_at_half_strength_in_pvp() {
    let mut a = Arena::pvp();
    let p0 = a.player(0, Vec3::new(0.0, 0.0, 10.0), 0.0);
    let p1 = a.player(1, Vec3::new(6.0, 0.0, -10.0), 0.0);
    a.ticks(5);
    let t = a.tuning.clone();
    a.world
        .spawn_projectile(
            &t,
            ProjectileKind::Gravewell,
            Owner::Player(p0),
            Vec3::new(0.0, 0.3, -10.0),
            Vec3::new(0.0, -6.0, 0.0),
        )
        .unwrap();
    a.ticks(3);
    let pl = &a.world.players[&p1];
    let (body, _, _) = pl.movement.capsule(&t.movement);
    let dir = (Vec3::new(0.0, 0.3, -10.0) - body).normalize();
    let along = pl.movement.velocity.dot(dir);
    assert!(along > 3.0 && along < 4.6, "half of 9 m/s: {along}");
    // The thrower isn't pulled by their own.
    assert!(a.world.players[&p0].movement.velocity.length() < 0.5);
}

// ---- Hit assist ---------------------------------------------------------------------------

#[test]
fn hit_assist_pads_player_attacks_only() {
    // Quill: 0.15 m past a dummy's body still hits (the body pad, 0.35), 0.1 m beyond the pad
    // misses.
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -15.0)], 1000.0);
    let pad = a.tuning.combat.hit_assist.enemy_body_pad;
    assert!(pad > 0.15);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body + Vec3::X * (DUMMY_RADIUS + 0.15));
    let ev = a.tap(p, Buttons::FIRE, 20);
    assert_eq!(dmg(&ev, d[0]), 24.0, "near miss caught by the pad");
    a.aim(p, body + Vec3::X * (DUMMY_RADIUS + pad + 0.1));
    let ev = a.tap(p, Buttons::FIRE, 20);
    assert_eq!(dmg(&ev, d[0]), 0.0);
    // Fist: 2.7 m to the body surface (range 2.6) lands thanks to the pad.
    let (mut a, p, d) = range(&[Vec3::new(0.0, 0.0, -(2.7 + DUMMY_RADIUS))], 1000.0);
    let body = dummy_body(&a, d[0]);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::PUNCH, 1);
    assert_eq!(dmg(&ev, d[0]), 25.0);
    // Explosions: 0.15 m beyond the radius still counts for a player's blast ...
    let t = a.tuning.clone();
    let center = dummy_body(&a, d[0]) + Vec3::X * (DUMMY_RADIUS + 4.0 + 0.15);
    let spec = Explosive {
        radius: 4.0,
        damage: 50.0,
        knockback: 0.0,
    };
    let before = a.enemy_health(d[0]);
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
    assert!(a.enemy_health(d[0]) < before);

    // PvP: 0.1 m past a player's body hits (pad 0.12), 0.2 m doesn't.
    let mut a = Arena::pvp();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -12.0), PI);
    a.ticks(5);
    let radius = a.tuning.movement.radius;
    let chest = a.world.players[&p1].movement.position + Vec3::Y * 0.7;
    a.aim(p0, chest + Vec3::X * (radius + 0.1));
    a.tap(p0, Buttons::FIRE, 20);
    assert_eq!(a.health(p1), 76.0);
    a.aim(p0, chest + Vec3::X * (radius + 0.2));
    a.tap(p0, Buttons::FIRE, 20);
    assert_eq!(a.health(p1), 76.0);
}

#[test]
fn enemy_attacks_use_the_plain_hitboxes() {
    let (mut a, p, d) = range(&[Vec3::new(30.0, 0.0, 30.0)], 1000.0);
    let t = a.tuning.clone();
    let kind = enemy_projectile_kind();
    let r = kind.radius(&t.combat);
    let radius = t.movement.radius;
    let chest = a.world.players[&p].movement.position + Vec3::Y * 0.7;
    // An enemy projectile passing 0.08 m beside the player's body: no hit (a player-owned
    // one would have hit with the 0.12 m pad).
    let start = chest + Vec3::new(radius + r + 0.08, 0.0, -3.0);
    a.world.spawn_enemy_projectile(
        &t,
        kind,
        d[0],
        Attack::Enemy {
            kind: 60,
            attack: 0,
        },
        start,
        Vec3::new(0.0, 0.0, 60.0),
        20.0,
    );
    let ev = a.ticks(10);
    assert!(hits_on(&ev, HitTarget::Player(p)).is_empty());
    assert_eq!(a.health(p), 100.0);
    // An enemy explosion 0.05 m beyond its radius from the player: nothing.
    let center = chest + Vec3::X * (radius + 3.0 + 0.05);
    let spec = Explosive {
        radius: 3.0,
        damage: 50.0,
        knockback: 10.0,
    };
    a.world.explode(
        &t,
        center,
        spec,
        Owner::Enemy(d[0]),
        Attack::Enemy {
            kind: 60,
            attack: 0,
        },
        DamageFlags::NONE,
        None,
        None,
        Owner::Enemy(d[0]),
    );
    assert_eq!(a.health(p), 100.0);
    // While a player's bomb passing 0.12 m beside a dummy (pad 0.2) explodes on it.
    let body = dummy_body(&a, d[0]);
    let br = ProjectileKind::Bomb.radius(&t.combat);
    let start = body + Vec3::new(DUMMY_RADIUS + br + 0.12, 0.0, -2.0);
    let bomb = a
        .world
        .spawn_projectile(
            &t,
            ProjectileKind::Bomb,
            Owner::Player(p),
            start,
            Vec3::new(0.0, 0.0, 40.0),
        )
        .unwrap();
    let ev = a.ticks(4);
    assert!(!a.world.projectiles.contains_key(&bomb));
    let hit = hits_on(&ev, HitTarget::Entity(d[0]));
    assert!(
        hit.iter()
            .any(|h| h.info.flags.contains(DamageFlags::DIRECT))
    );
}

// ---- Rules shared by all weapons ------------------------------------------------------------

#[test]
fn no_friendly_fire_in_coop_but_knockback_applies() {
    let mut a = Arena::coop();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -3.5), PI);
    a.ticks(5);
    let target = a.world.players[&p1].movement.position + Vec3::Y;
    a.aim(p0, target);
    // Hitscan passes through teammates.
    let ev = a.tap(p0, Buttons::FIRE, 2);
    assert_eq!(damage_to(&ev, HitTarget::Player(p1)), 0.0);
    // A Breach in their face (it passes through them too): no damage, but they fly.
    a.select(p0, TUMBLER, 0);
    let ev = a.tap(p0, Buttons::ALT_FIRE, 2);
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
        48.0,
        "PvP headshot x2"
    );
    // PvP damage spawns an ember burst the attacker can draw in.
    assert!(!a.world.embers.is_empty());
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
    assert_eq!(a.health(p1), 76.0);
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
    // A big ember burst right here: heals only up to the effective max.
    a.world
        .spawn_embers(&t, Vec3::new(0.0, 1.0, 0.0), 100.0, 3.5, 0.6, true);
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
fn punch_burst_heals_the_closest_player() {
    let mut a = Arena::coop();
    let near = a.player(0, Vec3::new(0.0, 0.0, 0.0), 0.0);
    let far = a.player(1, Vec3::new(2.5, 0.0, 0.5), 0.0);
    a.world.players.get_mut(&near).unwrap().notarget = true;
    a.world.players.get_mut(&far).unwrap().notarget = true;
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -1.8), PI, 1000.0);
    a.ticks(10);
    let t = a.tuning.clone();
    for id in [near, far] {
        // 60 damage, 21 of it hard: room for 39 HP of healing.
        let info = DamageInfo::new(60.0, Owner::World, Attack::Dev);
        a.world.apply_damage(&t, HitTarget::Player(id), info);
    }
    // Punch: 25 damage, 100 % of it becomes a heal pool at the hit point.
    let body = dummy_body(&a, d);
    a.aim(near, body);
    let ev = a.tap(near, Buttons::PUNCH, 3);
    assert_eq!(dmg(&ev, d), 25.0);
    assert_eq!(
        a.health(near),
        65.0,
        "closest player draws in the 25 HP pool"
    );
    assert_eq!(a.health(far), 40.0, "the other player gets nothing");
}

#[test]
fn slam_shockwave_hits_nearby_enemies() {
    let (mut a, p, d) = range(
        &[Vec3::new(3.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -12.0)],
        1000.0,
    );
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
    assert!(dmg(&ev, d[0]) > 10.0, "near dummy hit");
    assert_eq!(dmg(&ev, d[1]), 0.0, "far dummy untouched");
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

// ---- Prediction -------------------------------------------------------------------------------

#[test]
fn server_weapon_state_is_exactly_the_predicted_one() {
    // Every mode, switches, charges, punches and thorns: the server's weapon state is what the
    // client gets by running `step_weapons` on its own inputs (nothing server-side touches it).
    let (mut a, p, _) = range(
        &[Vec3::new(0.0, 0.0, -12.0), Vec3::new(3.0, 0.0, -9.0)],
        100_000.0,
    );
    // Own blasts must not kill the shooter halfway (a dead player's weapons only tick).
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let t = a.tuning.clone();
    let mut predicted = a.world.players[&p].combat.weapons;
    for i in 0..900u32 {
        let phase = i / 75;
        let input = a.held.get_mut(&p).unwrap();
        input.weapon = (phase % 4) as u8;
        input.weapon_mode = ((phase / 4) % 2) as u8;
        input.yaw = (i as f32 * 0.013).sin() * 0.4;
        let mut b = Buttons::NONE;
        b.set(Buttons::FIRE, i % 9 < 5);
        b.set(Buttons::ALT_FIRE, i % 40 < 30);
        b.set(Buttons::PUNCH, i % 50 == 0);
        b.set(Buttons::THROW, i % 70 == 3);
        input.buttons = b;
        let sent: PlayerInput = *input;
        a.tick();
        step_weapons(&mut predicted, &sent, true, &t.combat, 60);
        assert_eq!(a.world.players[&p].combat.weapons, predicted, "input {i}");
    }
    assert_ne!(predicted, WeaponState::default());
}
