//! Style meter math (bonuses, decay, rank up / down, freshness and staleness, close call,
//! multikill windows) and style scoring of real combat in the simulation. Also checks that
//! `assets/tuning.ron` spells out every style / modes value equal to the defaults.

mod combat_support;

use std::f32::consts::PI;
use std::path::Path;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::protocol::kind;
use shared::sim::style::{
    PlayerStyle, RANK_NAMES, decay_rate, is_close_call, kill_bonuses, rank_name, ricochet_points,
};
use shared::sim::{
    Attack, DamageFlags, HitPart, HitTarget, KillInfo, Owner, PlayerId, SimEvent, StyleBonus,
    StyleEvent, StyleKey,
};
use shared::tuning::{ModeTuning, StyleTuning, Tuning};

fn st() -> StyleTuning {
    StyleTuning::default()
}

fn kill(flags: DamageFlags, attack: Attack) -> KillInfo {
    KillInfo {
        victim: HitTarget::Entity(shared::protocol::NetId(2000)),
        victim_kind: kind::GNAWER,
        killer: Owner::Player(PlayerId(0)),
        attack,
        flags,
        part: HitPart::Body,
        position: Vec3::ZERO,
        ricochets: 0,
        killer_airborne: false,
        victim_airborne: false,
        parried_projectile: false,
        killer_health: 100.0,
        killer_effective_max: 100.0,
        overkill: 0.0,
        tick: 100,
    }
}

fn labels(items: &[(StyleBonus, f32, u8)]) -> Vec<StyleBonus> {
    items.iter().map(|i| i.0).collect()
}

fn bonuses(events: &[StyleEvent]) -> Vec<(StyleBonus, f32)> {
    events
        .iter()
        .filter_map(|e| match e {
            StyleEvent::Bonus { bonus, points, .. } => Some((*bonus, *points)),
            _ => None,
        })
        .collect()
}

#[test]
fn every_kill_bonus_has_its_trigger() {
    let t = st();
    let plain = kill(DamageFlags::HITSCAN, Attack::Lancet);
    assert_eq!(
        kill_bonuses(&plain, false, 1, &t),
        vec![(StyleBonus::Kill, 30.0, 0)]
    );

    let head = kill(DamageFlags::HITSCAN | DamageFlags::HEADSHOT, Attack::Lancet);
    assert_eq!(
        labels(&kill_bonuses(&head, false, 1, &t)),
        [StyleBonus::Kill, StyleBonus::Headshot]
    );

    let mut air = plain;
    air.killer_airborne = true;
    air.victim_airborne = true;
    assert_eq!(
        labels(&kill_bonuses(&air, false, 1, &t)),
        [StyleBonus::Kill, StyleBonus::Airkill, StyleBonus::Airshot]
    );
    // Shooting a flying enemy out of the air it lives in is no AIRSHOT.
    assert_eq!(
        labels(&kill_bonuses(&air, true, 1, &t)),
        [StyleBonus::Kill, StyleBonus::Airkill]
    );

    let mut rts = kill(
        DamageFlags::PROJECTILE | DamageFlags::PARRIED,
        Attack::Enemy {
            kind: kind::CENSER,
            attack: 0,
        },
    );
    rts.parried_projectile = true;
    assert_eq!(
        kill_bonuses(&rts, false, 1, &t),
        vec![
            (StyleBonus::Kill, 30.0, 0),
            (StyleBonus::ReturnToSender, 80.0, 0)
        ]
    );

    let boom = kill(DamageFlags::EXPLOSIVE, Attack::Rocket);
    assert_eq!(
        kill_bonuses(&boom, false, 1, &t)[1],
        (StyleBonus::Exploded, 20.0, 0)
    );
    let slam = kill(DamageFlags::SLAM, Attack::Slam);
    assert_eq!(
        labels(&kill_bonuses(&slam, false, 1, &t)),
        [StyleBonus::Kill, StyleBonus::SlamKill]
    );
    let fist = kill(DamageFlags::MELEE, Attack::Fist);
    assert_eq!(
        kill_bonuses(&fist, false, 1, &t)[1],
        (StyleBonus::Fisted, 30.0, 0)
    );
    let mut frag = plain;
    frag.victim = HitTarget::Player(PlayerId(1));
    frag.victim_kind = kind::PLAYER;
    assert_eq!(
        kill_bonuses(&frag, false, 1, &t)[0],
        (StyleBonus::Frag, 100.0, 0)
    );

    // MULTIKILL: the n-th kill in the window adds 40 (n kills = 40 x (n - 1) in total).
    assert_eq!(
        kill_bonuses(&plain, false, 3, &t)[1],
        (StyleBonus::Multikill, 40.0, 3)
    );
}

