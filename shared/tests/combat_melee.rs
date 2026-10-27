//! Melee weapons and abilities in the world (docs/arsenal.md, "Melee"): the Trimmer's slashes
//! (three targets, nearest first) and deflects, the Tamper's heavy arc, reach / arc / line of
//! sight / lag compensation, no weak points, Lunge, Whirl, Hurl, Upswing, kill credit, the
//! melee embers, and the owner's prediction staying exact.

mod combat_support;
mod weapon_support;

use std::f32::consts::PI;

use combat_support::*;
use glam::Vec3;
use shared::input::{Buttons, PlayerInput};
use shared::level::LevelKind;
use shared::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponKind};
use shared::protocol::{NetFrame, NetId, kind};
use shared::sim::{
    Attack, DamageFlags, HitPart, HitTarget, Owner, ParryTarget, PlayerId, ProjectileKind,
    SimEvent, step_weapons,
};
use shared::tuning::{CombatTuning, Tuning};
use weapon_support::*;

/// The shipped combat numbers (melee damage is the balance pass's to tune: docs/balance.md).
fn c() -> CombatTuning {
    Tuning::default().combat
}

/// A co-op arena: player 0 at the origin facing -Z, carrying only `build` (god mode, no
/// Mould targets them).
fn holding(build: WeaponBuild) -> (Arena, PlayerId) {
    holding_in(Arena::coop(), build)
}

fn holding_in(mut a: Arena, build: WeaponBuild) -> (Arena, PlayerId) {
    let p = a.player(0, Vec3::ZERO, 0.0);
    let pl = a.world.players.get_mut(&p).unwrap();
    pl.combat.god = true;
    pl.notarget = true;
    a.world.request_loadout(p, Loadout::single(build));
    a.ticks(2);
    (a, p)
}

fn trimmer() -> WeaponBuild {
    WeaponKind::Trimmer.default_build()
}

fn tamper() -> WeaponBuild {
    WeaponKind::Tamper.default_build()
}

fn build(weapon: WeaponKind, abilities: [Ability; 2]) -> WeaponBuild {
    let b = WeaponBuild { weapon, abilities };
    assert!(b.is_valid());
    b
}

/// The swings in `events`: (attack, targets struck).
fn swings(events: &[SimEvent]) -> Vec<(Attack, u8)> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Swing { attack, hits, .. } => Some((*attack, *hits)),
            _ => None,
        })
        .collect()
}

/// Every damage instance on `target` (its flags and knockback included).
fn hits_on(events: &[SimEvent], target: HitTarget) -> Vec<shared::sim::DamageEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) if d.target == target => Some(*d),
            _ => None,
        })
        .collect()
}

const TOUGH: f32 = 1.0e5;

#[test]
fn the_trimmer_slashes_the_three_nearest_in_its_arc() {
    let (mut a, p) = holding(trimmer());
    let near = [
        dummy(&mut a, Vec3::new(0.0, 0.0, -1.6), 0.0, TOUGH),
        dummy(&mut a, Vec3::new(0.9, 0.0, -2.0), 0.0, TOUGH),
        dummy(&mut a, Vec3::new(-0.9, 0.0, -2.3), 0.0, TOUGH),
    ];
    let farthest = dummy(&mut a, Vec3::new(0.0, 0.0, -2.9), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(swings(&ev), vec![(Attack::Trimmer, 3)]);
    for d in near {
        let hits = hits_on(&ev, HitTarget::Entity(d));
        assert_eq!(hits.len(), 1, "{d:?}");
        assert_eq!(hits[0].dealt, c().trimmer.damage);
        assert_eq!(hits[0].info.attack, Attack::Trimmer);
        assert!(hits[0].info.flags.contains(DamageFlags::MELEE));
    }
    assert_eq!(damage_to(&ev, HitTarget::Entity(farthest)), 0.0, "a fourth");

    // Held, it keeps swinging every 0.32 s (19 ticks): 0, 19, 38, 57.
    a.ticks(30);
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(60);
    a.set_buttons(p, Buttons::NONE);
    assert_eq!(swings(&ev).len(), 4);
}

#[test]
fn melee_ignores_weak_points() {
    let (mut a, p) = holding(trimmer());
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -2.0), 0.0, TOUGH);
    a.ticks(2);
    a.aim(p, dummy_head(&a, d));
    let ev = a.tap(p, Buttons::FIRE, 1);
    let hits = hits_on(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].dealt, c().trimmer.damage, "no head multiplier");
    assert!(!hits[0].info.flags.contains(DamageFlags::HEADSHOT));
    assert!(!hits[0].info.flags.contains(DamageFlags::WEAK_POINT));

    // PvP: a player's head neither.
    let mut a = Arena::pvp();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.request_loadout(p, Loadout::single(trimmer()));
    let v = a.player(1, Vec3::new(0.0, 0.0, -2.2), PI);
    a.ticks(2);
    a.world.players.get_mut(&v).unwrap().combat.protection = 0;
    let head = a.eye(v);
    a.aim(p, head);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(damage_to(&ev, HitTarget::Player(v)), c().trimmer.damage);
}

