//! The third roster and the difficulty hooks (docs/enemies.md): the Bellwether's toll frenzies
//! nearby Moulds (and only them, for its duration) and shooting its bell mid-swing cancels it;
//! the Snuffer latches on, smothers the wick with hard damage, slows its victim modestly and
//! is punched or dashed off, one per player; the Smelter's pour lays a glowing line first and
//! only burns after its wind-up, along the floor, stopping at walls, and a cancelled pour
//! leaves nothing; the Effigy never targets, attacks, moves or dies and heals back to full;
//! elites; `SimWorld::enemy_scale` (damage, aggression, projectile speed) and frenzy stacking.

mod combat_support;

use combat_support::*;
use glam::Vec3;
use shared::input::Buttons;
use shared::protocol::{NetFrame, NetId, enemy_flags, kind, slag_flags};
use shared::sim::{
    AiState, Attack, DamageInfo, EnemyAttackPhase, EnemyScale, HitPart, HitTarget, Owner, PlayerId,
    ProjectileKind, SimEvent, SpawnOptions,
};

fn spawn(a: &mut Arena, name: &str, pos: Vec3) -> NetId {
    a.world.spawn_enemy(name, pos, 0.0).expect("enemy exists")
}

fn god(a: &mut Arena, p: PlayerId) {
    a.world.players.get_mut(&p).unwrap().combat.god = true;
}

fn attack_index(a: &Arena, def: &str, name: &str) -> u8 {
    a.world
        .enemy_defs
        .get(def)
        .unwrap()
        .attacks
        .iter()
        .position(|x| x.name == name)
        .unwrap() as u8
}

/// Ticks until `pred` holds (at most `max`); returns the events of those ticks, or `None`.
fn until(a: &mut Arena, max: u32, pred: impl Fn(&Arena) -> bool) -> Option<Vec<SimEvent>> {
    let mut all = Vec::new();
    for _ in 0..max {
        all.extend(a.tick());
        if pred(a) {
            return Some(all);
        }
    }
    None
}

fn state(a: &Arena, id: NetId) -> AiState {
    a.world.enemies[&id].state
}

fn flags(a: &Arena, id: NetId) -> u32 {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).map_or(0, |e| e.flags)
}

/// (tick, attack) of the strikes of `id` in `ev`.
fn strikes(ev: &[SimEvent], id: NetId, at: u32) -> Vec<(u32, u8)> {
    ev.iter()
        .filter_map(|e| match e {
            SimEvent::EnemyAttack {
                id: eid,
                attack,
                phase: EnemyAttackPhase::Strike,
                ..
            } if *eid == id => Some((at, *attack)),
            _ => None,
        })
        .collect()
}

/// Damage dealt to `p` by attack `attack` of enemy kind `k`.
fn damage_by(ev: &[SimEvent], p: PlayerId, k: u16, attack: u8) -> Vec<f32> {
    ev.iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d)
                if d.target == HitTarget::Player(p)
                    && d.info.attack == Attack::Enemy { kind: k, attack } =>
            {
                Some(d.dealt)
            }
            _ => None,
        })
        .collect()
}

// ---- Bellwether --------------------------------------------------------------------------------

