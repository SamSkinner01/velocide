//! The in-game screen: one network [`Session`] (single-player = in-process server, host =
//! listen server, join) plus everything that turns it into a game on screen.
//!
//! # Frame flow ([`InGame::frame`])
//!
//! 1. **Network**: `NetClient::update(real dt)` (receive, reconcile, interpolate), then every
//!    [`NetEvent`] is handled ([`InGame::on_net_event`]: level loads, spawns, chat, joins...).
//! 2. **Hotkeys**: pause, chat, restart, scoreboard (unless a text field owns the keyboard).
//! 3. **Look**: mouse / right stick update yaw & pitch *every frame* (`Controls::look`);
//!    button presses are latched so taps shorter than a tick still count.
//! 4. **Fixed ticks** at the server's tick rate: build `PlayerInput` (latest angles) →
//!    `NetClient::tick` (predict + send) → the predicted `MoveEvents` drive sounds, camera
//!    dips and effects ([`InGame::on_local_move_events`]).
//! 5. **Visual time**: `Feedback::update` turns real dt into visual dt (hitstop); camera rig,
//!    viewmodel and effects advance.
//! 6. **Draw**: level, remote players, effects, viewmodel, debug ghosts into the
//!    [`FrameBuffers`]; HUD, name tags, chat, scoreboard, pause / connecting UI.
//! 7. **Audio**: listener follows the camera, remote player sounds, music intensity.
//!
//! Gameplay agents extend this by handling their events in `on_net_event` (e.g. a
//! `GameEvent::Damaged` → `self.feedback.damage_flash(..)`, `self.effects.blood(..)`), drawing
//! their entity kinds in `draw_world`, and adding HUD fields in `draw_hud`. See docs/client.md.

pub mod audio_hooks;
pub mod camera;
pub mod chat;
pub mod debug;
pub mod effects;
pub mod encounter_hud; // arena encounter banners / sounds / readout (docs/enemies.md)
pub mod feedback;
pub mod hud;
pub mod pause;
pub mod remote_players;
pub mod scoreboard;
pub mod tuning_edit;
pub mod viewmodel;
pub mod world_render;

use std::time::Duration;

use engine::audio::Sfx;
use engine::gfx::Camera;
use engine::input::Key;
use engine::physics::MASK_SHOTS;
use engine::render::Renderer;
use engine::time::FixedTimestep;
use engine::ui::{Rect, TextStyle, Ui, UiDrawExt};
use glam::{Vec2, Vec3, Vec4, vec2};
use server::{ServerCommand, ServerConfig};
use shared::movement::MoveEvents;
use shared::protocol::GameEvent;

use crate::app::Common;
use crate::bot::Bot;
use crate::cli::StartMode;
use crate::controls::{Action, Controls, WeaponChange};
use crate::frame::{FrameBuffers, FrameIo, SceneParams, ScreenOutput, Transition};
use crate::net::{
    ClientConfig, ConnectionState, JoinTarget, NetClient, NetEvent, Session, SessionKind,
    SessionSpec, resolve_address,
};
use crate::settings::Settings;
use camera::{CameraFrame, CameraRig};
use chat::{Chat, ChatResult};
use debug::DebugOverlay;
use effects::Effects;
use feedback::Feedback;
use hud::{Hud, HudInfo, NameTag};
use pause::{PauseAction, PauseMenu};
use remote_players::{RemotePlayers, player_color};
use viewmodel::Viewmodel;
use world_render::WorldRender;

/// No snapshot for this long while playing shows "CONNECTION PROBLEM".
const STALL_WARNING_SECS: f64 = 0.6;
/// Placeholder until combat replicates health.
const PLACEHOLDER_HEALTH: f32 = 100.0;

