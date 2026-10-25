//! Husk AI (chase, telegraphed swipe, line of sight, targeting), co-op scaling, knockback,
//! and lag-compensated hits on a moving enemy.

mod combat_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::level::LevelKind;
use shared::protocol::{NetFrame, enemy_flags};
use shared::sim::{AiState, Attack, DamageFlags, HitTarget, Owner, SimEvent};

#[test]
fn husk_chases_and_swipes_a_standing_player() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let h = a.husk(Vec3::new(0.0, 0.0, -20.0), 0.0);
    let mut states = Vec::new();
    let mut hurt_at = None;
    for i in 0..360 {
        let ev = a.tick();
        states.push(a.world.enemies[&h].state);
        if hurt_at.is_none() && damage_to(&ev, HitTarget::Player(p)) > 0.0 {
            hurt_at = Some(i);
            let d = ev
                .iter()
                .find_map(|e| match e {
                    SimEvent::Damage(d) if d.target == HitTarget::Player(p) => Some(*d),
                    _ => None,
                })
                .unwrap();
            assert_eq!(d.info.source, Owner::Enemy(h));
            assert_eq!(
                d.info.attack,
                Attack::Enemy {
                    kind: 16,
                    attack: 0
                }
            );
            assert!(d.info.flags.contains(DamageFlags::MELEE));
            assert_eq!(d.dealt, 15.0);
        }
    }
    let hurt_at = hurt_at.expect("the husk reached and hit the player");
    // 20 m at 9 m/s + 0.35 s reaction + 0.45 s wind-up ≈ 2.8 s.
    assert!(hurt_at < 240, "took {hurt_at} ticks");
    // It went through every phase, telegraphing first.
    for s in [
        AiState::Alert,
        AiState::Chase,
        AiState::Windup,
        AiState::Strike,
        AiState::Recover,
    ] {
        assert!(states.contains(&s), "never {s:?}");
    }
    let first_windup = states.iter().position(|s| *s == AiState::Windup).unwrap();
    let first_strike = states.iter().position(|s| *s == AiState::Strike).unwrap();
    assert_eq!(first_strike - first_windup, 27, "0.45 s telegraph");
    assert!(a.health(p) < 100.0);
    assert!(
        a.world.enemies[&h].position.distance(Vec3::ZERO) < 3.5,
        "stays close"
    );
    let events = &a.events;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SimEvent::EnemyAlerted { id, .. } if *id == h))
    );
}

#[test]
fn husk_replicates_its_telegraph() {
    let mut a = Arena::coop();
    a.player(0, Vec3::ZERO, 0.0);
    let h = a.husk(Vec3::new(0.0, 0.0, -2.5), 0.0);
    let mut seen_windup = false;
    for _ in 0..120 {
        a.tick();
        let mut frame = NetFrame::new();
        a.world.replicate(&a.tuning, &mut frame);
        let e = frame.get(h).unwrap();
        assert_eq!(e.kind, 16);
        assert_eq!(e.aux >> 16, 40, "max health in aux");
        if e.state == AiState::Windup as u16 {
            seen_windup = true;
            assert_eq!(e.owner, 0, "targeting player 0");
            assert!(e.flags & enemy_flags::ALERT != 0);
            assert_eq!((e.aux >> 8) & 0xFF, 0, "attack index");
        }
    }
    assert!(seen_windup);
}

#[test]
fn husk_needs_line_of_sight() {
    let mut a = Arena::new(flat_level(
        LevelKind::Campaign,
        vec![brush([-10.0, 0.0, -11.0], [10.0, 5.0, -10.0])],
    ));
    let p = a.player(0, Vec3::ZERO, 0.0);
    let h = a.husk(Vec3::new(0.0, 0.0, -20.0), 0.0);
    a.ticks(240);
    assert_eq!(a.world.enemies[&h].state, AiState::Idle);
    assert_eq!(a.health(p), 100.0);
}

#[test]
fn husk_targets_the_nearest_player_and_switches_on_heavy_damage() {
    let mut a = Arena::coop();
    let near = a.player(0, Vec3::new(0.0, 0.0, -8.0), 0.0);
    let far = a.player(1, Vec3::new(0.0, 0.0, 25.0), 0.0);
    let h = a.husk(Vec3::new(0.0, 0.0, -20.0), 0.0);
    a.world.enemies.get_mut(&h).unwrap().health = 1000.0;
    a.world.enemies.get_mut(&h).unwrap().max_health = 1000.0;
    a.ticks(30);
    assert_eq!(a.world.enemies[&h].target, Some(near));
    // The far player deals > 30 % of its max health: aggro switches.
    let t = a.tuning.clone();
    let info = shared::sim::DamageInfo::new(400.0, Owner::Player(far), Attack::Rocket);
    a.world.apply_damage(&t, HitTarget::Entity(h), info);
    a.ticks(30);
    assert_eq!(a.world.enemies[&h].target, Some(far));
}

