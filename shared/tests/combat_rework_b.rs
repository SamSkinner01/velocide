//! The rework's arsenal-b abilities in the world (docs/rework.md §1, docs/arsenal.md): Stoke
//! (the faster fire, the Burn on hit, the overheat, the Kiln's reach, predicted exactly), the
//! Kite (steered by the crosshair, capped turning, straight once let go, its blast), the Bolas
//! (a bind, a cord between two hostiles sharing damage one way, never to the source, slack
//! after its time), the Wisp (orbiting and zapping, gone quietly, beside a wall, flung into a
//! pop, the cooldown from when it is gone), the Lantern (settling, the carrier's primary at
//! half rate and 0.6× damage for each Rapid weapon, one at a time, out after its time, lost
//! over a pit, broken by a hostile blast) and the Bulwark's slab (hostile projectiles and rays
//! stopped from the front, nothing behind it hit, your own shots pass, from behind it doesn't
//! help, shattering into the longer cooldown).

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::LevelKind;
use shared::loadout::{Ability, Loadout, WeaponBuild, WeaponKind};
use shared::protocol::{NetFrame, NetId, enemy_flags, kind, player_flags, projectile_flags};
use shared::sim::projectiles::Explosive;
use shared::sim::{
    Attack, DamageEvent, DamageFlags, DamageInfo, FireIntent, HitPart, HitTarget, Owner, PlayerId,
    ProjectileKind, SimEvent, step_weapons,
};
use weapon_support::*;

const BIG: f32 = 1.0e5;

/// Player `p` carries `weapon` with `abilities` alone (god mode: nothing hurts it).
fn arm(a: &mut Arena, p: PlayerId, weapon: WeaponKind, abilities: [Ability; 2]) {
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let build = WeaponBuild { weapon, abilities };
    assert!(build.is_valid(), "{build:?}");
    a.world.force_loadout(p, Some(Loadout::single(build)));
}

fn armed(weapon: WeaponKind, abilities: [Ability; 2]) -> (Arena, PlayerId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    arm(&mut a, p, weapon, abilities);
    (a, p)
}

fn damage_events(events: &[SimEvent]) -> Vec<DamageEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) => Some(*d),
            _ => None,
        })
        .collect()
}

fn hits_on(events: &[SimEvent], target: HitTarget, attack: Attack) -> Vec<DamageEvent> {
    damage_events(events)
        .into_iter()
        .filter(|d| d.target == target && d.info.attack == attack)
        .collect()
}

fn shots(events: &[SimEvent], attack: Attack) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, SimEvent::Shot(s) if s.attack == attack))
        .count()
}

fn explosions(events: &[SimEvent], attack: Attack) -> Vec<Vec3> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Explosion {
                position,
                attack: a,
                ..
            } if *a == attack => Some(*position),
            _ => None,
        })
        .collect()
}

fn entity(a: &Arena, id: NetId) -> Option<shared::protocol::EntityState> {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id)
}

/// Holds `buttons` for `ticks`, then releases them for `after` ticks.
fn hold(a: &mut Arena, p: PlayerId, buttons: Buttons, ticks: u32, after: u32) -> Vec<SimEvent> {
    a.set_buttons(p, buttons);
    let mut ev = a.ticks(ticks);
    a.set_buttons(p, Buttons::NONE);
    ev.extend(a.ticks(after));
    ev
}

fn projectiles_of(a: &Arena, kind: ProjectileKind) -> Vec<NetId> {
    a.world
        .projectiles
        .values()
        .filter(|p| p.kind == kind)
        .map(|p| p.id)
        .collect()
}

// ---- Stoke ------------------------------------------------------------------------------------