pub struct InGame {
    session: Option<Session>,
    kind: SessionKind,
    /// "SINGLE PLAYER", "HOSTING ON PORT 47420", "192.168.1.20:47420"...
    target: String,
    timestep: FixedTimestep,
    pub controls: Controls,
    bot: Option<Bot>,
    pub camera: CameraRig,
    pub feedback: Feedback,
    pub effects: Effects,
    pub viewmodel: Viewmodel,
    pub world: WorldRender,
    pub remote: RemotePlayers,
    pub hud: Hud,
    pub chat: Chat,
    pause: PauseMenu,
    remote_sounds: audio_hooks::RemoteSounds,
    pub debug: DebugOverlay,
    tick_events: Vec<MoveEvents>,
    /// Level timer (seconds of play since spawn / restart).
    level_time: f64,
    spawned: bool,
    sp_paused: bool,
    restart_pending: bool,
    leaving: bool,
    /// Server-side bots to add once we're connected (`--add-bots`, host only).
    pending_bots: u32,
    time: f64,
    last_camera: Camera,
    last_snapshot_count: u64,
    last_snapshot_time: f64,
}

impl InGame {
    /// Starts a session for `mode`. Errors (bad address, port in use, unknown level) are
    /// returned for the main menu to show.
    pub fn start(
        mode: &StartMode,
        common: &mut Common,
        renderer: &mut Renderer,
    ) -> anyhow::Result<InGame> {
        let name = common.player_name();
        let client = ClientConfig::new(&name).with_netsim(common.launch.netsim);
        let s = &common.settings;
        let (spec, target) = match mode {
            StartMode::Menu => anyhow::bail!("nothing to start"),
            StartMode::SinglePlayer { level } => {
                let level = level.clone().unwrap_or_else(|| s.last_level.clone());
                (
                    SessionSpec::SinglePlayer {
                        level: level.clone(),
                    },
                    format!(
                        "SINGLE PLAYER // {}",
                        crate::menus::main_menu::level_display_name(&level)
                    ),
                )
            }
            StartMode::Host { port, level } => {
                let port = port.unwrap_or(s.host_port);
                let level = level.clone().unwrap_or_else(|| s.last_level.clone());
                let max = common
                    .launch
                    .max_players
                    .unwrap_or(s.host_max_players)
                    .max(1 + common.launch.add_bots)
                    .clamp(1, 8);
                (
                    SessionSpec::Host {
                        server: ServerConfig::host(port, &level, max as usize),
                    },
                    format!("HOSTING ON PORT {port}"),
                )
            }
            StartMode::Join { address } => {
                let addr = resolve_address(address)
                    .map_err(|e| anyhow::anyhow!("can't join '{address}': {e}"))?;
                (
                    SessionSpec::Join {
                        target: JoinTarget::Address(addr),
                    },
                    addr.to_string(),
                )
            }
        };
        common.load_models(renderer);
        let session = Session::start(spec, client)?;
        log::info!("session started: {target} as '{name}'");
        let mut feedback = Feedback::new();
        feedback.apply_settings(&common.settings.retro);
        Ok(InGame {
            kind: session.kind(),
            session: Some(session),
            target,
            timestep: FixedTimestep::new(shared::sim::DEFAULT_TICK_RATE as f64),
            controls: Controls::new(),
            bot: common.launch.bot.then(|| Bot::new(&name)),
            camera: CameraRig::new(),
            feedback,
            effects: Effects::new(renderer),
            viewmodel: Viewmodel::new(),
            world: WorldRender::new(),
            remote: RemotePlayers::new(),
            hud: Hud::new(),
            chat: Chat::new(),
            pause: PauseMenu::new(),
            remote_sounds: audio_hooks::RemoteSounds::new(),
            debug: DebugOverlay::default(),
            tick_events: Vec::new(),
            level_time: 0.0,
            spawned: false,
            sp_paused: false,
            restart_pending: false,
            leaving: false,
            pending_bots: common.launch.add_bots,
            time: 0.0,
            last_camera: Camera::default(),
            last_snapshot_count: 0,
            last_snapshot_time: 0.0,
        })
    }

    pub fn net(&self) -> Option<&NetClient> {
        self.session.as_ref().map(Session::client)
    }

    pub fn session_kind(&self) -> SessionKind {
        self.kind
    }

    pub fn is_paused(&self) -> bool {
        self.pause.is_open()
    }

    /// The single-player simulation is frozen (pause menu open).
    pub fn is_sim_paused(&self) -> bool {
        self.sp_paused
    }

    pub fn is_playing(&self) -> bool {
        self.net().is_some_and(NetClient::is_in_game)
    }

    pub fn level_time(&self) -> f64 {
        self.level_time
    }