#[test]
fn ricochet_points_grow_with_the_chain() {
    let t = st();
    assert_eq!(ricochet_points(1, &t), 30.0);
    assert_eq!(ricochet_points(2, &t), 50.0);
    assert_eq!(ricochet_points(3, &t), 70.0);
}

#[test]
fn decay_rises_with_rank() {
    let t = st();
    assert_eq!(decay_rate(0, &t), 4.0);
    assert_eq!(decay_rate(7, &t), 30.0);
    assert!(decay_rate(3, &t) > decay_rate(2, &t));
    assert_eq!(RANK_NAMES.len(), 8);
    assert_eq!(rank_name(4), "MERCILESS");
}

#[test]
fn points_rank_up_and_decay_ranks_down() {
    let t = st();
    let mut s = PlayerStyle::new();
    assert_eq!(s.add_points(&t, 60.0), 0);
    assert_eq!((s.rank, s.meter), (0, 60.0));
    // 60 + 250 = 310: three ranks up, 10 into FERAL+1 = VICIOUS.
    assert_eq!(s.add_points(&t, 250.0), 3);
    assert_eq!(s.rank, 3);
    assert!((s.meter - 10.0).abs() < 1e-4);
    assert_eq!(s.best_rank, 3);
    // VICIOUS decays at 4 + 26 * 3/7 per second: 10 points last well under a second, but a
    // fresh rank holds for `rank_grace` (1 s) with an empty bar.
    for _ in 0..59 {
        assert!(!s.decay(&t, 1.0 / 60.0), "grace");
    }
    assert_eq!((s.rank, s.meter), (3, 0.0));
    let mut dropped = false;
    for _ in 0..3 {
        dropped |= s.decay(&t, 1.0 / 60.0);
    }
    assert!(dropped);
    assert_eq!(s.rank, 2, "rank down below zero");
    // Restarts at 60 % of the lower bar.
    assert!(s.meter > 55.0 && s.meter <= 60.01, "{}", s.meter);
    assert_eq!(s.best_rank, 3, "best rank is kept");
    // DULL bottoms out at zero.
    let mut d = PlayerStyle::new();
    d.add_points(&t, 5.0);
    for _ in 0..120 {
        assert!(!d.decay(&t, 1.0 / 60.0));
    }
    assert_eq!((d.rank, d.meter), (0, 0.0));
    // VELOCIDE caps with a full bar.
    let mut v = PlayerStyle::new();
    v.add_points(&t, 5000.0);
    assert_eq!(v.rank, 7);
    assert_eq!(v.fraction(&t), 1.0);
}

#[test]
fn freshness_costs_restores_and_floors() {
    let t = st();
    let mut s = PlayerStyle::new();
    assert_eq!(s.use_key(&t, StyleKey::Lancet, 1), 1.0);
    assert!((s.freshness(StyleKey::Lancet) - 0.88).abs() < 1e-5);
    // Same key, same tick: charged once, same multiplier.
    assert_eq!(s.use_key(&t, StyleKey::Lancet, 1), 1.0);
    assert!((s.freshness(StyleKey::Lancet) - 0.88).abs() < 1e-5);
    for tick in 2..20 {
        s.use_key(&t, StyleKey::Lancet, tick);
    }
    assert_eq!(s.freshness(StyleKey::Lancet), t.freshness_min, "floor");
    // Using something else restores it a little.
    s.use_key(&t, StyleKey::Slug, 30);
    assert!((s.freshness(StyleKey::Lancet) - 0.24).abs() < 1e-5);
    assert!((s.freshness(StyleKey::Slug) - 0.88).abs() < 1e-5);
    // And time heals: 0.02 / s.
    s.regen(&t, 10.0);
    assert!((s.freshness(StyleKey::Lancet) - 0.44).abs() < 1e-4);
    assert_eq!(s.freshness(StyleKey::Fist), 1.0);
}

#[test]
fn stale_gains_are_cut_and_reported() {
    let t = st();
    let mut s = PlayerStyle::new();
    let p = PlayerId(0);
    let mut ev = Vec::new();
    let items = [(StyleBonus::Kill, 30.0, 0)];
    // Fresh: full points.
    s.award(&t, p, Some(StyleKey::Thresher), &items, false, 1, &mut ev);
    assert_eq!(bonuses(&ev), [(StyleBonus::Kill, 30.0)]);
    // Spam the scattergun until it is stale.
    for tick in 2..8 {
        s.award(
            &t,
            p,
            Some(StyleKey::Thresher),
            &items,
            false,
            tick,
            &mut ev,
        );
    }
    ev.clear();
    let f = s.freshness(StyleKey::Thresher);
    assert!(f < t.stale_threshold);
    s.award(&t, p, Some(StyleKey::Thresher), &items, false, 10, &mut ev);
    let b = bonuses(&ev);
    assert!((b[0].1 - 30.0 * f).abs() < 1e-4, "gain x freshness");
    assert_eq!(
        b.last().unwrap().0,
        StyleBonus::Stale,
        "- STALE in the feed"
    );
    assert!(s.view(&t).is_stale(StyleKey::Thresher, &t));
    assert!(!s.view(&t).is_stale(StyleKey::Lancet, &t));
}