#[test]
fn stoke_fires_faster_burns_what_it_hits_and_overheats() {
    let (mut a, p) = armed(WeaponKind::Riveter, [Ability::Stoke, Ability::Lantern]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let c = a.tuning.combat.clone();
    // A second of plain fire: 12 rivets (5 ticks apart), no Burn.
    let ev = hold(&mut a, p, Buttons::FIRE, 60, 20);
    assert_eq!(shots(&ev, Attack::Riveter), 12);
    assert!(
        a.world
            .statuses
            .get(&HitTarget::Entity(d))
            .is_none_or(|s| s.burn.is_none())
    );
    // Stoked: 3 ticks apart (×1.6), every hit +1 Burn stack as the Stoke.
    a.tap(p, Buttons::ALT_FIRE, 0);
    let e = entity(&a, NetId::player(p)).unwrap();
    assert!(e.flags & player_flags::STOKED != 0);
    let ev = hold(&mut a, p, Buttons::FIRE, 60, 0);
    assert_eq!(shots(&ev, Attack::Riveter), 20);
    let burn = a.world.statuses[&HitTarget::Entity(d)]
        .burn
        .expect("burning");
    assert_eq!(burn.attack, Attack::Stoke);
    assert_eq!(burn.source, Owner::Player(p));
    assert!(burn.stacks as u32 >= c.stoke.burn_stacks);
    // To the end of the Stoke, then overheated: no primary for 1.5 s.
    let left = (c.stoke.duration * 60.0) as u32 - 61;
    a.set_buttons(p, Buttons::FIRE);
    a.ticks(left + 1);
    let e = entity(&a, NetId::player(p)).unwrap();
    assert!(e.flags & player_flags::OVERHEATED != 0, "{:x}", e.flags);
    let ev = a.ticks((c.stoke.overheat * 60.0) as u32 - 2);
    assert_eq!(shots(&ev, Attack::Riveter), 0, "venting");
    let ev = a.ticks(10);
    a.set_buttons(p, Buttons::NONE);
    assert!(shots(&ev, Attack::Riveter) > 0, "cooled");
}

#[test]
fn stoke_is_predicted_exactly() {
    let (mut a, p) = armed(WeaponKind::Riveter, [Ability::Stoke, Ability::Lantern]);
    dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), 0.0, BIG);
    let t = a.tuning.clone();
    a.tick();
    let mut predicted = a.world.players[&p].combat.weapons;
    for i in 0..900u32 {
        let input = a.held.get_mut(&p).unwrap();
        let mut b = Buttons::NONE;
        b.set(Buttons::FIRE, i % 50 < 40);
        b.set(Buttons::ALT_FIRE, i % 97 < 3);
        input.buttons = b;
        let sent: PlayerInput = *input;
        a.tick();
        step_weapons(&mut predicted, &sent, true, &t.combat, 60);
        assert_eq!(a.world.players[&p].combat.weapons, predicted, "input {i}");
    }
}

#[test]
fn a_stoked_kiln_reaches_further_instead_of_burning_twice() {
    let (mut a, p) = armed(WeaponKind::Kiln, [Ability::Stoke, Ability::Lantern]);
    let c = a.tuning.combat.clone();
    // Beyond the spray's reach, inside the stoked one.
    let at = c.kiln.range * (1.0 + c.stoke.kiln_reach) * 0.5 + DUMMY_RADIUS;
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -at), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let ev = hold(&mut a, p, Buttons::FIRE, 30, 5);
    assert!(hits_on(&ev, HitTarget::Entity(d), Attack::Kiln).is_empty());
    a.tap(p, Buttons::ALT_FIRE, 0);
    let ev = hold(&mut a, p, Buttons::FIRE, 30, 5);
    assert!(!hits_on(&ev, HitTarget::Entity(d), Attack::Kiln).is_empty());
    let burn = a.world.statuses[&HitTarget::Entity(d)].burn.unwrap();
    assert_eq!(burn.attack, Attack::Kiln, "its own Burn, not the Stoke's");
}

// ---- Kite -------------------------------------------------------------------------------------

/// Launches a kite (Cinderbox, ability B) along the current aim; its id.
fn launch_kite(a: &mut Arena, p: PlayerId) -> NetId {
    a.select(p, 0, 1);
    a.set_buttons(p, Buttons::ALT_FIRE);
    a.tick();
    *projectiles_of(a, ProjectileKind::Kite)
        .first()
        .expect("a kite")
}

