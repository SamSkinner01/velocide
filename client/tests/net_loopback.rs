//! End-to-end networking tests: a real server and real clients over UDP loopback, with the
//! network simulator injecting latency, jitter and loss. Driven on a virtual clock (see
//! `common`), so they are fast and reproducible.

mod common;

use client::net::{NetEvent, NetSimConfig};
use common::*;
use glam::Vec3;
use shared::protocol::NetId;

/// 100 ms one-way latency (200 ms RTT), +-20 ms jitter and 2% loss in both directions.
fn bad_network(seed: u64) -> Option<NetSimConfig> {
    Some(NetSimConfig::new(100.0, 20.0, 2.0).with_seed(seed))
}

#[test]
fn two_clients_with_latency_jitter_and_loss() {
    let mut h = Harness::new(server_config());
    let a = h.add_client("Alpha", bad_network(11), script_runner);
    let b = h.add_client("Bravo", bad_network(22), script_strafer);
    h.wait_in_game();

    let id_a = h.clients[a].id();
    let id_b = h.clients[b].id();
    assert_ne!(id_a, id_b, "distinct player ids");
    assert!(
        h.clients[a]
            .events
            .iter()
            .any(|e| matches!(e, NetEvent::Connected { .. }))
    );

    // Scripted play for 8 s. From t+1 s on, compare B's interpolated view of A with A's true
    // server trajectory at the tick B is displaying.
    let start = h.time;
    let mut view_errors = Vec::new();
    let mut view_delays_ms = Vec::new();
    let mut extrapolating_frames = 0;
    let mut frames = 0;
    h.run_for(8.0, |h| {
        if h.time - start < 1.0 {
            return;
        }
        let bv = &h.clients[b].net;
        let render_tick = bv.render_tick();
        let Some(seen) = bv.remote_entity(NetId::player(id_a)) else {
            return;
        };
        let Some(truth) = h.server_position_at(id_a, render_tick) else {
            return;
        };
        frames += 1;
        if bv.stats().extrapolating {
            extrapolating_frames += 1;
        }
        view_errors.push(seen.position.distance(truth));
        view_delays_ms.push((h.server.tick() as f64 - render_tick) as f32 * 1000.0 / 60.0);
    });

    // Bandwidth while playing.
    let active: Vec<_> = h.clients.iter().map(|c| c.net.stats().clone()).collect();
    let server_active = h.server.stats();

    // Remote view of the other player: B never sees its own body as remote.
    assert!(
        h.clients[b]
            .net
            .remote_entity(NetId::player(id_b))
            .is_none()
    );

    // The server knows what tick B was looking at (for lag compensation).
    let b_view = h.server.world().players[&id_b].last_input.view_time_ticks();
    let lag_ticks = h.server.tick() as f64 - b_view;
    assert!(
        (5.0..40.0).contains(&lag_ticks),
        "B's inputs report view tick {b_view}, server at {}",
        h.server.tick()
    );

    // Stop sending; let every input be acknowledged.
    for c in &mut h.clients {
        c.scripted = false;
    }
    h.run_for(1.5, |_| {});

    let stats = h.server.stats();
    let (mean, p95, max) = summarize(&view_errors);
    let (delay_mean, _, _) = summarize(&view_delays_ms);
    println!("--- two clients, 100 ms one-way, +-20 ms jitter, 2% loss ---");
    println!(
        "B's view of A vs server truth: mean {:.4} m, p95 {:.4} m, max {:.4} m over {frames} frames ({extrapolating_frames} extrapolating)",
        mean, p95, max
    );
    println!("B displays A {delay_mean:.0} ms behind the server's current tick");
    for (name, s) in [("A", &active[a]), ("B", &active[b])] {
        println!(
            "{name}: rtt {:.0} ms, interp delay {:.0} ms (jitter {:.1} ms), {} snapshots ({} full), corrections {}, netsim dropped {} of {} packets, in {:.0} B/s out {:.0} B/s",
            s.rtt_ms,
            s.interp_delay_ms,
            s.jitter_ms,
            s.snapshots_received,
            s.full_snapshots_received,
            s.corrections,
            s.transport.sim_dropped,
            s.transport.sim_packets,
            s.bytes_in_per_sec,
            s.bytes_out_per_sec,
        );
    }
    println!(
        "server while playing: out {:.0} B/s, in {:.0} B/s for 2 clients; avg snapshot {:.1} B ({:.1} B entities)",
        server_active.transport.bytes_sent_per_sec,
        server_active.transport.bytes_received_per_sec,
        stats.snapshot_bytes as f64 / stats.snapshots_sent as f64,
        stats.entity_bytes as f64 / stats.snapshots_sent as f64,
    );
    println!("server: {stats:?}");

    for c in &h.clients {
        let id = c.id();
        let s = c.net.stats();
        // Deterministic movement, no external forces: prediction is exact.
        assert_eq!(s.corrections, 0, "{id}: unexpected corrections ({s:?})");
        // Every input acknowledged, predicted == authoritative.
        let predicted = *c.net.local_state().unwrap();
        let server = h.server.world().players[&id].movement;
        assert_eq!(c.net.prediction().unwrap().unacked_len(), 0);
        assert!(
            predicted.position.distance(server.position) < 1e-5,
            "{id}: predicted {:?} server {:?}",
            predicted.position,
            server.position
        );
        assert_eq!(predicted, server);
        assert!(
            s.transport.sim_dropped > 0,
            "netsim should have dropped packets"
        );
        assert!(s.rtt_ms > 150.0 && s.rtt_ms < 400.0, "rtt {}", s.rtt_ms);
        assert!(c.net.stats().snapshot_rate > 0.0 || c.net.stats().snapshots_received > 200);
    }
    // Every input arrived despite loss (redundancy), and was simulated exactly once.
    assert_eq!(stats.inputs_skipped, 0, "inputs lost despite redundancy");
    let ticks_a = h.clients[a].ticks as u64;
    let ticks_b = h.clients[b].ticks as u64;
    assert_eq!(stats.inputs_processed, ticks_a + ticks_b);
    assert!(
        stats.inputs_redundant > stats.inputs_processed,
        "redundant copies were sent"
    );

    // Interpolation tracks the true trajectory.
    assert!(frames > 2000, "only {frames} frames compared");
    assert!(mean < 0.05, "mean view error {mean}");
    assert!(p95 < 0.15, "p95 view error {p95}");
    assert!(max < 1.5, "max view error {max}");
    // ~100 ms latency + ~100 ms interpolation delay.
    assert!(
        (150.0..350.0).contains(&delay_mean),
        "remote view delay {delay_mean} ms"
    );
    // Delta compression was used for nearly everything.
    assert!(
        stats.full_snapshots_sent * 10 < stats.snapshots_sent,
        "{stats:?}"
    );
}

