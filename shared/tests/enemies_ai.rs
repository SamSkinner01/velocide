//! Enemy AI in small test arenas (docs/enemies.md, docs/roster.md): every attack of every type
//! is telegraphed (no damage before its wind-up completed), the Gnawer pounces on fleeing
//! players, Censers hold their band and lob ember bombs onto the target, Harriers hover, dive
//! along a straight line and can be swatted, the Gaoler's bash is parryable only in its window
//! (stun + 150), its sweep can be jumped and its rush stops at walls, its armour; Herons hold
//! their band, their aim laser locks for the last 0.25 s (dodgeable) and their needles can be
//! parried; light vs heavy knockback, nobody walks into lava, separation, untargetable players,
//! determinism, the frozen world.

mod combat_support;

use std::collections::BTreeMap;
use std::f32::consts::PI;

use combat_support::*;
use engine::physics::MASK_MOVEMENT;
use glam::Vec3;
use shared::input::{Buttons, look_dir};
use shared::level::{LevelDef, LevelKind, Volume};
use shared::protocol::{EntityState, NetFrame, NetId, enemy_flags};
use shared::sim::{
    AiState, Attack, DamageFlags, DamageInfo, EnemyAttackPhase, HitPart, HitTarget, Owner,
    ParryTarget, PlayerId, ProjectileKind, SimEvent,
};

/// Per-tick log of one enemy: state, and the attack events it produced.
#[derive(Default)]
struct Log {
    states: Vec<(u32, AiState)>,
    /// (tick, attack index).
    windups: Vec<(u32, u8)>,
    strikes: Vec<(u32, u8)>,
    fired: Vec<u32>,
    /// (tick, attack index) of damage events on players from this enemy (blocked ones too).
    hits: Vec<(u32, u8)>,
    alerted: bool,
}

impl Log {
    fn strikes_of(&self, attack: u8) -> usize {
        self.strikes.iter().filter(|s| s.1 == attack).count()
    }
}

/// Runs `ticks` ticks, logging everything enemy `id` does.
fn run_logged(a: &mut Arena, id: NetId, ticks: u32, log: &mut Log) {
    for _ in 0..ticks {
        let ev = a.tick();
        log_events(a, id, &ev, log);
    }
}

fn log_events(a: &Arena, id: NetId, ev: &[SimEvent], log: &mut Log) {
    let now = a.world.tick;
    if let Some(e) = a.world.enemies.get(&id) {
        log.states.push((now, e.state));
    }
    for e in ev {
        match e {
            SimEvent::EnemyAttack {
                id: eid,
                phase,
                attack,
                ..
            } if *eid == id => match phase {
                EnemyAttackPhase::Windup => log.windups.push((now, *attack)),
                EnemyAttackPhase::Strike => log.strikes.push((now, *attack)),
            },
            SimEvent::EnemyFired { id: eid, .. } if *eid == id => log.fired.push(now),
            SimEvent::EnemyAlerted { id: eid, .. } if *eid == id => log.alerted = true,
            SimEvent::Damage(d)
                if d.info.source == Owner::Enemy(id)
                    && matches!(d.target, HitTarget::Player(_)) =>
            {
                let attack = match d.info.attack {
                    Attack::Enemy { attack, .. } => attack,
                    _ => u8::MAX,
                };
                log.hits.push((now, attack))
            }
            _ => {}
        }
    }
}

fn god(a: &mut Arena, p: PlayerId) {
    a.world.players.get_mut(&p).unwrap().combat.god = true;
}

fn spawn(a: &mut Arena, name: &str, pos: Vec3, yaw: f32) -> NetId {
    a.world.spawn_enemy(name, pos, yaw).expect("enemy exists")
}

/// Index of attack `name` of definition `def`.
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

fn windup_ticks(a: &Arena, def: &str, attack: u8) -> u32 {
    let w = a.world.enemy_defs.get(def).unwrap().attacks[attack as usize].windup;
    (w * 60.0).round() as u32
}

/// The replicated entity of `id` this tick.
fn replicated(a: &Arena, id: NetId) -> EntityState {
    let mut frame = NetFrame::new();
    a.world.replicate(&a.tuning, &mut frame);
    frame.get(id).unwrap()
}

/// The telegraph contract on a log: every strike comes exactly one wind-up of the same attack
/// after its wind-up event (in the wind-up state right before), every projectile leaves during
/// a strike, and no damage of an attack lands before that attack's first strike.
fn assert_telegraphs(a: &Arena, log: &Log, def: &str) {
    assert!(!log.strikes.is_empty(), "{def}: never attacked");
    for &(s, attack) in &log.strikes {
        let len = windup_ticks(a, def, attack);
        let ok = log
            .windups
            .iter()
            .any(|&(w, i)| i == attack && w + len == s);
        assert!(
            ok,
            "{def}: strike {attack} at tick {s} without a full {len}-tick wind-up ({:?})",
            log.windups
        );
        let before = log.states.iter().find(|(t, _)| *t == s - 1).map(|x| x.1);
        assert_eq!(
            before,
            Some(AiState::Windup),
            "{def}: state before the strike at {s}"
        );
    }
    for &f in &log.fired {
        let state = log.states.iter().find(|(t, _)| *t == f).map(|x| x.1);
        assert_eq!(
            state,
            Some(AiState::Strike),
            "{def}: fired outside a strike at {f}"
        );
    }
    for &(h, attack) in &log.hits {
        let first = log.strikes.iter().find(|s| s.1 == attack).map(|s| s.0);
        assert!(
            first.is_some_and(|s| h >= s),
            "{def}: attack {attack} hit at {h} before its first strike ({first:?})"
        );
    }
}

/// One attack scenario: `def` at `enemy` (feet), the player at the origin facing -Z.
struct Scenario {
    def: &'static str,
    attack: &'static str,
    enemy: Vec3,
    secs: f32,
    /// Player movement axes held throughout (forward, right) and the yaw they run along.
    run: Option<(i8, i8, f32)>,
}