#[test]
fn the_kite_follows_the_crosshair_while_alt_is_held() {
    let (mut a, p) = armed(WeaponKind::Cinderbox, [Ability::Limpet, Ability::Kite]);
    let d = dummy(&mut a, Vec3::new(14.0, 0.0, -14.0), 0.0, BIG);
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 1.6, -20.0));
    let kite = launch_kite(&mut a, p);
    assert_ne!(
        entity(&a, kite).unwrap().flags & projectile_flags::GUIDED,
        0,
        "steered"
    );
    // ALT stays down; the crosshair moves onto the dummy: the kite turns, at most 120°/s.
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let before = a.world.projectiles[&kite].velocity.normalize();
    a.tick();
    let after = a.world.projectiles[&kite].velocity.normalize();
    let turned = before.angle_between(after).to_degrees();
    let cap = a.tuning.combat.kite.turn_degrees / 60.0;
    assert!(turned > 0.5 && turned <= cap + 1e-3, "{turned}° vs {cap}°");
    let mut ev = Vec::new();
    for _ in 0..240 {
        ev.extend(a.tick());
        if !a.world.projectiles.contains_key(&kite) {
            break;
        }
    }
    let blast = explosions(&ev, Attack::Kite);
    assert_eq!(blast.len(), 1, "it went off");
    assert!(blast[0].distance(body) < 1.5, "at the dummy: {}", blast[0]);
    let hits = hits_on(&ev, HitTarget::Entity(d), Attack::Kite);
    assert!(hits[0].info.amount > a.tuning.combat.kite.damage * 0.8);
}

#[test]
fn a_released_kite_flies_straight() {
    let (mut a, p) = armed(WeaponKind::Cinderbox, [Ability::Limpet, Ability::Kite]);
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 1.6, -20.0));
    let kite = launch_kite(&mut a, p);
    let v0 = a.world.projectiles[&kite].velocity;
    a.set_buttons(p, Buttons::NONE);
    a.aim(p, Vec3::new(20.0, 1.6, 0.0));
    a.ticks(30);
    let k = &a.world.projectiles[&kite];
    assert!(k.velocity.normalize().dot(v0.normalize()) > 0.9999);
    assert_eq!(
        entity(&a, kite).unwrap().flags & projectile_flags::GUIDED,
        0
    );
    // Pressing again later doesn't take it back.
    a.set_buttons(p, Buttons::ALT_FIRE);
    a.ticks(10);
    let k = &a.world.projectiles[&kite];
    assert!(k.velocity.normalize().dot(v0.normalize()) > 0.9999);
}

// ---- Bolas ------------------------------------------------------------------------------------

#[test]
fn the_bolas_binds_two_hostiles_on_a_cord_that_shares_damage() {
    let (mut a, p) = armed(WeaponKind::Thresher, [Ability::Bolas, Ability::Flak]);
    let first = dummy(&mut a, Vec3::new(0.0, 0.0, -8.0), 0.0, BIG);
    let second = dummy(&mut a, Vec3::new(3.0, 0.0, -9.0), 0.0, BIG);
    let far = dummy(&mut a, Vec3::new(-12.0, 0.0, -9.0), 0.0, BIG);
    a.ticks(2);
    let body = dummy_body(&a, first);
    a.aim(p, body + Vec3::Y * 0.3);
    let ev = a.tap(p, Buttons::ALT_FIRE, 30);
    let bound = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::BolasBound { positions, .. } => Some(positions.clone()),
            _ => None,
        })
        .expect("it bound");
    assert_eq!(bound.len(), 2, "a cord to the second");
    for d in [first, second] {
        let target = HitTarget::Entity(d);
        assert!(a.world.is_bound(target) && a.world.is_pinned(target));
        assert_ne!(entity(&a, d).unwrap().flags & enemy_flags::BOUND, 0);
    }
    assert!(!a.world.is_bound(HitTarget::Entity(far)));
    // 60 % of what one end takes goes to the other, one way only.
    let t = a.tuning.clone();
    let hit = |a: &mut Arena, target: NetId| {
        let at = dummy_body(a, target);
        let info = DamageInfo::new(40.0, Owner::Player(p), Attack::Thresher)
            .at(at, Vec3::NEG_Z)
            .part(HitPart::Body)
            .flags(DamageFlags::HITSCAN);
        let before = [first, second].map(|d| a.enemy_health(d));
        a.world.apply_damage(&t, HitTarget::Entity(target), info);
        let after = [first, second].map(|d| a.enemy_health(d));
        [before[0] - after[0], before[1] - after[1]]
    };
    let share = t.combat.bolas.link_share;
    let [d1, d2] = hit(&mut a, first);
    assert!(
        (d1 - 40.0).abs() < 1e-3 && (d2 - 40.0 * share).abs() < 1e-3,
        "{d1} {d2}"
    );
    let [d1, d2] = hit(&mut a, second);
    assert!(
        (d2 - 40.0).abs() < 1e-3 && (d1 - 40.0 * share).abs() < 1e-3,
        "{d1} {d2}"
    );
    // Slack after its time.
    a.ticks((t.combat.bolas.link_time * 60.0) as u32);
    let [_, d2] = hit(&mut a, first);
    assert_eq!(d2, 0.0);
    assert!(!a.world.is_bound(HitTarget::Entity(second)));
}