#[test]
fn the_toll_frenzies_nearby_moulds_for_its_duration() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::new(0.0, 0.0, 30.0), 0.0);
    god(&mut a, p);
    let bell = spawn(&mut a, "bellwether", Vec3::new(0.0, 0.0, 10.0));
    // Censers hold their 14-30 m band: one stays near the ringer, one is far off.
    let near = spawn(&mut a, "censer", Vec3::new(6.0, 0.0, 6.0));
    let far = spawn(&mut a, "censer", Vec3::new(-30.0, 0.0, -20.0));
    let other_bell = spawn(&mut a, "bellwether", Vec3::new(-4.0, 0.0, 8.0));
    let toll = attack_index(&a, "bellwether", "toll");
    let mut windup_at = None;
    let mut strike_at = None;
    for _ in 0..900 {
        let ev = a.tick();
        let now = a.world.tick;
        for e in &ev {
            if let SimEvent::EnemyAttack {
                id, attack, phase, ..
            } = e
                && *id == bell
                && *attack == toll
            {
                match phase {
                    EnemyAttackPhase::Windup => windup_at = Some(now),
                    EnemyAttackPhase::Strike => strike_at = Some(now),
                }
            }
        }
        if strike_at.is_some() {
            break;
        }
        // Nothing is frenzied before the bell rings.
        assert_eq!(flags(&a, near) & enemy_flags::FRENZIED, 0);
    }
    let (w, s) = (windup_at.expect("wound up"), strike_at.expect("tolled"));
    println!("toll wind-up at {w}, rings at {s}");
    assert_eq!(s - w, 96, "a long 1.6 s wind-up");
    let rung = a.world.tick;
    assert_ne!(
        flags(&a, near) & enemy_flags::FRENZIED,
        0,
        "the ally in reach"
    );
    assert_eq!(flags(&a, far) & enemy_flags::FRENZIED, 0, "out of reach");
    assert_eq!(flags(&a, bell) & enemy_flags::FRENZIED, 0, "not itself");
    assert_eq!(
        flags(&a, other_bell) & enemy_flags::FRENZIED,
        0,
        "ringers don't frenzy each other"
    );
    // The frenzy lasts its 5 s (unless the bell rings again: kill the ringers).
    a.world.remove_enemy(bell);
    a.world.remove_enemy(other_bell);
    until(&mut a, 400, |a| flags(a, near) & enemy_flags::FRENZIED == 0).expect("it wears off");
    let lasted = a.world.tick - rung;
    assert!((299..=302).contains(&lasted), "frenzied for {lasted} ticks");
}

#[test]
fn a_lone_bellwether_never_tolls_but_shoves_rushers() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let bell = spawn(&mut a, "bellwether", Vec3::new(0.0, 0.0, -20.0));
    let toll = attack_index(&a, "bellwether", "toll");
    let ev = a.ticks(600);
    assert!(
        !ev.iter()
            .any(|e| matches!(e, SimEvent::EnemyAttack { id, attack, .. }
            if *id == bell && *attack == toll)),
        "no ally to frenzy: no toll"
    );
    // It keeps its distance.
    let d = a.world.enemies[&bell].position.length();
    assert!((12.0..=28.0).contains(&d), "keeps 14-26 m: {d}");
}

#[test]
fn shooting_the_bell_mid_swing_cancels_the_toll() {
    let run = |hits: &[(HitPart, f32)]| {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::new(0.0, 0.0, 25.0), 0.0);
        god(&mut a, p);
        let bell = spawn(&mut a, "bellwether", Vec3::new(0.0, 0.0, 5.0));
        let ally = spawn(&mut a, "censer", Vec3::new(4.0, 0.0, 4.0));
        let toll = attack_index(&a, "bellwether", "toll");
        until(&mut a, 900, |a| {
            let e = &a.world.enemies[&bell];
            e.state == AiState::Windup && e.attack == toll as usize && e.state_ticks >= 40
        })
        .expect("winding up the toll");
        let t = a.tuning.clone();
        for &(part, amount) in hits {
            let e = &a.world.enemies[&bell];
            let at = e.local_to_world(e.def.weak_points[0].offset);
            let info = DamageInfo::new(amount, Owner::Player(p), Attack::Quill).at(at, Vec3::NEG_Z);
            a.world
                .apply_damage(&t, HitTarget::Entity(bell), info.part(part));
        }
        let ev = a.ticks(80);
        let tolled = !strikes(&ev, bell, 0).is_empty();
        let frenzied = a.world.enemies[&ally].frenzied(a.world.tick);
        (tolled, frenzied, state(&a, bell))
    };
    // A 15-damage quill on the bell (x2 = 30): the swing cracks.
    let (tolled, frenzied, s) = run(&[(HitPart::Weak(0), 15.0)]);
    assert!(!tolled && !frenzied, "cancelled");
    assert_eq!(s, AiState::Stunned, "staggered");
    // Two small hits add up (10 + 10, x2).
    let (tolled, _, s) = run(&[(HitPart::Weak(0), 10.0), (HitPart::Weak(0), 10.0)]);
    assert!(!tolled && s == AiState::Stunned, "accumulated");
    // Body hits don't stop it; neither does too little on the bell.
    let (tolled, frenzied, _) = run(&[(HitPart::Body, 60.0)]);
    assert!(tolled && frenzied, "body hits don't interrupt");
    let (tolled, _, _) = run(&[(HitPart::Weak(0), 10.0)]);
    assert!(tolled, "20 on the bell is not enough");
}

