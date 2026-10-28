//! The expansion arsenal in a live world (docs/arsenal.md): the expansion's primaries and its
//! kept abilities, the Burn and Pinned status effects (enemies and PvP players), abilities on a
//! lesser weapon of their class, and the PvP profile's one-shot rules and times to kill (from
//! the numbers, and measured in play with the balance harness). The rework's abilities and the
//! Lamplighter have their own tests (`combat_rework.rs`).

mod balance_support;
mod combat_support;
mod weapon_support;

use balance_support::{
    Aim, Assume, Side, Stop, Trigger, build_with, duel, knack_run, on_player, per_trigger,
    per_use_burst, weapon_range,
};
use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponKind};
use shared::lobby::GameMode;
use shared::protocol::{NetFrame, NetId, enemy_flags, player_flags};
use shared::sim::combat::secs_to_ticks;
use shared::sim::style::is_longshot;
use shared::sim::{
    Attack, DamageEvent, DamageFlags, HitTarget, Owner, PlayerId, ProjectileKind, SimEvent,
};
use shared::tuning::Tuning;
use weapon_support::*;

/// A co-op arena with one god-mode player at the origin looking down -Z, carrying `weapon`
/// with `abilities`.
fn armed(weapon: WeaponKind, abilities: [Ability; 2]) -> (Arena, PlayerId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(&mut a, p, weapon, abilities);
    (a, p)
}

fn arm(a: &mut Arena, p: PlayerId, weapon: WeaponKind, abilities: [Ability; 2]) {
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let build = WeaponBuild { weapon, abilities };
    assert!(build.is_valid(), "{build:?}");
    a.world.force_loadout(p, Some(Loadout::single(build)));
}

/// Every damage event in `events`.
fn damage_events(events: &[SimEvent]) -> Vec<DamageEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) => Some(*d),
            _ => None,
        })
        .collect()
}

fn hits_on(events: &[SimEvent], target: NetId, attack: Attack) -> Vec<DamageEvent> {
    damage_events(events)
        .into_iter()
        .filter(|d| d.target == HitTarget::Entity(target) && d.info.attack == attack)
        .collect()
}

fn explosions(events: &[SimEvent], attack: Attack) -> Vec<(Vec3, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Explosion {
                position,
                radius,
                attack: a,
                ..
            } if *a == attack => Some((*position, *radius)),
            _ => None,
        })
        .collect()
}

/// Holds `buttons` for `ticks`, then releases them for `after` ticks.
fn hold(a: &mut Arena, p: PlayerId, buttons: Buttons, ticks: u32, after: u32) -> Vec<SimEvent> {
    a.set_buttons(p, buttons);
    let mut ev = a.ticks(ticks);
    a.set_buttons(p, Buttons::NONE);
    ev.extend(a.ticks(after));
    ev
}

/// Arms the bow's ability `mode`, draws for `draw` ticks and looses.
fn bow_shot(a: &mut Arena, p: PlayerId, mode: u8, draw: u32, after: u32) -> Vec<SimEvent> {
    a.select(p, 0, mode);
    let mut ev = a.tap(p, Buttons::ALT_FIRE, 0);
    ev.extend(hold(a, p, Buttons::FIRE, draw, after));
    ev
}

const BIG: f32 = 1.0e5;

// ---- primaries --------------------------------------------------------------------------------