#[test]
fn close_call_multiplies_the_kill() {
    let t = st();
    let mut k = kill(DamageFlags::HITSCAN | DamageFlags::HEADSHOT, Attack::Lancet);
    assert!(!is_close_call(&k, &t));
    k.killer_health = 20.0;
    assert!(is_close_call(&k, &t));
    let mut s = PlayerStyle::new();
    let mut ev = Vec::new();
    let items = kill_bonuses(&k, false, 1, &t);
    s.award(
        &t,
        PlayerId(0),
        Some(StyleKey::Lancet),
        &items,
        true,
        1,
        &mut ev,
    );
    let b = bonuses(&ev);
    assert_eq!(
        b.last().unwrap(),
        &(StyleBonus::CloseCall, 22.5),
        "(30 + 15) x 0.5"
    );
    assert!((s.total - 67.5).abs() < 1e-4, "x1.5 overall");
    assert!(
        ev.iter().all(|e| !matches!(e, StyleEvent::Rank { .. })),
        "67.5 < 100: no rank-up yet"
    );
}

#[test]
fn award_ranks_up_with_an_event() {
    let t = st();
    let mut s = PlayerStyle::new();
    let mut ev = Vec::new();
    s.award(
        &t,
        PlayerId(3),
        Some(StyleKey::Parry),
        &[
            (StyleBonus::Parry, 60.0, 0),
            (StyleBonus::ReturnToSender, 80.0, 0),
        ],
        false,
        1,
        &mut ev,
    );
    assert_eq!(
        ev.last(),
        Some(&StyleEvent::Rank {
            player: PlayerId(3),
            rank: 1,
            up: true
        })
    );
    assert!(s.reset());
    assert_eq!(s.rank, 0);
}

#[test]
fn multikill_window_counts_kills_within_it() {
    let mut s = PlayerStyle::new();
    let window = 36; // 0.6 s at 60 Hz
    assert_eq!(s.register_kill(100, window), 1);
    assert_eq!(s.register_kill(110, window), 2);
    assert_eq!(s.register_kill(130, window), 3);
    // 0.7 s after the first: the first one dropped out.
    assert_eq!(s.register_kill(142, window), 3);
    // A second later: alone again.
    assert_eq!(s.register_kill(250, window), 1);
}

// ---- in the simulation ------------------------------------------------------------------------

fn style_events(events: &[SimEvent]) -> Vec<StyleEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            SimEvent::Style(s) => Some(*s),
            _ => None,
        })
        .collect()
}

fn range(husks: &[Vec3]) -> (Arena, PlayerId, Vec<shared::protocol::NetId>) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    a.world.players.get_mut(&p).unwrap().notarget = true;
    let ids = husks.iter().map(|pos| a.husk(*pos, PI)).collect();
    a.ticks(10);
    (a, p, ids)
}

#[test]
fn a_headshot_kill_scores_kill_and_headshot() {
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -15.0)]);
    let head = a.husk_head(h[0]);
    a.aim(p, head);
    let ev = a.tap(p, Buttons::FIRE, 1);
    let b = bonuses(&style_events(&ev));
    assert_eq!(b, [(StyleBonus::Kill, 30.0), (StyleBonus::Headshot, 15.0)]);
    let s = &a.world.style.players[&p];
    assert_eq!(s.total, 45.0);
    assert!(s.freshness(StyleKey::Lancet) < 1.0);
    // The owner's view and the scoreboard follow.
    let v = a.world.style.view(p, &a.tuning.style);
    assert_eq!(v.rank, 0);
    assert!(v.fraction() > 0.4);
    assert_eq!(a.world.mode.stats(p).kills, 1);
}