    pub fn apply_settings(&mut self, settings: &Settings) {
        self.feedback.apply_settings(&settings.retro);
    }

    /// Leaves the game: disconnects, stops the in-process server, frees GPU resources.
    pub fn shutdown(mut self, renderer: &mut Renderer, common: &mut Common) {
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        common.audio.stop(Sfx::Slide);
        self.world.free(renderer);
        self.effects.free(renderer);
    }

    fn send_server(&self, cmd: ServerCommand) {
        if let Some(server) = self.session.as_ref().and_then(Session::server) {
            server.send(cmd);
        }
    }

    fn set_sp_paused(&mut self, paused: bool) {
        if self.kind != SessionKind::SinglePlayer || self.sp_paused == paused {
            return;
        }
        self.sp_paused = paused;
        self.send_server(ServerCommand::SetPaused(paused));
        if !paused {
            // Don't try to catch up the paused time.
            let rate = self
                .net()
                .map_or(shared::sim::DEFAULT_TICK_RATE, NetClient::tick_rate);
            self.timestep = FixedTimestep::new(rate as f64);
        }
    }

    pub fn open_pause(&mut self) {
        self.pause.open();
        self.set_sp_paused(true);
    }

    pub fn resume(&mut self) {
        self.pause.close();
        self.set_sp_paused(false);
    }

    /// Single-player instant restart: the server puts us back at the start; the timer resets
    /// when the `Respawned` arrives.
    pub fn restart(&mut self) {
        if self.kind != SessionKind::SinglePlayer {
            return;
        }
        self.resume();
        self.send_server(ServerCommand::RestartLevel);
        self.restart_pending = true;
    }

    // ---- network events -----------------------------------------------------------------------

    /// Reacts to one network event. Gameplay systems add their `GameEvent`s here.
    fn on_net_event(
        &mut self,
        ev: NetEvent,
        common: &mut Common,
        renderer: &mut Renderer,
    ) -> Option<Transition> {
        let t = &common.theme;
        let now = self.time;
        match ev {
            NetEvent::Connected { player_id } => {
                log::info!("connected as {player_id}");
                for i in 0..std::mem::take(&mut self.pending_bots) {
                    self.send_server(ServerCommand::AddBot {
                        name: format!("BOT {}", i + 1),
                    });
                }
            }
            NetEvent::Rejected { reason } => {
                log::warn!("rejected: {reason}");
            }
            NetEvent::LevelLoaded { name } => {
                if let Some(world) = self.net().and_then(NetClient::world) {
                    let level = world.level.clone();
                    self.world.load(renderer, &name, &level);
                }
                self.effects.clear();
                common
                    .audio
                    .set_music(Common::game_music(self.world.kind()));
                common.audio.set_music_intensity(0.2);
            }
            NetEvent::TuningChanged => {
                if let Some(net) = self.net() {
                    let tuning = net.tuning().clone();
                    self.debug.editor.on_server_tuning(&tuning, now);
                }
                if self.spawned && common.debug_overlay {
                    self.hud.event("TUNING UPDATED", t.stamina, now);
                }
            }
            NetEvent::Spawned { position, yaw } => {
                log::info!("spawned at {position:?}");
                self.controls.set_view(yaw, 0.0);
                self.camera.reset();
                if !self.spawned || self.restart_pending {
                    self.level_time = 0.0;
                }
                if self.restart_pending {
                    self.restart_pending = false;
                    self.effects.clear();
                    self.hud.banner("RESTART", t.blood_bright, now);
                }
                self.spawned = true;
                common.audio.play_2d(Sfx::Respawn);
            }
            NetEvent::PlayerJoined(info) => {
                if Some(info.id) != self.net().and_then(NetClient::local_player_id) && self.spawned
                {
                    self.hud.event(
                        format!("+ {}", info.name.to_uppercase()),
                        player_color(info.id),
                        now,
                    );
                }
            }
            NetEvent::PlayerLeft { id, name } => {
                self.hud.event(
                    format!("- {} LEFT", name.to_uppercase()),
                    player_color(id),
                    now,
                );
            }
            NetEvent::PlayerUpdated(_) => {}
            NetEvent::Chat { from, name, text } => {
                match from {
                    Some(id) => self.chat.push(&name, player_color(id), &text, now),
                    None => self.chat.system(&text, t.bone_dim, now),
                }
                common.audio.play_2d(Sfx::ChatMessage);
            }
            NetEvent::Game(GameEvent::Respawned { .. }) => {}
            // Arena encounters: banners + sounds (encounter_hud.rs).
            NetEvent::Game(
                ref ev @ (GameEvent::EncounterStarted { .. }
                | GameEvent::WaveStarted { .. }
                | GameEvent::EncounterCleared { .. }),
            ) => {
                encounter_hud::on_event(ev, &mut self.hud, &mut common.audio, t, now);
            }
            // Combat events (damage, hit confirms, kills, parries) are presented by the combat
            // client layer; see docs/combat.md "client presentation contract".
            NetEvent::Game(_) => {}
            NetEvent::Disconnected { reason } => {
                if !self.leaving {
                    let what = if self.spawned {
                        "Disconnected"
                    } else {
                        "Couldn't connect"
                    };
                    return Some(Transition::MainMenu {
                        message: Some(format!("{what}: {reason}")),
                    });
                }
            }
        }
        None
    }

