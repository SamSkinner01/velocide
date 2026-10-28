//! Settings persistence and the command line.

use client::cli::{self, Cli, StartMode};
use client::controls::Action;
use client::menus::settings_menu::{ACCESSIBILITY_TAB, SettingsMenu, SettingsResponse};
use client::settings::{Accessibility, CueMode, Settings};
use engine::color_vision::{ColorVision, ColorVisionMode};
use engine::gfx::UiDrawList;
use engine::input::{Binding, InputState, Key};
use engine::ui::{Rect, Theme, Ui, UiInput, UiKey, UiState};
use glam::{Vec2, vec2};

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
fn accessibility_round_trips_in_its_own_section() {
    let path = scratch("accessibility");
    let mut s = Settings::default();
    s.accessibility.color_vision = ColorVision::new(ColorVisionMode::Deuteranopia, 0.65);
    s.accessibility.shape_cues = CueMode::Off;
    s.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("accessibility:"), "{text}");
    assert!(text.contains("Deuteranopia"), "{text}");
    assert_eq!(
        text.matches("color_vision").count(),
        1,
        "not duplicated under retro: {text}"
    );
    let back = Settings::load(&path);
    assert_eq!(back, s);
    assert!(!back.accessibility.shape_cues_on());
    // The renderer gets the correction through `render_settings`, never through `retro`.
    assert_eq!(back.retro.color_vision, ColorVision::default());
    assert_eq!(
        back.render_settings().color_vision,
        s.accessibility.color_vision
    );
    assert_eq!(
        back.render_settings().dithering,
        back.retro.dithering,
        "the rest of retro is kept"
    );
}

#[test]
fn settings_files_from_before_accessibility_still_load() {
    // A file written before the ACCESSIBILITY page existed: no `accessibility` section.
    let old = r#"(
        retro: (internal_height: 240, vertex_snap: true, vertex_snap_strength: 1.0,
            affine_textures: false, dithering: false, color_bits: 5, fog: true,
            vertex_lighting: true, vsync: true, screen_shake: true, screen_flashes: true,
            hitstop: true),
        fov: 100.0,
        player_name: "OLD SHRIKE",
        crosshair: 2,
    )"#;
    let s = Settings::from_ron(old).unwrap();
    assert_eq!(s.fov, 100.0);
    assert_eq!(s.retro.internal_height, 240);
    assert!(!s.retro.dithering);
    assert_eq!(s.accessibility, Accessibility::default());
    assert_eq!(s.accessibility.color_vision.mode, ColorVisionMode::Off);
    assert!(!s.accessibility.shape_cues_on());
}

#[test]
fn check_for_updates_round_trips_and_defaults_on() {
    // On for new players and for files written before the setting existed.
    assert!(Settings::default().check_for_updates);
    let old = Settings::from_ron(r#"(fov: 100.0, player_name: "OLD SHRIKE")"#).unwrap();
    assert!(old.check_for_updates);
    let path = scratch("updates");
    let s = Settings {
        check_for_updates: false,
        ..Default::default()
    };
    s.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("check_for_updates: false"), "{text}");
    let back = Settings::load(&path);
    assert_eq!(back, s);
    assert!(!back.check_for_updates);
}

#[test]
fn accessibility_values_are_repaired_and_cues_follow_the_mode() {
    let s = Settings::from_ron(
        "(accessibility: (color_vision: (mode: Protanopia, strength: 7.5), future: 1))",
    )
    .unwrap();
    assert_eq!(
        s.accessibility.color_vision.mode,
        ColorVisionMode::Protanopia
    );
    assert_eq!(s.accessibility.color_vision.strength, 1.0);
    // AUTO turns the shape cues on with any colour mode.
    assert_eq!(s.accessibility.shape_cues, CueMode::Auto);
    assert!(s.accessibility.shape_cues_on());
    let s = Settings::from_ron("(accessibility: (color_vision: (strength: -1.0)))").unwrap();
    assert_eq!(s.accessibility.color_vision.strength, 0.0);
    let on = Accessibility {
        shape_cues: CueMode::On,
        ..Default::default()
    };
    assert!(on.shape_cues_on());
}

/// One frame of the settings screen with scripted UI keys.
fn settings_frame(
    menu: &mut SettingsMenu,
    state: &mut UiState,
    s: &mut Settings,
    keys: &[UiKey],
) -> SettingsResponse {
    let theme = Theme::default();
    let mut list = UiDrawList::new(vec2(640.0, 360.0));
    let ui_input = UiInput {
        mouse_pos: Vec2::splat(-100.0),
        keys: keys.to_vec(),
        ..Default::default()
    };
    let mut ui = Ui::new(&mut list, state, &ui_input, &theme);
    let rect = Rect::from_center_size(vec2(320.0, 180.0), vec2(460.0, 330.0));
    menu.ui(&mut ui, s, &InputState::new(), rect)
}