#[test]
fn every_attack_of_every_type_is_telegraphed_before_it_hurts() {
    let scenarios = [
        Scenario {
            def: "gnawer",
            attack: "bite",
            enemy: Vec3::new(0.0, 0.0, -12.0),
            secs: 5.0,
            run: None,
        },
        // A player running across in front of it: pounced on.
        Scenario {
            def: "gnawer",
            attack: "pounce",
            enemy: Vec3::new(0.0, 0.0, -8.0),
            secs: 4.0,
            run: Some((0, 40, 0.0)),
        },
        Scenario {
            def: "censer",
            attack: "swing",
            enemy: Vec3::new(0.0, 0.0, -2.6),
            secs: 3.0,
            run: None,
        },
        Scenario {
            def: "censer",
            attack: "volley",
            enemy: Vec3::new(0.0, 0.0, -20.0),
            secs: 6.0,
            run: None,
        },
        Scenario {
            def: "harrier",
            attack: "dive",
            enemy: Vec3::new(0.0, 7.0, -12.0),
            secs: 8.0,
            run: None,
        },
        Scenario {
            def: "gaoler",
            attack: "bash",
            enemy: Vec3::new(0.0, 0.0, -3.4),
            secs: 5.0,
            run: None,
        },
        Scenario {
            def: "gaoler",
            attack: "sweep",
            enemy: Vec3::new(0.0, 0.0, -5.0),
            secs: 5.0,
            run: None,
        },
        Scenario {
            def: "gaoler",
            attack: "rush",
            enemy: Vec3::new(0.0, 0.0, -15.0),
            secs: 5.0,
            run: None,
        },
        Scenario {
            def: "heron",
            attack: "kick",
            enemy: Vec3::new(0.0, 0.0, -2.8),
            secs: 3.0,
            run: None,
        },
        Scenario {
            def: "heron",
            attack: "needle",
            enemy: Vec3::new(0.0, 0.0, -32.0),
            secs: 6.0,
            run: None,
        },
        // The third roster (the toll does no damage: `enemies_expansion.rs`).
        Scenario {
            def: "bellwether",
            attack: "clapper",
            enemy: Vec3::new(0.0, 0.0, -2.8),
            secs: 3.0,
            run: None,
        },
        Scenario {
            def: "snuffer",
            attack: "latch",
            enemy: Vec3::new(0.0, 0.0, -9.0),
            secs: 3.0,
            run: None,
        },
        Scenario {
            def: "smelter",
            attack: "swing",
            enemy: Vec3::new(0.0, 0.0, -3.2),
            secs: 4.0,
            run: None,
        },
        Scenario {
            def: "smelter",
            attack: "pour",
            enemy: Vec3::new(0.0, 0.0, -14.0),
            secs: 4.0,
            run: None,
        },
    ];
    for s in scenarios {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        if let Some((forward, right, yaw)) = s.run {
            let input = a.held.get_mut(&p).unwrap();
            input.forward = forward;
            input.right = right;
            input.yaw = yaw;
        }
        let id = spawn(&mut a, s.def, s.enemy, 0.0);
        let attack = attack_index(&a, s.def, s.attack);
        let mut log = Log::default();
        run_logged(&mut a, id, (s.secs * 60.0) as u32, &mut log);
        let what = format!("{} {}", s.def, s.attack);
        assert!(log.alerted, "{what}: never noticed the player");
        assert_eq!(a.world.enemies[&id].target, Some(p), "{what}: target");
        assert!(
            log.strikes_of(attack) > 0,
            "{what}: never used ({:?})",
            log.strikes
        );
        assert!(
            log.hits.iter().any(|h| h.1 == attack),
            "{what}: never landed ({:?})",
            log.hits
        );
        assert_telegraphs(&a, &log, s.def);
        let alert = log
            .states
            .iter()
            .find(|s| s.1 == AiState::Alert)
            .map(|s| s.0);
        assert!(
            alert.is_some_and(|t| t < log.windups[0].0),
            "{what}: alert before the first wind-up"
        );
        println!(
            "{what}: {} strikes ({} of it), {} hits, {} projectiles in {} s",
            log.strikes.len(),
            log.strikes_of(attack),
            log.hits.len(),
            log.fired.len(),
            s.secs
        );
        if s.attack == "volley" {
            // Volleys of three (the last one may still be under way when the run ends).
            let full = 3 * log.strikes_of(attack);
            assert!(
                log.fired.len() + 2 >= full && log.fired.len() <= full,
                "volleys of three"
            );
        }
    }
}

#[test]
fn gnawer_pounces_on_an_arc() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    a.held.get_mut(&p).unwrap().right = 40;
    let id = spawn(&mut a, "gnawer", Vec3::new(0.0, 0.0, -8.0), 0.0);
    let pounce = attack_index(&a, "gnawer", "pounce");
    let mut peak = 0.0f32;
    let mut launched = None;
    let mut log = Log::default();
    for _ in 0..240 {
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
        let e = &a.world.enemies[&id];
        if e.state == AiState::Strike && e.attack == pounce as usize {
            launched.get_or_insert(e.velocity);
            peak = peak.max(e.position.y);
        }
        if log.hits.iter().any(|h| h.1 == pounce) {
            break;
        }
    }
    let v = launched.expect("pounced");
    let flat = Vec3::new(v.x, 0.0, v.z).length();
    println!("pounce launch {v} (flat {flat:.1} m/s), apex {peak:.2} m");
    assert!(
        flat > 11.0,
        "13 m/s forward (less a tick of braking): {flat}"
    );
    assert!(peak > 0.25, "it leaves the ground: apex {peak}");
    assert!(
        log.hits.iter().any(|h| h.1 == pounce),
        "the pounce raked the player"
    );
    let hits = log.hits.iter().filter(|h| h.1 == pounce).count();
    assert_eq!(hits, 1, "once per player");
}

/// Ticks until `pred` holds for enemy `id` (logging), at most `max` ticks.
fn until(a: &mut Arena, id: NetId, max: u32, log: &mut Log, pred: impl Fn(&Arena) -> bool) -> bool {
    for _ in 0..max {
        let ev = a.tick();
        log_events(a, id, &ev, log);
        if pred(a) {
            return true;
        }
    }
    false
}

#[test]
fn censer_keeps_its_distance_and_side_steps() {
    for start in [6.0f32, 45.0] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        let id = spawn(&mut a, "censer", Vec3::new(0.0, 0.0, -start), 0.0);
        a.world.enemies.get_mut(&id).unwrap().aware = true;
        let mut in_band = 0;
        let mut total = 0;
        let mut lateral = 0.0f32;
        let mut log = Log::default();
        let mut last = a.world.enemies[&id].position;
        for i in 0..(16 * 60) {
            let ev = a.tick();
            log_events(&a, id, &ev, &mut log);
            let e = &a.world.enemies[&id];
            let d = (e.eye() - Vec3::new(0.0, 1.2, 0.0)).length();
            if i > 7 * 60 {
                total += 1;
                if (13.0..=31.0).contains(&d) {
                    in_band += 1;
                }
                let to = (-e.position).normalize_or_zero();
                let step = e.position - last;
                lateral += (step - to * step.dot(to)).length();
            }
            last = e.position;
        }
        let frac = in_band as f32 / total as f32;
        let volleys = log.strikes_of(attack_index(&a, "censer", "volley"));
        println!(
            "censer from {start} m: {:.0} % of the time in 14-30 m, side-stepped {lateral:.1} m, {volleys} volleys",
            frac * 100.0
        );
        assert!(frac > 0.9, "censer from {start} m: in band {frac}");
        assert!(lateral > 4.0, "censer side-steps ({lateral} m)");
        assert!(volleys >= 2, "lobs ({volleys})");
    }
}

/// (tick, position) of the ember bomb explosions of enemy `id`'s volley.
fn ember_bursts(ev: &[SimEvent], kind: u16) -> Vec<Vec3> {
    ev.iter()
        .filter_map(|e| match e {
            SimEvent::Explosion {
                position,
                attack: Attack::Enemy { kind: k, .. },
                ..
            } if *k == kind => Some(*position),
            _ => None,
        })
        .collect()
}

#[test]
fn censer_lobs_land_on_the_target() {
    let kind = shared::protocol::kind::CENSER;
    // Standing still at three ranges: the first (straight) bomb bursts on the player, the
    // other two land 8 degrees to either side.
    for dist in [15.0f32, 22.0, 29.0] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        spawn(&mut a, "censer", Vec3::new(0.0, 0.0, -dist), 0.0);
        let mut bursts = Vec::new();
        for _ in 0..(6 * 60) {
            bursts.extend(ember_bursts(&a.tick(), kind));
            if bursts.len() >= 3 {
                break;
            }
        }
        assert_eq!(bursts.len(), 3, "{dist} m: a volley of three");
        let miss = Vec3::new(bursts[0].x, 0.0, bursts[0].z).length();
        let side: Vec<f32> = bursts[1..].iter().map(|b| b.x).collect();
        println!("censer at {dist} m: straight bomb {miss:.2} m off, side bombs at x {side:?}");
        assert!(
            miss < 1.2,
            "{dist} m: the straight bomb lands on the player ({miss} m)"
        );
        assert!(side[0] * side[1] < 0.0, "fanned left and right");
        for x in side {
            let expect = (dist - 0.35) * 8f32.to_radians().sin();
            assert!(
                (x.abs() - expect).abs() < 1.2,
                "{dist} m: side bomb {x} (expected ~{expect})"
            );
        }
    }
    // A player walking sideways: the lead puts the straight bomb within its blast radius.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::new(-6.0, 0.0, 0.0), 0.0);
    god(&mut a, p);
    a.held.get_mut(&p).unwrap().right = 40;
    spawn(&mut a, "censer", Vec3::new(0.0, 0.0, -22.0), 0.0);
    let mut first = None;
    for _ in 0..(6 * 60) {
        if let Some(b) = ember_bursts(&a.tick(), kind).first() {
            first = Some((*b, a.world.players[&p].movement.position));
            break;
        }
    }
    let (burst, player) = first.expect("lobbed at the walking player");
    let miss = Vec3::new(burst.x - player.x, 0.0, burst.z - player.z).length();
    println!("walking target: burst {miss:.2} m from the player");
    assert!(miss < 2.5, "inside the 2.5 m blast: {miss}");
}

