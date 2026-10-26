//! End-to-end combat over the real network stack: a server and real `NetClient`s talking over
//! UDP loopback with the network simulator, on a virtual clock (deterministic, fast). Inputs
//! are produced per tick by closures that can look at what each client *displays*, so shots
//! and parries are aimed exactly like a player would.

use std::path::PathBuf;
use std::time::Duration;

use client::net::{ClientConfig, NetClient, NetEvent, NetSimConfig};
use engine::time::FixedTimestep;
use glam::Vec3;
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput};
use shared::protocol::{GameEvent, NetId, kind};
use shared::sim::projectiles::replicated_position;
use shared::sim::{AiState, Attack, DamageFlags, HitTarget, Owner, PlayerId, ProjectileKind};
use shared::tuning::Tuning;

const STEP: Duration = Duration::from_millis(2);

fn config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: shared::sim::BUILTIN_LEVEL.to_string(),
        // Defaults only: tests must not depend on the repo's assets/tuning.ron.
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        enemies_dir: Some(PathBuf::from("/nonexistent/velocide-test/enemies")),
        watch_tuning: false,
        ..Default::default()
    }
}

struct TestClient {
    net: NetClient,
    timestep: FixedTimestep,
    events: Vec<NetEvent>,
    ticks: u32,
}

impl TestClient {
    fn id(&self) -> PlayerId {
        self.net.local_player_id().expect("connected")
    }

    fn eye(&self) -> Vec3 {
        let t = self.net.tuning();
        self.net
            .local_state()
            .expect("in game")
            .eye_position(&t.movement)
    }

    /// Yaw/pitch looking from the local eye at `target`.
    fn aim_at(&self, target: Vec3) -> (f32, f32) {
        let d = (target - self.eye()).normalize();
        ((-d.x).atan2(-d.z), d.y.clamp(-1.0, 1.0).asin())
    }

    fn game_events(&self) -> impl Iterator<Item = &GameEvent> {
        self.events.iter().filter_map(|e| match e {
            NetEvent::Game(g) => Some(g),
            _ => None,
        })
    }
}

struct Net {
    server: Server,
    clients: Vec<TestClient>,
    time: f64,
}

/// Per-tick input of client `i` (None: send nothing).
type InputFn<'a> = dyn FnMut(usize, &TestClient) -> Option<PlayerInput> + 'a;

impl Net {
    fn new(config: ServerConfig) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();
        Self {
            server: Server::new(config).expect("server starts"),
            clients: Vec::new(),
            time: 0.0,
        }
    }

    fn add(&mut self, name: &str, netsim: Option<NetSimConfig>) -> usize {
        let net = NetClient::connect(
            self.server.local_addr(),
            ClientConfig::new(name).with_netsim(netsim),
        )
        .expect("client starts");
        self.clients.push(TestClient {
            net,
            timestep: FixedTimestep::new(60.0),
            events: Vec::new(),
            ticks: 0,
        });
        self.clients.len() - 1
    }

    fn step(&mut self, input: &mut InputFn) {
        self.server.update(STEP);
        for (i, c) in self.clients.iter_mut().enumerate() {
            c.net.update(STEP);
            c.events.extend(c.net.drain_events());
            c.timestep.set_hz(c.net.tick_rate() as f64);
            for _ in 0..c.timestep.advance(STEP.as_secs_f64()) {
                if c.net.is_in_game()
                    && let Some(inp) = input(i, c)
                {
                    c.net.tick(inp);
                    c.ticks += 1;
                }
            }
        }
        self.time += STEP.as_secs_f64();
    }

    fn run(&mut self, secs: f64, input: &mut InputFn) {
        let end = self.time + secs;
        while self.time < end {
            self.step(input);
        }
    }

    fn run_until(
        &mut self,
        secs: f64,
        input: &mut InputFn,
        mut done: impl FnMut(&Net) -> bool,
    ) -> bool {
        let end = self.time + secs;
        while self.time < end {
            self.step(input);
            if done(self) {
                return true;
            }
        }
        false
    }

    fn wait_in_game(&mut self) {
        let ok = self.run_until(10.0, &mut |_, _| Some(PlayerInput::default()), |n| {
            n.clients.iter().all(|c| c.net.is_in_game())
        });
        assert!(ok, "clients did not get in game");
    }

    fn tuning(&self) -> Tuning {
        self.server.tuning().clone()
    }

    fn health(&self, p: PlayerId) -> f32 {
        self.server.world().players[&p]
            .combat
            .health(&self.server.tuning().combat)
    }

    /// Teleports a player (server-side; the client reconciles) and gives it a view yaw.
    fn place(&mut self, p: PlayerId, pos: Vec3) {
        let w = self.server.world_mut();
        let pl = w.players.get_mut(&p).unwrap();
        pl.movement = shared::movement::MoveState::spawn(pos);
    }
}