#[test]
fn lancet_fires_needles_that_bloom() {
    let (mut a, p) = armed(WeaponKind::Lancet, [Ability::Tally, Ability::Waxwork]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -15.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let ev = hold(&mut a, p, Buttons::FIRE, 60, 5);
    let hits = hits_on(&ev, d, Attack::Lancet);
    // 0.15 s (9 ticks): 7 needles in a second, all on target despite the bloom.
    assert_eq!(hits.len(), 7);
    assert!(hits.iter().all(|h| h.info.amount == 14.0));
    assert!(
        hits.iter()
            .all(|h| h.info.flags.contains(DamageFlags::HITSCAN))
    );
}

#[test]
fn thresher_is_an_automatic_scattergun() {
    let (mut a, p) = armed(WeaponKind::Thresher, [Ability::Bolas, Ability::Flak]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -4.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    // Held 0.9 s: blasts at 0, 0.42 and 0.84 s, every pellet landing at 4 m.
    let ev = hold(&mut a, p, Buttons::FIRE, 54, 2);
    let hits = hits_on(&ev, d, Attack::Thresher);
    let th = Tuning::default().combat.thresher;
    assert_eq!(hits.len(), 3 * th.pellets as usize);
    assert!(hits.iter().all(|h| h.info.amount == th.pellet_damage));
}

#[test]
fn riveter_rivets_fly_then_hit() {
    let (mut a, p) = armed(WeaponKind::Riveter, [Ability::Stoke, Ability::Lantern]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -27.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let first = a.tap(p, Buttons::FIRE, 0);
    assert!(
        hits_on(&first, d, Attack::Riveter).is_empty(),
        "not instant"
    );
    let rivets = a.world.projectiles.values();
    assert_eq!(
        rivets.filter(|r| r.kind == ProjectileKind::Rivet).count(),
        1
    );
    let ev = a.ticks(30);
    let hits = hits_on(&ev, d, Attack::Riveter);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].info.amount, 7.0);
    assert!(hits[0].info.flags.contains(DamageFlags::PROJECTILE));
}

#[test]
fn cinderbox_fires_a_three_rocket_salvo() {
    let (mut a, p) = armed(WeaponKind::Cinderbox, [Ability::Limpet, Ability::Kite]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    // Rockets 0.08 s apart: the first ones' blasts shove the target, so the last may miss
    // and burn out at the end of its flight.
    let ev = a.tap(p, Buttons::FIRE, 160);
    assert_eq!(explosions(&ev, Attack::Cinderbox).len(), 3);
    let hits = hits_on(&ev, d, Attack::Cinderbox);
    let direct = hits
        .iter()
        .filter(|h| h.info.flags.contains(DamageFlags::DIRECT));
    assert!(direct.count() >= 2);
    assert!(damage_to(&ev, HitTarget::Entity(d)) > 60.0);
}

#[test]
fn longwick_draw_sets_speed_and_damage() {
    let t = Tuning::default().combat.longwick;
    // A full draw at 20 m.
    let (mut a, p) = armed(WeaponKind::Longwick, [Ability::BlinkArrow, Ability::Brand]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -20.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let ev = hold(&mut a, p, Buttons::FIRE, 50, 30);
    let hits = hits_on(&ev, d, Attack::Longwick);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].info.amount, t.max_damage);
    // A tap: a weak, slow arrow (close by: it drops).
    let (mut a, p) = armed(WeaponKind::Longwick, [Ability::BlinkArrow, Ability::Brand]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let ev = hold(&mut a, p, Buttons::FIRE, 1, 30);
    let hits = hits_on(&ev, d, Attack::Longwick);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].info.amount < t.min_damage + 3.0);
}

#[test]
fn kiln_sets_targets_alight_and_the_burn_outlasts_the_spray() {
    let (mut a, p) = armed(WeaponKind::Kiln, [Ability::Lantern, Ability::Discharge]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let target = HitTarget::Entity(d);
    let ev = hold(&mut a, p, Buttons::FIRE, 30, 0);
    let sprays = hits_on(&ev, d, Attack::Kiln)
        .into_iter()
        .filter(|h| !h.info.flags.contains(DamageFlags::BURN))
        .count();
    assert_eq!(sprays, 6, "every 0.08 s (5 ticks)");
    assert!(a.world.is_burning(target));
    assert_eq!(a.world.statuses[&target].burn.unwrap().stacks, 3, "capped");
    assert_ne!(
        replicated(&a, d).flags & enemy_flags::BURNING,
        0,
        "every client sees it"
    );
    // It keeps burning for `burn.duration` after the spray stops: 3 stacks x 5 dps x 3 s.
    let ev = a.ticks(200);
    let burns: Vec<DamageEvent> = hits_on(&ev, d, Attack::Kiln);
    let total: f32 = burns.iter().map(|h| h.dealt).sum();
    let b = Tuning::default().combat.burn;
    let expected = b.max_stacks as f32 * b.dps * b.duration;
    assert!(
        (total - expected).abs() <= b.max_stacks as f32 * b.dps * b.tick_interval,
        "{total}"
    );
    // Burn ticks are only BURN: never hitscan or projectile damage (no LONGSHOT).
    assert!(burns.iter().all(|h| h.info.flags == DamageFlags::BURN));
    assert!(!a.world.is_burning(target));
    assert!(!a.world.statuses.contains_key(&target));
}

#[test]
fn burn_stacks_cap_refresh_and_credit_the_source() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -30.0), 0.0, 18.0);
    let target = HitTarget::Entity(d);
    let me = Owner::Player(p);
    let t = a.tuning.clone();
    a.world.apply_burn(&t, me, Attack::Kiln, target, 2);
    a.ticks(60);
    a.world.apply_burn(&t, me, Attack::Kiln, target, 5);
    let burn = a.world.statuses[&target].burn.unwrap();
    assert_eq!(burn.stacks, 3);
    assert_eq!(burn.until, a.world.tick + 180, "refreshed");
    // Its damage kills: the burning player gets the kill with the attack that set it.
    let ev = a.ticks(60);
    let k = kills(&ev);
    assert_eq!(k.len(), 1);
    assert_eq!((k[0].killer, k[0].attack), (me, Attack::Kiln));
    assert_eq!(k[0].weapon, Some(WeaponKind::Kiln));
    assert!(k[0].flags.contains(DamageFlags::BURN));
    assert!(!is_longshot(&k[0], 0.0), "a burn death is never a LONGSHOT");
}

