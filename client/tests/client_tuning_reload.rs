//! The debug overlay's tuning editor end to end: edit a value → the editor rewrites the tuning
//! file → the server's file watcher reloads it and broadcasts it → the client predicts with
//! the new values and stays in sync with the server (no ongoing corrections). Real time and a
//! real file watcher, so it takes a couple of seconds.

use std::time::{Duration, Instant};

use client::ingame::debug::TuningEditor;
use client::net::{ClientConfig, NetClient, NetEvent};
use engine::time::FixedTimestep;
use server::{Server, ServerConfig};
use shared::input::{Buttons, PlayerInput};

struct Pair {
    server: Server,
    net: NetClient,
    timestep: FixedTimestep,
    last: Instant,
    ticks: u32,
    events: Vec<NetEvent>,
}

impl Pair {
    fn pump(&mut self, jump: bool) {
        std::thread::sleep(Duration::from_millis(2));
        let now = Instant::now();
        let dt = now - self.last;
        self.last = now;
        self.server.update(dt);
        self.net.update(dt);
        self.events.extend(self.net.drain_events());
        for _ in 0..self.timestep.advance(dt.as_secs_f64()) {
            self.ticks += 1;
            let mut input = PlayerInput {
                forward: 127,
                yaw: self.ticks as f32 * 0.01,
                ..Default::default()
            };
            input
                .buttons
                .set(Buttons::JUMP, jump && self.ticks % 50 < 2);
            self.net.tick(input);
        }
    }

    fn run_until(&mut self, secs: f32, jump: bool, mut done: impl FnMut(&Pair) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed().as_secs_f32() < secs {
            self.pump(jump);
            if done(self) {
                return true;
            }
        }
        false
    }
}

#[test]
fn edited_tuning_reaches_server_and_prediction() {
    let dir = std::env::temp_dir().join(format!("velocide-tuning-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tuning.ron");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/tuning.ron");
    std::fs::copy(&repo, &path).unwrap();

    let server = Server::new(ServerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        level: shared::sim::BUILTIN_LEVEL.to_string(),
        tuning_path: Some(path.clone()),
        watch_tuning: true,
        ..Default::default()
    })
    .unwrap();
    let net = NetClient::connect(server.local_addr(), ClientConfig::new("tuner")).unwrap();
    let mut p = Pair {
        server,
        net,
        timestep: FixedTimestep::new(60.0),
        last: Instant::now(),
        ticks: 0,
        events: Vec::new(),
    };
    assert!(p.run_until(5.0, false, |p| p.net.is_in_game()), "connects");
    let old = p.net.tuning().movement.jump_speed;
    p.run_until(1.0, true, |_| false);
    assert_eq!(p.net.stats().corrections, 0, "in sync before the change");

    // Edit like the debug overlay does.
    let mut editor = TuningEditor::default();
    editor.sync_from(p.net.tuning());
    let field = editor
        .movement
        .iter_mut()
        .find(|f| f.name == "jump_speed")
        .unwrap();
    field.value = old as f64 + 4.5;
    editor.mark_dirty(0.0);
    assert!(editor.is_dirty());
    editor.save_to(&path, 0.0);
    assert!(editor.status.contains("saved 1 value"), "{}", editor.status);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("// VELOCIDE gameplay tuning"),
        "comments kept"
    );

    p.events.clear();
    let want = old + 4.5;
    assert!(
        p.run_until(6.0, true, |p| p.net.tuning().movement.jump_speed == want),
        "the client gets the reloaded tuning"
    );
    assert_eq!(p.server.tuning().movement.jump_speed, want);
    assert!(
        p.events
            .iter()
            .any(|e| matches!(e, NetEvent::TuningChanged))
    );
    editor.on_server_tuning(p.net.tuning(), 1.0);
    assert!(!editor.is_dirty());
    assert!(
        editor.status.contains("server applied"),
        "{}",
        editor.status
    );

    // The switch itself may cost a correction (inputs predicted with the old values while the
    // new tuning was in flight); after that prediction must stay exact.
    p.run_until(0.5, true, |_| false);
    let settled = p.net.stats().corrections;
    p.run_until(2.0, true, |_| false);
    assert_eq!(
        p.net.stats().corrections,
        settled,
        "prediction in sync with the new tuning"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
