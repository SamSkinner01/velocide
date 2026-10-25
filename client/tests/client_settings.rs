//! Settings persistence and the command line.

use client::cli::{self, Cli, StartMode};
use client::controls::Action;
use client::settings::Settings;
use engine::input::{Binding, Key};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "velocide-client-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("velocide").join("settings.ron")
}

#[test]
fn missing_file_gives_defaults() {
    let path = scratch("missing");
    assert_eq!(Settings::load(&path), Settings::default());
}

#[test]
fn invalid_file_gives_defaults() {
    let path = scratch("invalid");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "(fov: \"wide\", this is not ron").unwrap();
    assert_eq!(Settings::load(&path), Settings::default());
    std::fs::write(&path, [0xff, 0xfe, 0x00, 0x13]).unwrap();
    assert_eq!(Settings::load(&path), Settings::default());
}

#[test]
fn save_and_load_round_trip() {
    let path = scratch("roundtrip");
    let mut s = Settings {
        fov: 118.0,
        mouse_sensitivity: 1.75,
        invert_y: true,
        player_name: "Gravel Shrike".into(),
        last_join_address: "10.0.0.7:47420".into(),
        ..Default::default()
    };
    s.retro.dithering = false;
    s.retro.internal_height = 240;
    s.bindings.bind(Action::Dash, Binding::Key(Key::KeyV));
    s.save(&path).unwrap();
    let back = Settings::load(&path);
    assert_eq!(back, s);
    assert_eq!(back.bindings.get(Action::Dash)[0], Binding::Key(Key::KeyV));
}

#[test]
fn partial_and_hostile_files_are_repaired() {
    // Unknown fields are ignored, missing ones default, absurd values are clamped.
    let s = Settings::from_ron(
        "(fov: 999.0, mouse_sensitivity: -3.0, master_volume: 7.0, future_option: true)",
    )
    .unwrap();
    assert_eq!(s.fov, 140.0);
    assert!(s.mouse_sensitivity > 0.0);
    assert_eq!(s.master_volume, 1.0);
    assert_eq!(s.music_volume, Settings::default().music_volume);
    // Bindings missing from the file get their defaults.
    assert_eq!(
        s.bindings.get(Action::Jump),
        Settings::default().bindings.get(Action::Jump)
    );
}

#[test]
fn an_unbound_action_stays_unbound() {
    let mut s = Settings::default();
    s.bindings.clear(Action::ThrowPrism, false);
    s.bindings.clear(Action::ThrowPrism, true);
    let back = Settings::from_ron(&s.to_ron()).unwrap();
    assert!(back.bindings.get(Action::ThrowPrism).is_empty());
}

#[test]
fn fov_is_horizontal_at_16_9() {
    let s = Settings {
        fov: 90.0,
        ..Default::default()
    };
    let v = s.vertical_fov(0.0);
    assert!(
        (v - 58.7).abs() < 0.2,
        "90 horizontal at 16:9 = {v} vertical"
    );
    assert!(s.vertical_fov(10.0) > v);
}

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn run(s: &str) -> client::cli::LaunchOptions {
    match cli::parse(args(s)).unwrap() {
        Cli::Run(o) => *o,
        Cli::Help => panic!("help"),
    }
}

#[test]
fn cli_modes() {
    assert_eq!(run("").start, StartMode::Menu);
    assert_eq!(run("--sp").start, StartMode::SinglePlayer { level: None });
    assert_eq!(
        run("--sp movement_test --bot").start,
        StartMode::SinglePlayer {
            level: Some("movement_test".into())
        }
    );
    assert_eq!(
        run("--level proving_ground").start,
        StartMode::SinglePlayer {
            level: Some("proving_ground".into())
        }
    );
    assert_eq!(
        run("--host --level arena").start,
        StartMode::Host {
            port: None,
            level: Some("arena".into())
        }
    );
    assert_eq!(
        run("--host 5000").start,
        StartMode::Host {
            port: Some(5000),
            level: None
        }
    );
    assert_eq!(
        run("--connect 127.0.0.1:47420").start,
        StartMode::Join {
            address: "127.0.0.1:47420".into()
        }
    );
    assert_eq!(cli::parse(args("--help")).unwrap(), Cli::Help);
}

#[test]
fn cli_options() {
    let o = run(
        "--connect 1.2.3.4:5 --name P2 --netsim 100,20,2 --windowed 960x540 --bot --capture a.png \
         --capture-after 4 --quit-after 6 --no-audio --debug --max-players 6",
    );
    assert_eq!(o.name.as_deref(), Some("P2"));
    let n = o.netsim.unwrap();
    assert_eq!(
        (n.latency_ms, n.jitter_ms, n.loss_percent),
        (100.0, 20.0, 2.0)
    );
    assert_eq!(o.window_size, Some((960, 540)));
    assert_eq!(o.fullscreen, Some(false));
    assert!(o.bot && o.no_audio && o.debug_overlay);
    assert_eq!(o.capture.as_deref(), Some(std::path::Path::new("a.png")));
    assert_eq!(o.capture_after, 4.0);
    assert_eq!(o.quit_after, Some(6.0));
    assert_eq!(o.max_players, Some(6));
    assert_eq!(run("--fullscreen").fullscreen, Some(true));
}

#[test]
fn cli_errors() {
    for bad in [
        "--bogus",
        "--connect",
        "--windowed 12",
        "--windowed 0x0",
        "--netsim fast",
        "--quit-after -1",
        "--host notaport",
        "--connect 1.2.3.4:5 --level x",
    ] {
        assert!(cli::parse(args(bad)).is_err(), "{bad} should be rejected");
    }
}
