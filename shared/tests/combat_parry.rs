//! Parry: projectile deflection at 0 and ~200 ms of view lag (≈100 ms RTT + interpolation),
//! the delayed-damage tolerance window, too-late failures, players' projectiles (not punchable;
//! the Gust turns them around), the telegraphed melee parry hook and the Harrier swat (a parry
//! during its dive) under lag.

mod combat_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::protocol::{NetId, kind};
use shared::sim::enemy::{AttackDef, AttackKind, EnemyDef};
use shared::sim::{
    Attack, DamageFlags, DamageInfo, HitTarget, Owner, ParryTarget, PlayerId, ProjectileKind,
    SimEvent,
};

/// Player at the origin facing -Z, a (passive) target dummy 60 m away owning a Heron needle
/// slowed to 22 m/s (20 damage) that flies at the player from 20 m.
fn incoming_orb(view_lag: f64) -> (Arena, PlayerId, NetId, NetId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.view_lag.insert(p, view_lag);
    let husk = a.husk(Vec3::new(0.0, 0.0, -60.0), PI);
    a.ticks(30);
    let t = a.tuning.clone();
    let orb = a
        .world
        .spawn_enemy_projectile(
            &t,
            ProjectileKind::Needle,
            husk,
            Attack::Enemy {
                kind: kind::HERON,
                attack: 1,
            },
            Vec3::new(0.0, 1.4, -20.0),
            Vec3::new(0.0, 0.0, 22.0),
            20.0,
        )
        .expect("needle");
    (a, p, husk, orb)
}

/// Distance from the eye to where the player *sees* the orb (its position `lag` ticks ago).
fn seen_distance(a: &Arena, p: PlayerId, orb: NetId, lag: f64) -> Option<f32> {
    let o = a.world.projectiles.get(&orb)?;
    let pos = o.trail.at(a.world.tick as f64 - lag)?;
    Some(pos.distance(a.eye(p)))
}

fn parries(events: &[SimEvent]) -> Vec<shared::sim::ParryEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Parried(p) => Some(*p),
            _ => None,
        })
        .collect()
}

/// Runs until the orb is seen within `distance`, then punches. Returns the events from the
/// punch on (2 s).
fn punch_when_seen(
    a: &mut Arena,
    p: PlayerId,
    orb: NetId,
    lag: f64,
    distance: f32,
) -> Vec<SimEvent> {
    for _ in 0..200 {
        if seen_distance(a, p, orb, lag).is_some_and(|d| d <= distance) {
            break;
        }
        a.tick();
    }
    let mut ev = a.tap(p, Buttons::PUNCH, 0);
    ev.extend(a.ticks(120));
    ev
}

#[test]
fn parry_at_zero_latency_returns_to_sender() {
    let (mut a, p, husk, orb) = incoming_orb(0.0);
    let t = a.tuning.clone();
    // Some damage first, to see the reward.
    a.world.apply_damage(
        &t,
        HitTarget::Player(p),
        DamageInfo::new(30.0, Owner::World, Attack::Dev),
    );
    let ev = punch_when_seen(&mut a, p, orb, 0.0, 2.0);
    let parried = parries(&ev);
    assert_eq!(parried.len(), 1, "{ev:?}");
    assert_eq!(parried[0].late_ticks, 0, "parried before it hit");
    assert!(matches!(
        parried[0].target,
        ParryTarget::Projectile {
            kind: ProjectileKind::Needle,
            ..
        }
    ));
    // Reward: half the 10.5 hard damage cleared, healed to the effective max (95 - 70).
    assert!(
        (parried[0].healed - (30.0 - 10.5 * 0.5)).abs() < 1e-3,
        "healed {}",
        parried[0].healed
    );
    // No damage to the player; the needle flew back (auto-aimed) and killed its owner:
    // 20 x 2 = 40 = the target dummy's health. RETURN TO SENDER.
    assert_eq!(damage_to(&ev, HitTarget::Player(p)), 0.0);
    let k = kills(&ev);
    assert_eq!(k.len(), 1, "{ev:?}");
    assert_eq!(k[0].victim, HitTarget::Entity(husk));
    assert_eq!(k[0].killer, Owner::Player(p));
    assert!(k[0].parried_projectile);
    assert!(
        k[0].flags
            .contains(DamageFlags::PARRIED | DamageFlags::RETURNED)
    );
}