// ---- Snuffer -----------------------------------------------------------------------------------

/// A player at the origin facing -Z and a Snuffer 9 m in front: ticks until it latches on.
fn latched_duel() -> (Arena, PlayerId, NetId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let s = spawn(&mut a, "snuffer", Vec3::new(0.0, 0.0, -9.0));
    until(&mut a, 300, |a| state(a, s) == AiState::Latched).expect("latched on");
    (a, p, s)
}

#[test]
fn a_snuffer_latches_on_and_smothers_the_wick() {
    let (mut a, p, s) = latched_duel();
    let snuffer = kind::SNUFFER;
    let latch = attack_index(&a, "snuffer", "latch");
    // Replicated: LATCHED, owner = the victim.
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    let e = frame.get(s).unwrap();
    assert_ne!(e.flags & enemy_flags::LATCHED, 0);
    assert_eq!(e.owner, 0);
    assert_eq!(e.state, AiState::Latched as u16);
    // It clings to the victim's chest, in front of them.
    let feet = a.world.players[&p].movement.position;
    let at = a.world.enemies[&s].center();
    assert!(
        at.distance(feet + Vec3::Y * 1.1) < 1.0,
        "on the chest: {at}"
    );
    // One second: 16 damage, all of it hard damage.
    let before = a.world.players[&p].combat;
    let ev = a.ticks(60);
    let dealt: f32 = damage_by(&ev, p, snuffer, latch).iter().sum();
    let after = a.world.players[&p].combat;
    println!(
        "latched 1 s: {dealt} damage, hard {} -> {}",
        before.hard, after.hard
    );
    assert!((15.0..=17.0).contains(&dealt), "16 per second: {dealt}");
    assert!(
        (after.hard - before.hard - dealt).abs() < 0.01,
        "smothered: all hard damage"
    );
    // It goes where the victim goes.
    a.held.get_mut(&p).unwrap().forward = 127;
    a.ticks(30);
    let feet = a.world.players[&p].movement.position;
    let at = a.world.enemies[&s].center();
    assert!(feet.z < -2.0, "the victim walked on");
    assert!(
        at.distance(feet + Vec3::Y * 1.1) < 1.0,
        "still on the chest"
    );
}

#[test]
fn a_latched_victim_is_slowed_not_rooted() {
    let free = {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        a.held.get_mut(&p).unwrap().forward = 127;
        a.ticks(30);
        let z0 = a.world.players[&p].movement.position.z;
        a.ticks(60);
        z0 - a.world.players[&p].movement.position.z
    };
    let (mut a, p, s) = latched_duel();
    god(&mut a, p);
    a.held.get_mut(&p).unwrap().forward = 127;
    a.ticks(30);
    let z0 = a.world.players[&p].movement.position.z;
    a.ticks(60);
    let slowed = z0 - a.world.players[&p].movement.position.z;
    let v = a.world.players[&p].movement.velocity;
    println!("1 s running: free {free:.1} m, latched {slowed:.1} m (velocity {v})");
    assert_eq!(state(&a, s), AiState::Latched);
    assert!(
        slowed > free * 0.55 && slowed < free * 0.8,
        "a modest slow: {slowed} of {free}"
    );
}

#[test]
fn punching_a_snuffer_throws_it_off() {
    let (mut a, p, s) = latched_duel();
    a.ticks(10);
    let hp = a
        .enemy_alive(s)
        .then(|| a.world.enemies[&s].health)
        .unwrap();
    let ev = a.tap(p, Buttons::PUNCH, 1);
    let e = &a.world.enemies[&s];
    println!(
        "punched: state {:?}, health {hp} -> {}, velocity {}",
        e.state, e.health, e.velocity
    );
    assert_eq!(e.state, AiState::Stunned, "thrown off and stunned");
    assert_eq!(e.latched, None);
    assert!(e.velocity.z < -5.0, "flung along the punch");
    assert_eq!(flags(&a, s) & enemy_flags::LATCHED, 0);
    let counter: f32 = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d) if d.target == HitTarget::Entity(s) => Some(d.dealt),
            _ => None,
        })
        .sum();
    assert!(counter >= 30.0, "the counter (and the punch): {counter}");
    // No more smothering.
    let latch = attack_index(&a, "snuffer", "latch");
    let ev = a.ticks(60);
    assert!(damage_by(&ev, p, kind::SNUFFER, latch).is_empty());
}