    /// Feedback for the local player's predicted movement (sounds are in `audio_hooks`).
    fn on_local_move_events(
        &mut self,
        ev: &MoveEvents,
        feet: Vec3,
        tuning: &shared::tuning::MovementTuning,
    ) {
        if let Some(speed) = ev.landed {
            self.camera.on_landed(speed, tuning);
            if speed > 22.0 && ev.slam_landed.is_none() {
                self.feedback.shake(0.12);
                self.effects.dust(feet, 8, 3.0);
            }
        }
        if let Some(h) = ev.stepped_up {
            self.camera.on_stepped_up(h);
        }
        if let Some(slam) = ev.slam_landed {
            self.feedback.shake(0.25 + 0.4 * slam.power);
            if slam.power > 0.6 {
                self.feedback.hitstop(0.04);
            }
            self.effects.shockwave(
                slam.position,
                tuning.slam_shockwave_radius * (0.4 + 0.6 * slam.power),
            );
        }
        if let Some(speed) = ev.wall_impact {
            self.feedback.shake((speed / 80.0).min(0.3));
        }
        if ev.slide_started {
            self.effects.dust(feet, 5, 2.0);
        }
        if ev.jump_pad.is_some() {
            self.effects.glow(
                feet + Vec3::Y * 0.3,
                Vec4::new(0.3, 1.0, 0.9, 0.9),
                2.5,
                0.35,
            );
        }
    }

    // ---- the frame ----------------------------------------------------------------------------