#[test]
fn coop_scales_enemy_health() {
    let mut a = Arena::coop();
    a.player(0, Vec3::ZERO, 0.0);
    a.player(1, Vec3::new(2.0, 0.0, 0.0), 0.0);
    a.player(2, Vec3::new(4.0, 0.0, 0.0), 0.0);
    let h = a.husk(Vec3::new(0.0, 0.0, -30.0), 0.0);
    assert_eq!(a.world.enemies[&h].health, 80.0, "40 x (1 + 0.5 x 2)");
    let mut pvp = Arena::pvp();
    pvp.player(0, Vec3::ZERO, 0.0);
    pvp.player(1, Vec3::new(2.0, 0.0, 0.0), 0.0);
    let h = pvp.husk(Vec3::new(0.0, 0.0, -30.0), 0.0);
    assert_eq!(pvp.world.enemies[&h].health, 40.0);
}

#[test]
fn punches_knock_husks_back_and_enemies_dont_hurt_enemies() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let h = a.husk(Vec3::new(0.0, 0.0, -1.9), PI);
    let other = a.husk(Vec3::new(5.0, 0.0, -5.0), PI);
    a.ticks(10);
    let body = a.husk_body(h);
    a.aim(p, body);
    a.tap(p, Buttons::PUNCH, 0);
    let e = &a.world.enemies[&h];
    assert!(e.velocity.z < -5.0, "knocked away: {:?}", e.velocity);
    assert!(!e.on_ground, "and up");
    a.ticks(60);
    assert!(a.world.enemies[&h].position.z < -3.0);
    // Enemy-sourced damage to an enemy is blocked.
    let t = a.tuning.clone();
    let info = shared::sim::DamageInfo::new(
        100.0,
        Owner::Enemy(h),
        Attack::Enemy {
            kind: 16,
            attack: 0,
        },
    );
    let r = a.world.apply_damage(&t, HitTarget::Entity(other), info);
    assert_eq!(r.dealt, 0.0);
    assert_eq!(r.blocked, Some(shared::sim::Blocked::SameTeam));
}

#[test]
fn lag_compensated_hit_on_a_moving_husk() {
    // A husk runs past (chasing player 1); player 0 sees it 12 ticks (200 ms) in the past and
    // shoots at what it sees. Lag compensation registers the hit; without it, a miss.
    let run = |use_history: bool| {
        let mut a = Arena::coop();
        let shooter = a.player(0, Vec3::new(-15.0, 0.0, 0.0), -PI / 2.0);
        a.world.players.get_mut(&shooter).unwrap().notarget = true;
        a.player(1, Vec3::new(0.0, 0.0, 30.0), 0.0);
        let h = a.husk(Vec3::new(0.0, 0.0, -25.0), 0.0);
        a.world.enemies.get_mut(&h).unwrap().health = 1000.0;
        a.use_history = use_history;
        let lag = 12.0;
        a.view_lag.insert(shooter, lag);
        // Let it get up to speed.
        a.ticks(90);
        let speed = a.world.enemies[&h].velocity.length();
        assert!(speed > 8.0, "husk running at {speed}");
        // Where the shooter sees it: its hitbox 12 ticks ago.
        let seen = a.world.enemies[&h]
            .trail
            .at(a.world.tick as f64 + 1.0 - lag)
            .unwrap()
            + Vec3::Y * 0.9;
        let now = a.husk_body(h);
        assert!(
            seen.distance(now) > 1.5,
            "moved {} m since",
            seen.distance(now)
        );
        a.aim(shooter, seen);
        let ev = a.tap(shooter, Buttons::FIRE, 0);
        damage_to(&ev, HitTarget::Entity(h))
    };
    assert_eq!(run(true), 22.0, "hit as seen");
    assert_eq!(
        run(false),
        0.0,
        "without lag compensation the same shot misses"
    );
}

#[test]
fn simulation_with_combat_is_deterministic() {
    let run = || {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        for i in 0..5 {
            a.husk(Vec3::new(i as f32 * 3.0 - 6.0, 0.0, -25.0), 0.0);
        }
        a.select(p, 2, 0);
        a.set_buttons(p, Buttons::FIRE);
        for i in 0..400u32 {
            let input = a.held.get_mut(&p).unwrap();
            input.yaw = (i as f32 * 0.02).sin() * 0.4;
            input.pitch = -0.05;
            input.weapon = ((i / 100) % 4) as u8;
            a.tick();
        }
        let enemies: Vec<_> = a
            .world
            .enemies
            .values()
            .map(|e| (e.position, e.health, e.state))
            .collect();
        (
            a.world.players[&p].combat,
            a.world.players[&p].movement,
            enemies,
            a.events.len(),
        )
    };
    assert_eq!(run(), run());
}