// ---- abilities --------------------------------------------------------------------------------

fn bursts(events: &[SimEvent]) -> Vec<(Vec3, usize)> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Burst { origin, ends, .. } => Some((*origin, ends.len())),
            _ => None,
        })
        .collect()
}

#[test]
fn flak_airbursts_into_a_ring_at_range() {
    let (mut a, p) = armed(WeaponKind::Thresher, [Ability::Flak, Ability::Bolas]);
    let eye = a.eye(p);
    // Off the line of flight, where the first pellet of the ring (towards +x, 25° off the
    // axis) passes 6 m beyond the burst.
    let side = 6.0 * 25f32.to_radians().tan();
    let d = dummy(&mut a, Vec3::new(side, 0.0, -20.75), 0.0, BIG);
    a.aim(p, eye + Vec3::NEG_Z);
    let ev = a.tap(p, Buttons::ALT_FIRE, 40);
    let b = bursts(&ev);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].1, 10, "ten pellets");
    assert!(
        (b[0].0.z + 14.75).abs() < 1.0,
        "bursts ~14 m out: {:?}",
        b[0].0
    );
    assert!(
        !hits_on(&ev, d, Attack::Flak).is_empty(),
        "the ring reaches round corners"
    );
}

#[test]
fn flak_bursts_on_contact_too() {
    let (mut a, p) = armed(WeaponKind::Thresher, [Ability::Flak, Ability::Bolas]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let ev = a.tap(p, Buttons::ALT_FIRE, 20);
    assert_eq!(bursts(&ev).len(), 1);
    // Bursting just short of the target, the ring closes on it.
    assert!(hits_on(&ev, d, Attack::Flak).len() >= 5);
}

#[test]
fn limpets_stick_and_a_hold_detonates_them_all() {
    let (mut a, p) = armed(WeaponKind::Cinderbox, [Ability::Limpet, Ability::Kite]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -5.0), 0.0, BIG);
    let head = dummy_head(&a, d);
    a.aim(p, head);
    let c = Tuning::default().combat.limpet;
    let gap = secs_to_ticks(c.cooldown, 60, 1) as u32;
    for _ in 0..3 {
        // A tap throws (on release); `limpet.cooldown` between throws.
        hold(&mut a, p, Buttons::ALT_FIRE, 2, gap);
    }
    let limpets = a.world.limpets_of(p);
    assert_eq!(limpets.len(), 3);
    for id in &limpets {
        let stuck = a.world.projectiles[id].stuck.expect("stuck");
        assert_eq!(stuck.to, Some(HitTarget::Entity(d)));
    }
    // Holding detonates every charge out.
    let ev = hold(&mut a, p, Buttons::ALT_FIRE, 20, 2);
    assert_eq!(explosions(&ev, Attack::Limpet).len(), 3);
    assert!(a.world.limpets_of(p).is_empty());
    // Three direct blasts (a charge sits its radius off the surface: ~2 % falloff each).
    assert!(damage_to(&ev, HitTarget::Entity(d)) > 3.0 * c.damage * 0.95);

    // On the floor; and a fourth throw with three out detonates instead.
    a.aim(p, Vec3::new(0.0, 0.0, -3.0));
    for _ in 0..3 {
        hold(&mut a, p, Buttons::ALT_FIRE, 2, gap);
    }
    let floor: Vec<_> = a.world.limpets_of(p);
    assert_eq!(floor.len(), 3);
    assert!(floor.iter().all(|id| {
        a.world.projectiles[id]
            .stuck
            .is_some_and(|s| s.to.is_none())
    }));
    let ev = hold(&mut a, p, Buttons::ALT_FIRE, 2, 2);
    assert_eq!(explosions(&ev, Attack::Limpet).len(), 3);
    assert!(a.world.limpets_of(p).is_empty());
}