#[test]
fn a_lone_bind_and_never_back_to_the_source() {
    // Alone: bound and pinned, no cord.
    let (mut a, p) = armed(WeaponKind::Thresher, [Ability::Bolas, Ability::Flak]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -8.0), 0.0, BIG);
    a.ticks(2);
    let body = dummy_body(&a, d);
    a.aim(p, body + Vec3::Y * 0.3);
    let ev = a.tap(p, Buttons::ALT_FIRE, 30);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::BolasBound { positions, .. } if positions.len() == 1))
    );
    assert!(a.world.is_bound(HitTarget::Entity(d)));
    assert_eq!(a.world.cord_partner(HitTarget::Entity(d)), None);

    // PvP: two opponents on a cord; one hurting the other isn't shared back onto itself.
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Thresher,
        [Ability::Bolas, Ability::Flak],
    );
    let b = a.player(1, Vec3::new(0.0, 0.0, -8.0), PI);
    let c = a.player(2, Vec3::new(2.0, 0.0, -9.0), PI);
    for id in [b, c] {
        a.world.players.get_mut(&id).unwrap().combat.protection = 0;
    }
    a.ticks(2);
    let chest = a.world.players[&b].movement.position + Vec3::Y * 1.1;
    a.aim(p, chest);
    a.tap(p, Buttons::ALT_FIRE, 30);
    assert_eq!(
        a.world.cord_partner(HitTarget::Player(b)),
        Some(HitTarget::Player(c))
    );
    assert_ne!(
        entity(&a, NetId::player(b)).unwrap().flags & player_flags::BOUND,
        0
    );
    let t = a.tuning.clone();
    let at = a.world.players[&c].movement.position + Vec3::Y;
    let info = DamageInfo::new(30.0, Owner::Player(b), Attack::Quill)
        .at(at, Vec3::X)
        .flags(DamageFlags::HITSCAN);
    let before = a.health(b);
    a.world.apply_damage(&t, HitTarget::Player(c), info);
    assert_eq!(a.health(b), before, "not back to the one who dealt it");
}

// ---- Wisp -------------------------------------------------------------------------------------

fn the_wisp(a: &Arena) -> NetId {
    *projectiles_of(a, ProjectileKind::Wisp)
        .first()
        .expect("a wisp")
}