#[test]
fn swings_need_reach_the_arc_and_line_of_sight() {
    let (mut a, p) = holding(trimmer());
    let t = a.tuning.combat.clone();
    let pad = t.hit_assist.enemy_body_pad;
    // Beyond reach (from the eye to the padded surface), well outside the arc.
    let far = dummy(
        &mut a,
        Vec3::new(0.0, 0.0, -(t.trimmer.reach + pad + DUMMY_RADIUS + 0.6)),
        0.0,
        TOUGH,
    );
    let side = 2.4;
    let wide = dummy(
        &mut a,
        Vec3::new(
            (70f32).to_radians().sin() * side,
            0.0,
            -(70f32).to_radians().cos() * side,
        ),
        0.0,
        TOUGH,
    );
    // Low and right under the eye while looking ahead: still in the arc.
    let close = dummy(&mut a, Vec3::new(0.0, 0.0, -1.0), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(damage_to(&ev, HitTarget::Entity(far)), 0.0, "out of reach");
    assert_eq!(
        damage_to(&ev, HitTarget::Entity(wide)),
        0.0,
        "outside the arc"
    );
    assert_eq!(
        damage_to(&ev, HitTarget::Entity(close)),
        c().trimmer.damage,
        "at your feet"
    );

    // A wall between: nothing.
    let wall = brush([-3.0, 0.0, -1.5], [3.0, 3.0, -1.3]);
    let mut a = Arena::new(flat_level(LevelKind::Campaign, vec![wall]));
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.request_loadout(p, Loadout::single(trimmer()));
    let behind = dummy(&mut a, Vec3::new(0.0, 0.0, -2.4), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(
        damage_to(&ev, HitTarget::Entity(behind)),
        0.0,
        "through a wall"
    );
}

#[test]
fn swings_hit_what_the_swinger_saw() {
    // A dummy stood at A for a while and is now at B, out of reach. A swinger 10 ticks behind
    // swings at A: lag compensation registers it (as for hitscan); without it, a miss.
    let run = |use_history: bool| {
        let (mut a, p) = holding(trimmer());
        a.use_history = use_history;
        a.view_lag.insert(p, 10.0);
        let d = dummy(&mut a, Vec3::new(-1.0, 0.0, -2.2), 0.0, TOUGH);
        a.ticks(30);
        a.aim(p, dummy_body(&a, d));
        a.world.enemies.get_mut(&d).unwrap().position = Vec3::new(6.0, 0.0, -2.2);
        let ev = a.tap(p, Buttons::FIRE, 0);
        damage_to(&ev, HitTarget::Entity(d))
    };
    assert_eq!(run(true), c().trimmer.damage, "hit as seen");
    assert_eq!(
        run(false),
        0.0,
        "without lag compensation the same swing misses"
    );
}

#[test]
fn the_tamper_hits_everything_in_its_arc_hard() {
    let (mut a, p) = holding(tamper());
    let ds: Vec<_> = [-1.2f32, -0.4, 0.4, 1.2, 0.0]
        .iter()
        .enumerate()
        .map(|(i, x)| {
            dummy(
                &mut a,
                Vec3::new(*x, 0.0, -1.8 - i as f32 * 0.2),
                0.0,
                TOUGH,
            )
        })
        .collect();
    a.ticks(2);
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert_eq!(swings(&ev), vec![(Attack::Tamper, 5)]);
    for d in &ds {
        let hits = hits_on(&ev, HitTarget::Entity(*d));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].dealt, c().tamper.damage);
        let kb = hits[0].info.knockback;
        assert!((kb.length() - 16.0).abs() < 1e-3 && kb.z < 0.0, "{kb}");
    }
    // Every 0.85 s (51 ticks): 0, 51, 102.
    a.ticks(60);
    a.set_buttons(p, Buttons::FIRE);
    let ev = a.ticks(120);
    a.set_buttons(p, Buttons::NONE);
    assert_eq!(swings(&ev).len(), 3);
}

