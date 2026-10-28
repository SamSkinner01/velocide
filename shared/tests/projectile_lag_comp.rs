//! Projectile lag compensation (`sim::projectiles`, `docs/feel.md` "Hit registration"): a
//! player's projectile flies from the view time its shot was aimed at — on its first step it
//! makes up the flight its shooter already watched, against the hit shapes as they were then —
//! and sweeps targets in their moving frame, so it touches what the shooter saw it touch.

mod combat_support;
mod weapon_support;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::loadout::{Loadout, WeaponKind};
use shared::sim::projectiles::replicated_position;
use shared::sim::{HitPart, HitTarget, Owner, PlayerId, Projectile, ProjectileKind, SimEvent};
use weapon_support::*;

/// A PvP arena with an archer at (0, 0, 20) holding the Longwick, its inputs aimed at what it
/// saw `lag` ticks ago.
fn archer(lag: f64) -> (Arena, PlayerId) {
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::new(0.0, 0.0, 20.0), 0.0);
    a.world.force_loadout(
        p,
        Some(Loadout::single(WeaponKind::Longwick.default_build())),
    );
    a.view_lag.insert(p, lag);
    (a, p)
}

/// Draws fully aimed at `at`, then looses: the release tick's events (`before`: called before
/// every tick of it, the release included).
fn loose(
    a: &mut Arena,
    p: PlayerId,
    at: Vec3,
    mut before: impl FnMut(&mut Arena),
) -> Vec<SimEvent> {
    a.aim(p, at);
    a.set_buttons(p, Buttons::FIRE);
    for _ in 0..45 {
        before(a);
        a.tick();
    }
    a.set_buttons(p, Buttons::NONE);
    before(a);
    a.tick()
}

fn arrow_of(a: &Arena, p: PlayerId) -> Option<&Projectile> {
    a.world
        .projectiles
        .values()
        .find(|pr| pr.owner == Owner::Player(p) && pr.kind == ProjectileKind::Arrow)
}

/// Was `target` damaged by `p`'s projectile in `events`?
fn arrow_hit(events: &[SimEvent], p: PlayerId, target: HitTarget) -> Option<HitPart> {
    events.iter().find_map(|e| match e {
        SimEvent::Damage(d)
            if d.target == target
                && d.info.source == Owner::Player(p)
                && d.info.projectile.is_some() =>
        {
            Some(d.info.part)
        }
        _ => None,
    })
}

#[test]
fn an_arrow_flies_from_the_moment_it_was_aimed() {
    // The shooter saw 12.25 ticks ago what it aimed at (a round trip and an interpolation
    // delay): its arrow is where 12.25 ticks of flight take it, whenever its input arrived.
    for (lag, flown) in [(12.25, 12.25), (1.0, 1.0), (40.0, 21.0)] {
        let (mut a, p) = archer(lag);
        let target = Vec3::new(0.0, 1.6, -60.0);
        loose(&mut a, p, target, |_| {});
        let now = a.world.tick as f64;
        let t = a.tuning.clone();
        let arrow = arrow_of(&a, p).expect("in flight").entity_state();
        let eye = a.eye(p);
        let dir = (target - eye).normalize();
        let speed = t.combat.longwick.max_speed;
        let g = t.combat.longwick.arrow_gravity;
        let flight = |ticks: f64| {
            let s = (ticks / 60.0) as f32;
            eye + dir * 0.4 + dir * speed * s - Vec3::Y * (0.5 * g * s * s)
        };
        // At the end of the release tick: the flight made up (40 ticks is past the rewind
        // bound, `lag_comp_max_ms` 350 = 21 ticks) plus that tick's own step.
        for later in [0.0, 5.0] {
            let at = replicated_position(&arrow, now + later, 60, &t.combat);
            let want = flight(flown + later);
            assert!(
                at.distance(want) < 0.01,
                "lag {lag}: {at} vs {want} {later} ticks on"
            );
        }
    }
    // Switched off, it starts when the input is processed.
    let (mut a, p) = archer(12.25);
    a.tuning.net.projectile_lag_comp_max_ms = 0.0;
    let target = Vec3::new(0.0, 1.6, -60.0);
    loose(&mut a, p, target, |_| {});
    let arrow = arrow_of(&a, p).expect("in flight");
    let start = a.eye(p) + (target - a.eye(p)).normalize() * 0.4;
    let one_tick = a.tuning.combat.longwick.max_speed / 60.0;
    assert!((arrow.position.distance(start) - one_tick).abs() < 0.05);
}