#[test]
fn dashing_shakes_a_snuffer_off() {
    let (mut a, p, s) = latched_duel();
    a.held.get_mut(&p).unwrap().right = 127;
    a.tap(p, Buttons::DASH, 2);
    let e = &a.world.enemies[&s];
    assert_eq!(e.state, AiState::Stunned, "shaken off");
    assert_eq!(e.latched, None);
    a.ticks(30);
    let e = &a.world.enemies[&s];
    assert!(e.on_ground, "it fell to the floor");
}

#[test]
fn one_snuffer_per_player_the_others_nip() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let first = spawn(&mut a, "snuffer", Vec3::new(0.0, 0.0, -9.0));
    until(&mut a, 300, |a| state(a, first) == AiState::Latched).expect("latched");
    let second = spawn(&mut a, "snuffer", Vec3::new(4.0, 0.0, -6.0));
    let nip = attack_index(&a, "snuffer", "nip");
    let mut nipped = false;
    for _ in 0..300 {
        let ev = a.tick();
        assert_ne!(state(&a, second), AiState::Latched, "only one latch");
        nipped |= !damage_by(&ev, p, kind::SNUFFER, nip).is_empty();
        if state(&a, first) != AiState::Latched {
            break;
        }
    }
    assert!(nipped, "the second one nips");
}

#[test]
fn a_latch_lets_go_of_the_dead_and_after_its_time() {
    let (mut a, p, s) = latched_duel();
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
    a.ticks(2);
    assert_ne!(state(&a, s), AiState::Latched, "dead victims are dropped");
    let (mut a, p, s) = latched_duel();
    god(&mut a, p);
    a.ticks(6 * 60 + 2);
    assert_ne!(state(&a, s), AiState::Latched, "lets go after 6 s");
}

// ---- Smelter -----------------------------------------------------------------------------------

fn line_of(a: &Arena) -> Option<&shared::sim::enemy::SlagLine> {
    a.world.enemy_hazards.values().next()
}

#[test]
fn the_pour_telegraphs_a_line_then_burns_whoever_stands_on_it() {
    let mut a = Arena::coop();
    // One on the line's path, one well beside it, one jumping over it.
    let on = a.player(0, Vec3::ZERO, 0.0);
    let beside = a.player(1, Vec3::new(5.0, 0.0, 2.0), 0.0);
    let smelter = spawn(&mut a, "smelter", Vec3::new(0.0, 0.0, -14.0));
    let pour = attack_index(&a, "smelter", "pour");
    let ev = until(&mut a, 600, |a| line_of(a).is_some()).expect("a line is laid");
    let laid = a.world.tick;
    let windup = ev.iter().any(|e| {
        matches!(e, SimEvent::EnemyAttack { id, attack, phase: EnemyAttackPhase::Windup, .. }
            if *id == smelter && *attack == pour)
    });
    assert!(windup, "laid with the wind-up");
    let line = line_of(&a).unwrap().clone();
    println!(
        "line {} -> {} ({:.1} m, {} points), ignites at {} (laid {laid})",
        line.start(),
        line.end(),
        line.length(),
        line.points.len(),
        line.ignite_tick
    );
    assert_eq!(line.ignite_tick - laid, 66, "after the 1.1 s wind-up");
    assert!(line.touches(Vec3::ZERO, 0.4), "aimed at the target");
    assert!(!line.touches(Vec3::new(5.0, 0.0, 2.0), 0.4));
    assert!(line.end().z > 2.0, "runs past the target");
    // Replicated as a telegraph (not LIVE) first.
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    let h = frame.get(line.id).expect("replicated");
    assert_eq!(h.kind, kind::SLAG_LINE);
    assert_eq!(h.flags & slag_flags::LIVE, 0);
    assert_eq!(h.aux, line.ignite_tick);
    assert_eq!(h.state, 180, "1.8 m wide");
    assert!((h.position + h.velocity - line.end()).length() < 0.05);
    // Nothing burns during the wind-up.
    let mut ev = Vec::new();
    while a.world.tick + 1 < line.ignite_tick {
        ev.extend(a.tick());
    }
    assert!(
        damage_by(&ev, on, kind::SMELTER, pour).is_empty(),
        "telegraph only"
    );
    // Then it burns: 22 per second for whoever stands on it, nobody beside it.
    let ev = a.ticks(60);
    let burned: f32 = damage_by(&ev, on, kind::SMELTER, pour).iter().sum();
    println!("burned {burned} in the first second");
    assert!((20.0..=28.0).contains(&burned), "22 per second: {burned}");
    assert!(damage_by(&ev, beside, kind::SMELTER, pour).is_empty());
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    let h = frame.get(line.id).unwrap();
    assert_ne!(h.flags & slag_flags::LIVE, 0);
    // A player in the air over it is safe.
    a.world.players.get_mut(&on).unwrap().movement.position.y = 1.2;
    a.world.players.get_mut(&on).unwrap().movement.on_ground = false;
    let air = a.tick();
    assert!(damage_by(&air, on, kind::SMELTER, pour).is_empty());
    // It burns out after 5 s.
    until(&mut a, 400, |a| {
        !a.world.enemy_hazards.contains_key(&line.id)
    })
    .expect("burns out");
    assert!(a.world.tick >= line.ignite_tick + 299);
}