    pub fn frame(
        &mut self,
        common: &mut Common,
        io: &mut FrameIo,
        out: &mut FrameBuffers,
    ) -> ScreenOutput {
        let dt = io.dt;
        self.time += dt as f64;
        let mut transition = None;

        // 1. Network.
        let events = match &mut self.session {
            Some(s) => {
                let net = s.client_mut();
                net.update(Duration::from_secs_f32(dt));
                net.drain_events()
            }
            None => Vec::new(),
        };
        for ev in events {
            if let Some(t) = self.on_net_event(ev, common, io.renderer) {
                transition = Some(t);
            }
        }
        let in_game = self.is_playing();

        // 2. Hotkeys.
        let input = io.input;
        let typing = common.ui_state.wants_text_input() || common.ui_state.is_capturing();
        let pressed = |a: Action| common.settings.bindings.pressed(input, a);
        if !typing && !self.chat.is_open() && in_game {
            if self.pause.is_open() {
                // Gamepad Start toggles back (Escape is handled by the pause menu itself).
                if pressed(Action::Pause) && !input.key_pressed(Key::Escape) {
                    self.resume();
                }
            } else if pressed(Action::Pause) {
                self.open_pause();
                common.ui_sound(Sfx::UiSelect);
            } else if pressed(Action::Chat) && self.bot.is_none() {
                self.chat.open();
            } else if pressed(Action::Restart) && self.kind == SessionKind::SinglePlayer {
                self.restart();
            }
        }
        let playing = in_game && !self.pause.is_open() && !self.chat.is_open();
        let scoreboard = in_game
            && !self.chat.is_open()
            && !self.pause.is_open()
            && common.settings.bindings.is_down(input, Action::Scoreboard);

        // 3. Look (every frame) + latch presses / weapon selection.
        if playing && self.bot.is_none() {
            self.controls.look(input, &common.settings, dt);
        }
        if playing {
            let change = self.controls.latch(input, &common.settings.bindings);
            match change {
                WeaponChange::None => {}
                WeaponChange::Weapon => {
                    common.audio.play_2d(Sfx::WeaponSwitch);
                    self.viewmodel.on_switch();
                }
                WeaponChange::Mode => common.audio.play_2d(Sfx::ModeSwitch),
            }
        } else {
            self.controls.clear_latched();
        }

        // 4. Fixed ticks: input -> predict + send.
        if in_game && !self.sp_paused {
            self.run_ticks(common, io, playing || self.bot.is_some());
        }
        if in_game && !self.sp_paused {
            self.level_time += dt as f64;
        }

        // 5. Visual time (hitstop) and the camera.
        let vdt = self.feedback.update(dt);
        let alpha = self.timestep.alpha();
        let scene = self.update_camera(common, alpha, dt, vdt);

        // 6. Draw the world and the UI.
        self.draw_world(common, out, dt, vdt);
        self.draw_ui(common, io, out, scoreboard, &mut transition);

        // 7. Audio.
        common
            .audio
            .set_listener(self.last_camera.position, self.last_camera.rotation);
        if let Some(net) = self.session.as_ref().map(Session::client) {
            self.remote_sounds
                .update(&mut common.audio, net.remote_players());
        }
        if !in_game {
            common.audio.set_music_intensity(0.0);
        }
        self.debug.editor.update(self.time);

        if matches!(transition, Some(Transition::MainMenu { .. })) {
            self.leaving = true;
        }
        // Bots (test clients) never lock the cursor, so several windows stay usable.
        ScreenOutput {
            scene,
            grab_cursor: playing && self.bot.is_none(),
            transition,
        }
    }

    fn run_ticks(&mut self, common: &mut Common, io: &FrameIo, gameplay: bool) {
        let InGame {
            session,
            bot,
            controls,
            timestep,
            tick_events,
            ..
        } = self;
        let Some(session) = session else {
            return;
        };
        let net = session.client_mut();
        timestep.set_hz(net.tick_rate() as f64);
        let dt = shared::sim::tick_dt(net.tick_rate());
        tick_events.clear();
        for _ in 0..timestep.advance(io.dt as f64) {
            if let Some(bot) = bot.as_mut() {
                let world = net.world().map(|w| &w.collision.world);
                let me = net.local_state().map(|s| s.position);
                let target = me.and_then(|me| {
                    net.remote_players()
                        .map(|e| e.position)
                        .min_by(|a, b| a.distance_squared(me).total_cmp(&b.distance_squared(me)))
                });
                bot.tick(dt, net.local_state(), world, target, controls);
            }
            let input = controls.build(io.input, &common.settings.bindings, gameplay);
            if let Some(ev) = net.tick(input) {
                audio_hooks::play_local_move_events(&mut common.audio, &ev);
                tick_events.push(ev);
            }
        }
        let feet = net.local_state().map_or(Vec3::ZERO, |s| s.position);
        let tuning = net.tuning().movement.clone();
        let events = std::mem::take(&mut self.tick_events);
        for ev in &events {
            self.on_local_move_events(ev, feet, &tuning);
        }
        self.tick_events = events;
    }

