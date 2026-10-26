//! Client combat plumbing over a real server + clients (UDP loopback, virtual clock): the
//! client-side weapon prediction (`step_weapons` on predicted inputs) stays exactly in sync
//! with the authoritative `OwnState::combat.weapons`, own combat state reaches the client,
//! and cosmetic Fx arrive exactly once (repeats de-duplicated).

use std::path::PathBuf;
use std::time::Duration;

use client::ingame::combat::weapon_feel::WeaponPredictor;
use client::net::{ClientConfig, NetClient, NetSimConfig};
use engine::time::FixedTimestep;
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput};
use shared::protocol::Fx;
use shared::sim::FireIntent;

const STEP: Duration = Duration::from_millis(2);

fn server() -> Server {
    let _ = env_logger::builder().is_test(true).try_init();
    Server::new(ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: shared::sim::BUILTIN_LEVEL.to_string(),
        tuning_path: Some(PathBuf::from("/nonexistent/velocide-test/tuning.ron")),
        enemies_dir: Some(PathBuf::from("/nonexistent/velocide-test/enemies")),
        watch_tuning: false,
        ..Default::default()
    })
    .expect("server starts")
}

struct C {
    net: NetClient,
    ts: FixedTimestep,
    ticks: u32,
    predictor: WeaponPredictor,
    intents: Vec<FireIntent>,
    fx: Vec<shared::protocol::FxEvent>,
}

fn client(s: &Server, latency: f32, seed: u64) -> C {
    C {
        net: NetClient::connect(
            s.local_addr(),
            ClientConfig::new("c")
                .with_netsim(Some(NetSimConfig::new(latency, 3.0, 0.0).with_seed(seed))),
        )
        .expect("client"),
        ts: FixedTimestep::new(60.0),
        ticks: 0,
        predictor: WeaponPredictor::new(),
        intents: Vec::new(),
        fx: Vec::new(),
    }
}

fn step(s: &mut Server, cs: &mut [C], input: &dyn Fn(usize, u32) -> Option<PlayerInput>) {
    s.update(STEP);
    for (i, c) in cs.iter_mut().enumerate() {
        c.net.update(STEP);
        c.net.drain_events();
        c.fx.extend(c.net.drain_fx());
        for _ in 0..c.ts.advance(STEP.as_secs_f64()) {
            if !c.net.is_in_game() {
                continue;
            }
            if let Some(inp) = input(i, c.ticks) {
                c.net.tick(inp);
                if let Some(shot) = c.predictor.after_tick(&c.net) {
                    c.intents.extend(shot.intents);
                }
                c.ticks += 1;
            }
        }
    }
}

fn run(s: &mut Server, cs: &mut [C], secs: f64, input: &dyn Fn(usize, u32) -> Option<PlayerInput>) {
    for _ in 0..(secs / STEP.as_secs_f64()) as usize {
        step(s, cs, input);
    }
}

/// Fires every weapon and mode, charges Discharges, switches, punches and throws thorns.
fn script(t: u32) -> PlayerInput {
    let mut inp = PlayerInput {
        yaw: t as f32 * 0.01,
        pitch: 0.2,
        weapon: ((t / 90) % 4) as u8,
        weapon_mode: ((t / 360) % 2) as u8,
        forward: if (t / 60).is_multiple_of(2) {
            100
        } else {
            -100
        },
        ..Default::default()
    };
    inp.buttons.set(Buttons::FIRE, t % 40 < 25);
    inp.buttons.set(Buttons::ALT_FIRE, t % 70 > 30);
    inp.buttons.set(Buttons::PUNCH, t.is_multiple_of(33));
    inp.buttons.set(Buttons::THROW, t % 150 == 10);
    inp.buttons.set(Buttons::JUMP, t.is_multiple_of(55));
    inp
}