#[test]
fn slag_stops_at_walls_and_a_cancelled_pour_leaves_nothing() {
    // A wall 2 m behind the target: the line stops there (it would run on 4 m past it).
    let level = flat_level(
        shared::level::LevelKind::Campaign,
        vec![brush([-6.0, 0.0, 2.0], [6.0, 3.0, 3.0])],
    );
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let smelter = spawn(&mut a, "smelter", Vec3::new(0.0, 0.0, -12.0));
    until(&mut a, 600, |a| line_of(a).is_some()).expect("laid");
    let end = line_of(&a).unwrap().end();
    println!("walled line ends at {end}");
    assert!(end.z < 2.0, "stopped at the wall: {end}");
    // Killed during the wind-up: the line goes with it.
    let t = a.tuning.clone();
    a.world.kill_enemy(&t, smelter);
    assert!(line_of(&a).is_none(), "no line from a dead smelter's pour");
    let ev = a.ticks(120);
    assert_eq!(damage_to(&ev, HitTarget::Player(p)), 0.0);
}

// ---- Effigy ------------------------------------------------------------------------------------

#[test]
fn the_effigy_stands_flinches_and_heals_but_never_fights_or_dies() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let dummy = spawn(&mut a, "effigy", Vec3::new(0.0, 0.0, -3.0));
    let ev = a.ticks(300);
    let e = &a.world.enemies[&dummy];
    assert_eq!(e.target, None, "never targets");
    assert!(!ev.iter().any(|e| matches!(e, SimEvent::EnemyAttack { .. })));
    assert_eq!(damage_to(&ev, HitTarget::Player(p)), 0.0);
    assert!((e.position - Vec3::new(0.0, 0.0, -3.0)).length() < 0.05);
    // A hit shows in full, flinches it, doesn't move it.
    let t = a.tuning.clone();
    let hit = DamageInfo::new(300.0, Owner::Player(p), Attack::Quill)
        .at(Vec3::new(0.0, 1.2, -3.0), Vec3::NEG_Z)
        .knockback(Vec3::new(0.0, 3.0, -30.0));
    let d = a.world.apply_damage(&t, HitTarget::Entity(dummy), hit);
    assert_eq!(d.dealt, 300.0);
    assert_eq!(a.world.enemies[&dummy].health, 700.0);
    assert_eq!(state(&a, dummy), AiState::Recover, "flinches");
    a.ticks(30);
    assert_eq!(state(&a, dummy), AiState::Idle);
    assert!((a.world.enemies[&dummy].position - Vec3::new(0.0, 0.0, -3.0)).length() < 0.05);
    // It can't be killed.
    let huge = DamageInfo::new(5000.0, Owner::Player(p), Attack::Quill).at(Vec3::Y, Vec3::Z);
    let d = a.world.apply_damage(&t, HitTarget::Entity(dummy), huge);
    assert!(!d.killed && a.enemy_alive(dummy));
    assert_eq!(a.world.enemies[&dummy].health, 1.0);
    // Back to full 3 s after the last hit (not before).
    a.ticks(170);
    assert!(a.world.enemies[&dummy].health < 2.0, "waits 3 s");
    a.ticks(200);
    assert_eq!(a.world.enemies[&dummy].health, 1000.0, "healed");
    // No elite dummies, no frenzied dummies, never in waves.
    assert!(!a.world.make_elite(dummy));
    assert!(!a.world.enemies[&dummy].frenzies());
    assert!(!a.world.enemy_defs.wave_names().any(|n| n == "effigy"));
}