#[test]
fn the_wisp_orbits_its_caster_zapping_then_goes_quietly() {
    let (mut a, p) = armed(WeaponKind::Arclash, [Ability::Wisp, Ability::Discharge]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -5.0), 0.0, BIG);
    a.ticks(2);
    a.tap(p, Buttons::ALT_FIRE, 0);
    let wisp = the_wisp(&a);
    let e = entity(&a, wisp).unwrap();
    assert_ne!(e.flags & projectile_flags::ORBITING, 0);
    let c = a.tuning.combat.arclash.clone();
    let check_orbit = |a: &Arena| {
        let feet = a.world.players[&p].movement.position;
        let w = a.world.projectiles[&wisp].position;
        let flat = Vec3::new(w.x - feet.x, 0.0, w.z - feet.z).length();
        assert!((flat - c.wisp_orbit_radius).abs() < 0.05, "{flat}");
        assert!((w.y - feet.y - c.wisp_orbit_height).abs() < 0.05);
    };
    check_orbit(&a);
    // It goes round, and with its caster.
    let p0 = a.world.projectiles[&wisp].position;
    a.held.get_mut(&p).unwrap().forward = 127;
    let ev = a.ticks(60);
    a.held.get_mut(&p).unwrap().forward = 0;
    check_orbit(&a);
    assert!(a.world.projectiles[&wisp].position.distance(p0) > 1.0);
    // Zapping the dummy every 0.25 s for 9.
    let zaps = hits_on(&ev, HitTarget::Entity(d), Attack::Wisp);
    assert!((3..=4).contains(&zaps.len()), "{}", zaps.len());
    assert!(zaps.iter().all(|z| z.info.amount == c.wisp_zap_damage));
    // Gone at the end of its orbit, without a pop; the cooldown runs from then.
    let mut ev = Vec::new();
    for _ in 0..(c.wisp_orbit_time * 60.0) as u32 {
        ev.extend(a.tick());
        if !a.world.projectiles.contains_key(&wisp) {
            break;
        }
    }
    assert!(!a.world.projectiles.contains_key(&wisp));
    assert!(explosions(&ev, Attack::Wisp).is_empty());
    let w = a.world.players[&p].combat.weapons;
    let cd = w.alt_cooldown[w.slot as usize] as f32 / 60.0;
    assert!(cd > c.wisp_cooldown - 0.2 && cd <= c.wisp_cooldown, "{cd}");
}

#[test]
fn an_orbit_beside_a_wall_never_pops() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-5.0, 0.0, -2.0], [5.0, 4.0, -1.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    arm(
        &mut a,
        p,
        WeaponKind::Arclash,
        [Ability::Wisp, Ability::Discharge],
    );
    a.ticks(2);
    a.tap(p, Buttons::ALT_FIRE, 0);
    let wisp = the_wisp(&a);
    let mut closest = f32::MAX;
    let mut ev = Vec::new();
    for _ in 0..300 {
        ev.extend(a.tick());
        if let Some(w) = a.world.projectiles.get(&wisp) {
            // In front of the wall's face (z = -1).
            closest = closest.min(w.position.z + 1.0);
        }
    }
    assert!(explosions(&ev, Attack::Wisp).is_empty());
    assert!(closest >= 0.0, "kept out of the wall: {closest}");
}

#[test]
fn a_flung_wisp_pops_on_what_it_hits_and_its_cooldown_starts_then() {
    let (mut a, p) = armed(WeaponKind::Arclash, [Ability::Wisp, Ability::Discharge]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -12.0), 0.0, BIG);
    a.ticks(2);
    a.tap(p, Buttons::ALT_FIRE, 30);
    let wisp = the_wisp(&a);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let mut ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(
        entity(&a, wisp).unwrap().flags & projectile_flags::ORBITING,
        0
    );
    let c = a.tuning.combat.arclash.clone();
    let v = a.world.projectiles[&wisp].velocity;
    assert!((v.length() - c.wisp_speed).abs() < 1e-3);
    for _ in 0..90 {
        ev.extend(a.tick());
        if !a.world.projectiles.contains_key(&wisp) {
            break;
        }
    }
    let pops = explosions(&ev, Attack::Wisp);
    assert_eq!(pops.len(), 1);
    let hit = hits_on(&ev, HitTarget::Entity(d), Attack::Wisp)
        .into_iter()
        .find(|h| h.info.flags.contains(DamageFlags::EXPLOSIVE))
        .expect("the pop");
    assert!(hit.info.flags.contains(DamageFlags::DIRECT));
    assert!(hit.info.amount > c.wisp_pop_damage * 0.9);
    // The cooldown from the pop (the server re-based the predicted one).
    let w = a.world.players[&p].combat.weapons;
    let cd = w.alt_cooldown[w.slot as usize] as f32 / 60.0;
    assert!(cd > c.wisp_cooldown - 0.1 && cd <= c.wisp_cooldown, "{cd}");
}