#[test]
fn parry_with_200ms_view_lag_is_saved_by_the_tolerance_window() {
    // 12 ticks = 200 ms: what a client at ~100 ms RTT with ~100 ms of interpolation sees.
    let lag = 12.0;
    let (mut a, p, _husk, orb) = incoming_orb(lag);
    let ev = punch_when_seen(&mut a, p, orb, lag, 2.0);
    let parried = parries(&ev);
    assert_eq!(parried.len(), 1, "parry made on what the player saw counts");
    // On the server the orb had already touched the player; its damage was still pending.
    assert!(
        parried[0].late_ticks > 0,
        "late by {} ticks",
        parried[0].late_ticks
    );
    assert_eq!(damage_to(&ev, HitTarget::Player(p)), 0.0);
    println!(
        "200 ms view lag: parried {} ticks ({:.0} ms) after the server-side impact",
        parried[0].late_ticks,
        parried[0].late_ticks as f32 * 1000.0 / 60.0
    );

    // The same timing with only the fixed tolerance (no view-lag compensation) is too late.
    let (mut a, p, _husk, orb) = incoming_orb(lag);
    a.tuning.combat.parry_compensate_view_lag = false;
    let ev = punch_when_seen(&mut a, p, orb, lag, 2.0);
    assert!(parries(&ev).is_empty());
    assert_eq!(a.health(p), 80.0);
}

#[test]
fn parry_too_late_fails() {
    let lag = 12.0;
    let (mut a, p, _husk, orb) = incoming_orb(lag);
    // Wait until the orb (as seen) is long past: the delayed damage has been applied.
    let mut ev = Vec::new();
    for _ in 0..200 {
        ev.extend(a.tick());
        if damage_to(&ev, HitTarget::Player(p)) > 0.0 {
            break;
        }
    }
    assert_eq!(damage_to(&ev, HitTarget::Player(p)), 20.0, "orb hit");
    let ev = a.tap(p, Buttons::PUNCH, 30);
    assert!(parries(&ev).is_empty());
    assert!(!a.world.projectiles.contains_key(&orb), "orb is gone");
    assert_eq!(a.health(p), 80.0);
}

#[test]
fn parry_window_catches_projectiles_arriving_during_it() {
    let (mut a, p, _husk, orb) = incoming_orb(0.0);
    // Punch early: the orb is 6 m away, the 0.2 s window lasts while it closes in (4.4 m).
    let punch_tick = a.world.tick + 1;
    let ev = punch_when_seen(&mut a, p, orb, 0.0, 6.0);
    let parried = parries(&ev);
    assert_eq!(parried.len(), 1);
    assert!(
        parried[0].tick > punch_tick,
        "caught by the open window, not the press"
    );
    // Punching far too early (orb 15 m away) misses: the window closes first.
    let (mut a, p, _husk, orb) = incoming_orb(0.0);
    let ev = punch_when_seen(&mut a, p, orb, 0.0, 15.0);
    assert!(parries(&ev).is_empty());
    assert_eq!(a.health(p), 80.0);
    // Facing away doesn't parry either.
    let (mut a, p, _husk, orb) = incoming_orb(0.0);
    a.held.get_mut(&p).unwrap().yaw = PI;
    let ev = punch_when_seen(&mut a, p, orb, 0.0, 2.5);
    assert!(parries(&ev).is_empty());
}