    fn update_camera(&mut self, common: &Common, alpha: f32, dt: f32, vdt: f32) -> SceneParams {
        let mut scene = self.world.scene();
        let net = self.session.as_ref().map(Session::client);
        let local = net.and_then(|n| Some((n.local_state()?, n.local_render_position(alpha)?, n)));
        if let Some((state, feet, net)) = local {
            let tuning = &net.tuning().movement;
            self.last_camera = self.camera.update(&CameraFrame {
                feet,
                state,
                tuning,
                yaw: self.controls.yaw,
                pitch: self.controls.pitch,
                settings: &common.settings,
                shake: self.feedback.shake_offset(),
                dt,
            });
            if common.settings.show_viewmodel {
                self.viewmodel.update(
                    vdt,
                    state,
                    self.controls.yaw,
                    self.controls.pitch,
                    common.settings.view_bob,
                );
            }
        } else if let Some(spawn) = net
            .and_then(NetClient::world)
            .and_then(|w| w.level.spawns.first().copied())
        {
            // Loading: look at the level from its first spawn.
            let yaw = spawn.yaw.to_radians() + (self.time * 0.1) as f32;
            self.last_camera = Camera {
                position: spawn.position + Vec3::Y * 1.6,
                rotation: Camera::rotation_from_angles(yaw, -0.05, 0.0),
                fov_y_degrees: common.settings.vertical_fov(0.0),
                near: 0.05,
                far: 600.0,
            };
        }
        scene.camera = self.last_camera;
        scene.viewmodel_fov = common.settings.viewmodel_fov;
        scene.screen_flash = self.feedback.screen_flash();
        if !self.world.is_loaded() {
            scene.clear_color = Vec3::new(0.03, 0.01, 0.015);
        }
        scene
    }

    fn draw_world(&mut self, common: &Common, out: &mut FrameBuffers, dt: f32, vdt: f32) {
        let net = self.session.as_ref().map(Session::client);
        let world = net.and_then(NetClient::world);
        let lib = common.models.as_ref();
        self.world.draw(world, &mut out.draws, &mut out.lights);
        self.world.draw_props(lib, self.time, &mut out.draws);
        if let Some(net) = net {
            self.remote
                .draw(lib, net.remote_players(), dt, &mut out.draws);
            // Future replicated entity kinds (enemies, projectiles, pickups): iterate
            // `net.remote_entities()` filtered by `kind` here (see docs/client.md).
        }
        self.effects.update(vdt, world.map(|w| &w.collision.world));
        self.effects.draw(&mut out.sprites, &mut out.decals);

        if let Some(net) = net
            && net.local_state().is_some()
        {
            if common.settings.show_viewmodel {
                let team = net
                    .local_player_id()
                    .map_or(crate::models::DEFAULT_TEAM_COLOR, player_color);
                self.viewmodel.draw(
                    lib,
                    self.controls.weapons.current,
                    self.controls.weapons.mode(),
                    team,
                    self.camera.dip(),
                    self.camera.roll(),
                    &mut out.viewmodel,
                );
            }
            if common.debug_overlay && self.debug.show_ghosts {
                let t = &net.tuning().movement;
                if let Some(server) = net.server_state() {
                    world_render::capsule_lines(
                        server,
                        t,
                        Vec4::new(1.0, 0.2, 0.2, 1.0),
                        &mut out.lines,
                    );
                }
                if let Some(pred) = net.local_state() {
                    world_render::foot_ring_lines(
                        pred,
                        t,
                        Vec4::new(0.2, 1.0, 0.3, 1.0),
                        &mut out.lines,
                    );
                }
                // Remote players' interpolated capsules.
                for e in net.remote_players() {
                    let mut s = shared::movement::MoveState::spawn(e.position);
                    s.crouched = e.flags & shared::protocol::player_flags::LOW_PROFILE != 0;
                    s.velocity = e.velocity;
                    world_render::capsule_lines(
                        &s,
                        t,
                        Vec4::new(0.4, 0.6, 1.0, 1.0),
                        &mut out.lines,
                    );
                }
            }
        }
    }

    fn status_line(&mut self, common: &Common) -> Option<(String, Vec4)> {
        let t = &common.theme;
        let (count, in_game) = {
            let net = self.net()?;
            (net.stats().snapshots_received, net.is_in_game())
        };
        if count != self.last_snapshot_count {
            self.last_snapshot_count = count;
            self.last_snapshot_time = self.time;
        }
        if self.sp_paused {
            return None;
        }
        if in_game && self.time - self.last_snapshot_time > STALL_WARNING_SECS {
            return Some(("CONNECTION PROBLEM".into(), t.blood_bright));
        }
        if self.restart_pending {
            return Some(("RESTARTING".into(), t.bone));
        }
        None
    }