fn idle(yaw: f32) -> PlayerInput {
    PlayerInput {
        yaw,
        ..Default::default()
    }
}

/// One-way latency `ms` (RTT ≈ 2 ms), small jitter, no loss.
fn latency(ms: f32, seed: u64) -> Option<NetSimConfig> {
    Some(NetSimConfig::new(ms, 5.0, 0.0).with_seed(seed))
}

/// What a lag-compensated shot did.
struct ShotResult {
    dealt: f32,
    /// How far (m) the target really was from where the shooter saw it when the server
    /// processed the shot.
    offset: f32,
    /// How far back (ms) the server had to rewind.
    rewind_ms: f32,
}

/// A shooter with `one_way_ms` latency taps FIRE at a target strafing at full speed (16 m/s),
/// aiming at where its screen shows the target.
fn lag_comp_shot(one_way_ms: f32, lag_comp_max_ms: f32) -> ShotResult {
    let mut tuning = Tuning::default();
    tuning.net.lag_comp_max_ms = lag_comp_max_ms;
    let mut n = Net::new(config());
    let shooter = n.add("Shooter", latency(one_way_ms, 3));
    let target = n.add("Target", None);
    n.wait_in_game();
    n.server.set_tuning(tuning);
    let (sid, tid) = (n.clients[shooter].id(), n.clients[target].id());
    n.place(sid, Vec3::new(0.0, 0.0, 20.0));
    n.place(tid, Vec3::new(0.0, 0.0, 0.0));
    let fire_tick = n.clients[shooter].ticks + 150;
    let seen_at_fire = std::cell::Cell::new(None);
    let mut input = |i: usize, c: &TestClient| {
        if i == target {
            // Strafe left/right at full speed, swapping every 1.5 s.
            let mut inp = idle(0.0);
            inp.right = if (c.ticks / 90).is_multiple_of(2) {
                127
            } else {
                -127
            };
            return Some(inp);
        }
        let seen = c
            .net
            .remote_entity(NetId::player(c_target_id(c)))
            .map(|e| e.position + Vec3::Y * 1.0);
        let mut inp = idle(0.0);
        if let Some(seen) = seen {
            let (yaw, pitch) = c.aim_at(seen);
            inp.yaw = yaw;
            inp.pitch = pitch;
        }
        inp.buttons.set(Buttons::FIRE, c.ticks == fire_tick);
        if c.ticks == fire_tick {
            seen_at_fire.set(seen);
        }
        Some(inp)
    };
    n.run(1.0, &mut input);
    let before = n.health(tid);
    n.server.drain_sim_events();
    let mut shot_tick = None;
    let mut offset = 0.0;
    let mut rewind_ms = 0.0;
    let end = n.time + 3.5;
    while n.time < end {
        n.step(&mut input);
        if shot_tick.is_none()
            && n.server
                .drain_sim_events()
                .iter()
                .any(|e| matches!(e, shared::sim::SimEvent::Shot(_)))
        {
            shot_tick = Some(n.server.tick());
            let w = n.server.world();
            let truth = w.players[&tid].movement.position + Vec3::Y;
            offset = seen_at_fire.get().map_or(0.0, |s: Vec3| s.distance(truth));
            rewind_ms = w.players[&sid].view_lag_ticks * 1000.0 / 60.0;
        }
    }
    assert!(shot_tick.is_some(), "the shot was fired");
    ShotResult {
        dealt: before - n.health(tid),
        offset,
        rewind_ms,
    }
}

fn c_target_id(c: &TestClient) -> PlayerId {
    // The only other player.
    c.net
        .players()
        .keys()
        .copied()
        .find(|p| Some(*p) != c.net.local_player_id())
        .unwrap_or(PlayerId(0))
}