#[test]
fn accessibility_tab_changes_colour_vision_live() {
    let mut menu = SettingsMenu::new();
    menu.tab = ACCESSIBILITY_TAB;
    let mut state = UiState::new();
    let mut s = Settings::default();
    settings_frame(&mut menu, &mut state, &mut s, &[]);
    // Focus order: tabs, COLOUR MODE, (FILTER STRENGTH only when a mode is on), SHAPE CUES.
    settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Down]);
    let r = settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Right]);
    assert!(r.changed, "applied live");
    assert_eq!(
        s.accessibility.color_vision.mode,
        ColorVisionMode::Protanopia
    );
    let r = settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Right]);
    assert!(r.changed);
    assert_eq!(
        s.accessibility.color_vision.mode,
        ColorVisionMode::Deuteranopia
    );
    assert!(s.accessibility.shape_cues_on(), "AUTO follows the mode");
    // The strength slider is enabled now.
    settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Down]);
    let r = settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Left, UiKey::Left]);
    assert!(r.changed);
    assert!((s.accessibility.color_vision.strength - 0.9).abs() < 1e-4);
    assert_eq!(
        s.render_settings().color_vision,
        s.accessibility.color_vision
    );
    // SHAPE CUES: AUTO -> ON -> OFF.
    settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Down]);
    settings_frame(&mut menu, &mut state, &mut s, &[UiKey::Right, UiKey::Right]);
    assert_eq!(s.accessibility.shape_cues, CueMode::Off);
    assert!(!s.accessibility.shape_cues_on());
}

#[test]
fn an_unbound_action_stays_unbound() {
    let mut s = Settings::default();
    s.bindings.clear(Action::Knack, false);
    s.bindings.clear(Action::Knack, true);
    let back = Settings::from_ron(&s.to_ron()).unwrap();
    assert!(back.bindings.get(Action::Knack).is_empty());
}

#[test]
fn old_settings_files_keep_their_throw_binding() {
    // The KNACK action used to be called `ThrowThorn` (before the knacks) and `ThrowPrism`
    // (before the thorns); files saved then still load.
    let mut s = Settings::default();
    s.bindings.bind(
        Action::Knack,
        engine::input::Binding::Key(engine::input::Key::KeyG),
    );
    let text = s.to_ron();
    assert!(text.contains("Knack"));
    for old in ["ThrowThorn", "ThrowPrism"] {
        let old_text = text.replace("Knack:", &format!("{old}:"));
        assert!(old_text.contains(old));
        let back = Settings::from_ron(&old_text).unwrap();
        assert!(
            back.bindings
                .get(Action::Knack)
                .contains(&engine::input::Binding::Key(engine::input::Key::KeyG)),
            "{old}"
        );
    }
}