    fn draw_ui(
        &mut self,
        common: &mut Common,
        io: &FrameIo,
        out: &mut FrameBuffers,
        scoreboard: bool,
        transition: &mut Option<Transition>,
    ) {
        let now = io.time;
        let status = self.status_line(common);
        let in_game = self.is_playing();
        let list = &mut out.ui;
        let canvas = Rect::from_min_size(Vec2::ZERO, list.canvas_size);

        if in_game {
            // Name tags.
            if let Some(net) = self.net() {
                let world = net.world().map(|w| &w.collision.world);
                let cam = self.last_camera;
                let tags: Vec<NameTag> = net
                    .remote_players()
                    .filter_map(|e| {
                        let id = e.id.as_player()?;
                        let pos = self.remote.name_tag_position(e);
                        Some(NameTag {
                            position: pos,
                            name: net.player_name(id).unwrap_or("?"),
                            color: player_color(id),
                            occluded: world
                                .is_some_and(|w| w.line_blocked(cam.position, pos, MASK_SHOTS)),
                        })
                    })
                    .collect();
                hud::draw_name_tags(list, &common.theme, &self.last_camera, &tags);
            }
            self.draw_hud(
                common,
                list,
                status.as_ref().map(|(s, c)| (s.as_str(), *c)),
                now,
            );
            self.chat.draw_log(list, &common.theme, canvas, now);
            if scoreboard {
                self.draw_scoreboard(common, list, canvas);
            }
        }
        #[cfg(not(feature = "debug-ui"))]
        if common.debug_overlay {
            let numbers = debug::summary(
                self.net(),
                Some(self.kind),
                &common.stats,
                &io.renderer.stats(),
                common.audio.active_voices(),
            );
            debug::text_overlay(list, &common.theme, &numbers);
        }

        // Interactive layers (only build a Ui when something needs input).
        let connecting = !in_game;
        if !(connecting || self.pause.is_open() || self.chat.is_open()) {
            return;
        }
        let theme = &common.theme;
        let mut ui = Ui::new(&mut out.ui, &mut common.ui_state, io.ui_input, theme);
        let mut sound = None;
        if connecting {
            if self.connecting_ui(&mut ui) {
                sound = Some(Sfx::UiBack);
                *transition = Some(Transition::MainMenu { message: None });
            }
        } else if self.pause.is_open() {
            let subtitle = self.target.clone();
            let single = self.kind == SessionKind::SinglePlayer;
            let r = self
                .pause
                .ui(&mut ui, &mut common.settings, io.input, single, &subtitle);
            sound = r.sound;
            if r.settings.changed {
                common.settings_changed = true;
            }
            if r.settings.name_changed {
                common.name_override = None;
            }
            if r.settings.back {
                common.settings_save = true;
            }
            match r.action {
                PauseAction::None => {}
                PauseAction::Resume => self.resume(),
                PauseAction::Restart => self.restart(),
                PauseAction::Leave => {
                    *transition = Some(Transition::MainMenu { message: None });
                }
            }
        } else if self.chat.is_open() {
            match self.chat.input_ui(&mut ui) {
                ChatResult::Send(text) => {
                    if let Some(s) = &mut self.session {
                        s.client_mut().send_chat(&text);
                    }
                }
                ChatResult::Cancel | ChatResult::None => {}
            }
        }
        drop(ui);
        if let Some(s) = sound {
            common.ui_sound(s);
        }
    }

    /// Connecting / loading panel with CANCEL. Returns true when cancelled.
    fn connecting_ui(&mut self, ui: &mut Ui) -> bool {
        let Some(net) = self.net() else {
            return true;
        };
        let state = match net.state() {
            ConnectionState::Connecting => "CONNECTING",
            ConnectionState::Loading => "LOADING LEVEL",
            ConnectionState::InGame => "READY",
            ConnectionState::Disconnected => "DISCONNECTED",
        };
        let dots = ".".repeat(((self.time * 3.0) as usize) % 4);
        let elapsed = self.time;
        let canvas = ui.canvas();
        crate::menus::settings_menu::dim(ui, if self.world.is_loaded() { 0.4 } else { 0.0 });
        let mut cancel = false;
        let target = self.target.clone();
        ui.panel_auto(
            canvas.center() - vec2(140.0, 50.0),
            280.0,
            "JOINING",
            |ui| {
                ui.default_focus();
                ui.label(&target);
                ui.label_colored(&format!("{state}{dots}"), ui.theme().bone);
                ui.text(&format!("{elapsed:.1} S"), &TextStyle::new(ui.theme().ash));
                ui.spacing(4.0);
                cancel = ui.button("CANCEL");
            },
        );
        if ui.back_pressed() {
            cancel = true;
        }
        let hint = TextStyle::new(ui.theme().ash).centered();
        ui.painter().text(
            vec2(canvas.center().x, canvas.max.y - 16.0),
            "ESC CANCEL",
            &hint,
        );
        cancel
    }