#[test]
fn ember_bombs_explode_with_falloff_and_can_be_punched_back() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let censer = spawn(&mut a, "censer", Vec3::new(0.0, 0.0, -18.0), 0.0);
    // Wait for a bomb in flight, then punch it when it is close.
    let mut parried = None;
    for _ in 0..(6 * 60) {
        let near = a.world.projectiles.values().find(|pr| {
            pr.kind == ProjectileKind::EmberBomb
                && !pr.parried
                && (pr.position.distance(a.eye(p)) < 2.6 || pr.pending.is_some())
        });
        if let Some(bomb) = near {
            let (id, pos) = (bomb.id, bomb.position);
            a.aim(p, pos);
            let ev = a.tap(p, Buttons::PUNCH, 0);
            parried = ev.iter().find_map(|e| match e {
                SimEvent::Parried(pe)
                    if pe.target
                        == ParryTarget::Projectile {
                            id,
                            kind: ProjectileKind::EmberBomb,
                        } =>
                {
                    Some(id)
                }
                _ => None,
            });
            break;
        }
        a.tick();
    }
    let bomb = parried.expect("an ember bomb was punched");
    let b = &a.world.projectiles[&bomb];
    assert!(b.parried && b.owner == Owner::Player(p));
    assert_eq!(b.gravity, 0.0, "a parried bomb flies straight");
    // Explosion damage: 18 at the centre, doubled by the parry.
    assert_eq!(b.explosion.unwrap().damage, 36.0);
    assert_eq!(b.explosion.unwrap().radius, 2.5);
    let _ = censer;

    // Falloff: a bomb bursting 1.5 m from a player hurts less than a direct hit.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let t = a.tuning.clone();
    let owner = spawn(&mut a, "censer", Vec3::new(0.0, 0.0, -40.0), 0.0);
    a.world.enemies.get_mut(&owner).unwrap().state = AiState::Stunned;
    let attack = Attack::Enemy {
        kind: shared::protocol::kind::CENSER,
        attack: 1,
    };
    a.world
        .spawn_enemy_projectile(
            &t,
            ProjectileKind::EmberBomb,
            owner,
            attack,
            Vec3::new(1.5, 0.6, 0.0),
            Vec3::new(0.0, -5.0, 0.0),
            18.0,
        )
        .unwrap();
    let mut hurt = 0.0;
    for _ in 0..30 {
        hurt += damage_to(&a.tick(), HitTarget::Player(p));
    }
    println!("ember bomb 1.5 m away: {hurt:.1} damage");
    assert!(hurt > 5.0 && hurt < 18.0, "falloff: {hurt}");
}

/// The Harrier without its dive (pure flight behaviour), registered as "harrier_hover".
fn hover_only(a: &mut Arena) {
    let mut def = (*a.world.enemy_defs.get("harrier").unwrap()).clone();
    def.attacks.clear();
    a.world.enemy_defs.insert("harrier_hover", def);
}

#[test]
fn harrier_hovers_in_its_band_circles_and_never_clips_the_level() {
    // Pillars on its 8-16 m circling band, walls just outside it, a roof over part of it.
    let level = flat_level(
        LevelKind::Campaign,
        vec![
            brush([-13.0, 0.0, -2.0], [-11.0, 16.0, 0.0]),
            brush([10.0, 0.0, 3.0], [12.0, 16.0, 5.0]),
            brush([2.0, 0.0, 11.0], [4.0, 16.0, 13.0]),
            brush([-22.0, 0.0, 18.0], [22.0, 16.0, 19.0]),
            brush([19.0, 0.0, -22.0], [20.0, 16.0, 18.0]),
            brush([-16.0, 9.0, -16.0], [-6.0, 10.0, -6.0]),
        ],
    );
    let mut a = Arena::new(level);
    hover_only(&mut a);
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let flyers: Vec<NetId> = (0..3)
        .map(|i| {
            spawn(
                &mut a,
                "harrier_hover",
                Vec3::new(-12.0 + 8.0 * i as f32, 6.0, -16.0),
                0.0,
            )
        })
        .collect();
    let mut low = f32::INFINITY;
    let mut high = 0.0f32;
    let mut angle_travel = 0.0f32;
    let mut last_angle = BTreeMap::new();
    for i in 0..(25 * 60) {
        a.tick();
        for &g in &flyers {
            let e = &a.world.enemies[&g];
            assert!(!e.on_ground, "harrier landed");
            assert_inside_free_space(&a, g, i);
            if i > 4 * 60 {
                let ground = a
                    .world
                    .collision
                    .world
                    .cast_ray(e.center(), Vec3::NEG_Y, 50.0, MASK_MOVEMENT)
                    .map_or(50.0, |h| h.distance);
                low = low.min(ground);
                high = high.max(ground);
                let ang = e.position.z.atan2(e.position.x);
                if let Some(prev) = last_angle.insert(g, ang) {
                    let mut d: f32 = ang - prev;
                    if d > PI {
                        d -= 2.0 * PI;
                    }
                    if d < -PI {
                        d += 2.0 * PI;
                    }
                    angle_travel += d.abs();
                }
            }
        }
    }
    println!(
        "harriers hovered {low:.1}..{high:.1} m above the ground, circled {angle_travel:.1} rad"
    );
    assert!(low >= 3.5, "too low: {low}");
    assert!(high <= 11.0, "too high: {high}");
    assert!(angle_travel > 2.0 * PI, "harriers circle the target");
}

fn assert_inside_free_space(a: &Arena, id: NetId, tick: u32) {
    let e = &a.world.enemies[&id];
    let r = e.def.radius;
    let h = e.def.height.max(2.0 * r);
    let half = (h * 0.5 - r).max(0.0);
    let center = e.position + Vec3::Y * (h * 0.5);
    assert!(
        !a.world
            .collision
            .world
            .intersects_capsule(center, half, r - 0.05, MASK_MOVEMENT),
        "{} inside the level at {} (tick {tick})",
        e.def.name,
        e.position
    );
}

#[test]
fn harrier_dives_in_a_straight_line_hits_once_and_climbs_back() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "harrier", Vec3::new(0.0, 7.0, -12.0), 0.0);
    let mut log = Log::default();
    assert!(
        until(&mut a, id, 8 * 60, &mut log, |a| a.world.enemies[&id].state
            == AiState::Strike),
        "dived"
    );
    let start = a.world.enemies[&id].center();
    let aim = a.world.enemies[&id].aim;
    let mut strike_ticks = 1;
    let mut off_line = 0.0f32;
    while a.world.enemies[&id].state == AiState::Strike {
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
        assert_inside_free_space(&a, id, a.world.tick);
        if a.world.enemies[&id].state == AiState::Strike {
            strike_ticks += 1;
            let c = a.world.enemies[&id].center() - start;
            off_line = off_line.max((c - aim * c.dot(aim)).length());
        }
    }
    let end_height = a.world.enemies[&id].center().y;
    println!(
        "dive: aim {aim}, {strike_ticks} ticks, {off_line:.3} m off the line, ended {end_height:.2} m up"
    );
    assert!(aim.y < -0.2, "dives down at the player");
    assert!(off_line < 0.05, "a straight 3D line ({off_line} m)");
    assert!(
        strike_ticks < 36,
        "the floor behind the player ended the dive early ({strike_ticks} ticks)"
    );
    // It raked the player once (the hit is parry-delayed, so it lands a few ticks later).
    a.ticks(15);
    run_logged(&mut a, id, 1, &mut log);
    assert_eq!(log.hits.len(), 1, "one hit per player per dive");
    // Recover: it climbs back towards hover height.
    let low = a.world.enemies[&id].center().y;
    run_logged(&mut a, id, 50, &mut log);
    let back = a.world.enemies[&id].center().y;
    println!("climbed from {low:.2} to {back:.2} m");
    assert!(back > low + 1.5, "climbs back after the dive");
    assert_telegraphs(&a, &log, "harrier");
}