#[test]
fn lag_compensated_hit_at_100ms_latency() {
    // ~100 ms ping (50 ms each way), default tuning (lag_comp_max_ms 250).
    let r = lag_comp_shot(50.0, 250.0);
    println!(
        "~100 ms RTT: target {:.2} m away from where the shooter saw it, server rewound {:.0} ms, dealt {}",
        r.offset, r.rewind_ms, r.dealt
    );
    assert!(
        r.offset > 2.0,
        "the target moved {} m: a real test",
        r.offset
    );
    assert_eq!(r.dealt, 24.0, "hit as seen");
    let r = lag_comp_shot(50.0, 0.0);
    assert_eq!(
        r.dealt, 0.0,
        "the same shot without lag compensation misses"
    );

    // ~200 ms ping (100 ms each way) needs ~330 ms of rewind: more than the default
    // `net.lag_comp_max_ms` (250) allows, so it is raised here (see docs/combat.md).
    let r = lag_comp_shot(100.0, 400.0);
    println!(
        "~200 ms RTT: target {:.2} m away from where the shooter saw it, server rewound {:.0} ms, dealt {}",
        r.offset, r.rewind_ms, r.dealt
    );
    assert!(r.offset > 4.0);
    assert_eq!(r.dealt, 24.0, "hit as seen");
}

#[test]
fn cover_blocks_shots_over_the_network() {
    let mut n = Net::new(config());
    let shooter = n.add("Shooter", latency(50.0, 4));
    let target = n.add("Target", None);
    n.wait_in_game();
    let (sid, tid) = (n.clients[shooter].id(), n.clients[target].id());
    // The proving ground's 3 m metal block (24..30, 0..3, 24..30) is between them.
    n.place(sid, Vec3::new(27.0, 0.0, 36.0));
    n.place(tid, Vec3::new(27.0, 0.0, 18.0));
    let mut input = |i: usize, c: &TestClient| {
        if i == target {
            return Some(idle(0.0));
        }
        let mut inp = idle(0.0);
        if let Some(e) = c.net.remote_entity(NetId::player(c_target_id(c))) {
            let (yaw, pitch) = c.aim_at(e.position + Vec3::Y * 1.0);
            inp.yaw = yaw;
            inp.pitch = pitch;
        }
        inp.buttons.set(Buttons::FIRE, c.ticks > 60);
        Some(inp)
    };
    n.run(3.0, &mut input);
    assert_eq!(n.health(tid), 100.0);
    assert!(
        !n.clients[shooter]
            .game_events()
            .any(|e| matches!(e, GameEvent::HitConfirm(_)))
    );
    // Step out from behind the block: hits land.
    n.place(sid, Vec3::new(42.0, 0.0, 36.0));
    n.run(2.0, &mut input);
    assert!(n.health(tid) < 100.0);
    assert!(
        n.clients[shooter]
            .game_events()
            .any(|e| matches!(e, GameEvent::HitConfirm(h) if h.target == HitTarget::Player(tid)))
    );
    assert!(
        n.clients[target]
            .game_events()
            .any(|e| matches!(e, GameEvent::Damaged(d) if d.source == Owner::Player(sid)))
    );
}

#[test]
fn breach_jump_reconciles() {
    let mut n = Net::new(config());
    let a = n.add("Jumper", latency(50.0, 5));
    n.wait_in_game();
    let id = n.clients[a].id();
    n.run(1.0, &mut |_, _| Some(idle(0.0)));
    assert_eq!(n.clients[a].net.stats().corrections, 0);
    let fire = n.clients[a].ticks + 5;
    let mut max_y: f32 = 0.0;
    let mut input = |_: usize, c: &TestClient| {
        let mut inp = idle(0.0);
        inp.weapon = 3;
        inp.pitch = -1.5;
        inp.buttons.set(Buttons::ALT_FIRE, c.ticks == fire);
        inp.buttons.set(Buttons::JUMP, c.ticks == fire);
        Some(inp)
    };
    let end = n.time + 3.0;
    while n.time < end {
        n.step(&mut input);
        max_y = max_y.max(n.server.world().players[&id].movement.position.y);
    }
    // Stop sending and let every input be acknowledged before comparing states.
    n.run(1.5, &mut |_, _| None);
    let c = &n.clients[a];
    let s = c.net.stats();
    let damaged = c
        .game_events()
        .find_map(|e| match e {
            GameEvent::Damaged(d) => Some(*d),
            _ => None,
        })
        .expect("self damage reported");
    println!(
        "breach jump: apex {max_y:.2} m (a jump alone: ~2.6 m), self damage {:.1}, corrections {}, max error at ack {:.3} m, snaps {}",
        damaged.amount, s.corrections, s.max_correction_m, s.correction_snaps
    );
    assert!(
        damaged
            .flags
            .contains(DamageFlags::SELF | DamageFlags::EXPLOSIVE)
    );
    assert_eq!(damaged.attack, Attack::Breach);
    assert!(max_y > 4.0, "breach jump apex {max_y}");
    assert!(
        s.corrections >= 1 && s.corrections <= 3,
        "{} corrections",
        s.corrections
    );
    assert_eq!(s.correction_snaps, 0);
    // Prediction converged on the server state.
    assert_eq!(
        *c.net.local_state().unwrap(),
        n.server.world().players[&id].movement
    );
    assert!(n.health(id) < 100.0 && n.health(id) > 70.0);
}