/// Player 0 hurt to 50 HP facing -Z holding `build`; a passive dummy 60 m away owns a Heron
/// needle (22 m/s, 20 dmg) flying at them from 20 m. Ticks until it is `at` metres from the
/// eye, then taps FIRE. (parried?, the tick's events, the needle).
fn swing_at_a_needle(
    build: WeaponBuild,
    at: f32,
) -> (Arena, PlayerId, Vec<SimEvent>, shared::protocol::NetId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    a.world.request_loadout(p, Loadout::single(build));
    let husk = a.husk(Vec3::new(0.0, 0.0, -60.0), PI);
    a.ticks(10);
    a.world.players.get_mut(&p).unwrap().combat.damage = 50.0;
    let t = a.tuning.clone();
    let eye = a.eye(p);
    let needle = a
        .world
        .spawn_enemy_projectile(
            &t,
            ProjectileKind::Needle,
            husk,
            Attack::Enemy {
                kind: kind::HERON,
                attack: 1,
            },
            Vec3::new(0.0, eye.y, -20.0),
            Vec3::new(0.0, 0.0, 22.0),
            20.0,
        )
        .expect("needle");
    while a.world.projectiles[&needle].position.distance(eye) > at {
        a.tick();
    }
    let ev = a.tap(p, Buttons::FIRE, 0);
    (a, p, ev, needle)
}

#[test]
fn trimmer_swings_deflect_parryable_projectiles() {
    let (a, p, ev, needle) = swing_at_a_needle(trimmer(), 2.6);
    let parried: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Parried(pe) => Some(*pe),
            _ => None,
        })
        .collect();
    assert_eq!(parried.len(), 1, "{ev:?}");
    assert_eq!(
        parried[0].target,
        ParryTarget::Projectile {
            id: needle,
            kind: ProjectileKind::Needle
        }
    );
    // A punch parry's reward and deflection.
    assert!(parried[0].healed > 0.0);
    assert_eq!(a.health(p), 100.0);
    let n = &a.world.projectiles[&needle];
    assert_eq!(n.owner, Owner::Player(p));
    assert!(n.parried && n.velocity.z < 0.0);

    // The Tamper doesn't: the needle comes through.
    let (mut a, p, ev, needle) = swing_at_a_needle(tamper(), 2.6);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::Parried(_))));
    let ev = a.ticks(60);
    assert!(!a.world.projectiles.contains_key(&needle));
    assert!(damage_to(&ev, HitTarget::Player(p)) > 0.0);
}