#[test]
fn weapon_prediction_matches_the_server() {
    let mut s = server();
    let mut cs = vec![client(&s, 50.0, 1)];
    run(&mut s, &mut cs, 2.0, &|_, _| Some(PlayerInput::default()));
    assert!(cs[0].net.is_in_game());
    run(&mut s, &mut cs, 8.0, &|_, t| Some(script(t)));
    // Stop and let everything be acknowledged.
    run(&mut s, &mut cs, 1.0, &|_, _| None);
    let c = &mut cs[0];
    c.predictor.sync(&c.net);
    let own = *c.net.own_combat().expect("own combat state");
    let id = c.net.local_player_id().unwrap();
    let server_weapons = s.world().players[&id].combat.weapons;
    println!(
        "{} predicted intents, {} re-base mismatches, {} inputs",
        c.intents.len(),
        c.predictor.mismatches,
        c.ticks
    );
    assert_eq!(
        own.combat.weapons, server_weapons,
        "client got the latest state"
    );
    assert_eq!(*c.predictor.state(), server_weapons, "prediction == server");
    assert_eq!(c.predictor.mismatches, 0, "prediction never disagreed");
    assert!(
        c.intents
            .iter()
            .any(|i| matches!(i, FireIntent::Primary { .. }))
    );
    assert!(
        c.intents
            .iter()
            .any(|i| matches!(i, FireIntent::Ability { .. }))
    );
    assert!(c.intents.iter().any(|i| matches!(i, FireIntent::Punch)));
    // An arena runs the PvP balance profile: no Thorns, and the client predicts none either
    // (it got the effective tuning). Thorn prediction itself: shared/tests/combat_weapons.rs.
    assert_eq!(c.net.tuning().combat.thorn.charges, 0);
    assert!(
        !c.intents
            .iter()
            .any(|i| matches!(i, FireIntent::ThrowThorn))
    );
    assert!(
        c.intents
            .iter()
            .any(|i| matches!(i, FireIntent::Ability { charge, .. } if *charge > 0.0)),
        "a charged Discharge"
    );
}

#[test]
fn fx_arrive_once_and_own_state_follows_damage() {
    let mut s = server();
    let mut cs = vec![client(&s, 30.0, 2), client(&s, 30.0, 3)];
    run(&mut s, &mut cs, 2.0, &|_, _| Some(PlayerInput::default()));
    // Client 0 breaches the floor and lobs bombs (explosions are repeated in two snapshots).
    run(&mut s, &mut cs, 4.0, &|i, t| {
        let mut inp = PlayerInput {
            weapon: 3,
            pitch: -1.2,
            ..Default::default()
        };
        if i == 0 {
            inp.buttons
                .set(Buttons::FIRE | Buttons::ALT_FIRE, t % 80 == 5);
        }
        Some(inp)
    });
    run(&mut s, &mut cs, 1.0, &|_, _| None);
    let explosions = cs[1]
        .fx
        .iter()
        .filter(|e| matches!(e.fx, Fx::Explosion { .. }))
        .count();
    let mut seqs: Vec<u32> = cs[1].fx.iter().map(|e| e.seq).collect();
    let n = seqs.len();
    seqs.sort_unstable();
    seqs.dedup();
    println!("client 1 got {n} fx ({explosions} explosions)");
    assert_eq!(seqs.len(), n, "every fx exactly once");
    assert!(explosions >= 3, "{explosions}");
    // The shooter's breaches hurt it (35 % self damage): its own state shows it.
    let own = cs[0].net.own_combat().unwrap();
    let id = cs[0].net.local_player_id().unwrap();
    assert_eq!(own.combat.damage, s.world().players[&id].combat.damage);
    assert!(own.combat.damage > 0.0);
    // Shots of client 0 reach client 1 (bombs = projectile weapon: no tracer ends).
    assert!(cs[1].fx.iter().any(|e| matches!(e.fx, Fx::Shot { .. })));
    assert!(
        !cs[0].fx.iter().any(|e| matches!(e.fx, Fx::Shot { .. })),
        "own shots not echoed"
    );
}