#[test]
fn a_harrier_dive_is_flagged_parryable_from_start_to_end() {
    // The client's parry cues (strobe + the accessibility "!" icon) read `PARRYABLE`; the
    // swat works for the whole dive, so the flag must cover all of it.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "harrier", Vec3::new(0.0, 7.0, -12.0), 0.0);
    a.world.enemies.get_mut(&id).unwrap().health = 500.0;
    let mut log = Log::default();
    assert!(!parryable_flag(&a, id), "not while hovering");
    assert!(
        until(&mut a, id, 8 * 60, &mut log, |a| a.world.enemies[&id].state
            == AiState::Strike),
        "dived"
    );
    let mut ticks = 0;
    while a.world.enemies[&id].state == AiState::Strike && ticks < 120 {
        assert!(parryable_flag(&a, id), "dive tick {ticks} without the flag");
        a.tick();
        ticks += 1;
    }
    assert!(ticks >= 5, "a real dive ({ticks} ticks)");
    a.ticks(2);
    assert!(!parryable_flag(&a, id), "cleared once it pulls out");
}

#[test]
fn swatting_a_diving_harrier_is_a_parry() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    // Some damage taken, to see the parry heal.
    {
        let c = &mut a.world.players.get_mut(&p).unwrap().combat;
        c.damage = 40.0;
        c.hard = 20.0;
    }
    let before = a.health(p);
    let id = spawn(&mut a, "harrier", Vec3::new(0.0, 7.0, -12.0), 0.0);
    a.world.enemies.get_mut(&id).unwrap().health = 500.0;
    let mut log = Log::default();
    // Punching while it hovers does nothing.
    a.ticks(30);
    let c = a.world.enemies[&id].center();
    a.aim(p, c);
    let ev = a.tap(p, Buttons::PUNCH, 1);
    assert!(
        !ev.iter().any(|e| matches!(e, SimEvent::Parried(_))),
        "not parryable while hovering"
    );
    assert!(
        until(&mut a, id, 8 * 60, &mut log, |a| a.world.enemies[&id].state
            == AiState::Strike),
        "dived"
    );
    assert!(
        replicated(&a, id).flags & enemy_flags::PARRYABLE != 0,
        "the dive shows the parry flash"
    );
    // Swat it when it is a few metres out (the parry window stays open for a few ticks: the
    // swat lands as soon as it is within reach).
    let mut swat = Vec::new();
    for _ in 0..40 {
        let c = a.world.enemies[&id].center();
        a.aim(p, c);
        if c.distance(a.eye(p)) < 5.0 {
            swat = a.tap(p, Buttons::PUNCH, 0);
            break;
        }
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
    }
    for _ in 0..12 {
        if swat.iter().any(|e| matches!(e, SimEvent::Parried(_))) {
            break;
        }
        let c = a.world.enemies[&id].center();
        a.aim(p, c);
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
        swat.extend(ev);
    }
    let parried = swat
        .iter()
        .find_map(|e| match e {
            SimEvent::Parried(pe) if pe.target == ParryTarget::Melee { enemy: id } => Some(*pe),
            _ => None,
        })
        .expect("the punch swatted the diving harrier");
    assert!(parried.healed > 0.0, "a swat heals like a parry");
    let counter: f32 = swat
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d)
                if d.target == HitTarget::Entity(id)
                    && d.info.flags.contains(DamageFlags::PARRIED) =>
            {
                Some(d.dealt)
            }
            _ => None,
        })
        .sum();
    assert_eq!(counter, 50.0, "2x punch damage");
    let e = &a.world.enemies[&id];
    assert_eq!(e.state, AiState::Stunned);
    assert!(e.velocity.y < 0.0, "knocked out of the sky");
    // It falls to the floor and never hurts the player.
    let mut landed = false;
    for _ in 0..100 {
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
        landed |= a.world.enemies[&id].on_ground;
    }
    assert!(landed, "fell to the floor while stunned");
    assert!(log.hits.is_empty(), "the dive never landed: {:?}", log.hits);
    assert!(
        a.health(p) > before,
        "healed: {} -> {}",
        before,
        a.health(p)
    );
    // Then it takes off again.
    a.ticks(120);
    assert!(a.world.enemies[&id].center().y > 3.0, "back in the air");
}

/// A Gaoler facing a player 1.5 m in front of it (surface to surface).
fn gaoler_duel() -> (Arena, PlayerId, NetId) {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, PI);
    let id = spawn(&mut a, "gaoler", Vec3::new(0.0, 0.0, -3.2), 0.0);
    a.world.enemies.get_mut(&id).unwrap().health = 5000.0;
    a.world.enemies.get_mut(&id).unwrap().max_health = 5000.0;
    // The player faces the Gaoler (yaw 0 looks down -Z).
    a.held.get_mut(&p).unwrap().yaw = 0.0;
    (a, p, id)
}

/// Ticks until the enemy starts winding up; returns the wind-up start tick.
fn until_windup(a: &mut Arena, id: NetId) -> u32 {
    for _ in 0..300 {
        a.tick();
        if a.world.enemies[&id].state == AiState::Windup {
            return a.world.tick;
        }
    }
    panic!("never wound up");
}

fn parryable_flag(a: &Arena, id: NetId) -> bool {
    replicated(a, id).flags & enemy_flags::PARRYABLE != 0
}