#[test]
fn lunge_dashes_and_strikes_the_first_target_it_reaches() {
    let (mut a, p) = holding(trimmer());
    let first = dummy(&mut a, Vec3::new(0.0, 0.0, -4.5), 0.0, TOUGH);
    let second = dummy(&mut a, Vec3::new(0.0, 0.0, -6.0), 0.0, TOUGH);
    a.ticks(2);
    // Ability A of the Trimmer's default build.
    a.select(p, 0, 0);
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(swings(&ev), vec![(Attack::Lunge, 0)]);
    let v = a.world.players[&p].movement.velocity;
    let lift = a.tuning.combat.lunge.lift;
    assert!(
        v.z < -12.0 && (v.y - lift).abs() < 1e-3,
        "an impulse along the aim, lifted: {v}"
    );
    let ev = a.ticks(30);
    let struck: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::LungeStruck { target, .. } => Some(*target),
            _ => None,
        })
        .collect();
    assert_eq!(struck, vec![HitTarget::Entity(first)]);
    let hits = hits_on(&ev, HitTarget::Entity(first));
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].dealt,
        c().trimmer.damage * c().lunge.damage_scale,
        "1.5 x the Trimmer's swing"
    );
    assert_eq!(hits[0].info.attack, Attack::Lunge);
    assert!(hits[0].info.flags.contains(DamageFlags::MELEE));
    assert_eq!(
        damage_to(&ev, HitTarget::Entity(second)),
        0.0,
        "only the first"
    );
    assert!(a.world.melee.lunges.is_empty());

    // A lunge into nothing runs out.
    let (mut a, p) = holding(build(WeaponKind::Tamper, [Ability::Lunge, Ability::Whirl]));
    let ev = a.tap(p, Buttons::ALT_FIRE, 40);
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::LungeStruck { .. })));
    assert!(a.world.melee.lunges.is_empty());
    // On the Tamper it is 1.5 x its swing.
    let (mut a, p) = holding(build(WeaponKind::Tamper, [Ability::Lunge, Ability::Whirl]));
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -4.0), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::ALT_FIRE, 30);
    assert_eq!(
        damage_to(&ev, HitTarget::Entity(d)),
        c().tamper.damage * c().lunge.damage_scale
    );
}

#[test]
fn whirl_hits_all_around_and_knocks_outward() {
    let (mut a, p) = holding(tamper());
    let around: Vec<(NetId, Vec3)> = [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z]
        .into_iter()
        .map(|dir| (dummy(&mut a, dir * 2.4, 0.0, TOUGH), dir))
        .collect();
    let far = dummy(&mut a, Vec3::new(6.0, 0.0, 0.0), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert_eq!(swings(&ev), vec![(Attack::Whirl, 4)]);
    for (d, dir) in around {
        let hits = hits_on(&ev, HitTarget::Entity(d));
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].dealt,
            c().tamper.damage * c().whirl.damage_scale,
            "1.5 x the Tamper's swing"
        );
        assert!(hits[0].info.knockback.dot(dir) > 8.0, "outward");
        assert!(hits[0].info.flags.contains(DamageFlags::MELEE));
    }
    assert_eq!(damage_to(&ev, HitTarget::Entity(far)), 0.0);

    // On the Trimmer: 1.5 x its swing.
    let (mut a, p) = holding(build(WeaponKind::Trimmer, [Ability::Whirl, Ability::Hurl]));
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, 2.0), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    assert_eq!(
        damage_to(&ev, HitTarget::Entity(d)),
        c().trimmer.damage * c().whirl.damage_scale,
        "behind you too"
    );
}