// ---- elites ------------------------------------------------------------------------------------

/// Ticks between consecutive bites of a gnawer on a standing (god) player.
fn bite_interval(scale: EnemyScale, elite: bool) -> (u32, u32, f32) {
    let mut a = Arena::coop();
    a.world.enemy_scale = scale;
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let id = a
        .world
        .spawn_enemy_with(
            "gnawer",
            Vec3::new(0.0, 0.0, -2.0),
            0.0,
            SpawnOptions {
                elite,
                ..Default::default()
            },
        )
        .unwrap();
    let bite = attack_index(&a, "gnawer", "bite");
    let mut windups = Vec::new();
    let mut strikes = Vec::new();
    let mut dealt = 0.0;
    for _ in 0..600 {
        let ev = a.tick();
        for e in &ev {
            match e {
                SimEvent::EnemyAttack {
                    id: eid,
                    attack,
                    phase,
                    ..
                } if *eid == id && *attack == bite => match phase {
                    EnemyAttackPhase::Windup => windups.push(a.world.tick),
                    EnemyAttackPhase::Strike => strikes.push(a.world.tick),
                },
                SimEvent::Damage(d) if d.info.source == Owner::Enemy(id) => {
                    dealt = d.info.amount;
                }
                _ => {}
            }
        }
    }
    assert!(strikes.len() >= 4, "{strikes:?}");
    let interval = strikes[3] - strikes[2];
    let windup = strikes[3] - *windups.iter().rfind(|w| **w <= strikes[3]).unwrap();
    (interval, windup, dealt)
}

#[test]
fn elites_have_more_health_glow_and_attack_faster() {
    let mut a = Arena::coop();
    let id = a
        .world
        .spawn_enemy_with(
            "gaoler",
            Vec3::new(0.0, 0.0, -10.0),
            0.0,
            SpawnOptions {
                health_scale: 1.5,
                elite: true,
                aware: true,
            },
        )
        .unwrap();
    let e = &a.world.enemies[&id];
    assert!(e.elite && e.aware);
    assert_eq!(e.max_health, 520.0 * 1.5 * 2.5);
    assert_eq!(e.health, e.max_health);
    assert_ne!(flags(&a, id) & enemy_flags::ELITE, 0, "replicated");
    // `make_elite` turns a live one into an elite (once).
    let plain = spawn(&mut a, "gnawer", Vec3::new(5.0, 0.0, -10.0));
    assert!(a.world.make_elite(plain) && a.world.make_elite(plain));
    assert_eq!(a.world.enemies[&plain].max_health, 45.0 * 2.5);
    // Faster attacks, same wind-up.
    let (normal, w0, _) = bite_interval(EnemyScale::default(), false);
    let (elite, w1, _) = bite_interval(EnemyScale::default(), true);
    println!("bite every {normal} ticks, elite every {elite} (wind-ups {w0} / {w1})");
    assert_eq!(w0, w1, "wind-ups keep their length");
    assert!(elite < normal, "{elite} < {normal}");
    // Twice the heat when it dies.
    let t = a.tuning.clone();
    let before: Vec<NetId> = a.world.embers.keys().copied().collect();
    a.world.kill_enemy(&t, plain);
    let pool = a
        .world
        .embers
        .iter()
        .filter(|(k, _)| !before.contains(k))
        .map(|(_, b)| b.pool)
        .fold(0.0f32, f32::max);
    assert_eq!(pool, 50.0, "an elite gnawer's heat");
}