#[test]
fn players_projectiles_are_not_punchable() {
    // No parryable grenades: the fist does nothing to your own (or anyone's) bombs, wisps and
    // gravewells; the Bellows Gust is how you turn a player's projectile around.
    for kind in [
        ProjectileKind::Bomb,
        ProjectileKind::Wisp,
        ProjectileKind::Gravewell,
    ] {
        assert!(!kind.parryable(), "{kind:?}");
        assert!(!kind.owner_can_parry(), "{kind:?}");
    }
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.ticks(5);
    a.select(p, 2, 0);
    a.held.get_mut(&p).unwrap().pitch = 0.0;
    a.tap(p, Buttons::ALT_FIRE, 3);
    let wisp = *a.world.projectiles.keys().next().expect("wisp");
    let ev = a.tap(p, Buttons::PUNCH, 0);
    assert!(parries(&ev).is_empty());
    assert!(!a.world.projectiles[&wisp].parried);
}

#[test]
fn pvp_players_gust_back_each_others_bombs() {
    let mut a = Arena::pvp();
    let p0 = a.player(0, Vec3::ZERO, 0.0);
    let p1 = a.player(1, Vec3::new(0.0, 0.0, -10.0), PI);
    a.ticks(5);
    a.select(p1, 3, 0);
    let target = a.eye(p0);
    a.aim(p1, target);
    a.tap(p1, Buttons::FIRE, 0);
    let bomb = *a.world.projectiles.keys().next().unwrap();
    // A punch does nothing to it ...
    let mut ev = Vec::new();
    for _ in 0..60 {
        let d = a.world.projectiles[&bomb].position.distance(a.eye(p0));
        if d < 6.0 {
            break;
        }
        ev.extend(a.tick());
    }
    ev.extend(a.tap(p0, Buttons::PUNCH, 0));
    assert!(parries(&ev).is_empty());
    assert!(!a.world.projectiles[&bomb].parried);
    // ... a Gust sends it back: the shooter takes it, x1.5.
    a.select(p0, 1, 0);
    let back = a.eye(p1);
    a.aim(p0, back);
    let ev = a.tap(p0, Buttons::ALT_FIRE, 40);
    assert!(
        ev.iter()
            .any(|e| matches!(e, SimEvent::Deflected { player, .. } if *player == p0))
    );
    assert!(
        a.health(p0) >= 94.0,
        "only the gust's own recoil, no blast: {}",
        a.health(p0)
    );
    let hit = ev
        .iter()
        .find_map(|e| match e {
            SimEvent::Damage(d) if d.target == HitTarget::Player(p1) => Some(*d),
            _ => None,
        })
        .expect("back to sender");
    assert!(
        hit.info
            .flags
            .contains(DamageFlags::RETURNED | DamageFlags::EXPLOSIVE)
    );
    assert_eq!(hit.info.attack, Attack::Gust);
    assert!(hit.dealt > 70.0, "{}", hit.dealt);
}