#[test]
fn a_limpet_rides_along_on_a_moving_target() {
    let (mut a, p) = armed(WeaponKind::Cinderbox, [Ability::Limpet, Ability::Kite]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -5.0), 0.0, BIG);
    let head = dummy_head(&a, d);
    a.aim(p, head);
    hold(&mut a, p, Buttons::ALT_FIRE, 2, 20);
    let id = a.world.limpets_of(p)[0];
    let offset = a.world.projectiles[&id].position - a.world.enemies[&d].position;
    a.world.enemies.get_mut(&d).unwrap().position += Vec3::new(3.0, 0.0, -2.0);
    a.tick();
    let now = a.world.enemies[&d].position;
    let limpet = a.world.projectiles[&id].position;
    assert!((limpet - now - offset).length() < 0.05);
    // Its replicated state is current (stuck: no trajectory).
    let e = replicated(&a, id);
    assert!((e.position - limpet).length() < 0.01);
}

fn replicated(a: &Arena, id: NetId) -> shared::protocol::EntityState {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).expect("replicated")
}

#[test]
fn pinning_arrow_roots_enemies() {
    let (mut a, p) = armed(
        WeaponKind::Longwick,
        [Ability::BlinkArrow, Ability::PinningArrow],
    );
    let h = a.husk(Vec3::new(0.0, 0.0, -6.0), 0.0);
    let body = a.husk_body(h);
    a.aim(p, body);
    // A quick (weak) pinning arrow: the 40 HP husk survives it.
    let ev = bow_shot(&mut a, p, 1, 1, 20);
    assert!(!hits_on(&ev, h, Attack::PinningArrow).is_empty());
    assert!(a.enemy_alive(h));
    let target = HitTarget::Entity(h);
    assert!(a.world.is_pinned(target));
    assert_ne!(replicated(&a, h).flags & enemy_flags::PINNED, 0);
    let held = a.world.enemies[&h].position;
    a.ticks(60);
    let now = a.world.enemies[&h].position;
    assert!(Vec3::new(now.x - held.x, 0.0, now.z - held.z).length() < 1e-4);
    // 1.4 s, then it moves again.
    a.ticks(40);
    assert!(!a.world.is_pinned(target));
    a.ticks(20);
    assert!(a.world.enemies[&h].position.distance(held) > 0.3);

    // A heavy is held for 0.6 s only.
    let heavy = dummy(&mut a, Vec3::new(5.0, 0.0, -10.0), 0.0, 400.0);
    let t = a.tuning.clone();
    a.world
        .apply_pin(&t, Owner::Player(p), HitTarget::Entity(heavy));
    a.ticks(30);
    assert!(a.world.is_pinned(HitTarget::Entity(heavy)));
    a.ticks(8);
    assert!(!a.world.is_pinned(HitTarget::Entity(heavy)));
}

/// Horizontal speed of a player after running forward for `ticks`.
fn run_speed(a: &mut Arena, p: PlayerId, ticks: u32) -> f32 {
    a.held.get_mut(&p).unwrap().forward = 127;
    a.ticks(ticks);
    a.held.get_mut(&p).unwrap().forward = 0;
    let v = a.world.players[&p].movement.velocity;
    Vec3::new(v.x, 0.0, v.z).length()
}