/// An orb flies at the player; the client punches when its *display* shows the orb within
/// `punch_at` metres (or `late_ms` after the display shows it touching). Returns
/// (parried, damage taken, parry late_ticks).
fn net_parry(one_way_ms: f32, punch_at: Option<f32>, late_ms: f32) -> (bool, f32, u32) {
    let mut n = Net::new(config());
    let a = n.add("Parrier", latency(one_way_ms, 6));
    n.wait_in_game();
    let id = n.clients[a].id();
    n.place(id, Vec3::new(0.0, 0.0, 0.0));
    n.server.world_mut().players.get_mut(&id).unwrap().notarget = true;
    n.run(1.0, &mut |_, _| Some(idle(0.0)));
    let tuning = n.tuning();
    let owner = n
        .server
        .world_mut()
        .spawn_enemy("gnawer", Vec3::new(0.0, 0.0, -50.0), 0.0)
        .unwrap();
    let orb = n
        .server
        .world_mut()
        .spawn_enemy_projectile(
            &tuning,
            ProjectileKind::Needle,
            owner,
            Attack::Enemy {
                kind: kind::HERON,
                attack: 0,
            },
            Vec3::new(0.0, 1.4, -25.0),
            Vec3::new(0.0, 0.0, 22.0),
            20.0,
        )
        .expect("orb");
    let mut punched_at: Option<f64> = None;
    let mut touched_at: Option<f64> = None;
    let mut time = 0.0;
    let mut input = |_: usize, c: &TestClient| {
        time += 1.0 / 60.0;
        let mut inp = idle(0.0);
        let seen = c.net.remote_entity(orb).map(|e| {
            replicated_position(e, c.net.render_tick(), 60, &c.net.tuning().combat)
                .distance(c.eye())
        });
        if let Some(d) = seen
            && d <= 0.8
            && touched_at.is_none()
        {
            touched_at = Some(time);
        }
        let go = match punch_at {
            Some(dist) => seen.is_some_and(|d| d <= dist),
            None => touched_at.is_some_and(|t0| time - t0 >= late_ms as f64 / 1000.0),
        };
        if go && punched_at.is_none() {
            punched_at = Some(time);
            inp.buttons.set(Buttons::PUNCH, true);
        }
        Some(inp)
    };
    n.run(3.0, &mut input);
    let c = &n.clients[a];
    let parry = c.game_events().find_map(|e| match e {
        GameEvent::Parried(p) => Some(*p),
        _ => None,
    });
    let damage: f32 = c
        .game_events()
        .filter_map(|e| match e {
            GameEvent::Damaged(d) => Some(d.amount),
            _ => None,
        })
        .sum();
    (parry.is_some(), damage, parry.map_or(0, |p| p.late_ticks))
}