#[test]
fn hurl_throws_a_blade_or_a_hammer_head() {
    let scale = c().hurl.damage_scale;
    for (weapon, damage) in [
        (WeaponKind::Trimmer, c().trimmer.damage * scale),
        (WeaponKind::Tamper, c().tamper.damage * scale),
    ] {
        let (mut a, p) = holding(build(weapon, [Ability::Hurl, Ability::Lunge]));
        let d = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), 0.0, TOUGH);
        a.ticks(2);
        a.aim(p, dummy_body(&a, d));
        let ev = a.tap(p, Buttons::ALT_FIRE, 0);
        assert!(swings(&ev).is_empty(), "a throw, not a swing");
        let (id, proj) = a
            .world
            .projectiles
            .iter()
            .find(|(_, p)| p.kind == ProjectileKind::Hurl)
            .map(|(id, p)| (*id, p.clone()))
            .expect("in flight");
        // Replicated with the carrier: a blade or a hammer-head.
        let mut frame = NetFrame::new();
        a.world.replicate(&a.tuning, &mut frame);
        let e = frame.get(id).expect("replicated");
        assert_eq!(e.kind, kind::HURL);
        assert_eq!(e.state, weapon.id() as u16);
        assert_eq!(proj.attack, Attack::Hurl);
        let ev = a.ticks(40);
        let hits = hits_on(&ev, HitTarget::Entity(d));
        assert_eq!(hits.len(), 1, "{weapon:?}");
        assert!(
            (hits[0].dealt - damage).abs() < 1e-3,
            "{weapon:?}: {}",
            hits[0].dealt
        );
        assert!(hits[0].info.flags.contains(DamageFlags::PROJECTILE));
        // Aimed at the head it still strikes the body: melee has no weak points (a Tamper's
        // Hurl to a PvP head was a one-shot).
        let (mut a, p) = holding(build(weapon, [Ability::Hurl, Ability::Lunge]));
        let d = dummy(&mut a, Vec3::new(0.0, 0.0, -10.0), 0.0, TOUGH);
        a.ticks(2);
        a.aim(p, dummy_head(&a, d));
        let ev = a.tap(p, Buttons::ALT_FIRE, 40);
        let hits = hits_on(&ev, HitTarget::Entity(d));
        assert_eq!(hits.len(), 1, "{weapon:?} at the head");
        assert_eq!(hits[0].info.part, HitPart::Body);
        assert!((hits[0].dealt - damage).abs() < 1e-3, "{}", hits[0].dealt);
    }
}

#[test]
fn upswing_launches_what_it_hits_and_hops_the_swinger() {
    let (mut a, p) = holding(tamper());
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -2.0), 0.0, TOUGH);
    a.ticks(2);
    a.select(p, 0, 1);
    a.tick();
    let ev = a.tap(p, Buttons::ALT_FIRE, 0);
    assert_eq!(swings(&ev), vec![(Attack::Upswing, 1)]);
    let hits = hits_on(&ev, HitTarget::Entity(d));
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].dealt,
        c().tamper.damage * c().upswing.damage_scale,
        "1.4 x the Tamper's swing"
    );
    assert_eq!(hits[0].info.knockback, Vec3::Y * 14.0);
    let vy = a.world.players[&p].movement.velocity.y;
    assert!(vy >= 5.0 - 1e-3, "the hop: {vy}");
}

#[test]
fn melee_kills_name_the_weapon_and_heal_at_the_melee_rate() {
    // A Whirl kill on the Tamper (both melee weapons can carry it): the Tamper fired it.
    let (mut a, p) = holding(tamper());
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -2.0), 0.0, 50.0);
    a.ticks(2);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    let k = kills(&ev);
    assert_eq!(k.len(), 1);
    assert_eq!(k[0].attack, Attack::Whirl);
    assert_eq!(k[0].weapon, Some(WeaponKind::Tamper));
    assert_eq!(
        Attack::Whirl.weapon(),
        None,
        "an ability two weapons can carry"
    );
    assert!(!a.world.enemies.get(&d).is_some_and(|e| e.is_alive()));

    // A slash sheds embers at `melee_weapon_heal_fraction` of the damage (not the fist's).
    let (mut a, p) = holding(trimmer());
    dummy(&mut a, Vec3::new(0.0, 0.0, -2.0), 0.0, TOUGH);
    a.ticks(2);
    a.tap(p, Buttons::FIRE, 0);
    let pool: f32 = a.world.embers.values().map(|b| b.pool).sum();
    let f = a.tuning.combat.embers.melee_weapon_heal_fraction;
    assert!((pool - c().trimmer.damage * f).abs() < 1e-3, "{pool}");
}