#[test]
fn pinning_arrow_slows_pvp_players_and_the_slow_is_predicted() {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    arm(
        &mut a,
        p,
        WeaponKind::Longwick,
        [Ability::PinningArrow, Ability::BeaconArrow],
    );
    let v = a.player(1, Vec3::new(0.0, 0.0, -10.0), 0.0);
    a.ticks(2);
    let chest = a.world.players[&v].movement.position + Vec3::Y * 1.0;
    a.aim(p, chest);
    let ev = bow_shot(&mut a, p, 0, 1, 20);
    let hit = damage_events(&ev)
        .into_iter()
        .any(|d| d.target == HitTarget::Player(v) && d.info.attack == Attack::PinningArrow);
    assert!(hit);
    let m = a.world.players[&v].movement;
    assert!(m.slow_time > 0.8 && m.slow_scale == 0.5, "{m:?}");
    assert!(a.world.is_pinned(HitTarget::Player(v)));
    let e = replicated(&a, NetId::player(v));
    assert_ne!(e.flags & player_flags::PINNED, 0);
    // Half speed while it lasts (part of the predicted movement state) ...
    let slowed = run_speed(&mut a, v, 30);
    assert!((slowed - 8.0).abs() < 0.5, "{slowed}");
    // ... full speed after.
    a.ticks(30);
    let free = run_speed(&mut a, v, 40);
    assert!((free - 16.0).abs() < 0.5, "{free}");
    assert!(!a.world.is_pinned(HitTarget::Player(v)));
}

#[test]
fn brand_on_the_bow_brands_what_the_arrow_hits() {
    let (mut a, p) = armed(WeaponKind::Longwick, [Ability::Brand, Ability::BlinkArrow]);
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -12.0), 0.0, BIG);
    let body = dummy_body(&a, d);
    a.aim(p, body);
    let ev = bow_shot(&mut a, p, 0, 30, 20);
    assert!(!hits_on(&ev, d, Attack::Brand).is_empty());
    assert!(a.world.is_branded(HitTarget::Entity(d)));
}

#[test]
fn burn_hurts_pvp_players_but_never_teammates() {
    for pvp in [false, true] {
        let mut a = if pvp { Arena::pvp() } else { Arena::coop() };
        let p = a.player(0, Vec3::ZERO, 0.0);
        let v = a.player(1, Vec3::new(5.0, 0.0, 0.0), 0.0);
        let t = a.tuning.clone();
        a.world
            .apply_burn(&t, Owner::Player(p), Attack::Kiln, HitTarget::Player(v), 3);
        assert_eq!(a.world.is_burning(HitTarget::Player(v)), pvp);
        let ev = a.ticks(60);
        let dealt: f32 = damage_events(&ev)
            .iter()
            .filter(|d| d.target == HitTarget::Player(v))
            .map(|d| d.dealt)
            .sum();
        assert_eq!(dealt > 0.0, pvp);
        if pvp {
            let e = replicated(&a, NetId::player(v));
            assert_ne!(e.flags & player_flags::BURNING, 0);
            // Death puts it out.
            a.world.kill_player(&t, v, Attack::Dev);
            a.tick();
            assert!(!a.world.is_burning(HitTarget::Player(v)));
        }
    }
}

#[test]
fn abilities_on_a_lesser_weapon_of_their_class_hit_softer() {
    let brand = |weapon| {
        let (mut a, p) = armed(weapon, [Ability::Brand, Ability::Wickline]);
        let d = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), 0.0, BIG);
        let body = dummy_body(&a, d);
        a.aim(p, body);
        let ev = a.tap(p, Buttons::ALT_FIRE, 2);
        hits_on(&ev, d, Attack::Brand)[0].info.amount
    };
    let c = Tuning::default().combat;
    assert_eq!(brand(WeaponKind::Quill), c.quill.brand_damage);
    let lancet = c.quill.brand_damage * c.lancet.ability_power / c.quill.ability_power;
    assert!((brand(WeaponKind::Lancet) - lancet).abs() < 1e-3);
}

// ---- the PvP profile --------------------------------------------------------------------------