#[test]
fn parry_over_the_network() {
    let (ok, dmg, late) = net_parry(0.0, Some(2.0), 0.0);
    println!("parry, 0 ms: parried {ok}, damage {dmg}, late {late} ticks");
    assert!(ok && dmg == 0.0);
    // ~100 ms RTT.
    let (ok, dmg, late) = net_parry(50.0, Some(2.0), 0.0);
    println!("parry, 50 ms one-way (~100 ms RTT): parried {ok}, damage {dmg}, late {late} ticks");
    assert!(
        ok && dmg == 0.0,
        "parry on what the client saw works at ~100 ms"
    );
    // ~200 ms RTT still works.
    let (ok, dmg, late) = net_parry(100.0, Some(2.0), 0.0);
    println!("parry, 100 ms one-way (~200 ms RTT): parried {ok}, damage {dmg}, late {late} ticks");
    assert!(ok && dmg == 0.0);
    // Punching 300 ms after the orb (as displayed) touched you is too late.
    let (ok, dmg, _) = net_parry(50.0, None, 300.0);
    assert!(!ok);
    assert_eq!(dmg, 20.0);
}

#[test]
fn gnawer_chases_and_hits_a_client() {
    let mut n = Net::new(config());
    let a = n.add("Bait", latency(30.0, 7));
    n.wait_in_game();
    let id = n.clients[a].id();
    n.place(id, Vec3::ZERO);
    let gnawer = n
        .server
        .spawn_enemies("gnawer", Vec3::new(0.0, 0.0, -20.0), 0.0, 1)[0];
    let mut saw_windup = false;
    let hit = n.run_until(6.0, &mut |_, _| Some(idle(0.0)), |n| {
        n.clients[a]
            .game_events()
            .any(|e| matches!(e, GameEvent::Damaged(d) if d.source == Owner::Enemy(gnawer)))
    });
    assert!(hit, "the gnawer reached and hit the player");
    // The client saw the telegraph (interpolated entity state) before the hit.
    n.run(1.5, &mut |i, c| {
        let _ = i;
        if let Some(e) = c.net.remote_entity(gnawer)
            && e.state == AiState::Windup as u16
        {
            saw_windup = true;
        }
        Some(idle(0.0))
    });
    assert!(saw_windup);
    let e = n.clients[a]
        .net
        .remote_entity(gnawer)
        .copied()
        .expect("gnawer replicated");
    assert_eq!(e.kind, kind::GNAWER);
    assert_eq!(e.owner, id.0 as u16, "targeting us");
    assert!(n.health(id) <= 86.0, "bitten (14)");
}

/// A stationary 20 HP target defined here (independent of the enemy roster).
fn target_dummy() -> shared::sim::EnemyDef {
    shared::sim::EnemyDef::parse(
        "(name: \"Dummy\", net_kind: 60, model: \"dummy\", health: 20.0, speed: 0.0,
          acceleration: 60.0, radius: 0.45, height: 1.9, body_top: Some(1.45),
          weak_points: [(name: \"head\", kind: Head, offset: (0.0, 1.66, 0.0), radius: 0.26,
          multiplier: 2.0)])",
    )
    .expect("dummy definition")
}

