//! Renders the UI toolkit to PNGs on the CPU (no GPU / window needed): the settings menu
//! (every widget type, one image per tab), the main menu, a lobby / join screen, the in-game
//! HUD and a font specimen.
//!
//! ```text
//! cargo run -p engine --example ui_demo -- [out_dir]   # default: target/ui_demo
//! ```

#[cfg(feature = "client")]
fn main() -> anyhow::Result<()> {
    demo::run()
}

#[cfg(not(feature = "client"))]
fn main() {
    eprintln!("ui_demo needs the engine `client` feature");
}

#[cfg(feature = "client")]
mod demo {
    use std::path::{Path, PathBuf};

    use engine::gfx::{TextureData, UiDrawList};
    use engine::glam::{Vec2, Vec4, vec2};
    use engine::ui::draw::{self, fade};
    use engine::ui::hud::{self, ChatLine, CrosshairStyle, TextFeed};
    use engine::ui::{
        Align, Rect, TextFilter, TextStyle, Theme, Ui, UiDrawExt, UiInput, UiKey, UiState, font,
        raster, text,
    };

    const W: u32 = 1280;
    const H: u32 = 720;
    /// A time at which blinking elements are in their "on" phase.
    const T: f64 = 10.0;

    pub fn run() -> anyhow::Result<()> {
        let out = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/ui_demo"));
        std::fs::create_dir_all(&out)?;
        let atlas = font::build_atlas();
        let theme = Theme::default();
        let canvas = UiDrawList::canvas_for_window(W, H);

        let mut shots: Vec<(&str, UiDrawList, Scene)> = Vec::new();
        for (i, name) in [
            "settings_video",
            "settings_audio",
            "settings_controls",
            "settings_gameplay",
        ]
        .into_iter()
        .enumerate()
        {
            shots.push((name, settings_screen(&theme, canvas, i), Scene::Dim));
        }
        shots.push(("main_menu", main_menu(&theme, canvas), Scene::Dim));
        shots.push(("lobby", lobby(&theme, canvas), Scene::Dim));
        shots.push(("hud", hud_screen(&theme, canvas), Scene::Bright));
        shots.push(("font_specimen", font_specimen(&theme, canvas), Scene::Flat));
        shots.push(("font_zoom", font_zoom(&theme, canvas), Scene::Flat));

        for (name, list, scene) in &shots {
            let mut img = background(*scene);
            raster::rasterize_onto(list, &atlas, &mut img);
            let path = out.join(format!("{name}.png"));
            save(&path, &img)?;
            println!("wrote {} ({} quads)", path.display(), list.quads.len());
        }
        Ok(())
    }