/// Seconds to deal 100 with hits of `damage` every `interval` seconds (whole ticks).
fn ttk(damage: f32, interval: f32) -> f32 {
    let hits = (100.0 / damage).ceil() - 1.0;
    hits * secs_to_ticks(interval, 60, 1) as f32 / 60.0
}

#[test]
fn pvp_profile_keeps_the_one_shot_rules_and_the_time_to_kill_band() {
    let base = Tuning::default();
    let pvp = base.for_mode(GameMode::Deathmatch).combat;
    let head = pvp.player_head_multiplier;
    let one_shot = |damage: f32| damage >= 100.0;

    // No body one-shot from any single trigger pull or ability use.
    let body_max = [
        ("quill", pvp.quill.damage),
        ("lancet", pvp.lancet.damage),
        (
            "bellows",
            pvp.bellows.pellets as f32 * pvp.bellows.pellet_damage,
        ),
        (
            "thresher",
            pvp.thresher.pellets as f32 * pvp.thresher.pellet_damage,
        ),
        ("tumbler", pvp.tumbler.bomb_damage),
        (
            "cinderbox salvo",
            pvp.cinderbox.rockets as f32 * pvp.cinderbox.damage,
        ),
        ("longwick", pvp.longwick.max_damage),
        ("lamplighter", pvp.lamplighter.max_damage),
        (
            "flak ring",
            pvp.flak.pellets as f32 * pvp.flak.pellet_damage,
        ),
        ("breach", pvp.tumbler.breach_damage),
        ("gravewell", pvp.tumbler.gravewell_damage),
        ("discharge", pvp.arclash.discharge_max_damage),
        (
            "three limpets",
            pvp.limpet.max_live as f32 * pvp.limpet.damage,
        ),
        ("kite", pvp.kite.damage),
        ("tally (a shot each)", pvp.tally.damage),
        ("waxwork burst", pvp.waxwork.burst_damage),
        ("wickline", pvp.wickline.damage),
        (
            "riposte on the tamper",
            pvp.tamper.damage * pvp.riposte.max_scale,
        ),
        (
            "blink arrow",
            pvp.longwick.max_damage * pvp.blink_arrow.damage_scale,
        ),
        (
            "beacon arrow",
            pvp.longwick.max_damage * pvp.beacon_arrow.damage_scale,
        ),
    ];
    for (name, damage) in body_max {
        assert!(!one_shot(damage), "{name}: {damage}");
    }
    // The stated exception: a full Bellows blast at point blank kills a body; the Thresher's
    // takes three blasts on the body, two on the head.
    let bellows = pvp.bellows.pellets as f32 * pvp.bellows.pellet_damage;
    assert!(one_shot(bellows * pvp.bellows.point_blank_multiplier));
    let thresher = pvp.thresher.pellets as f32 * pvp.thresher.pellet_damage;
    assert!(!one_shot(2.0 * thresher) && one_shot(3.0 * thresher));
    assert!(!one_shot(
        thresher * pvp.thresher.point_blank_multiplier * head
    ));
    assert!(one_shot(2.0 * thresher * head));

    // Headshot one-shots: a full-draw arrow, a full-focus Lamplighter beam — and neither short
    // of full (at 0.54 a beam focused past 61 % killed a head: 0.4 keeps it to the full focus).
    let arrow = |draw: f32| {
        let l = &pvp.longwick;
        l.min_damage + (l.max_damage - l.min_damage) * draw
    };
    let beam = |focus: f32| {
        let l = &pvp.lamplighter;
        l.min_damage + (l.max_damage - l.min_damage) * focus
    };
    assert!(one_shot(arrow(1.0) * head));
    assert!(!one_shot(arrow(0.9) * head));
    assert!(one_shot(beam(1.0) * head));
    assert!(
        !one_shot(beam(0.9) * head),
        "a 90 % focus: {}",
        beam(0.9) * head
    );
    assert!(!one_shot(pvp.lamplighter.min_damage * head), "a tap");
    // Nothing else one-shots a head.
    for (name, damage) in [
        ("quill", pvp.quill.damage),
        ("lancet", pvp.lancet.damage),
        ("brand", pvp.quill.brand_damage),
        ("tally", pvp.tally.damage),
    ] {
        assert!(!one_shot(damage * head), "{name}");
    }

    // Time to kill with every hit landing on the body: 0.8–1.6 s (the Kiln and its burn a
    // little slower up close, the bow's two full draws a little faster than two seconds).
    let draw = pvp.longwick.draw_time;
    let kiln = (pvp.kiln.damage / pvp.kiln.interval + 3.0 * pvp.burn.dps) / 100.0;
    let band = [
        ("quill", ttk(pvp.quill.damage, pvp.quill.interval)),
        ("lancet", ttk(pvp.lancet.damage, pvp.lancet.interval)),
        (
            "bellows",
            ttk(
                pvp.bellows.pellets as f32 * pvp.bellows.pellet_damage,
                pvp.bellows.interval,
            ),
        ),
        (
            "thresher",
            ttk(
                pvp.thresher.pellets as f32 * pvp.thresher.pellet_damage,
                pvp.thresher.interval,
            ),
        ),
        (
            "arclash",
            ttk(pvp.arclash.zap_damage, pvp.arclash.zap_interval),
        ),
        ("riveter", ttk(pvp.riveter.damage, pvp.riveter.interval)),
        (
            "tumbler",
            ttk(pvp.tumbler.bomb_damage, pvp.tumbler.interval),
        ),
        (
            "cinderbox",
            ttk(
                pvp.cinderbox.rockets as f32 * pvp.cinderbox.damage,
                pvp.cinderbox.interval,
            ),
        ),
        ("longwick", 2.0 * draw + pvp.longwick.interval),
        (
            "lamplighter",
            2.0 * pvp.lamplighter.focus_time + pvp.lamplighter.interval,
        ),
        ("kiln", 1.0 / kiln),
    ];
    for (name, secs) in band {
        assert!((0.8..=1.8).contains(&secs), "{name}: {secs:.2} s");
    }

    // PvE is the reference: nothing changed.
    assert_eq!(base.for_mode(GameMode::Campaign).combat, base.combat);
}