#[test]
fn gaoler_bash_is_parryable_only_in_its_window() {
    // In the window (last 0.35 s of the 1.0 s wind-up): stunned 2.5 s, 150 damage, no bash.
    let (mut a, p, id) = gaoler_duel();
    let w0 = until_windup(&mut a, id);
    assert_eq!(a.world.enemies[&id].attack, 0, "close: the bash");
    assert!(
        !parryable_flag(&a, id),
        "not parryable at the start of the wind-up"
    );
    a.ticks(45 - 1);
    assert_eq!(a.world.tick, w0 + 44);
    assert!(parryable_flag(&a, id), "parryable in the window");
    let ev = a.tap(p, Buttons::PUNCH, 2);
    let parried = ev.iter().any(
        |e| matches!(e, SimEvent::Parried(pe) if pe.target == ParryTarget::Melee { enemy: id }),
    );
    assert!(parried, "the punch parried the bash");
    let e = &a.world.enemies[&id];
    assert_eq!(e.state, AiState::Stunned);
    assert_eq!(e.state_len, 150, "2.5 s stun");
    // The counter: 150, through the front armour (the punch itself is armoured).
    let counter: f32 = ev
        .iter()
        .filter_map(|e| match e {
            SimEvent::Damage(d)
                if d.target == HitTarget::Entity(id)
                    && d.info.flags.contains(DamageFlags::PARRIED) =>
            {
                Some(d.dealt)
            }
            _ => None,
        })
        .sum();
    assert_eq!(counter, 150.0, "parry counter damage");
    let before = a.health(p);
    a.ticks(60);
    assert_eq!(a.health(p), before, "the bash never landed");
    a.ticks(100);
    assert_ne!(
        a.world.enemies[&id].state,
        AiState::Stunned,
        "stun wears off"
    );

    // Too early (first half of the wind-up): nothing, and the bash lands (40, knocked back).
    let (mut a, p, id) = gaoler_duel();
    until_windup(&mut a, id);
    a.ticks(5);
    let ev = a.tap(p, Buttons::PUNCH, 0);
    assert!(
        !ev.iter().any(|e| matches!(e, SimEvent::Parried(_))),
        "parried too early"
    );
    let mut hurt = 0.0;
    for _ in 0..90 {
        hurt += damage_to(&a.tick(), HitTarget::Player(p));
    }
    assert_eq!(hurt, 40.0, "the bash landed");
    assert_ne!(a.world.enemies[&id].state, AiState::Stunned);

    // Too late (after the strike's damage was applied): nothing.
    let (mut a, p, id) = gaoler_duel();
    let w0 = until_windup(&mut a, id);
    let mut hurt_at = None;
    for _ in 0..120 {
        let ev = a.tick();
        if damage_to(&ev, HitTarget::Player(p)) > 0.0 {
            hurt_at = Some(a.world.tick);
            break;
        }
    }
    let hurt_at = hurt_at.expect("bash hit");
    assert!(hurt_at >= w0 + 60, "damage only after the full 1 s wind-up");
    let v = a.world.players[&p].movement.velocity;
    assert!(
        v.y > 4.0 && Vec3::new(v.x, 0.0, v.z).length() > 10.0,
        "knocked back and up: {v}"
    );
    a.ticks(3);
    let ev = a.tap(p, Buttons::PUNCH, 0);
    assert!(
        !ev.iter().any(|e| matches!(e, SimEvent::Parried(_))),
        "parried after the hit"
    );
    assert_ne!(a.world.enemies[&id].state, AiState::Stunned);
}

#[test]
fn gaoler_front_armour_and_weak_points() {
    let mut a = Arena::coop();
    a.player(0, Vec3::new(0.0, 0.0, 40.0), 0.0);
    let id = spawn(&mut a, "gaoler", Vec3::ZERO, 0.0); // faces -Z
    a.world.enemies.get_mut(&id).unwrap().health = 10_000.0;
    let t = a.tuning.clone();
    let fwd = a.world.enemies[&id].forward();
    let hit = |a: &mut Arena, dir: Vec3, part: HitPart, flags: DamageFlags| {
        let info = DamageInfo::new(100.0, Owner::Player(PlayerId(0)), Attack::Dev)
            .at(Vec3::new(0.0, 1.5, 0.0), dir)
            .part(part)
            .flags(flags);
        a.world.apply_damage(&t, HitTarget::Entity(id), info)
    };
    // Shot travelling against the facing = from the front.
    let front = hit(&mut a, -fwd, HitPart::Body, DamageFlags::HITSCAN);
    let back = hit(&mut a, fwd, HitPart::Body, DamageFlags::HITSCAN);
    let side = hit(&mut a, Vec3::X, HitPart::Body, DamageFlags::HITSCAN);
    assert!((front.dealt - 15.0).abs() < 0.01, "front {}", front.dealt);
    assert!(front.info.flags.contains(DamageFlags::ARMOURED));
    assert_eq!(back.dealt, 100.0, "back");
    assert_eq!(
        side.dealt, 100.0,
        "90° from the front is outside the 120° arc"
    );
    let head = hit(&mut a, -fwd, HitPart::Weak(0), DamageFlags::HITSCAN);
    assert_eq!(
        head.dealt, 150.0,
        "the helm grille ignores the armour (x1.5)"
    );
    assert!(head.info.flags.contains(DamageFlags::HEADSHOT));
    let lantern = hit(&mut a, fwd, HitPart::Weak(1), DamageFlags::HITSCAN);
    assert_eq!(lantern.dealt, 250.0, "the back lantern x2.5");
    let blast = hit(&mut a, -fwd, HitPart::Body, DamageFlags::EXPLOSIVE);
    assert!(
        (blast.dealt - 15.0).abs() < 0.01,
        "explosions from the front are armoured"
    );
    // The weak points are where the contract puts them: the lantern behind, the helm on top.
    let e = &a.world.enemies[&id];
    let lantern_at = e.local_to_world(e.def.weak_points[1].offset);
    assert!(
        lantern_at.z > 0.9 && (lantern_at.y - 2.2).abs() < 1e-4,
        "{lantern_at}"
    );
}

#[test]
fn gaoler_sweep_only_hits_players_on_the_ground() {
    // (jump, expected damage)
    for jump in [false, true] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        let id = spawn(&mut a, "gaoler", Vec3::new(0.0, 0.0, -5.0), 0.0);
        let sweep = attack_index(&a, "gaoler", "sweep");
        let len = windup_ticks(&a, "gaoler", sweep);
        let mut log = Log::default();
        assert!(until(&mut a, id, 300, &mut log, |a| a.world.enemies[&id]
            .state
            == AiState::Windup));
        assert_eq!(
            a.world.enemies[&id].attack, sweep as usize,
            "3.3 m away: the sweep"
        );
        let w0 = a.world.tick;
        // Jump 0.3 s before the sweep comes round.
        run_logged(&mut a, id, len - 18 - 1, &mut log);
        if jump {
            a.set_buttons(p, Buttons::JUMP);
        }
        run_logged(&mut a, id, 1, &mut log);
        a.set_buttons(p, Buttons::NONE);
        let mut feet_at_strike = None;
        while a.world.tick < w0 + len + 30 {
            let ev = a.tick();
            log_events(&a, id, &ev, &mut log);
            if log.strikes.iter().any(|s| s.1 == sweep) && feet_at_strike.is_none() {
                feet_at_strike = Some(a.world.players[&p].movement.position.y);
            }
        }
        let feet = feet_at_strike.expect("swept");
        let hurt: f32 = log.hits.iter().filter(|h| h.1 == sweep).count() as f32;
        println!(
            "sweep, jump {jump}: feet {feet:.2} m up at the strike, {hurt} hits, health {}",
            a.health(p)
        );
        if jump {
            assert!(feet > 1.2, "the jump cleared the sweep height ({feet})");
            assert_eq!(hurt, 0.0, "jumped over the sweep");
            assert_eq!(a.health(p), 100.0);
        } else {
            assert_eq!(hurt, 1.0, "the sweep hit the grounded player");
            assert_eq!(a.health(p), 75.0, "25 damage");
        }
    }
}

#[test]
fn gaoler_rush_hits_in_the_open_and_stops_at_walls() {
    // Open field: the rush closes 14 m and hits.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "gaoler", Vec3::new(0.0, 0.0, -13.0), 0.0);
    let rush = attack_index(&a, "gaoler", "rush");
    let mut log = Log::default();
    run_logged(&mut a, id, 4 * 60, &mut log);
    assert!(log.strikes_of(rush) >= 1, "rushed");
    assert!(log.hits.iter().any(|h| h.1 == rush), "the rush hit");

    // A knee-high wall in between: it rushes, the wall stops it early, no hit.
    let level = flat_level(
        LevelKind::Campaign,
        vec![brush([-20.0, 0.0, -7.0], [20.0, 1.0, -6.0])],
    );
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "gaoler", Vec3::new(0.0, 0.0, -17.0), 0.0);
    let mut log = Log::default();
    assert!(
        until(&mut a, id, 4 * 60, &mut log, |a| a.world.enemies[&id].state
            == AiState::Strike),
        "rushed at the player behind the wall"
    );
    assert_eq!(a.world.enemies[&id].attack, rush as usize);
    let mut ticks = 1;
    while a.world.enemies[&id].state == AiState::Strike {
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
        ticks += 1;
    }
    let e = &a.world.enemies[&id];
    println!(
        "rush into the wall: {ticks} strike ticks, stopped at z {:.2}",
        e.position.z
    );
    assert!(
        ticks < 48,
        "the wall ended the 0.8 s rush early ({ticks} ticks)"
    );
    assert!(
        e.position.z < -7.0 && e.position.z > -10.0,
        "stopped at the wall: {}",
        e.position
    );
    assert!(log.hits.is_empty(), "no hit through the wall");
}