#[test]
fn every_weapon_mode_damages_targets_over_the_network() {
    let mut n = Net::new(config());
    let a = n.add("Gunner", latency(30.0, 8));
    n.wait_in_game();
    let id = n.clients[a].id();
    n.place(id, Vec3::ZERO);
    n.server.world_mut().players.get_mut(&id).unwrap().notarget = true;
    n.server
        .world_mut()
        .enemy_defs
        .insert("dummy", target_dummy());
    // (weapon, mode, button, distance, name, ticks held)
    let cases: [(u8, u8, Buttons, f32, &str, u32); 12] = [
        (0, 0, Buttons::FIRE, 12.0, "quill", 1),
        (0, 0, Buttons::ALT_FIRE, 12.0, "brand", 1),
        (0, 1, Buttons::ALT_FIRE, 12.0, "splinter", 1),
        (1, 0, Buttons::FIRE, 6.0, "bellows", 1),
        (1, 0, Buttons::ALT_FIRE, 5.0, "gust", 1),
        (1, 1, Buttons::ALT_FIRE, 6.0, "both barrels", 1),
        (2, 0, Buttons::FIRE, 10.0, "arc", 1),
        (2, 0, Buttons::ALT_FIRE, 4.0, "wisp", 1),
        (2, 1, Buttons::ALT_FIRE, 5.0, "discharge", 40),
        (3, 0, Buttons::FIRE, 10.0, "tumbler", 1),
        (3, 0, Buttons::ALT_FIRE, 2.5, "breach", 1),
        (3, 1, Buttons::ALT_FIRE, 10.0, "gravewell", 1),
    ];
    for (weapon, mode, button, dist, name, hold) in cases {
        n.server.kill_all_enemies();
        n.run(1.0, &mut |_, _| Some(idle(0.0)));
        let target =
            n.server
                .spawn_enemies("dummy", Vec3::new(0.0, 0.0, -dist), std::f32::consts::PI, 1)[0];
        n.run(0.5, &mut |_, _| Some(idle(0.0)));
        let start = n.clients[a].ticks;
        // Bombs drop on the way: aim at the head.
        let lift = if name == "tumbler" { 1.6 } else { 0.8 };
        let mut input = |_: usize, c: &TestClient| {
            let mut inp = idle(0.0);
            inp.weapon = weapon;
            inp.weapon_mode = mode;
            let aim = c
                .net
                .remote_entity(target)
                .map(|e| e.position + Vec3::Y * lift)
                .unwrap_or(Vec3::new(0.0, 1.0, -dist));
            let (yaw, pitch) = c.aim_at(aim);
            inp.yaw = yaw;
            inp.pitch = pitch;
            let t = c.ticks - start;
            inp.buttons.set(button, t >= 10 && t < 10 + hold);
            Some(inp)
        };
        n.run(3.0, &mut input);
        let health = n
            .server
            .world()
            .enemies
            .get(&target)
            .map_or(0.0, |e| e.health.max(0.0));
        let confirmed = n.clients[a].game_events().any(|e| {
            matches!(e, GameEvent::HitConfirm(h) if h.target == HitTarget::Entity(target) && h.damage > 0.0)
        });
        println!("{name}: target health {health:.0}, hit confirmed {confirmed}");
        assert!(health < 20.0, "{name} damaged the target");
        assert!(confirmed, "{name}: client got a hit confirm");
    }
    let kills = n.clients[a]
        .game_events()
        .filter(|e| matches!(e, GameEvent::Killed(k) if k.killer == Owner::Player(id)))
        .count();
    assert!(kills >= 7, "{kills} kills reported to the client");
}