#[test]
fn a_settings_file_naming_retired_abilities_still_loads() {
    use shared::loadout::{Ability, Knack, WeaponKind};
    // Saved before the rework (docs/rework.md): Splinter, Flare, Upswing, Triple Nock, Ember
    // Arrow, Skewer, Flurry, Both Barrels and Mortar are gone.
    let old = r#"(
        fov: 95.0,
        loadout: (
            current: [
                "Quill: Brand, Splinter",
                "Kiln: Flare, Discharge",
                "Tamper: Upswing, Whirl",
                "Longwick: Triple Nock, Ember Arrow",
                "Knack: Snare",
            ],
            presets: [
                (name: "OLD", loadout: [
                    "Lancet: Skewer, Flurry",
                    "Bellows: Gust, Both Barrels",
                    "-",
                    "Cinderbox: Limpet, Mortar",
                ]),
            ],
        ),
    )"#;
    let path = scratch("retired");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, old).unwrap();
    for s in [Settings::from_ron(old).unwrap(), Settings::load(&path)] {
        // The rest of the file survives.
        assert_eq!(s.fov, 95.0);
        let current = &s.loadout.current;
        assert!(current.is_valid(), "{current:?}");
        assert_eq!(current.knack, Knack::Snare);
        // Each unknown name became that weapon's default for the slot (then repaired).
        let abilities = |slot: u8| current.slot(slot).map(|b| b.abilities);
        assert_eq!(abilities(0), Some([Ability::Brand, Ability::Wickline]));
        assert_eq!(abilities(1), Some([Ability::Lantern, Ability::Discharge]));
        assert_eq!(abilities(2), Some([Ability::Whirl, Ability::Riposte]));
        assert_eq!(
            abilities(3),
            Some(WeaponKind::Longwick.default_build().abilities)
        );
        let preset = &s.loadout.presets[0];
        assert_eq!(preset.name, "OLD");
        assert!(preset.loadout.is_valid());
        let abilities = |slot: u8| preset.loadout.slot(slot).map(|b| b.abilities);
        assert_eq!(
            abilities(0),
            Some(WeaponKind::Lancet.default_build().abilities)
        );
        assert_eq!(abilities(1), Some([Ability::Gust, Ability::Bulwark]));
        assert_eq!(abilities(2), None);
        assert_eq!(abilities(3), Some([Ability::Limpet, Ability::Kite]));
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[test]
fn loadout_and_presets_round_trip_and_garbage_falls_back() {
    use client::settings::{LoadoutPreset, MAX_PRESETS};
    use shared::loadout::{Ability, Knack, Loadout, WeaponBuild, WeaponKind};
    let path = scratch("loadout");
    let mut s = Settings::default();
    s.loadout.current = Loadout {
        slots: [
            Some(WeaponBuild {
                weapon: WeaponKind::Longwick,
                abilities: [Ability::BeaconArrow, Ability::Brand],
            }),
            None,
            Some(WeaponKind::Kiln.default_build()),
            None,
        ],
        knack: Knack::SootPot,
    };
    s.loadout.presets.push(LoadoutPreset {
        name: "BOWS".into(),
        loadout: s.loadout.current,
    });
    s.save(&path).unwrap();
    let back = Settings::load(&path);
    assert_eq!(back.loadout, s.loadout);
    assert_eq!(back, s);
    assert!(s.to_ron().contains("Knack: Soot Pot"));
    // Files from before knacks (four slot entries only) load with the Thorn.
    let old = s
        .to_ron()
        .replace("\"Knack: Soot Pot\",", "")
        .replace(", \"Knack: Soot Pot\"", "")
        .replace(", \"Knack: Thorn\"", "");
    // (The KNACK binding is `Knack: [..]` in the bindings; the loadout's are strings.)
    assert!(!old.contains("\"Knack:"), "{old}");
    let old = Settings::from_ron(&old).unwrap();
    assert_eq!(
        old.loadout.current,
        s.loadout.current.with_knack(Knack::Thorn)
    );

    // Loadouts are text: an unknown weapon empties its slot, an unknown ability falls back to
    // the weapon's default, and everything else survives.
    let text = s
        .to_ron()
        .replace("Longwick", "Railgun")
        .replace("Kiln: Lantern", "kiln: SPARKLER");
    assert!(text.contains("Railgun") && text.contains("Beacon Arrow"));
    let broken = Settings::from_ron(&text).unwrap();
    assert_eq!(broken.loadout.current.slot(0), None);
    assert_eq!(
        broken.loadout.current.slot(2),
        Some(&WeaponKind::Kiln.default_build())
    );
    assert_eq!(broken.loadout.presets.len(), s.loadout.presets.len());
    assert_eq!(broken.fov, s.fov);
    assert_eq!(broken.bindings, s.bindings);
    // A loadout with no weapon left gets the default weapons (and keeps its knack).
    let text = s
        .to_ron()
        .replace("Kiln:", "Nothing:")
        .replace("Longwick:", "Nothing:");
    let empty = Settings::from_ron(&text).unwrap();
    assert_eq!(
        empty.loadout.current,
        Loadout::default().with_knack(Knack::SootPot)
    );

    // Invalid builds and loadouts are repaired, too many presets dropped, names cleaned.
    let mut s = Settings::default();
    s.loadout.current = Loadout {
        slots: [
            Some(WeaponBuild {
                weapon: WeaponKind::Lancet,
                abilities: [Ability::Gust, Ability::Gust],
            }),
            Some(WeaponKind::Lancet.default_build()),
            None,
            None,
        ],
        ..Default::default()
    };
    s.loadout.presets = vec![
        LoadoutPreset {
            name: "   ".into(),
            loadout: Loadout::unarmed(),
        };
        MAX_PRESETS + 3
    ];
    let back = Settings::from_ron(&s.to_ron()).unwrap();
    assert!(back.loadout.current.is_valid());
    assert_eq!(
        back.loadout.current.slot(1),
        None,
        "the repeated Lancet is dropped"
    );
    assert_eq!(back.loadout.presets.len(), MAX_PRESETS);
    assert!(back.loadout.presets.iter().all(|p| p.loadout.is_valid()));
    assert_eq!(back.loadout.presets[2].name, "PRESET 3");
    // Files from before loadouts get the default loadout and presets.
    let old = Settings::from_ron("(fov: 100.0)").unwrap();
    assert_eq!(old.loadout, client::settings::LoadoutSettings::default());
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