// ---- difficulty scaling ------------------------------------------------------------------------

#[test]
fn enemy_scale_sets_damage_aggression_and_projectile_speed() {
    let base = bite_interval(EnemyScale::default(), false);
    let hard = bite_interval(
        EnemyScale {
            damage: 2.0,
            aggression: 2.0,
            projectile_speed: 1.0,
        },
        false,
    );
    println!("bite: {base:?} -> {hard:?}");
    assert_eq!(base.2, 14.0);
    assert_eq!(hard.2, 28.0, "damage x2");
    assert_eq!(base.1, hard.1, "wind-ups keep their length");
    // Cooldown + recover (0.9 s) halved: 0.45 s less between bites... the rest is the wind-up,
    // the strike and the approach.
    assert!(
        base.0 - hard.0 >= 25,
        "aggression x2: {} -> {}",
        base.0,
        hard.0
    );

    // Projectile speed: the Heron's needle.
    let needle_speed = |scale: f32| {
        let mut a = Arena::coop();
        a.world.enemy_scale.projectile_speed = scale;
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        spawn(&mut a, "heron", Vec3::new(0.0, 0.0, -30.0));
        until(&mut a, 600, |a| {
            a.world
                .projectiles
                .values()
                .any(|p| p.kind == ProjectileKind::Needle)
        })
        .expect("fired");
        a.world
            .projectiles
            .values()
            .next()
            .unwrap()
            .velocity
            .length()
    };
    let (n1, n2) = (needle_speed(1.0), needle_speed(1.5));
    println!("needle {n1} -> {n2}");
    assert!((n1 - 110.0).abs() < 0.5 && (n2 - 165.0).abs() < 0.5);

    // Lobs fly faster (a flatter arc) and still land on the target.
    let lob = |scale: f32| {
        let mut a = Arena::coop();
        a.world.enemy_scale.projectile_speed = scale;
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        spawn(&mut a, "censer", Vec3::new(0.0, 0.0, -20.0));
        until(&mut a, 600, |a| {
            a.world
                .projectiles
                .values()
                .any(|p| p.kind == ProjectileKind::EmberBomb)
        })
        .expect("lobbed");
        let speed = a
            .world
            .projectiles
            .values()
            .next()
            .unwrap()
            .velocity
            .length();
        let ev = a.ticks(150);
        let landed = ev
            .iter()
            .find_map(|e| match e {
                SimEvent::Explosion { position, .. } => Some(*position),
                _ => None,
            })
            .expect("burst");
        (speed, landed)
    };
    let ((s1, l1), (s2, l2)) = (lob(1.0), lob(1.4));
    println!("lob {s1:.1} m/s lands {l1}; x1.4: {s2:.1} m/s lands {l2}");
    assert!(s2 > s1 * 1.3, "faster lob");
    assert!(
        l1.length() < 2.0 && l2.length() < 2.0,
        "both land on the target"
    );
}

#[test]
fn frenzy_stacks_on_the_difficulty_scale() {
    let mut a = Arena::coop();
    a.world.enemy_scale.damage = 2.0;
    let p = a.player(0, Vec3::ZERO, 0.0);
    let id = spawn(&mut a, "gnawer", Vec3::new(0.0, 0.0, -2.0));
    let tick = a.world.tick;
    {
        let e = a.world.enemies.get_mut(&id).unwrap();
        e.frenzy_until = tick + 600;
    }
    let fury = a.world.fury(&a.world.enemies[&id]);
    assert_eq!(fury.damage, 2.5, "2 x 1.25");
    assert!((fury.aggression - 1.4).abs() < 1e-6);
    assert!((fury.speed - 1.3).abs() < 1e-6);
    let bite = attack_index(&a, "gnawer", "bite");
    let ev = until(&mut a, 300, |a| {
        a.events
            .iter()
            .any(|e| matches!(e, SimEvent::Damage(d) if d.target == HitTarget::Player(p)))
    })
    .expect("bitten");
    let dealt = damage_by(&ev, p, kind::GNAWER, bite);
    assert_eq!(dealt, vec![35.0], "14 x 2 x 1.25");
}