#[test]
fn heron_keeps_its_distance_and_repositions() {
    for start in [10.0f32, 60.0] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        let id = spawn(&mut a, "heron", Vec3::new(0.0, 0.0, -start), 0.0);
        a.world.enemies.get_mut(&id).unwrap().aware = true;
        let needle = attack_index(&a, "heron", "needle");
        let mut in_band = 0;
        let mut total = 0;
        let mut log = Log::default();
        let mut shots_from = Vec::new();
        for i in 0..(20 * 60) {
            let ev = a.tick();
            log_events(&a, id, &ev, &mut log);
            let e = &a.world.enemies[&id];
            if ev
                .iter()
                .any(|x| matches!(x, SimEvent::EnemyFired { id: f, .. } if *f == id))
            {
                shots_from.push(e.position);
            }
            let d = Vec3::new(e.position.x, 0.0, e.position.z).length();
            if i > 8 * 60 {
                total += 1;
                if (24.0..=46.0).contains(&d) {
                    in_band += 1;
                }
            }
        }
        let frac = in_band as f32 / total as f32;
        let moved: Vec<f32> = shots_from.windows(2).map(|w| w[0].distance(w[1])).collect();
        println!(
            "heron from {start} m: {:.0} % of the time in 25-45 m, {} needles, moved between shots {moved:.1?}",
            frac * 100.0,
            log.strikes_of(needle)
        );
        assert!(frac > 0.85, "heron from {start} m: in band {frac}");
        assert!(log.strikes_of(needle) >= 2, "fires");
        assert!(
            moved.iter().all(|m| *m > 2.0),
            "repositions between shots: {moved:?}"
        );
    }
}

#[test]
fn heron_laser_tracks_then_locks_and_the_needle_follows_it() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::new(-4.0, 0.0, 0.0), 0.0);
    god(&mut a, p);
    // Walking sideways: the laser has to track.
    a.held.get_mut(&p).unwrap().right = 30;
    let id = spawn(&mut a, "heron", Vec3::new(0.0, 0.0, -32.0), 0.0);
    let needle = attack_index(&a, "heron", "needle");
    let len = windup_ticks(&a, "heron", needle);
    let mut log = Log::default();
    assert!(until(&mut a, id, 6 * 60, &mut log, |a| a.world.enemies
        [&id]
        .state
        == AiState::Windup));
    assert_eq!(a.world.enemies[&id].attack, needle as usize);
    let w0 = a.world.tick;
    // (tick, aiming, locked, yaw, pitch) every tick of the wind-up.
    let mut frames = Vec::new();
    let mut needle_velocity = None;
    while a.world.tick < w0 + len + 2 {
        let e = replicated(&a, id);
        frames.push((
            a.world.tick,
            e.flags & enemy_flags::AIMING != 0,
            e.flags & enemy_flags::AIM_LOCKED != 0,
            e.yaw,
            e.pitch,
        ));
        let ev = a.tick();
        log_events(&a, id, &ev, &mut log);
        if needle_velocity.is_none()
            && let Some(pr) = a
                .world
                .projectiles
                .values()
                .find(|pr| pr.kind == ProjectileKind::Needle)
        {
            needle_velocity = Some(pr.velocity);
        }
    }
    let strike = w0 + len;
    assert!(
        log.strikes.iter().any(|s| s.0 == strike),
        "the needle at the end of the 1.3 s wind-up"
    );
    let during: Vec<_> = frames
        .iter()
        .filter(|f| f.0 >= w0 && f.0 < strike)
        .collect();
    assert!(
        during.iter().all(|f| f.1),
        "the laser is on for the whole wind-up"
    );
    assert!(
        !frames.iter().any(|f| f.0 >= strike && f.1),
        "and off once it fired"
    );
    let first_lock = during.iter().find(|f| f.2).map(|f| f.0).expect("locked");
    assert_eq!(strike - first_lock, 15, "locks for the last 0.25 s");
    let locked: Vec<_> = during.iter().filter(|f| f.2).collect();
    assert!(
        locked
            .iter()
            .all(|f| f.3 == locked[0].3 && f.4 == locked[0].4),
        "the aim stays put once locked"
    );
    let tracked = (during[2].3 - locked[0].3).abs();
    println!(
        "laser tracked {:.2} degrees before locking",
        tracked.to_degrees()
    );
    assert!(tracked > 0.01, "it tracked the walking player");
    // The needle flies exactly along the replicated (locked) laser.
    let v = needle_velocity.expect("a needle");
    let laser = look_dir(locked[0].3, locked[0].4);
    let err = v.normalize().angle_between(laser);
    println!(
        "needle {:.1} m/s, {:.4} degrees off the drawn laser",
        v.length(),
        err.to_degrees()
    );
    assert!((v.length() - 110.0).abs() < 0.01);
    assert!(err < 0.002, "needle off the laser by {err} rad");
}

#[test]
fn dodging_the_locked_laser_makes_the_needle_miss() {
    for dodge in [false, true] {
        let mut a = Arena::coop();
        let p = a.player(0, Vec3::ZERO, 0.0);
        god(&mut a, p);
        let id = spawn(&mut a, "heron", Vec3::new(0.0, 0.0, -32.0), 0.0);
        let mut log = Log::default();
        assert!(until(&mut a, id, 6 * 60, &mut log, |a| replicated(a, id)
            .flags
            & enemy_flags::AIM_LOCKED
            != 0));
        if dodge {
            // A sideways dash the moment it locks.
            a.world.players.get_mut(&p).unwrap().movement.position.x += 2.5;
        }
        run_logged(&mut a, id, 60, &mut log);
        println!("heron needle, dodged {dodge}: {} hits", log.hits.len());
        assert_eq!(log.hits.is_empty(), dodge, "dodge {dodge}");
    }
}

#[test]
fn a_parried_needle_kills_its_heron() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let id = spawn(&mut a, "heron", Vec3::new(0.0, 0.0, -30.0), 0.0);
    let mut log = Log::default();
    // Wait until the needle reached the player (its hit waits out the parry tolerance).
    assert!(until(&mut a, id, 6 * 60, &mut log, |a| {
        a.world
            .projectiles
            .values()
            .any(|pr| pr.kind == ProjectileKind::Needle && pr.pending.is_some())
    }));
    a.held.get_mut(&p).unwrap().yaw = 0.0;
    a.held.get_mut(&p).unwrap().pitch = 0.05;
    let ev = a.tap(p, Buttons::PUNCH, 30);
    let parried = ev.iter().any(|e| {
        matches!(e, SimEvent::Parried(pe) if matches!(pe.target, ParryTarget::Projectile { kind: ProjectileKind::Needle, .. }))
    });
    assert!(parried, "the needle was parried");
    assert_eq!(a.health(p), 100.0, "no needle damage");
    let k = kills(&ev);
    assert_eq!(k.len(), 1, "the returned needle killed the heron");
    assert_eq!(k[0].victim, HitTarget::Entity(id));
    assert!(k[0].parried_projectile, "RETURN TO SENDER");
}