// ---- Lantern ----------------------------------------------------------------------------------

/// Tosses the lantern (ability `mode`) at the floor a few metres ahead; ticks until it stands.
fn toss_lantern(a: &mut Arena, p: PlayerId, mode: u8) -> NetId {
    let feet = a.world.players[&p].movement.position;
    a.aim(p, feet + Vec3::new(0.0, 0.0, -3.0));
    a.select(p, 0, mode);
    a.tap(p, Buttons::ALT_FIRE, 0);
    for _ in 0..120 {
        if let Some(l) = a.world.abilities.lantern.of(p) {
            return l.id;
        }
        a.tick();
    }
    panic!("no lantern");
}

#[test]
fn the_lantern_fires_the_carriers_primary_at_half_rate_and_less_damage() {
    let c = shared::tuning::Tuning::default().combat;
    let scale = c.lantern.damage_scale;
    for (weapon, abilities, mode, damage, interval) in [
        (
            WeaponKind::Arclash,
            [Ability::Wisp, Ability::Lantern],
            1,
            c.arclash.zap_damage,
            c.arclash.zap_interval,
        ),
        (
            WeaponKind::Riveter,
            [Ability::Stoke, Ability::Lantern],
            1,
            c.riveter.damage,
            c.riveter.interval,
        ),
        (
            WeaponKind::Kiln,
            [Ability::Lantern, Ability::Discharge],
            0,
            c.kiln.damage,
            c.kiln.interval,
        ),
    ] {
        let (mut a, p) = armed(weapon, abilities);
        a.ticks(2);
        let lantern = toss_lantern(&mut a, p, mode);
        let e = entity(&a, lantern).expect("replicated");
        assert_eq!(e.kind, kind::LANTERN);
        assert_eq!(e.state, weapon.id() as u16);
        let at = a.world.abilities.lantern.of(p).unwrap().position;
        assert!(at.y.abs() < 0.1, "on the floor: {at}");
        // A dummy in range (inside the Kiln lantern's cone too).
        let d = dummy(&mut a, at + Vec3::new(0.0, 0.0, -6.0), 0.0, BIG);
        let ev = a.ticks(120);
        let fired = ev
            .iter()
            .filter(|e| matches!(e, SimEvent::LanternShot { lantern: l, .. } if *l == lantern))
            .count() as f32;
        let want = 2.0 / (interval / c.lantern.rate_scale);
        assert!(
            (fired - want).abs() <= 1.5,
            "{weapon:?}: {fired} shots vs {want}"
        );
        // (The Kiln's Burn ticks aside.)
        let hits: Vec<_> = hits_on(&ev, HitTarget::Entity(d), Attack::Lantern)
            .into_iter()
            .filter(|h| !h.info.flags.contains(DamageFlags::BURN))
            .collect();
        assert!(!hits.is_empty(), "{weapon:?}");
        assert!(
            hits.iter()
                .all(|h| (h.info.amount - damage * scale).abs() < 1e-3),
            "{weapon:?}: {}",
            hits[0].info.amount
        );
        // Out after its time.
        a.ticks((c.lantern.duration * 60.0) as u32);
        assert!(a.world.abilities.lantern.of(p).is_none(), "{weapon:?}");
        assert!(entity(&a, lantern).is_none());
    }
}