#[test]
fn the_made_up_flight_hits_the_target_where_the_shooter_saw_it() {
    // A player 20 m ahead steps 2.5 m aside two ticks before the archer's release; the archer
    // saw it 14 ticks ago, still standing there, and its arrow reaches it 9 ticks into the
    // flight: it was there then.
    for (lag, hits) in [(14.0, true), (1.0, false)] {
        let (mut a, p) = archer(lag);
        let seen = Vec3::new(0.0, 0.0, 0.0);
        let moved = Vec3::new(2.5, 0.0, 0.0);
        let b = a.player(1, seen, std::f32::consts::PI);
        a.ticks(30);
        let mut ticks = 0;
        let mut events = loose(&mut a, p, seen + Vec3::Y * 0.9, |a| {
            ticks += 1;
            // Before the tick two ticks ahead of the release.
            if ticks == 44 {
                a.world.players.get_mut(&b).unwrap().movement.position = moved;
            }
        });
        events.extend(a.ticks(20));
        let hit = arrow_hit(&events, p, HitTarget::Player(b));
        assert_eq!(hit.is_some(), hits, "lag {lag}: {hit:?}");
        if hits {
            // Where it lands is the target as it is now (a blast, a sticking charge).
            let point = events
                .iter()
                .find_map(|e| match e {
                    SimEvent::Damage(d) if d.target == HitTarget::Player(b) => Some(d.info.point),
                    _ => None,
                })
                .unwrap();
            assert!(
                (point - (moved + Vec3::Y * 0.9)).length() < 1.0,
                "on the target now: {point}"
            );
        }
    }
}

#[test]
fn a_target_crossing_the_path_between_ticks_is_hit_where_it_was() {
    // Something crosses the arrow's path so fast it is off it (beyond the padded reach) on
    // both sides at the ends of the tick the arrow passes, and on it halfway through: swept
    // in the target's moving frame, the arrow touches it there, as a screen drawing every
    // frame shows it. (1.8 m per tick; a dashing player covers a quarter of that.)
    for (speed, hits) in [(1.8f32, true), (0.0, false)] {
        let (mut a, p) = archer(1.0);
        // The arrow (2.25 m per tick from z = 19.6) is level with the target, 8.5 ticks into
        // its flight, during the tick that ends 8 ticks after the release; x of the target at
        // the end of tick `release + k`: the line crossed halfway through that one.
        let x = |k: i32| speed * (k as f32 - 7.5) + if speed == 0.0 { 0.9 } else { 0.0 };
        let z = 19.6 - 2.25 * 8.5;
        let b = a.player(1, Vec3::new(x(0), 0.0, z), 0.0);
        loose(&mut a, p, Vec3::new(0.0, 0.8, z), |_| {});
        let mut events = Vec::new();
        for k in 1..=12 {
            a.world.players.get_mut(&b).unwrap().movement.position.x = x(k);
            events.extend(a.tick());
        }
        let hit = arrow_hit(&events, p, HitTarget::Player(b));
        assert_eq!(hit.is_some(), hits, "{speed} m/tick: {hit:?}");
    }
}

#[test]
fn fast_projectiles_never_pass_through_thin_shapes() {
    // A dummy's head (r 0.26) from 40 m, at a tap's speed up to well beyond a parried arrow's:
    // every one touches it (and it stays the head).
    for speed in [55.0, 135.0, 250.0, 600.0] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::new(0.0, 0.0, 40.0), 0.0);
        let d = dummy(&mut a, Vec3::ZERO, 0.0, 1.0e5);
        let head = dummy_head(&a, d);
        let from = Vec3::new(0.0, head.y, 38.0);
        let id = a
            .world
            .spawn_projectile(
                &a.tuning,
                ProjectileKind::Arrow,
                Owner::Player(p),
                from,
                Vec3::NEG_Z * speed,
            )
            .expect("spawned");
        a.world.projectiles.get_mut(&id).unwrap().gravity = 0.0;
        let events = a.ticks(60);
        let part = events.iter().find_map(|e| match e {
            SimEvent::Damage(dmg) if dmg.target == HitTarget::Entity(d) => Some(dmg.info.part),
            _ => None,
        });
        assert_eq!(part, Some(HitPart::Weak(0)), "{speed} m/s");
    }
}