#[test]
fn every_type_rests_on_the_floor_wherever_it_spawns() {
    // The contact query once reported a 4.7 m wrong-way penetration for a Gaoler spawned
    // exactly on the floor at z -3.4: it fell through the level.
    let mut a = Arena::coop();
    let mut bad = Vec::new();
    for name in [
        "gnawer",
        "censer",
        "gaoler",
        "heron",
        "bellwether",
        "snuffer",
        "smelter",
        "effigy",
    ] {
        for i in 0..40 {
            let at = if i == 0 {
                Vec3::new(0.0, 0.0, -3.4)
            } else {
                Vec3::new(
                    (i % 8) as f32 * 0.37 - 1.0,
                    0.0,
                    (i / 8) as f32 * -0.2 - 3.0,
                )
            };
            let id = spawn(&mut a, name, at, 0.0);
            a.ticks(2);
            let y = a.world.enemies[&id].position.y;
            if !(-0.05..0.1).contains(&y) {
                bad.push((name, at, y));
            }
            a.world.remove_enemy(id);
        }
    }
    assert!(bad.is_empty(), "fell through / floated: {bad:?}");
}

#[test]
fn light_enemies_fly_gaolers_barely_move() {
    let mut a = Arena::coop();
    a.player(0, Vec3::new(0.0, 0.0, 40.0), 0.0);
    let gnawer = spawn(&mut a, "gnawer", Vec3::new(-5.0, 0.0, -15.0), 0.0);
    let gaoler = spawn(&mut a, "gaoler", Vec3::new(5.0, 0.0, 0.0), 0.0);
    let t = a.tuning.clone();
    for id in [gnawer, gaoler] {
        let info = DamageInfo::new(1.0, Owner::Player(PlayerId(0)), Attack::Dev)
            .at(a.world.enemies[&id].center(), Vec3::X)
            .knockback(Vec3::new(20.0, 4.0, 0.0));
        a.world.apply_damage(&t, HitTarget::Entity(id), info);
    }
    let lv = a.world.enemies[&gnawer].velocity.length();
    let hv = a.world.enemies[&gaoler].velocity.length();
    assert!(lv > 20.0, "gnawer knocked back at {lv}");
    assert!(hv < 4.0, "gaoler barely moves: {hv}");
    a.ticks(40);
    let moved_l = a.world.enemies[&gnawer].position.x + 5.0;
    let moved_h = a.world.enemies[&gaoler].position.x - 5.0;
    assert!(
        moved_l > 3.0 && moved_h < 0.6,
        "gnawer {moved_l} m, gaoler {moved_h} m"
    );
}

/// Two floors separated by a lava trench (kill volume) with one 5 m bridge on the east side.
fn lava_level() -> LevelDef {
    let mut level = flat_level(LevelKind::Campaign, Vec::new());
    level.brushes = vec![
        // North and south floors.
        brush([-30.0, -1.0, -40.0], [30.0, 0.0, -6.0]),
        brush([-30.0, -1.0, 6.0], [30.0, 0.0, 40.0]),
        // Trench bottom (lava surface at -2).
        brush([-30.0, -3.0, -6.0], [30.0, -2.0, 6.0]),
        // Bridge.
        brush([10.0, -1.0, -6.0], [15.0, 0.0, 6.0]),
        // Walls all around so the bridge is the only way.
        brush([-31.0, -3.0, -41.0], [-30.0, 8.0, 41.0]),
        brush([30.0, -3.0, -41.0], [31.0, 8.0, 41.0]),
        brush([-31.0, -3.0, -41.0], [31.0, 8.0, -40.0]),
        brush([-31.0, -3.0, 40.0], [31.0, 8.0, 41.0]),
    ];
    level.kill_volumes = vec![
        Volume {
            min: Vec3::new(-30.0, -3.0, -6.0),
            max: Vec3::new(30.0, -1.0, 6.0),
        },
        Volume {
            min: Vec3::new(-500.0, -100.0, -500.0),
            max: Vec3::new(500.0, -20.0, 500.0),
        },
    ];
    level
}

#[test]
fn enemies_take_the_bridge_and_never_walk_into_lava() {
    let mut a = Arena::new(lava_level());
    let p = a.player(0, Vec3::new(-8.0, 0.0, 20.0), 0.0);
    god(&mut a, p);
    let mut ids = Vec::new();
    let roster = [
        "gnawer", "gnawer", "gnawer", "gnawer", "gaoler", "censer", "heron",
    ];
    for (i, name) in roster.iter().enumerate() {
        let id = spawn(
            &mut a,
            name,
            Vec3::new(-18.0 + 5.0 * i as f32, 0.0, -20.0),
            0.0,
        );
        a.world.enemies.get_mut(&id).unwrap().aware = true;
        ids.push(id);
    }
    let kill_volume = a.world.level.kill_volumes[0];
    let mut reached = BTreeMap::new();
    for tick in 0..(30 * 60) {
        let ev = a.tick();
        for e in &ev {
            if let SimEvent::Killed(k) = e {
                panic!("{:?} died ({:?})", k.victim, k.attack);
            }
        }
        for &id in &ids {
            let e = &a.world.enemies[&id];
            assert!(
                !kill_volume.contains(e.feet()),
                "{} in the lava at {}",
                e.def.name,
                e.position
            );
            if e.position.z > 6.0 {
                reached.entry(id).or_insert(tick);
            }
        }
    }
    println!(
        "crossed the bridge after (s): {:?}",
        ids.iter()
            .map(|id| (
                a.world.enemies[id].def.name.clone(),
                reached.get(id).map(|t| *t as f32 / 60.0)
            ))
            .collect::<Vec<_>>()
    );
    // The melee ones crossed (the censer and the heron may shoot from the far side).
    for &id in &ids[..5] {
        assert!(
            reached.contains_key(&id),
            "{} never crossed",
            a.world.enemies[&id].def.name
        );
    }
    // Knocked into the lava, an enemy dies.
    let gnawer = ids[0];
    a.world.enemies.get_mut(&gnawer).unwrap().position = Vec3::new(0.0, -1.5, 0.0);
    let ev = a.tick();
    assert!(
        ev.iter().any(|e| matches!(e, SimEvent::Killed(k) if k.victim == HitTarget::Entity(gnawer) && k.attack == Attack::World)),
        "lava kills enemies"
    );
}

#[test]
fn tall_enemies_route_around_low_ceilings() {
    // A wall with two openings: a 3 m high tunnel straight ahead, an open gap further east.
    let level = flat_level(
        LevelKind::Campaign,
        vec![
            brush([-30.0, 0.0, -11.0], [-1.5, 8.0, -10.0]),
            brush([1.5, 0.0, -11.0], [14.0, 8.0, -10.0]),
            brush([18.0, 0.0, -11.0], [30.0, 8.0, -10.0]),
            // The tunnel's roof (3 m clear).
            brush([-1.5, 3.0, -11.0], [1.5, 8.0, -10.0]),
        ],
    );
    let mut a = Arena::new(level);
    // A melee walker as tall as a Heron (r 0.5 x 3.4 m): it needs 3.5 m of headroom.
    let mut stilt = (*a.world.enemy_defs.get("gnawer").unwrap()).clone();
    (stilt.radius, stilt.height, stilt.body_top, stilt.eye_height) = (0.5, 3.4, None, 3.1);
    stilt.attacks.truncate(1);
    a.world.enemy_defs.insert("stilt_walker", stilt);
    let p = a.player(0, Vec3::new(0.0, 0.0, 2.0), 0.0);
    god(&mut a, p);
    let tall = spawn(&mut a, "stilt_walker", Vec3::new(0.0, 0.0, -30.0), 0.0);
    let gnawer = spawn(&mut a, "gnawer", Vec3::new(-3.0, 0.0, -24.0), 0.0);
    for id in [tall, gnawer] {
        a.world.enemies.get_mut(&id).unwrap().aware = true;
    }
    let mut through = BTreeMap::new();
    for _ in 0..(14 * 60) {
        a.tick();
        for id in [tall, gnawer] {
            let e = &a.world.enemies[&id];
            if e.position.z > -10.0 && e.position.z < -9.0 {
                through.entry(id).or_insert(e.position.x);
            }
        }
    }
    println!("came through the wall at x: {through:?}");
    let gx = through
        .get(&gnawer)
        .copied()
        .expect("the gnawer came through");
    assert!(gx.abs() < 1.5, "the gnawer takes the tunnel: {gx}");
    let hx = through
        .get(&tall)
        .copied()
        .expect("the tall walker came through");
    assert!(hx > 13.0, "a 3.4 m walker goes round by the open gap: {hx}");
}