#[test]
fn one_lantern_at_a_time_none_over_a_pit_and_a_blast_breaks_it() {
    let (mut a, p) = armed(WeaponKind::Arclash, [Ability::Wisp, Ability::Lantern]);
    a.ticks(2);
    let first = toss_lantern(&mut a, p, 1);
    let slot = a.world.players[&p].combat.weapons.slot as usize;
    a.world
        .players
        .get_mut(&p)
        .unwrap()
        .combat
        .weapons
        .alt_cooldown[slot] = 0;
    a.ticks(2);
    let feet = a.world.players[&p].movement.position;
    a.aim(p, feet + Vec3::new(4.0, 0.0, -3.0));
    a.tap(p, Buttons::ALT_FIRE, 60);
    assert_eq!(a.world.abilities.lantern.by_id.len(), 1);
    let second = a.world.abilities.lantern.of(p).unwrap().id;
    assert_ne!(first, second, "the new one replaced it");
    // Nothing in range: it waits.
    let far = dummy(&mut a, Vec3::new(0.0, 0.0, -40.0), 0.0, BIG);
    let ev = a.ticks(30);
    assert!(hits_on(&ev, HitTarget::Entity(far), Attack::Lantern).is_empty());
    // A Mould's blast breaks it.
    let t = a.tuning.clone();
    let at = a.world.abilities.lantern.of(p).unwrap().position + Vec3::Y * 0.5;
    let spec = Explosive {
        radius: 3.0,
        damage: 200.0,
        knockback: 0.0,
    };
    let enemy = Owner::Enemy(far);
    let attack = Attack::Enemy { kind: 0, attack: 0 };
    a.world.explode(
        &t,
        at,
        spec,
        enemy,
        attack,
        DamageFlags::NONE,
        None,
        None,
        enemy,
    );
    a.tick();
    assert!(a.world.abilities.lantern.of(p).is_none(), "broken");

    // Over a pit: lost.
    let mut level = flat_level(LevelKind::Campaign, Vec::new());
    level.brushes = vec![
        brush([-100.0, -1.0, -100.0], [100.0, 0.0, -8.0]),
        brush([-100.0, -1.0, -1.0], [100.0, 0.0, 100.0]),
    ];
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    arm(
        &mut a,
        p,
        WeaponKind::Arclash,
        [Ability::Wisp, Ability::Lantern],
    );
    a.ticks(2);
    a.aim(p, Vec3::new(0.0, 0.0, -3.5));
    a.select(p, 0, 1);
    a.tap(p, Buttons::ALT_FIRE, 240);
    assert!(a.world.abilities.lantern.of(p).is_none());
}

// ---- Bulwark ----------------------------------------------------------------------------------

/// PvP: `a` holds up a Bulwark facing −Z at the origin; `b` stands 15 m in front with
/// `weapon`. Returns (holder, shooter).
fn bulwark_duel(weapon: WeaponKind) -> (Arena, PlayerId, PlayerId) {
    let mut a = Arena::pvp();
    let holder = a.player(0, Vec3::ZERO, 0.0);
    let shooter = a.player(1, Vec3::new(0.0, 0.0, -15.0), PI);
    let build = WeaponBuild {
        weapon: WeaponKind::Bellows,
        abilities: [Ability::Gust, Ability::Bulwark],
    };
    a.world.force_loadout(holder, Some(Loadout::single(build)));
    a.world
        .force_loadout(shooter, Some(Loadout::single(weapon.default_build())));
    for id in [holder, shooter] {
        a.world.players.get_mut(&id).unwrap().combat.protection = 0;
    }
    a.ticks(2);
    a.select(holder, 0, 1);
    a.set_buttons(holder, Buttons::ALT_FIRE);
    a.tick();
    assert!(a.world.guard(holder).is_some(), "raised");
    (a, holder, shooter)
}