#[test]
fn server_impulse_is_corrected_smoothly() {
    let mut h = Harness::new(server_config());
    let a = h.add_client(
        "Rocket",
        Some(NetSimConfig::new(50.0, 0.0, 0.0).with_seed(5)),
        script_runner,
    );
    h.wait_in_game();
    let id = h.clients[a].id();
    h.run_for(2.0, |_| {});
    assert_eq!(h.clients[a].net.stats().corrections, 0);

    // An explosion launches the player (server-side only).
    h.server.apply_impulse(id, Vec3::new(4.0, 9.0, 0.0));
    let jump_before = h.clients[a].max_update_jump;
    h.run_for(2.0, |_| {});
    for c in &mut h.clients {
        c.scripted = false;
    }
    h.run_for(1.0, |_| {});

    let c = &h.clients[a];
    let s = c.net.stats();
    println!(
        "impulse: {} correction(s), error at ack {:.3} m, predicted position shifted {:.3} m, snaps {}, largest visible pop {:.4} m",
        s.corrections,
        s.max_correction_m,
        s.max_correction_shift_m,
        s.correction_snaps,
        c.max_update_jump
    );
    assert_eq!(jump_before, 0.0, "no pops before the impulse");
    assert!(s.corrections >= 1, "the impulse must be corrected");
    assert!(
        s.corrections <= 2,
        "only the impulse should cause corrections ({})",
        s.corrections
    );
    assert_eq!(
        s.correction_snaps, 0,
        "small corrections are smoothed, not snapped"
    );
    assert!(
        s.max_correction_shift_m > 0.3,
        "shift {}",
        s.max_correction_shift_m
    );
    // The drawn position doesn't pop when the correction arrives: the shift goes into the
    // decaying render offset (only the per-frame decay, ~2.4% of it, shows per 2 ms frame).
    assert!(
        c.max_update_jump < s.max_correction_shift_m * 0.05,
        "render position popped {} m",
        c.max_update_jump
    );
    let predicted = *c.net.local_state().unwrap();
    assert_eq!(predicted, h.server.world().players[&id].movement);
    assert!(c.net.prediction().unwrap().render_offset().length() < 1e-3);
}