#[test]
fn melee_enemies_route_around_walls() {
    // A long wall with a gap at the west end; the gnawer can't see the player.
    let level = flat_level(
        LevelKind::Campaign,
        vec![brush([-14.0, 0.0, -11.0], [30.0, 4.0, -10.0])],
    );
    let mut a = Arena::new(level);
    let p = a.player(0, Vec3::new(10.0, 0.0, 0.0), 0.0);
    god(&mut a, p);
    let id = spawn(&mut a, "gnawer", Vec3::new(10.0, 0.0, -20.0), 0.0);
    a.world.enemies.get_mut(&id).unwrap().aware = true;
    let mut hit = None;
    for i in 0..(10 * 60) {
        a.tick();
        if a.events
            .iter()
            .any(|e| matches!(e, SimEvent::Damage(d) if d.target == HitTarget::Player(p)))
        {
            hit = Some(i);
            break;
        }
    }
    let hit = hit.expect("the gnawer found the way around");
    println!(
        "gnawer around the wall: {:.1} s, {:?}",
        hit as f32 / 60.0,
        a.world.ai.stats
    );
    // Walking ~24 m around + 20 m back at 10 m/s: well under 8 s.
    assert!(hit < 8 * 60);
    assert!(a.world.ai.stats.paths >= 1, "planned with A*");
}

#[test]
fn separation_keeps_enemies_apart() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    god(&mut a, p);
    let mut ids = Vec::new();
    for _ in 0..10 {
        ids.push(spawn(&mut a, "gnawer", Vec3::new(0.0, 0.0, -20.0), 0.0));
    }
    // Stacked at first; apart within a second.
    a.ticks(60);
    let check = |a: &Arena, tol: f32| {
        let mut worst = f32::INFINITY;
        for (i, x) in ids.iter().enumerate() {
            for y in &ids[i + 1..] {
                let (ex, ey) = (&a.world.enemies[x], &a.world.enemies[y]);
                let d = Vec3::new(
                    ex.position.x - ey.position.x,
                    0.0,
                    ex.position.z - ey.position.z,
                )
                .length();
                worst = worst.min(d / (ex.radius() + ey.radius()));
            }
        }
        assert!(worst >= tol, "enemies overlap: {worst}");
        worst
    };
    check(&a, 0.85);
    // Crowding the player (and pouncing): still apart.
    let mut worst: f32 = 1.0;
    for _ in 0..(6 * 60) {
        a.tick();
        worst = worst.min(check(&a, 0.75));
    }
    println!("closest pair while swarming: {:.2} x (r1 + r2)", worst);
}

#[test]
fn dead_and_untargetable_players_are_dropped() {
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let q = a.player(1, Vec3::new(0.0, 0.0, 30.0), 0.0);
    let id = spawn(&mut a, "gnawer", Vec3::new(0.0, 0.0, -10.0), 0.0);
    a.ticks(30);
    assert_eq!(a.world.enemies[&id].target, Some(p));
    // p dies: the gnawer switches to q right away and never hits a corpse.
    let t = a.tuning.clone();
    a.world.kill_player(&t, p, Attack::Dev);
    assert!(!shared::sim::enemy::is_targetable(&a.world.players[&p]));
    a.tick();
    assert_eq!(a.world.enemies[&id].target, Some(q));
    // q too: nobody left.
    a.world.kill_player(&t, q, Attack::Dev);
    a.tick();
    assert_eq!(a.world.enemies[&id].target, None);
    a.ticks(5);
    assert_eq!(a.world.enemies[&id].state, AiState::Idle);
}

#[test]
fn hurting_an_unaware_enemy_alerts_it() {
    // Shot from beyond its sight range (sight is 360°).
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let id = spawn(&mut a, "gnawer", Vec3::new(0.0, 0.0, -80.0), 0.0);
    a.ticks(30);
    assert_eq!(
        a.world.enemies[&id].state,
        AiState::Idle,
        "80 m is out of sight range"
    );
    let t = a.tuning.clone();
    let info = DamageInfo::new(5.0, Owner::Player(p), Attack::Dev)
        .at(Vec3::new(0.0, 1.0, -80.0), Vec3::NEG_Z);
    a.world.apply_damage(&t, HitTarget::Entity(id), info);
    a.ticks(5);
    let e = &a.world.enemies[&id];
    assert_eq!(e.target, Some(p));
    assert!(matches!(e.state, AiState::Alert | AiState::Chase));
}

#[test]
fn all_enemy_types_are_deterministic() {
    let run = || {
        let mut a = Arena::new(lava_level());
        let p = a.player(0, Vec3::new(-8.0, 0.0, 20.0), 0.0);
        god(&mut a, p);
        let roster = [
            "gnawer",
            "censer",
            "harrier",
            "gaoler",
            "heron",
            "gnawer",
            "harrier",
            "bellwether",
            "snuffer",
            "smelter",
        ];
        for (i, name) in roster.iter().enumerate() {
            let y = if *name == "harrier" { 6.0 } else { 0.0 };
            let id = spawn(
                &mut a,
                name,
                Vec3::new(-15.0 + 5.0 * i as f32, y, -20.0),
                0.0,
            );
            a.world.enemies.get_mut(&id).unwrap().aware = true;
        }
        a.ticks(900);
        let v: Vec<_> = a
            .world
            .enemies
            .values()
            .map(|e| {
                (
                    e.position,
                    e.velocity,
                    e.state,
                    e.health,
                    e.aim,
                    e.frenzy_until,
                )
            })
            .collect();
        let p: Vec<_> = a
            .world
            .projectiles
            .values()
            .map(|p| (p.position, p.velocity))
            .collect();
        let h: Vec<_> = a
            .world
            .enemy_hazards
            .values()
            .map(|h| (h.points.clone(), h.ignite_tick))
            .collect();
        (v, p, h, a.events.len())
    };
    assert_eq!(run(), run());
}

#[test]
fn frozen_world_stops_enemies() {
    // Results screen: `SimWorld::frozen` — nobody attacks, nobody thinks.
    let mut a = Arena::coop();
    let p = a.player(0, Vec3::ZERO, 0.0);
    let ids: Vec<NetId> = [
        "gnawer",
        "censer",
        "gaoler",
        "heron",
        "harrier",
        "bellwether",
        "snuffer",
        "smelter",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| {
        let y = if *n == "harrier" { 6.0 } else { 0.0 };
        spawn(&mut a, n, Vec3::new(-14.0 + 4.0 * i as f32, y, -10.0), 0.0)
    })
    .collect();
    a.ticks(50);
    assert!(ids.iter().any(|id| a.world.enemies[id].target == Some(p)));
    a.world.frozen = true;
    let ev = a.ticks(300);
    assert_eq!(
        damage_to(&ev, HitTarget::Player(p)),
        0.0,
        "no damage while frozen"
    );
    assert!(!ev.iter().any(|e| matches!(
        e,
        SimEvent::EnemyAttack { .. } | SimEvent::EnemyFired { .. }
    )));
    for id in &ids {
        let e = &a.world.enemies[id];
        assert_eq!(e.state, AiState::Idle);
        assert_eq!(e.target, None);
        if e.is_flying() {
            assert!(
                !e.on_ground && e.velocity.length() < 0.5,
                "the harrier hovers in place"
            );
        } else {
            assert!(e.on_ground, "still standing on the floor");
        }
    }
}