#[test]
fn the_slab_stops_hostile_rivets_and_rays_from_the_front() {
    // Rivets: destroyed at the slab, no damage, the slab wears.
    let (mut a, holder, shooter) = bulwark_duel(WeaponKind::Riveter);
    let chest = a.world.players[&holder].movement.position + Vec3::Y * 1.1;
    a.aim(shooter, chest);
    a.set_buttons(shooter, Buttons::FIRE);
    let ev = a.ticks(30);
    a.set_buttons(shooter, Buttons::NONE);
    let ev: Vec<SimEvent> = ev.into_iter().chain(a.ticks(10)).collect();
    assert_eq!(damage_to(&ev, HitTarget::Player(holder)), 0.0);
    let blocks = ev
        .iter()
        .filter(|e| matches!(e, SimEvent::GuardBlock { player, .. } if *player == holder))
        .count();
    assert!(blocks >= 4, "{blocks}");
    let g = *a.world.guard(holder).unwrap();
    assert!(g.absorb_left < g.absorb);
    assert!(projectiles_of(&a, ProjectileKind::Rivet).is_empty());

    // A ray ends on the slab; one at a player standing behind it doesn't get through either.
    let (mut a, holder, shooter) = bulwark_duel(WeaponKind::Quill);
    let behind = a.player(2, Vec3::new(0.8, 0.0, 3.0), 0.0);
    a.world.players.get_mut(&behind).unwrap().combat.protection = 0;
    a.tick();
    let chest = a.world.players[&holder].movement.position + Vec3::Y * 1.1;
    a.aim(shooter, chest);
    let ev = a.tap(shooter, Buttons::FIRE, 1);
    assert_eq!(damage_to(&ev, HitTarget::Player(holder)), 0.0);
    let end = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Shot(s) if s.shooter == shooter => s.ends.first().copied(),
            _ => None,
        })
        .unwrap();
    let slab_z = -a.tuning.combat.bulwark.distance;
    assert!((end.z - slab_z).abs() < 0.3, "ends on the slab: {end}");
    a.ticks(30);
    let far = a.world.players[&behind].movement.position + Vec3::Y * 1.1;
    a.aim(shooter, far);
    let ev = a.tap(shooter, Buttons::FIRE, 1);
    assert_eq!(damage_to(&ev, HitTarget::Player(behind)), 0.0);
}

#[test]
fn your_own_shots_pass_and_from_behind_the_slab_is_no_help() {
    // The holder's own pellets go through to the opponent.
    let (mut a, holder, shooter) = bulwark_duel(WeaponKind::Quill);
    let chest = a.world.players[&shooter].movement.position + Vec3::Y * 1.1;
    a.aim(holder, chest);
    a.set_buttons(holder, Buttons::ALT_FIRE | Buttons::FIRE);
    let ev = a.ticks(2);
    assert!(damage_to(&ev, HitTarget::Player(shooter)) > 0.0);
    // Shot from behind: it lands.
    let (mut a, holder, shooter) = bulwark_duel(WeaponKind::Quill);
    a.world.players.get_mut(&shooter).unwrap().movement.position = Vec3::new(0.0, 0.0, 15.0);
    a.tick();
    let chest = a.world.players[&holder].movement.position + Vec3::Y * 1.1;
    a.aim(shooter, chest);
    let ev = a.tap(shooter, Buttons::FIRE, 1);
    assert!(damage_to(&ev, HitTarget::Player(holder)) > 0.0);
}

#[test]
fn a_spent_slab_shatters_into_the_longer_cooldown() {
    let (mut a, holder, shooter) = bulwark_duel(WeaponKind::Riveter);
    a.world
        .abilities
        .guards
        .raised
        .get_mut(&holder)
        .unwrap()
        .absorb_left = 1.0;
    let chest = a.world.players[&holder].movement.position + Vec3::Y * 1.1;
    a.aim(shooter, chest);
    a.set_buttons(shooter, Buttons::FIRE);
    let mut broke = false;
    for _ in 0..30 {
        for e in a.tick() {
            broke |= matches!(e, SimEvent::GuardBlock { broke: true, .. });
        }
        if broke {
            break;
        }
    }
    a.set_buttons(shooter, Buttons::NONE);
    assert!(broke, "shattered");
    assert!(a.world.guard(holder).is_none());
    let w = a.world.players[&holder].combat.weapons;
    let cd = w.alt_cooldown[w.slot as usize] as f32 / 60.0;
    let b = &a.tuning.combat.bulwark;
    assert!(cd > b.cooldown && cd <= b.broken_cooldown, "{cd}");
    // Prediction carries on from the server's state (no intents spill out of it).
    let mut predicted = a.world.players[&holder].combat.weapons;
    let input = a.held[&holder];
    let intents = step_weapons(&mut predicted, &input, true, &a.tuning.combat, 60);
    assert!(
        intents
            .iter()
            .all(|i| !matches!(i, FireIntent::Ability { .. })),
        "{intents:?}"
    );
}