#[test]
fn a_rocket_through_a_crowd_is_a_multikill() {
    let (mut a, p, h) = range(&[
        Vec3::new(-0.8, 0.0, -14.0),
        Vec3::new(0.8, 0.0, -14.0),
        Vec3::new(0.0, 0.0, -15.0),
    ]);
    a.select(p, 3, 0);
    let target = a.husk_body(h[2]);
    a.aim(p, target);
    let ev = a.tap(p, Buttons::FIRE, 40);
    let style = style_events(&ev);
    let b = bonuses(&style);
    let kills = b.iter().filter(|x| x.0 == StyleBonus::Kill).count();
    assert_eq!(kills, 3, "{b:?}");
    let exploded = b.iter().filter(|x| x.0 == StyleBonus::Exploded).count();
    assert_eq!(exploded, 3);
    let multi: Vec<_> = b.iter().filter(|x| x.0 == StyleBonus::Multikill).collect();
    assert_eq!(multi.len(), 2, "3 kills = 2 multikill awards");
    // One rocket = one use of the Cinderbox: every kill got full points.
    assert!(b.iter().all(|x| x.1 >= 20.0), "{b:?}");
    // 3 x (30 + 20) + 2 x 40 = 230: two ranks up.
    assert_eq!(a.world.style.players[&p].rank, 2);
    assert!(style.iter().any(|e| matches!(
        e,
        StyleEvent::Rank {
            rank: 2,
            up: true,
            ..
        }
    )));
}

#[test]
fn parries_score_and_meter_decays_back_down() {
    let (mut a, p, h) = range(&[Vec3::new(0.0, 0.0, -60.0)]);
    let t = a.tuning.clone();
    let orb = a
        .world
        .spawn_enemy_projectile(
            &t,
            shared::sim::ProjectileKind::Needle,
            h[0],
            Attack::Enemy {
                kind: kind::HERON,
                attack: 0,
            },
            Vec3::new(0.0, 1.4, -20.0),
            Vec3::new(0.0, 0.0, 22.0),
            20.0,
        )
        .expect("orb");
    let mut all = Vec::new();
    for _ in 0..200 {
        let d = a.world.projectiles[&orb].position.distance(a.eye(p));
        if d < 2.0 {
            break;
        }
        all.extend(a.tick());
    }
    all.extend(a.tap(p, Buttons::PUNCH, 2));
    let b = bonuses(&style_events(&all));
    assert!(b.contains(&(StyleBonus::Parry, 60.0)), "{b:?}");
    let before = a.world.style.players[&p].meter;
    a.ticks(60);
    assert!(a.world.style.players[&p].meter < before, "decays");
}

#[test]
fn a_ricochet_kill_scores_the_bounce() {
    let (mut a, p, _) = range(&[]);
    let husk = a.husk(Vec3::new(9.0, 0.0, -6.0), -PI / 2.0);
    let t = a.tuning.clone();
    let prism = a
        .world
        .spawn_prism(&t, p, Vec3::new(0.0, 1.6, -6.0), Vec3::ZERO)
        .expect("prism");
    {
        let pr = a.world.prisms.get_mut(&prism).unwrap();
        pr.position = Vec3::new(0.0, 1.6, -6.0);
        pr.resting = true;
    }
    a.ticks(5);
    a.aim(p, Vec3::new(0.4, 1.6, -6.0));
    let ev = a.tap(p, Buttons::FIRE, 1);
    assert!(!a.enemy_alive(husk));
    let b = bonuses(&style_events(&ev));
    // KILL + HEADSHOT with the Lancet, then RICOCHET (same tick, same key: charged once).
    assert!(b.contains(&(StyleBonus::Kill, 30.0)), "{b:?}");
    assert!(b.contains(&(StyleBonus::Headshot, 15.0)), "{b:?}");
    assert!(b.contains(&(StyleBonus::Ricochet, 30.0)), "{b:?}");
}

#[test]
fn dying_empties_the_meter() {
    let (mut a, p, _) = range(&[]);
    a.world.style.player(p).add_points(&a.tuning.style, 250.0);
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
    let ev = a.tick();
    let _ = ev;
    assert_eq!(a.world.style.players[&p].rank, 0);
    assert!(a.events.iter().any(|e| matches!(
        e,
        SimEvent::Style(StyleEvent::Rank {
            rank: 0,
            up: false,
            ..
        })
    )));
}

#[test]
fn tuning_file_has_every_style_and_mode_value() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
    let text = std::fs::read_to_string(&path).expect("assets/tuning.ron exists");
    let tuning: Tuning = ron::from_str(&text).expect("assets/tuning.ron parses");
    assert_eq!(tuning.style, StyleTuning::default());
    assert_eq!(tuning.modes, ModeTuning::default());
    for defaults in [
        ron::to_string(&StyleTuning::default()).unwrap(),
        ron::to_string(&ModeTuning::default()).unwrap(),
    ] {
        let fields: Vec<&str> = defaults
            .split([',', '('])
            .filter_map(|kv| kv.split_once(':').map(|(k, _)| k.trim()))
            .filter(|k| !k.is_empty())
            .collect();
        assert!(fields.len() > 10, "{fields:?}");
        for field in fields {
            assert!(
                text.contains(&format!("{field}:")),
                "assets/tuning.ron is missing `{field}`"
            );
        }
    }
}