/// The PvP rules measured in play (`balance_support`, docs/balance.md): every trigger pull and
/// every ability use on every carrier, at 3, 5 and 10 m, aimed at the body and at the head.
/// Body: never 100 — except a full Bellows blast at point blank (3 m, the stated exception: it
/// must kill there, and not from 5 m). Head: only a full-draw arrow (plain or special) and a
/// focused Lamplighter beam — and a Bellows blast inside 5 m, the scattergun's point-blank
/// reward (at 0.7 its pellets one-shot a head at 5 m too). At a 90 % draw no arrow kills,
/// special ones included. Both buttons at once never stack a swing and a Whirl on one tick.
/// Every primary's all-body time to kill at its range stays inside 0.8–1.8 s.
#[test]
fn pvp_one_shots_and_times_to_kill_measured_in_play() {
    let base = Tuning::default();
    let a = Assume::default();
    let kills = |damage: f32| damage >= 100.0;
    for w in balance_support::weapons() {
        let bow = w == WeaponKind::Longwick;
        let sniper = w == WeaponKind::Lamplighter;
        for range in [3.0, 5.0, 10.0] {
            let body = per_trigger(&base, Side::Pvp, w, range, Aim::Body, 1.0);
            let point_blank = w == WeaponKind::Bellows && range < 5.0;
            assert!(
                kills(body) == point_blank,
                "{w:?} body at {range} m: {body}"
            );
            let head = per_trigger(&base, Side::Pvp, w, range, Aim::Weak(0), 1.0);
            let allowed = bow || sniper || (w == WeaponKind::Bellows && range < 5.0);
            assert!(allowed || !kills(head), "{w:?} head at {range} m: {head}");
        }
        if bow || sniper {
            let head = per_trigger(&base, Side::Pvp, w, 5.0, Aim::Weak(0), 0.9);
            assert!(
                !kills(head),
                "{w:?}: a 90 % draw / focus to the head: {head}"
            );
        }
    }
    // Abilities: what one use lands within 1 s (a Lantern or an orbiting Wisp deals more over
    // its whole life: that is not a one-shot).
    for w in balance_support::weapons() {
        for &ability in w.abilities() {
            let bow = w == WeaponKind::Longwick;
            let allowed = bow;
            for range in [3.0, 5.0] {
                let body = per_use_burst(&base, Side::Pvp, w, ability, range, Aim::Body, 1.0);
                assert!(
                    !kills(body),
                    "{ability:?} on the {w:?}, body at {range} m: {body}"
                );
                let head = per_use_burst(&base, Side::Pvp, w, ability, range, Aim::Weak(0), 1.0);
                assert!(
                    allowed || !kills(head),
                    "{ability:?} on the {w:?}, head at {range} m: {head}"
                );
            }
            if bow {
                let head = per_use_burst(&base, Side::Pvp, w, ability, 5.0, Aim::Weak(0), 0.9);
                assert!(
                    !kills(head),
                    "{ability:?} at a 90 % draw to the head: {head}"
                );
            }
        }
    }
    // Both buttons on the first tick: the shot is either the primary's or the ability's.
    for (w, ability) in [
        (WeaponKind::Tamper, Ability::Whirl),
        (WeaponKind::Trimmer, Ability::Whirl),
    ] {
        let t = on_player(
            &base,
            a,
            build_with(w, ability),
            Trigger::Both(0),
            false,
            5.0,
            Stop::secs(0.1),
        );
        assert!(
            t.within(0.0) < 100.0,
            "{w:?} + {ability:?}: {}",
            t.within(0.0)
        );
    }
    // Every primary, every shot at the body, at its range.
    for w in balance_support::weapons() {
        let t = on_player(
            &base,
            a,
            w.default_build(),
            Trigger::Primary,
            false,
            weapon_range(w),
            Stop::damage(100.0, 4.0),
        );
        let secs = t.time_to(100.0, None);
        assert!(
            secs.is_some_and(|s| (0.8 - 1e-3..=1.8 + 1e-3).contains(&s)),
            "{w:?}: {secs:?} s"
        );
    }
}