/// A test brute with a telegraphed, parryable slam (the hook in general; the Gaoler's bash is
/// tested in `enemies_ai.rs`).
fn brute() -> EnemyDef {
    EnemyDef {
        name: "Brute".into(),
        net_kind: 62,
        health: 450.0,
        speed: 5.0,
        radius: 0.7,
        height: 2.6,
        attacks: vec![AttackDef {
            name: "slam".into(),
            kind: AttackKind::Melee,
            range: 3.0,
            arc: 140.0,
            damage: 45.0,
            knockback: 10.0,
            windup: 1.0,
            strike: 0.15,
            recover: 0.8,
            cooldown: 1.0,
            parry_window: Some(0.35),
            parry_stun: 2.5,
            parry_damage: 150.0,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn brute_arena(view_lag: f64) -> (Arena, PlayerId, NetId) {
    let mut a = Arena::coop();
    a.world.enemy_defs.insert("brute", brute());
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.view_lag.insert(p, view_lag);
    let b = a
        .world
        .spawn_enemy("brute", Vec3::new(0.0, 0.0, -3.2), PI)
        .unwrap();
    (a, p, b)
}

/// Ticks until the player's next input will see the brute `into` ticks into its parryable
/// phase (`into < 0`: that many ticks before its end). Returns the strike tick.
fn until_parry_phase_seen(a: &mut Arena, b: NetId, lag: f64, into: f64) -> u32 {
    for _ in 0..600 {
        let view = a.world.tick as f64 + 1.0 - lag;
        if let Some(w) = a.world.enemies[&b].parry_window {
            let at = if into >= 0.0 {
                w.start as f64 + into
            } else {
                w.end as f64 + into
            };
            if view >= at {
                return w.end;
            }
        }
        a.tick();
    }
    panic!("brute never wound up");
}

#[test]
fn telegraphed_melee_parry_stuns_and_counters() {
    let (mut a, p, b) = brute_arena(1.0);
    until_parry_phase_seen(&mut a, b, 1.0, 3.0);
    let ev = a.tap(p, Buttons::PUNCH, 30);
    let parried = parries(&ev);
    assert_eq!(parried.len(), 1);
    assert_eq!(parried[0].target, ParryTarget::Melee { enemy: b });
    assert_eq!(a.health(p), 100.0, "the slam was cancelled");
    let brute = &a.world.enemies[&b];
    assert_eq!(brute.state, shared::sim::AiState::Stunned);
    assert!(damage_to(&ev, HitTarget::Entity(b)) >= 150.0);

    // Without a parry the slam lands.
    let (mut a, p, _b) = brute_arena(1.0);
    a.ticks(200);
    assert!(a.health(p) <= 55.0, "{}", a.health(p));
}

/// A Harrier diving at a player who sees the world `lag` ticks late: the player punches when
/// the Harrier they see is `distance` m from their eye. Returns the events from the punch on.
fn swat_when_seen(lag: f64, distance: f32) -> (Arena, PlayerId, NetId, Vec<SimEvent>) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.view_lag.insert(p, lag);
    let h = a
        .world
        .spawn_enemy("harrier", Vec3::new(0.0, 7.0, -12.0), 0.0)
        .unwrap();
    a.world.enemies.get_mut(&h).unwrap().health = 500.0;
    let mut dive_from = None;
    for _ in 0..600 {
        let e = &a.world.enemies[&h];
        if e.state == shared::sim::AiState::Strike {
            dive_from.get_or_insert(a.world.tick);
        }
        // What the player sees: the harrier `lag` ticks ago, diving once that view reaches the
        // dive's start.
        let view = a.world.tick as f64 + 1.0 - lag;
        let half = Vec3::Y * (e.height() * 0.5);
        let seen = e.trail.at(view).unwrap_or(e.position) + half;
        let seen_diving = dive_from.is_some_and(|t| view >= t as f64);
        if seen_diving && seen.distance(a.eye(p)) <= distance {
            a.aim(p, seen);
            let ev = a.tap(p, Buttons::PUNCH, 30);
            return (a, p, h, ev);
        }
        a.tick();
    }
    panic!("the harrier never dived");
}

#[test]
fn swatting_a_diving_harrier_works_at_200ms_view_lag() {
    for lag in [1.0, 12.0] {
        let (a, p, h, ev) = swat_when_seen(lag, 3.5);
        let parried = parries(&ev);
        assert_eq!(parried.len(), 1, "lag {lag}: swatted");
        assert_eq!(parried[0].target, ParryTarget::Melee { enemy: h });
        println!(
            "swat at {lag} ticks of view lag: {} ticks late",
            parried[0].late_ticks
        );
        assert_eq!(a.health(p), 100.0, "lag {lag}: the dive never landed");
        assert_ne!(a.world.enemies[&h].state, shared::sim::AiState::Strike);
    }
}

#[test]
fn telegraphed_melee_parry_works_at_200ms_view_lag() {
    let lag = 12.0;
    let (mut a, p, b) = brute_arena(lag);
    let strike = until_parry_phase_seen(&mut a, b, lag, -2.0);
    // On the server the strike has already landed (pending), the player sees the window.
    assert!(a.world.tick >= strike, "server is past the strike");
    let ev = a.tap(p, Buttons::PUNCH, 30);
    assert_eq!(parries(&ev).len(), 1);
    assert_eq!(a.health(p), 100.0);
}