#[test]
fn delta_compression_reduces_bandwidth() {
    // Same scripted session with and without delta compression: 2 moving players, 4 idle bots
    // (stand-ins for idle enemies/props) and the level's door.
    let run = |delta: bool| {
        let mut config = server_config();
        config.delta_compression = delta;
        let mut h = Harness::new(config);
        h.add_client("A", None, script_runner);
        h.add_client("B", None, script_strafer);
        for i in 0..4 {
            h.server.add_bot(&format!("Bot{i}"));
        }
        h.wait_in_game();
        h.run_for(1.0, |_| {});
        let before = h.server.stats();
        let secs = 5.0;
        h.run_for(secs, |_| {});
        let after = h.server.stats();
        let snapshots = (after.snapshots_sent - before.snapshots_sent) as f64;
        let entity = (after.entity_bytes - before.entity_bytes) as f64;
        let payload = (after.snapshot_bytes - before.snapshot_bytes) as f64;
        let wire = (after.transport.bytes_sent - before.transport.bytes_sent) as f64;
        (
            entity / snapshots,
            payload / snapshots,
            wire / secs / h.clients.len() as f64,
            after.full_snapshots_sent,
        )
    };
    let (d_entity, d_payload, d_wire, d_full) = run(true);
    let (f_entity, f_payload, f_wire, _) = run(false);
    println!("--- snapshot bandwidth, 2 moving players + 4 idle bots + 1 door, 30 Hz ---");
    println!(
        "delta: {d_entity:.1} B entity block, {d_payload:.1} B snapshot message, {d_wire:.0} B/s per client on the wire ({d_full} full snapshots)"
    );
    println!(
        "full:  {f_entity:.1} B entity block, {f_payload:.1} B snapshot message, {f_wire:.0} B/s per client on the wire"
    );
    println!(
        "delta saves {:.0}% of entity bytes, {:.0}% of server->client bytes/s",
        100.0 * (1.0 - d_entity / f_entity),
        100.0 * (1.0 - d_wire / f_wire)
    );
    assert!(
        d_entity < f_entity * 0.5,
        "entity block: delta {d_entity} vs full {f_entity}"
    );
    assert!(
        d_wire < f_wire * 0.8,
        "wire: delta {d_wire} vs full {f_wire}"
    );
}

#[test]
fn inputs_survive_heavy_loss() {
    // 10% loss each way: redundancy (6 inputs per packet) still delivers every input.
    let mut h = Harness::new(server_config());
    let a = h.add_client(
        "Lossy",
        Some(NetSimConfig::new(30.0, 10.0, 10.0).with_seed(77)),
        script_strafer,
    );
    h.wait_in_game();
    h.run_for(5.0, |_| {});
    h.clients[a].scripted = false;
    h.run_for(1.5, |_| {});
    let stats = h.server.stats();
    let c = &h.clients[a];
    println!(
        "10% loss: dropped {} of {} packets, server skipped {} inputs, corrections {}",
        c.net.stats().transport.sim_dropped,
        c.net.stats().transport.sim_packets,
        stats.inputs_skipped,
        c.net.stats().corrections
    );
    assert!(c.net.stats().transport.sim_dropped > 50);
    assert_eq!(stats.inputs_skipped, 0);
    assert_eq!(stats.inputs_processed, c.ticks as u64);
    assert_eq!(
        *c.net.local_state().unwrap(),
        h.server.world().players[&c.id()].movement
    );
}