#[test]
fn ability_power_scales_melee_abilities_off_their_home_weapon() {
    let (mut a, p) = holding(build(WeaponKind::Trimmer, [Ability::Whirl, Ability::Hurl]));
    // Whirl's home is the Tamper: a weaker carrier hits for less (never more).
    a.tuning.combat.trimmer.ability_power = 0.5;
    let d = dummy(&mut a, Vec3::new(0.0, 0.0, -2.0), 0.0, TOUGH);
    a.ticks(2);
    let ev = a.tap(p, Buttons::ALT_FIRE, 1);
    let half = c().trimmer.damage * c().whirl.damage_scale * 0.5;
    assert!((damage_to(&ev, HitTarget::Entity(d)) - half).abs() < 1e-3);
}

#[test]
fn melee_and_knack_prediction_stays_exact() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().combat.god = true;
    let loadout = Loadout {
        slots: [
            Some(trimmer()),
            Some(tamper()),
            None,
            Some(WeaponKind::Quill.default_build()),
        ],
        knack: Knack::Surge,
    };
    assert!(loadout.is_valid());
    a.world.request_loadout(p, loadout);
    for i in 0..4 {
        dummy(&mut a, Vec3::new(i as f32 - 1.5, 0.0, -2.5), 0.0, 1.0e6);
    }
    let t = a.tuning.clone();
    let mut predicted = a.world.players[&p].combat.weapons;
    for i in 0..1500u32 {
        let phase = i / 60;
        let input = a.held.get_mut(&p).unwrap();
        input.weapon = (phase % 5) as u8; // 2 = empty, 4 = out of range: keep the weapon
        input.weapon_mode = ((phase / 5) % 2) as u8;
        input.yaw = (i as f32 * 0.02).sin() * 0.8;
        let mut b = Buttons::NONE;
        b.set(Buttons::FIRE, i % 37 < 22);
        b.set(Buttons::ALT_FIRE, i % 53 < 9);
        b.set(Buttons::PUNCH, i % 45 == 0);
        b.set(Buttons::THROW, i % 47 == 5);
        input.buttons = b;
        let sent: PlayerInput = *input;
        a.tick();
        step_weapons(&mut predicted, &sent, true, &t.combat, 60);
        assert_eq!(a.world.players[&p].combat.weapons, predicted, "input {i}");
    }
    let swung: Vec<Attack> = swings(&a.events).into_iter().map(|s| s.0).collect();
    for attack in [
        Attack::Trimmer,
        Attack::Tamper,
        Attack::Lunge,
        Attack::Whirl,
        Attack::Upswing,
    ] {
        assert!(swung.contains(&attack), "{attack:?}");
    }
    assert!(a.events.iter().any(|e| matches!(
        e,
        SimEvent::KnackUsed {
            knack: Knack::Surge,
            ..
        }
    )));
}

#[test]
fn melee_pvp_numbers_keep_the_one_shot_rules_and_the_time_to_kill_band() {
    use shared::lobby::GameMode;
    let pvp = shared::tuning::Tuning::default()
        .for_mode(GameMode::Deathmatch)
        .combat;
    // No body one-shot from any swing, ability or the Snare (melee has no headshots).
    for weapon in [WeaponKind::Trimmer, WeaponKind::Tamper] {
        let primary = weapon.primary_damage(&pvp);
        for (name, damage) in [
            ("swing", primary),
            ("lunge", primary * pvp.lunge.damage_scale),
            ("whirl", primary * pvp.whirl.damage_scale),
            ("hurl", primary * pvp.hurl.damage_scale),
            ("upswing", primary * pvp.upswing.damage_scale),
        ] {
            assert!(damage < 100.0, "{weapon:?} {name}: {damage}");
        }
    }
    assert!(pvp.knacks.snare.damage < 100.0);
    // Time to kill, every swing landing: inside the PvP band (0.8-1.8 s).
    let ttk = |damage: f32, interval: f32| {
        let hits = (100.0 / damage).ceil() - 1.0;
        hits * (interval * 60.0).round() / 60.0
    };
    for (name, secs) in [
        ("trimmer", ttk(pvp.trimmer.damage, pvp.trimmer.interval)),
        ("tamper", ttk(pvp.tamper.damage, pvp.tamper.interval)),
    ] {
        assert!((0.8..=1.8).contains(&secs), "{name}: {secs:.2} s");
    }
}