    fn save(path: &Path, img: &TextureData) -> anyhow::Result<()> {
        image::save_buffer(
            path,
            &img.rgba,
            img.width,
            img.height,
            image::ColorType::Rgba8,
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------ frame driver

    fn input(keys: &[UiKey]) -> UiInput {
        UiInput {
            keys: keys.to_vec(),
            mouse_pos: vec2(-100.0, -100.0),
            ..Default::default()
        }
    }

    /// Run the UI for each scripted input and return the last frame's draw list.
    fn run_frames(
        theme: &Theme,
        canvas: Vec2,
        state: &mut UiState,
        inputs: &[UiInput],
        mut build: impl FnMut(&mut Ui),
    ) -> UiDrawList {
        let mut list = UiDrawList::new(canvas);
        let n = inputs.len();
        for (i, inp) in inputs.iter().enumerate() {
            let mut inp = inp.clone();
            inp.time = T - (n - 1 - i) as f64 / 60.0;
            list.clear();
            let mut ui = Ui::new(&mut list, state, &inp, theme);
            build(&mut ui);
        }
        list
    }

    // ------------------------------------------------------------------ settings

    struct Settings {
        res_scale: usize,
        vertex_snap: bool,
        snap_strength: f32,
        affine: bool,
        dithering: bool,
        color_bits: f32,
        fog: bool,
        vertex_lighting: bool,
        vsync: bool,
        screen_shake: bool,
        flashes: bool,
        hitstop: bool,
        master: f32,
        sfx: f32,
        music: f32,
        music_intensity: usize,
        mute_unfocused: bool,
        sensitivity: f32,
        invert_y: bool,
        name: String,
        fov: f32,
        crosshair: usize,
        style_feed: bool,
        show_ping: bool,
        damage_flash: f32,
    }

    impl Default for Settings {
        fn default() -> Self {
            Self {
                res_scale: 2,
                vertex_snap: true,
                snap_strength: 1.0,
                affine: false,
                dithering: true,
                color_bits: 5.0,
                fog: true,
                vertex_lighting: true,
                vsync: true,
                screen_shake: true,
                flashes: true,
                hitstop: true,
                master: 0.8,
                sfx: 1.0,
                music: 0.65,
                music_intensity: 1,
                mute_unfocused: false,
                sensitivity: 2.35,
                invert_y: false,
                name: "GRAVEL_SHRIKE".into(),
                fov: 105.0,
                crosshair: 2,
                style_feed: true,
                show_ping: true,
                damage_flash: 0.7,
            }
        }
    }

    const BINDS: [(&str, &str); 16] = [
        ("MOVE FORWARD", "W"),
        ("MOVE BACK", "S"),
        ("STRAFE LEFT", "A"),
        ("STRAFE RIGHT", "D"),
        ("JUMP", "SPACE"),
        ("DASH", "L-SHIFT"),
        ("SLIDE / SLAM", "L-CTRL"),
        ("FIRE", "MOUSE 1"),
        ("ALT FIRE", "MOUSE 2"),
        ("PUNCH / PARRY", "F"),
        ("THROW PRISM", "Q"),
        ("LANCET", "1"),
        ("THRESHER", "2"),
        ("RIVETER", "3"),
        ("CINDERBOX", "4"),
        ("CHAT", ""),
    ];

    fn settings_ui(ui: &mut Ui, s: &mut Settings, tab: &mut usize) {
        let canvas = ui.canvas();
        let rect = Rect::from_center_size(canvas.center(), vec2(452.0, 326.0));
        ui.panel(rect, "SETTINGS", |ui| {
            ui.tabs(
                "tabs",
                tab,
                &["VIDEO/RETRO", "AUDIO", "CONTROLS", "GAMEPLAY"],
            );
            ui.spacing(2.0);
            let list_h = ui.available_rect().height() - 22.0;
            ui.scroll_area("page", list_h, |ui| match *tab {
                0 => {
                    ui.heading("DISPLAY");
                    ui.choice(
                        "RESOLUTION SCALE",
                        &mut s.res_scale,
                        &["NATIVE", "720P", "360P", "240P"],
                    );
                    ui.toggle("VSYNC", &mut s.vsync);
                    ui.heading("RETRO EFFECTS");
                    ui.toggle("VERTEX SNAPPING", &mut s.vertex_snap);
                    ui.enabled_scope(s.vertex_snap, |ui| {
                        ui.slider("SNAP STRENGTH", &mut s.snap_strength, 0.5, 4.0, 0.25, "{}x");
                    });
                    ui.toggle("AFFINE TEXTURES", &mut s.affine);
                    ui.toggle("DITHERING", &mut s.dithering);
                    ui.slider("COLOUR DEPTH", &mut s.color_bits, 3.0, 8.0, 1.0, "{} BIT");
                    ui.toggle("DISTANCE FOG", &mut s.fog);
                    ui.toggle("VERTEX LIGHTING", &mut s.vertex_lighting);
                    ui.heading("FEEDBACK");
                    ui.toggle("SCREEN SHAKE", &mut s.screen_shake);
                    ui.toggle("DAMAGE FLASHES", &mut s.flashes);
                    ui.toggle("HITSTOP", &mut s.hitstop);
                }
                1 => {
                    ui.heading("VOLUME");
                    ui.slider("MASTER", &mut s.master, 0.0, 1.0, 0.05, "{%}%");
                    ui.slider("EFFECTS", &mut s.sfx, 0.0, 1.0, 0.05, "{%}%");
                    ui.slider("MUSIC", &mut s.music, 0.0, 1.0, 0.05, "{%}%");
                    ui.heading("MUSIC");
                    ui.choice(
                        "COMBAT LAYERS",
                        &mut s.music_intensity,
                        &["CALM", "DYNAMIC", "ALWAYS LOUD"],
                    );
                    ui.toggle("MUTE WHEN UNFOCUSED", &mut s.mute_unfocused);
                    ui.separator();
                    ui.label_wrapped(
                        "Music stems swell as the fight heats up and fade between \
                         encounters. Rank-ups, parries and ricochets always cut through.",
                    );
                }
                2 => {
                    ui.heading("MOUSE");
                    ui.slider("SENSITIVITY", &mut s.sensitivity, 0.1, 10.0, 0.05, "{}");
                    ui.toggle("INVERT LOOK", &mut s.invert_y);
                    ui.heading("BINDINGS");
                    for (label, key) in BINDS {
                        let r = ui.keybind(label, key);
                        if r.capturing {
                            // The game would read its InputState here and call finish_capture.
                        }
                    }
                }
                _ => {
                    ui.heading("IDENTITY");
                    ui.text_field("PLAYER NAME", &mut s.name, 16, TextFilter::Name);
                    ui.heading("VIEW");
                    ui.slider("FIELD OF VIEW", &mut s.fov, 60.0, 130.0, 1.0, "{}");
                    ui.choice(
                        "CROSSHAIR",
                        &mut s.crosshair,
                        &["CROSS", "DOT", "CROSS + DOT", "NONE"],
                    );
                    ui.slider("DAMAGE FLASH", &mut s.damage_flash, 0.0, 1.0, 0.1, "{%}%");
                    ui.heading("HUD");
                    ui.toggle("STYLE FEED", &mut s.style_feed);
                    ui.toggle("PING DISPLAY", &mut s.show_ping);
                    ui.enabled_scope(false, |ui| {
                        ui.value_row("NETCODE BUILD", "PROTO 7");
                    });
                }
            });
            ui.row(3, |ui| {
                ui.button("APPLY");
                ui.button("DEFAULTS");
                ui.button("BACK");
            });
        });
    }

    fn settings_screen(theme: &Theme, canvas: Vec2, tab_index: usize) -> UiDrawList {
        let mut state = UiState::new();
        let mut s = Settings::default();
        let mut tab = tab_index;
        // Focus: tab strip is first; then the page's widgets.
        let mut inputs = vec![input(&[])];
        match tab_index {
            0 => inputs.push(input(&[UiKey::Down; 7])), // DITHERING
            1 => {
                inputs.push(input(&[UiKey::Down; 4])); // MUSIC
                inputs.push(input(&[UiKey::Left]));
            }
            2 => {
                inputs.push(input(&[UiKey::Down; 8])); // DASH
                inputs.push(input(&[UiKey::Enter]));
                inputs.push(input(&[]));
            }
            _ => inputs.push(input(&[UiKey::Down; 2])), // PLAYER NAME
        }
        // Hover a button to show the hover state.
        let mut last = input(&[]);
        if tab_index == 3 {
            last.text = "_X".into();
        }
        let hover = if tab_index == 0 {
            vec2(canvas.x * 0.5 + 140.0, canvas.y * 0.5 + 140.0)
        } else {
            vec2(-50.0, -50.0)
        };
        last.mouse_pos = hover;
        inputs.push(last);
        run_frames(theme, canvas, &mut state, &inputs, |ui| {
            settings_ui(ui, &mut s, &mut tab)
        })
    }

    // ------------------------------------------------------------------ main menu

    fn main_menu(theme: &Theme, canvas: Vec2) -> UiDrawList {
        let mut state = UiState::new();
        let inputs = [input(&[]), input(&[UiKey::Down]), input(&[])];
        run_frames(theme, canvas, &mut state, &inputs, |ui| {
            let t = ui.theme();
            let c = ui.canvas();
            let list = ui.painter();
            // Title block.
            let title_pos = vec2(40.0, 44.0);
            // Solid crimson extrusion (stacked copies), then the outlined title on top.
            for k in (1..=4).rev() {
                let o = k as f32 * 3.0;
                let shade = draw::mix(t.crimson, t.crimson_dark, k as f32 / 4.0);
                list.text(
                    title_pos + vec2(o, o),
                    "VELOCIDE",
                    &TextStyle::new(shade).scale(6.0),
                );
            }
            let style = TextStyle::new(t.bone).scale(6.0).outline(fade(t.ink, 0.9));
            let r = list.text(title_pos, "VELOCIDE", &style);
            let y = r.max.y + 6.0;
            draw::slanted(
                list,
                Rect::new(vec2(r.min.x, y), vec2(r.max.x + 40.0, y + 4.0)),
                true,
                t.crimson,
            );
            draw::hline(
                list,
                r.min.x,
                r.max.x + 120.0,
                y + 6.0,
                fade(t.crimson_bright, 0.6),
            );
            let tag = TextStyle::new(t.bone_dim).scale(1.0).shadow(t.shadow);
            list.text(vec2(r.min.x, y + 12.0), "WAX. WICK. VELOCITY.", &tag);

            ui.area(Rect::new(vec2(40.0, 140.0), vec2(300.0, 340.0)), |ui| {
                ui.default_focus();
                for item in [
                    "CAMPAIGN",
                    "CO-OP",
                    "ARENA",
                    "HOST GAME",
                    "JOIN GAME",
                    "SETTINGS",
                    "QUIT",
                ] {
                    ui.menu_item(item);
                }
            });
            let list = ui.painter();
            let hint = TextStyle::new(t.ash).shadow(t.shadow);
            list.text(
                vec2(40.0, c.max.y - 18.0),
                "UP/DOWN SELECT   ENTER CONFIRM   ESC BACK",
                &hint,
            );
            list.text(
                vec2(c.max.x - 16.0, c.max.y - 18.0),
                "v0.1.0 // M14 BUILD",
                &hint.right(),
            );
        })
    }

    // ------------------------------------------------------------------ lobby

    fn lobby(theme: &Theme, canvas: Vec2) -> UiDrawList {
        let mut state = UiState::new();
        let mut address = String::from("192.168.1.20");
        let mut port = String::from("27015");
        let mut chat = String::from("ready when you are");
        let mut mode = 1usize;
        let mut level = 0usize;
        let mut ready = true;
        let mut selected = 2usize;
        let players = [
            ("GRAVEL_SHRIKE", 12, true, true),
            ("Mx. Ossuary", 48, true, false),
            ("rivet_witch", 63, false, false),
            ("HOLLOWPOINT", 95, true, false),
            ("tin saint", 131, false, false),
            ("Waxwing", 22, true, false),
        ];
        let chat_log = [
            ("", "GRAVEL_SHRIKE opened the lobby."),
            (
                "Mx. Ossuary",
                "arena or campaign? I want to try the new gaoler room",
            ),
            ("rivet_witch", "arena. loser buys the prisms"),
            ("HOLLOWPOINT", "gl hf"),
            ("", "tin saint joined (131 ms)."),
            ("tin saint", "lag is fine, go go go"),
        ];
        let inputs = [input(&[]), input(&[UiKey::Down; 2]), input(&[])];
        run_frames(theme, canvas, &mut state, &inputs, |ui| {
            let t = ui.theme();
            ui.panel_auto(vec2(16.0, 16.0), 284.0, "JOIN BY ADDRESS", |ui| {
                ui.text_field("HOST / IP", &mut address, 40, TextFilter::Address);
                ui.text_field("PORT", &mut port, 5, TextFilter::Digits);
                ui.spacing(2.0);
                ui.row(2, |ui| {
                    ui.button("CONNECT");
                    ui.button("HOST GAME");
                });
            });
            ui.panel(
                Rect::new(vec2(16.0, 118.0), vec2(300.0, 344.0)),
                "CHAT",
                |ui| {
                    ui.scroll_area_sticky("log", 170.0, |ui| {
                        for (name, msg) in chat_log {
                            let color = if name.is_empty() {
                                t.ash
                            } else if name == "rivet_witch" {
                                t.stamina
                            } else {
                                t.crimson_bright
                            };
                            ui.chat_line(name, color, msg);
                        }
                    });
                    ui.row_weights(&[3.0, 1.0], |ui| {
                        ui.text_field("##chat", &mut chat, 120, TextFilter::Any);
                        ui.button("SEND");
                    });
                },
            );
            let right = Rect::new(vec2(328.0, 16.0), vec2(624.0, 344.0));
            ui.panel(right, "LOBBY // HOST: GRAVEL_SHRIKE", |ui| {
                ui.choice("MODE", &mut mode, &["CO-OP CAMPAIGN", "DEATHMATCH"]);
                ui.choice("ARENA", &mut level, &["THE KILN", "OSSUARY ROW", "SPINDLE"]);
                ui.heading("PLAYERS  6/8");
                ui.row_weights(&[5.0, 2.0, 2.0], |ui| {
                    let head = TextStyle::new(t.ash);
                    ui.text("NAME", &head);
                    ui.text("PING", &head.right());
                    ui.text("STATUS", &head.right());
                });
                ui.scroll_area("players", 110.0, |ui| {
                    for (i, (name, ping, is_ready, host)) in players.iter().enumerate() {
                        ui.with_id(i, |ui| {
                            let row_top = ui.available_rect().min;
                            let label = if *host {
                                format!("{name} *")
                            } else {
                                name.to_string()
                            };
                            if ui.selectable(&label, selected == i) {
                                selected = i;
                            }
                            // Overlay ping + status columns on the selectable row.
                            let w = ui.available_rect().width();
                            let rect = Rect::from_min_size(row_top, vec2(w, t.row_height));
                            let list = ui.painter();
                            let ping_x = rect.min.x + w * 7.0 / 9.0 - 4.0;
                            let pc = hud::ping_color(t, *ping as f32);
                            list.text_in(
                                Rect::new(rect.min, vec2(ping_x, rect.max.y)),
                                &format!("{ping}"),
                                &TextStyle::new(pc).right(),
                            );
                            let (st, sc) = if *is_ready {
                                ("READY", t.bone)
                            } else {
                                ("...", t.ash)
                            };
                            list.text_in(
                                Rect::new(rect.min, vec2(rect.max.x - 4.0, rect.max.y)),
                                st,
                                &TextStyle::new(sc).right(),
                            );
                        });
                    }
                });
                ui.toggle("READY", &mut ready);
                ui.separator();
                ui.row(2, |ui| {
                    ui.button("START");
                    ui.button("LEAVE");
                });
            });
        })
    }

    // ------------------------------------------------------------------ HUD

    fn hud_screen(theme: &Theme, canvas: Vec2) -> UiDrawList {
        let t = theme;
        let mut list = UiDrawList::new(canvas);
        let c = Rect::from_min_size(Vec2::ZERO, canvas);
        let now = T;

        // Top centre: level timer. Top right: ping + kill feed.
        let timer_style = TextStyle::new(t.bone)
            .scale(2.0)
            .centered()
            .outline(fade(t.ink, 0.9));
        list.text(
            vec2(c.center().x, 8.0),
            &hud::format_timer(143.27),
            &timer_style,
        );
        hud::readout(
            &mut list,
            t,
            vec2(c.center().x, 26.0),
            Align::Center,
            "WAVE",
            "3/5",
            t.crimson_bright,
        );
        hud::readout(
            &mut list,
            t,
            vec2(c.max.x - 10.0, 8.0),
            Align::Right,
            "PING",
            "42",
            hud::ping_color(t, 42.0),
        );
        hud::readout(
            &mut list,
            t,
            vec2(c.max.x - 10.0, 19.0),
            Align::Right,
            "FPS",
            "144",
            t.bone_dim,
        );
        let mut kills = TextFeed::new(4, 4.0);
        kills.push("rivet_witch  >  GNAWER", t.bone, now - 3.0);
        kills.push("GRAVEL_SHRIKE  >  GAOLER", t.crimson_bright, now - 1.5);
        kills.push("HOLLOWPOINT  >  HARRIER", t.bone, now - 0.2);
        kills.newest_on_top = false;
        kills.draw(
            &mut list,
            t,
            vec2(c.max.x - 10.0, 72.0),
            Align::Right,
            1.0,
            now,
        );

        // Right side: style meter + feed.
        let meter = Rect::new(vec2(c.max.x - 170.0, 96.0), vec2(c.max.x - 10.0, 130.0));
        hud::style_meter(&mut list, t, meter, 4, "MERCILESS", 0.62, now);
        let mut feed = TextFeed::new(6, 2.5);
        for (txt, color, age) in [
            ("- REPEAT", t.ash, 2.9),
            ("+ RICOCHET", t.bone_dim, 2.4),
            ("+ AIRSHOT", t.crimson_bright, 1.6),
            ("+ PARRY", t.bone_dim, 1.0),
            ("+ PARRY", t.bone_dim, 0.9),
            ("+ FRESH WEAPON", t.bone_dim, 0.4),
            ("+ SLAMSTORM", t.bone_dim, 0.15),
        ] {
            feed.push(txt, color, now - age);
        }
        feed.draw(
            &mut list,
            t,
            vec2(c.max.x - 10.0, 142.0),
            Align::Right,
            1.0,
            now,
        );

        // Rank-up banner mid-pulse.
        hud::banner(
            &mut list,
            t,
            vec2(c.center().x, 92.0),
            "MERCILESS",
            0.12,
            1.6,
            t.rank_color(4),
        );

        // Crosshair.
        hud::crosshair(&mut list, c.center(), &CrosshairStyle::new(t));

        // Bottom left: chat, health, stamina.
        let chat = [
            ChatLine {
                name: String::new(),
                name_color: t.ash,
                text: "Checkpoint reached.".into(),
                time: now - 9.6,
            },
            ChatLine {
                name: "rivet_witch".into(),
                name_color: t.stamina,
                text: "prism up, shoot it!".into(),
                time: now - 4.0,
            },
            ChatLine {
                name: "Mx. Ossuary".into(),
                name_color: t.crimson_bright,
                text: "gaoler on the left, parry the bash when the door glows".into(),
                time: now - 1.0,
            },
        ];
        hud::chat_box(
            &mut list,
            t,
            Rect::new(vec2(10.0, 200.0), vec2(230.0, 290.0)),
            &chat,
            now,
            false,
            8.0,
        );
        let health_rect = Rect::new(vec2(14.0, c.max.y - 48.0), vec2(200.0, c.max.y - 22.0));
        hud::health(&mut list, t, health_rect, "WICK", 64.0, 100.0, 22.0, now);
        let pip = vec2(14.0, 5.0);
        hud::pips(&mut list, t, vec2(14.0, c.max.y - 15.0), pip, 4.0, 2.4, 3);
        let stam = TextStyle::new(t.bone_dim).outline(fade(t.ink, 0.9));
        let label_x = 14.0 + hud::pips_width(pip, 4.0, 3) + 6.0;
        list.text(vec2(label_x, c.max.y - 16.0), "DASH", &stam);

        // Bottom right: weapon.
        hud::weapon(
            &mut list,
            t,
            vec2(c.max.x - 12.0, c.max.y - 12.0),
            2,
            "THRESHER",
            &["SPREAD", "SLUG", "FLAK"],
            1,
        );
        list
    }

    // ------------------------------------------------------------------ font

    fn ascii_rows(per_row: usize) -> Vec<String> {
        let all: Vec<char> = (32u8..=126).map(char::from).collect();
        all.chunks(per_row).map(|c| c.iter().collect()).collect()
    }

    fn font_specimen(theme: &Theme, canvas: Vec2) -> UiDrawList {
        let t = theme;
        let mut list = UiDrawList::new(canvas);
        let mut y = 10.0;
        let head = TextStyle::new(t.crimson_bright);
        let body = TextStyle::new(t.bone);
        list.text(
            vec2(10.0, y),
            "SHRIKE 5x7 // ALL PRINTABLE ASCII AT SCALE 1, 2, 3",
            &head,
        );
        y += 14.0;
        for row in ascii_rows(48) {
            list.text(vec2(10.0, y), &row, &body);
            y += 11.0;
        }
        y += 3.0;
        for row in ascii_rows(32) {
            list.text(vec2(10.0, y), &row, &body.scale(2.0));
            y += 22.0;
        }
        y += 2.0;
        for row in ascii_rows(23) {
            list.text(vec2(10.0, y), &row, &body.scale(3.0));
            y += 32.0;
        }
        y += 2.0;
        list.text(
            vec2(10.0, y),
            "THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG. 0123456789",
            &body,
        );
        y += 11.0;
        list.text(
            vec2(10.0, y),
            "the quick brown fox jumps over the lazy dog! (gjpqy) {[<>]}",
            &body,
        );
        y += 11.0;
        list.text(
            vec2(10.0, y),
            "Il1| O0o 5S 2Z 8B rn/m vv/w ,.;: \"'` +-=_ ~^ #$%&*@ \\/ unknown:\u{e9}\u{263a}",
            &body,
        );
        y += 15.0;
        // Effects and alignment.
        let x = 10.0;
        list.text(
            vec2(x, y),
            "SHADOW",
            &TextStyle::new(t.bone).scale(2.0).shadow(t.crimson),
        );
        list.text(
            vec2(x + 90.0, y),
            "OUTLINE",
            &TextStyle::new(t.bone).scale(2.0).outline(t.ink),
        );
        list.text(
            vec2(x + 186.0, y),
            "BOTH",
            &TextStyle::new(t.crimson_bright)
                .scale(2.0)
                .outline(t.ink)
                .shadow(t.crimson_dark),
        );
        list.text(
            vec2(x + 250.0, y),
            "+ PARRY",
            &TextStyle::new(t.bone).outline(t.ink),
        );
        list.text(
            vec2(x + 250.0, y + 10.0),
            "+ AIRSHOT",
            &TextStyle::new(t.crimson_bright).outline(t.ink),
        );
        // Wrapping + alignment boxes on the right.
        let bx = 446.0;
        let box_w = 184.0;
        let by = 10.0;
        let para = "Word wrap: Shrikes are masked hunters whose life is a wick burning in a chest lantern. Momentum is mercy; standing still is a slow death. Supercalifragilisticexpialidocious-breaks-too.";
        list.fill(
            Rect::new(vec2(bx - 3.0, by - 3.0), vec2(bx + box_w + 3.0, by + 74.0)),
            fade(t.crimson_dark, 0.6),
        );
        list.text_wrapped(vec2(bx, by), para, box_w, &TextStyle::new(t.bone_dim));
        let ay = by + 82.0;
        list.fill(
            Rect::new(vec2(bx - 3.0, ay - 3.0), vec2(bx + box_w + 3.0, ay + 34.0)),
            fade(t.crimson_dark, 0.6),
        );
        draw::hline(
            &mut list,
            bx + box_w * 0.5,
            bx + box_w * 0.5 + 1.0,
            ay - 3.0,
            t.crimson_bright,
        );
        list.text(vec2(bx, ay), "LEFT", &TextStyle::new(t.bone));
        list.text(
            vec2(bx + box_w * 0.5, ay + 10.0),
            "CENTER\nmulti-line",
            &TextStyle::new(t.bone).centered(),
        );
        list.text(
            vec2(bx + box_w, ay),
            "RIGHT",
            &TextStyle::new(t.bone).right(),
        );
        let _ = text::max_chars(box_w, 1.0);
        list
    }

    /// Every glyph at scale 4 on a grid of cells, to check each character's pixels.
    fn font_zoom(theme: &Theme, canvas: Vec2) -> UiDrawList {
        let t = theme;
        let mut list = UiDrawList::new(canvas);
        let scale = 3.0;
        let cell = vec2(font::GLYPH_W * scale + 8.0, font::GLYPH_H * scale + 12.0);
        let per_row = ((canvas.x - 20.0) / cell.x) as usize;
        for (i, code) in (32u8..=126).chain([0xe9]).enumerate() {
            let c = char::from(code);
            let (col, row) = (i % per_row, i / per_row);
            let p = vec2(10.0 + col as f32 * cell.x, 8.0 + row as f32 * cell.y);
            let glyph = Rect::from_min_size(
                p + vec2(0.0, 7.0),
                vec2(font::GLYPH_W, font::GLYPH_H) * scale,
            );
            list.fill(glyph, fade(t.crimson_dark, 0.8));
            // Baseline marker.
            let base = glyph.min.y + font::CAP_HEIGHT * scale;
            draw::hline(
                &mut list,
                glyph.min.x,
                glyph.max.x,
                base,
                fade(t.crimson_bright, 0.5),
            );
            text::draw_glyph(&mut list, glyph.min, c, scale, t.bone);
            let label = if c == ' ' {
                "SP".to_string()
            } else {
                format!("{code}")
            };
            list.text(p, &label, &TextStyle::new(t.ash));
        }
        list
    }

    // ------------------------------------------------------------------ backgrounds

    #[derive(Clone, Copy)]
    enum Scene {
        /// Dark, low-contrast scene (menus).
        Dim,
        /// Brighter scene with hot spots (HUD readability).
        Bright,
        /// Flat dark backdrop (font sheets).
        Flat,
    }

    /// A procedural stand-in for the 3D view: fogged sky, perspective floor grid, a few pillars,
    /// quantised to 5 bits with Bayer dithering like the real post-process.
    fn background(scene: Scene) -> TextureData {
        let mut rgba = vec![0u8; (W * H * 4) as usize];
        const BAYER: [[f32; 4]; 4] = [
            [0.0, 8.0, 2.0, 10.0],
            [12.0, 4.0, 14.0, 6.0],
            [3.0, 11.0, 1.0, 9.0],
            [15.0, 7.0, 13.0, 5.0],
        ];
        let horizon = H as f32 * 0.56;
        let bright = matches!(scene, Scene::Bright);
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f32, y as f32);
                let mut c = match scene {
                    Scene::Flat => Vec4::new(0.07, 0.05, 0.05, 1.0),
                    _ => {
                        if fy < horizon {
                            let k = fy / horizon;
                            let top = Vec4::new(0.10, 0.03, 0.03, 1.0);
                            let glow = if bright {
                                Vec4::new(0.95, 0.55, 0.30, 1.0)
                            } else {
                                Vec4::new(0.45, 0.16, 0.10, 1.0)
                            };
                            top.lerp(glow, k * k)
                        } else {
                            let d = (fy - horizon) / (H as f32 - horizon);
                            let base = Vec4::new(0.16, 0.08, 0.07, 1.0)
                                .lerp(Vec4::new(0.30, 0.16, 0.12, 1.0), d);
                            // Perspective grid.
                            let z = 1.0 / d.max(0.02);
                            let gx = ((fx - W as f32 * 0.5) * d / 60.0).rem_euclid(1.0);
                            let gz = (z * 0.6).rem_euclid(1.0);
                            if gx < 0.06 || gz < 0.08 * d.max(0.2) {
                                base * 1.6
                            } else {
                                base
                            }
                        }
                    }
                };
                if !matches!(scene, Scene::Flat) {
                    // Pillars.
                    for (px, pw, top) in [
                        (140.0, 70.0, 150.0),
                        (980.0, 120.0, 90.0),
                        (620.0, 40.0, 260.0),
                    ] {
                        if fx >= px && fx < px + pw && fy >= top && fy < horizon + 60.0 {
                            let edge = (fx - px) / pw;
                            let shade = if edge < 0.25 { 0.55 } else { 0.32 };
                            c = Vec4::new(shade * 0.9, shade * 0.7, shade * 0.65, 1.0);
                        }
                    }
                    if bright {
                        // A white-hot fire blob behind the HUD's left side and a bright wall.
                        let d = vec2(fx - 260.0, fy - 520.0).length();
                        if d < 180.0 {
                            c = c
                                .lerp(Vec4::new(1.0, 0.92, 0.75, 1.0), (1.0 - d / 180.0).powf(0.6));
                        }
                        if fx > 1040.0 && fy > 420.0 {
                            c = Vec4::new(0.86, 0.84, 0.80, 1.0);
                        }
                    }
                }
                // 5-bit quantisation with ordered dithering.
                let thr = (BAYER[(y % 4) as usize][(x % 4) as usize] + 0.5) / 16.0 - 0.5;
                let levels = 31.0;
                let q =
                    |v: f32| ((v * levels + thr).round().clamp(0.0, levels) / levels * 255.0) as u8;
                let i = ((y * W + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&[q(c.x), q(c.y), q(c.z), 255]);
            }
        }
        TextureData {
            width: W,
            height: H,
            rgba,
        }
    }
}