/// PvP knacks are choices, not a must-pick (`balance_support::duel`: real 100 HP, every primary
/// held at the opponent's body at its range, the opponent's knack used just before). Tallow
/// Skin at half damage for 3 s won every even duel (on average 1.9x every time to kill); at
/// 0.7 for 2 s (the PvP override) it's a strong defensive pick, at most 1.6x on average. The
/// drunk Kindle Flask's 40 HP matter against the two-hit killers (a bow's or a Lamplighter's
/// second shot) but slow the average duel far less than Tallow Skin, and its haste (PvP x1.15)
/// carries you about as far as Surge's two charges: at x1.3 it out-ran them by three times
/// (19 m to 6 in 4 s) on top of the heal.
#[test]
fn pvp_knacks_are_choices_not_a_must_pick() {
    let base = Tuning::default();
    let mut slowdowns = Vec::new();
    let mut flask_slowdowns = Vec::new();
    let mut flask_helps = false;
    for w in balance_support::weapons() {
        let b = w.default_build();
        let r = weapon_range(w);
        let plain = duel(&base, b, None, r).expect("a kill");
        let tallow = duel(&base, b, Some(Knack::TallowSkin), r).expect("a kill with Tallow");
        let flask = duel(&base, b, Some(Knack::KindleFlask), r).expect("a kill with a flask");
        assert!(tallow >= plain && flask >= plain, "{w:?}");
        slowdowns.push(tallow / plain);
        flask_slowdowns.push(flask / plain);
        flask_helps |= flask > plain + 0.1;
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    assert!(
        mean(&slowdowns) <= 1.6,
        "Tallow Skin: {:.2}x on average ({slowdowns:?})",
        mean(&slowdowns)
    );
    assert!(flask_helps, "the Kindle Flask never changed a duel");
    assert!(
        mean(&flask_slowdowns) < mean(&slowdowns),
        "the Kindle Flask out-defends Tallow Skin: {flask_slowdowns:?}"
    );
    let none = knack_run(&base, None, 4.0);
    let surge = knack_run(&base, Some(Knack::Surge), 4.0) - none;
    let flask = knack_run(&base, Some(Knack::KindleFlask), 4.0) - none;
    assert!(
        surge > 3.0 && flask > 3.0 && flask <= 2.0 * surge,
        "4 s of running: Surge +{surge:.1} m, the Kindle Flask +{flask:.1} m"
    );
}