    fn draw_hud(
        &mut self,
        common: &Common,
        list: &mut engine::gfx::UiDrawList,
        status: Option<(&str, Vec4)>,
        now: f64,
    ) {
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        let Some(state) = net.local_state() else {
            return;
        };
        let tuning = &net.tuning().movement;
        let speed = Vec2::new(state.velocity.x, state.velocity.z).length();
        let ping = (self.kind != SessionKind::SinglePlayer).then(|| net.stats().rtt_ms);
        let status = if self.sp_paused { None } else { status };
        let info = HudInfo {
            health: PLACEHOLDER_HEALTH,
            max_health: 100.0,
            hard_damage: 0.0,
            stamina: state.stamina(tuning),
            stamina_max: tuning.dash_charges.round().max(1.0) as u32,
            weapons: &self.controls.weapons,
            speed,
            timer: self.spawned.then_some(self.level_time),
            ping_ms: ping,
            fps: common.stats.fps,
            status,
            settings: &common.settings,
            time: now,
        };
        self.hud.draw(list, &common.theme, &info);
        // Arena encounter readout ("WAVE 2/3 - 5 LEFT").
        encounter_hud::draw(list, &common.theme, net.remote_entities());
    }

    fn draw_scoreboard(&self, common: &Common, list: &mut engine::gfx::UiDrawList, canvas: Rect) {
        let Some(net) = self.net() else {
            return;
        };
        let local = net.local_player_id();
        let rtt = net.stats().rtt_ms;
        let rows: Vec<scoreboard::ScoreRow> = net
            .players()
            .values()
            .map(|p| scoreboard::ScoreRow {
                name: p.name.clone(),
                color: player_color(p.id),
                ping_ms: (Some(p.id) == local && self.kind != SessionKind::SinglePlayer)
                    .then_some(rtt),
                is_local: Some(p.id) == local,
                status: if p.bot {
                    "BOT"
                } else if Some(p.id) == local && self.kind == SessionKind::Host {
                    "HOST"
                } else {
                    "ALIVE"
                },
            })
            .collect();
        let title = format!(
            "{}  //  {}",
            self.world.name().to_uppercase(),
            hud_mode_name(self.kind)
        );
        scoreboard::draw(list, &common.theme, canvas, &title, &rows);
    }

    /// The egui debug window (feature `debug-ui`).
    #[cfg(feature = "debug-ui")]
    pub fn debug_window(
        &mut self,
        ctx: &engine::debug_ui::egui::Context,
        common: &mut Common,
        renderer: &Renderer,
    ) {
        let numbers = debug::summary(
            self.net(),
            Some(self.kind),
            &common.stats,
            &renderer.stats(),
            common.audio.active_voices(),
        );
        let extra = vec![
            format!(
                "level time {:.2}s  particles {}  decals {}  paused {}",
                self.level_time,
                self.effects.particle_count(),
                self.effects.decal_count(),
                self.sp_paused
            ),
            format!(
                "trauma {:.2}  hitstop {}  view yaw {:.1} pitch {:.1}",
                self.feedback.trauma(),
                self.feedback.in_hitstop(),
                self.controls.yaw.to_degrees(),
                self.controls.pitch.to_degrees()
            ),
        ];
        let net = self.session.as_ref().map(Session::client);
        if self.debug.editor.movement.is_empty()
            && let Some(n) = net
        {
            self.debug.editor.sync_from(n.tuning());
        }
        debug::game_window(
            ctx,
            &numbers,
            &mut self.debug,
            net,
            renderer,
            &extra,
            self.time,
        );
    }
}

fn hud_mode_name(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::SinglePlayer => "SOLO",
        SessionKind::Host => "HOSTING",
        SessionKind::Join => "ONLINE",
    }
}