#[test]
fn bandwidth_with_8_players_50_gnawers_and_150_projectiles() {
    let mut n = Net::new(config());
    let clients: Vec<usize> = (0..4)
        .map(|i| n.add(&format!("P{i}"), latency(30.0, 10 + i as u64)))
        .collect();
    n.wait_in_game();
    for i in 0..4 {
        n.server.add_bot(&format!("Bot{i}"));
    }
    let ids: Vec<PlayerId> = n.server.world().players.keys().copied().collect();
    for (k, id) in ids.iter().enumerate() {
        n.place(*id, Vec3::new(k as f32 * 3.0 - 12.0, 0.0, 30.0));
        n.server.world_mut().players.get_mut(id).unwrap().combat.god = true;
    }
    for i in 0..50 {
        let x = (i % 10) as f32 * 4.0 - 18.0;
        let z = -20.0 - (i / 10) as f32 * 4.0;
        n.server.spawn_enemies("gnawer", Vec3::new(x, 0.0, z), 0.0, 1);
    }
    // Clients run around firing rivets into the sky; the server keeps ~150 projectiles alive.
    let tuning = n.tuning();
    let mut input = |i: usize, c: &TestClient| {
        let t = c.ticks;
        let mut inp = PlayerInput {
            forward: 127,
            right: if (t / 60).is_multiple_of(2) { 90 } else { -90 },
            yaw: t as f32 * 0.02 + i as f32,
            pitch: 0.9,
            // Arclash: a zap every 0.1 s.
            weapon: 2,
            ..Default::default()
        };
        inp.buttons.set(Buttons::FIRE, true);
        inp.buttons.set(Buttons::JUMP, t.is_multiple_of(50));
        Some(inp)
    };
    let mut max_projectiles = 0;
    let mut topped_up = 0;
    let mut measure = |n: &mut Net, secs: f64| {
        let end = n.time + secs;
        while n.time < end {
            n.step(&mut input);
            let live = n.server.world().projectiles.len();
            if live < 150 {
                let w = n.server.world_mut();
                for k in 0..(150 - live).min(4) {
                    let a = (w.tick as f32 + k as f32) * 0.37;
                    // Wisps high above everything (no gravity, nothing to zap), kept alive
                    // 5 s like any long flight.
                    let id = w.spawn_projectile(
                        &tuning,
                        ProjectileKind::Wisp,
                        Owner::Player(PlayerId(0)),
                        Vec3::new(a.cos() * 30.0, 40.0, a.sin() * 30.0),
                        Vec3::new(-a.sin() * 20.0, 5.0, a.cos() * 20.0),
                    );
                    if let Some(p) = id.and_then(|id| w.projectiles.get_mut(&id)) {
                        p.expire_tick = p.spawn_tick + 300;
                    }
                    topped_up += 1;
                }
            }
            max_projectiles = max_projectiles.max(n.server.world().projectiles.len());
        }
    };
    measure(&mut n, 2.0);
    let before = n.server.stats();
    let secs = 5.0;
    measure(&mut n, secs);
    let after = n.server.stats();
    let snapshots = (after.snapshots_sent - before.snapshots_sent) as f64;
    let per_client_wire = (after.transport.bytes_sent - before.transport.bytes_sent) as f64
        / secs
        / clients.len() as f64;
    let snapshot_avg = (after.snapshot_bytes - before.snapshot_bytes) as f64 / snapshots;
    let entity_avg = (after.entity_bytes - before.entity_bytes) as f64 / snapshots;
    let fx_per_snapshot = (after.fx_sent - before.fx_sent) as f64 / snapshots;
    println!(
        "8 players (4 clients + 4 bots), {} gnawers, {} projectiles live (max {max_projectiles}, {topped_up} spawned by the test):",
        after.enemies, after.projectiles
    );
    println!(
        "  server->client {per_client_wire:.0} B/s per client on the wire; snapshot {snapshot_avg:.0} B avg ({entity_avg:.0} B entities, {fx_per_snapshot:.1} fx); {} reliable combat events",
        after.combat_events_sent - before.combat_events_sent
    );
    let c = &n.clients[clients[0]];
    println!(
        "  client 0: in {:.0} B/s, out {:.0} B/s, corrections {}",
        c.net.stats().bytes_in_per_sec,
        c.net.stats().bytes_out_per_sec,
        c.net.stats().corrections
    );
    assert!(max_projectiles >= 140, "{max_projectiles}");
    assert!(after.enemies >= 50);
    assert!(
        per_client_wire < 40_000.0,
        "{per_client_wire} B/s per client"
    );
    // Every client still sees everything.
    let seen = c.net.remote_entities().len();
    assert!(seen > 150, "client sees {seen} entities");
    println!("  server tick time {:.0} us", after.tick_time_us);
}

#[test]
fn dev_chat_commands() {
    let mut n = Net::new(ServerConfig {
        dev_commands: true,
        ..config()
    });
    let a = n.add("Dev", None);
    n.wait_in_game();
    let id = n.clients[a].id();
    for cmd in [
        "/spawn gnawer 3",
        "/god",
        "/notarget",
        "/hurt 30",
        "/ff off",
        "/bogus",
    ] {
        n.clients[a].net.send_chat(cmd);
        // Commands share chat's rate limit (burst 5, then 1/s).
        n.run(0.5, &mut |_, _| Some(idle(0.0)));
    }
    let replies: Vec<String> = n.clients[a]
        .events
        .iter()
        .filter_map(|e| match e {
            NetEvent::Chat {
                from: None, text, ..
            } if !text.contains("joined") => Some(text.clone()),
            _ => None,
        })
        .collect();
    println!("dev replies: {replies:?}");
    assert_eq!(n.server.world().enemies.len(), 3);
    let p = &n.server.world().players[&id];
    assert!(p.combat.god && p.notarget);
    assert!(!n.server.world().rules.friendly_fire);
    assert!(replies.iter().any(|r| r.starts_with("spawned 3 gnawer")));
    assert!(
        replies.iter().any(|r| r.starts_with("took 0")),
        "god mode blocks /hurt"
    );
    assert!(replies.iter().any(|r| r.contains("unknown command")));
    // Commands are not broadcast as chat.
    assert!(
        !n.clients[a]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Chat { from: Some(_), .. }))
    );
    n.clients[a].net.send_chat("/killall");
    n.run(1.0, &mut |_, _| Some(idle(0.0)));
    assert!(n.server.world().enemies.values().all(|e| !e.is_alive()));
}
