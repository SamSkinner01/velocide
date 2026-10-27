//! Play of the game on the client (docs/potg.md): the rolling recording, the marked clips and
//! the end-of-match sequence.
//!
//! **Recording** ([`Recorder`]): during PvP matches the client keeps the last [`RECORD_SECS`]
//! of what it received — the decoded snapshot frames (the same `Arc`s as the interpolation
//! buffer: every client gets every entity), kills and parries, the cosmetic `Fx` — plus what it
//! predicted itself and therefore never receives: its own shots, punches, thorn throws and
//! Discharges (the server sends those `Fx` to everyone *but* their owner,
//! `server::combat::FxLog::collect`), stamped with the server tick their input runs at.
//! `GameEvent::HighlightMark` copies the marked stretch, with a lead-in and a tail, into a
//! [`Clip`] that is kept until a better one is marked.
//!
//! **Play of the game** ([`PotgClient`], the `impl InGame` block): `GameEvent::PlayOfTheGame`
//! (sent right before the results) starts the sequence: a title card (name in the player's
//! colour, the title, a stinger), then the clip replayed in first person from that player's
//! eyes ([`Playback`]) — the world drawn by the game's own drawers (`RemotePlayers`,
//! `EntityView`, `fx::play`, `Effects`, `Viewmodel`, `CameraRig`) fed from the clip instead of
//! the network, the player's pose and held weapon from their replicated entity, a slow motion
//! on the last kill, the kill feed and every sound — then the results screen starts its reveal.
//! Space / Esc / Enter / gamepad A, B or Start skip. A client without the clip (it joined
//! late) shows the card alone.
//!
//! Timing: the viewpoint is shown at its own server tick, everything else
//! [`VIEW_LAG_SECS`] behind it — what a client typically shows of the others — so the
//! viewpoint's shots meet what it was aiming at (lag compensation hit them there), exactly like
//! the live game draws the local player at the present and the rest at the render tick. The
//! viewpoint's own projectiles and explosions are at its present too.

use std::collections::VecDeque;
use std::sync::Arc;

use engine::audio::{PlayParams, Sfx};
use engine::gfx::{Camera, UiDrawList};
use engine::input::{GamepadButton, Key};
use engine::physics::{CollisionWorld, MASK_SHOTS};
use engine::ui::draw::{self, fade, mix};
use engine::ui::hud::{self as hud_draw, CrosshairStyle, TextFeed};
use engine::ui::text::line_width;
use engine::ui::{Align, Rect, TextStyle, Theme, UiDrawExt, UiKey};
use glam::{Vec2, Vec3, Vec4, vec2};
use shared::loadout::{Ability, Knack, WeaponClass, WeaponKind};
use shared::movement::{MoveMode, MoveState};
use shared::protocol::{
    EntityState, Fx, FxEvent, GameEvent, NetFrame, NetId, kind, player_armed, player_flags,
    player_weapon,
};
use shared::sim::{Attack, FireIntent, Highlight, HitPart, HitShape, KillInfo, Owner, PlayerId};

use super::InGame;
use super::audio_hooks::RemoteSounds;
use super::camera::{CameraFrame, CameraRig};
use super::combat::entities::{EntityFrame, EntityView};
use super::combat::weapon_feel::hitscan_dirs;
use super::combat::{Rng, attack_style, entity_capsule, fx, hitscan_range, kill_line};
use super::remote_players::{RemotePlayers, player_color};
use super::viewmodel::Viewmodel;
use crate::app::Common;
use crate::frame::{FrameBuffers, FrameIo, SceneParams, ScreenOutput, Transition};
use crate::net::interpolation::SnapshotBuffer;
use crate::net::{NetClient, NetEvent, Session};

/// Seconds of the rolling recording.
pub const RECORD_SECS: f32 = 20.0;
/// Seconds of the clip before the highlight's first moment (less if the player had just
/// respawned) ...
pub const LEAD_SECS: f32 = 2.0;
/// ... and after its last (less if the player died).
pub const TAIL_SECS: f32 = 1.5;
/// How far behind the viewpoint everything else is shown (a typical client's view lag).
pub const VIEW_LAG_SECS: f32 = 0.1;
/// The title card, then the replay.
pub const CARD_SECS: f64 = 3.0;
/// The card alone (no clip) stays a little longer.
pub const CARD_ONLY_SECS: f64 = 3.8;
/// The title slams in (with the stinger) this long after the card opens.
pub const STINGER_AT: f64 = 0.3;
/// Fade to black at the end of the replay.
const FADE_OUT_SECS: f64 = 0.45;
/// Replay speed around the last kill ...
pub const SLOWMO_SPEED: f64 = 0.3;
/// ... from this long before it ...
const SLOWMO_BEFORE: f64 = 0.25;
/// ... to this long after it (clip seconds), easing in and out over `SLOWMO_RAMP`.
const SLOWMO_AFTER: f64 = 0.6;
const SLOWMO_RAMP: f64 = 0.15;
/// Longest title / name shown (server data).
const MAX_TEXT: usize = 28;
/// Caps of the recording (a flood of events can't grow it without bound).
const MAX_EVENTS: usize = 1024;
const MAX_FX: usize = 8192;

/// Seconds → ticks.
fn ticks(secs: f32, tick_rate: u32) -> u32 {
    (secs.max(0.0) * tick_rate as f32).round() as u32
}

/// A reliable event worth replaying (kill feed, kills, deaths, parries) and its server tick.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedEvent {
    pub tick: u32,
    pub event: GameEvent,
}

/// The rolling recording of one client.
#[derive(Clone, Debug)]
pub struct Recorder {
    tick_rate: u32,
    frames: SnapshotBuffer,
    /// Newest frame recorded.
    last_frame: Option<u32>,
    events: VecDeque<RecordedEvent>,
    fx: VecDeque<FxEvent>,
    /// Nothing from this tick on is pruned (a play of the game waiting for its clip).
    pin: Option<u32>,
}

impl Recorder {
    pub fn new(tick_rate: u32) -> Self {
        Self {
            tick_rate: tick_rate.max(1),
            frames: SnapshotBuffer::with_retention(u32::MAX),
            last_frame: None,
            events: VecDeque::new(),
            fx: VecDeque::new(),
            pin: None,
        }
    }

    /// Forgets everything (new level, not a PvP match).
    pub fn clear(&mut self) {
        let rate = self.tick_rate;
        *self = Self::new(rate);
    }

    pub fn set_tick_rate(&mut self, tick_rate: u32) {
        if tick_rate.max(1) != self.tick_rate {
            *self = Self::new(tick_rate);
        }
    }

    pub fn tick_rate(&self) -> u32 {
        self.tick_rate
    }

    /// Nothing is being recorded.
    pub fn is_empty(&self) -> bool {
        self.last_frame.is_none()
    }

    /// Oldest and newest recorded frame ticks.
    pub fn span(&self) -> Option<(u32, u32)> {
        Some((self.frames.oldest()?.0, self.frames.latest()?.0))
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn fx_count(&self) -> usize {
        self.fx.len()
    }

    /// Keeps everything from `tick` on until unpinned (a play of the game cuts its clip once
    /// the tail is recorded).
    pub fn pin(&mut self, tick: Option<u32>) {
        self.pin = tick;
        self.prune();
    }

    /// Takes the frames the session decoded since the last call (they are shared, not copied).
    pub fn record_frames(&mut self, source: &SnapshotBuffer) {
        let Some((newest, _)) = source.latest() else {
            return;
        };
        let from = self.last_frame.map_or(0, |t| t.saturating_add(1));
        if newest < from {
            return;
        }
        for (tick, frame) in source.range(from, newest) {
            self.frames.insert(tick, frame.clone());
        }
        self.last_frame = Some(newest);
        self.prune();
    }

    /// Records one frame (tests, tools).
    pub fn record_frame(&mut self, tick: u32, frame: Arc<NetFrame>) {
        if self.last_frame.is_some_and(|t| tick <= t) {
            return;
        }
        self.frames.insert(tick, frame);
        self.last_frame = Some(tick);
        self.prune();
    }

    /// A reliable event at server tick `tick` (only while frames are being recorded).
    pub fn record_event(&mut self, tick: u32, event: GameEvent) {
        if self.is_empty() {
            return;
        }
        self.events.push_back(RecordedEvent { tick, event });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_front();
        }
    }

    /// Cosmetic events as received (and the local player's predicted ones).
    pub fn record_fx(&mut self, fx: &[FxEvent]) {
        if self.is_empty() {
            return;
        }
        self.fx.extend(fx.iter().cloned());
        while self.fx.len() > MAX_FX {
            self.fx.pop_front();
        }
    }

    fn prune(&mut self) {
        let Some(newest) = self.last_frame else {
            return;
        };
        let mut min = newest.saturating_sub(ticks(RECORD_SECS, self.tick_rate));
        if let Some(pin) = self.pin {
            min = min.min(pin);
        }
        self.frames.retain_from(min);
        self.events.retain(|e| e.tick >= min);
        self.fx.retain(|e| e.tick >= min);
    }

    /// Rough memory use in bytes (frames are counted in full even when shared).
    pub fn approx_bytes(&self) -> usize {
        // A BTreeMap entry costs its key and value plus roughly a third in node overhead.
        let entity =
            (std::mem::size_of::<NetId>() + std::mem::size_of::<shared::protocol::NetEntity>()) * 4
                / 3;
        let frames: usize = self
            .frames
            .range(0, u32::MAX)
            .map(|(_, f)| f.len() * entity + 64)
            .sum();
        let fx: usize = self
            .fx
            .iter()
            .map(|e| match &e.fx {
                Fx::Shot { ends, .. } | Fx::Splinters { ends, .. } => {
                    std::mem::size_of::<FxEvent>() + ends.len() * 12
                }
                Fx::Ricochet { points, .. } | Fx::ArcChain { points, .. } => {
                    std::mem::size_of::<FxEvent>() + points.len() * 12
                }
                _ => std::mem::size_of::<FxEvent>(),
            })
            .sum();
        frames + fx + self.events.len() * std::mem::size_of::<RecordedEvent>()
    }

    /// Copies the stretch of `h` out: from [`LEAD_SECS`] before its start (later if the player
    /// was dead or not there yet, or the recording starts later) to [`TAIL_SECS`] after its end
    /// (earlier if they died; never past what is recorded). `None` when the recording doesn't
    /// cover the highlight itself (joined late, not a PvP match).
    pub fn cut(&self, h: &Highlight) -> Option<Clip> {
        let rate = self.tick_rate;
        let (oldest, newest) = self.span()?;
        // A snapshot interval of slack at both ends.
        let slack = ticks(0.1, rate).max(2);
        if oldest > h.start_tick.saturating_add(slack)
            || newest.saturating_add(slack) < h.end_tick
            || h.end_tick < h.start_tick
        {
            return None;
        }
        let vp = NetId::player(h.player);
        let gone = |f: &NetFrame| {
            f.get(vp)
                .is_none_or(|e| e.flags & (player_flags::DEAD | player_flags::DOWNED) != 0)
        };
        // Lead-in: back from the start while the player is alive in the recording.
        let lead_from = h
            .start_tick
            .saturating_sub(ticks(LEAD_SECS, rate))
            .max(oldest);
        let start = h.start_tick.clamp(oldest, newest);
        let lead: Vec<(u32, bool)> = self
            .frames
            .range(lead_from, start)
            .map(|(t, f)| (t, gone(f)))
            .collect();
        let from = match lead.iter().rposition(|(_, dead)| *dead) {
            Some(i) => lead.get(i + 1).map_or(start, |(t, _)| *t),
            None => lead.first().map_or(start, |(t, _)| *t),
        };
        // Tail: until the player dies (and a moment of their death view) or it runs out.
        let tail_to = h
            .end_tick
            .saturating_add(ticks(TAIL_SECS, rate))
            .min(newest)
            .max(from);
        let to = self
            .frames
            .range(h.end_tick.saturating_add(1), tail_to)
            .find(|(_, f)| gone(f))
            .map_or(tail_to, |(t, _)| {
                t.saturating_add(ticks(0.8, rate)).min(tail_to)
            });
        // Others are shown `VIEW_LAG_SECS` behind the viewpoint: keep that much more.
        let keep_from = from.saturating_sub(ticks(VIEW_LAG_SECS, rate) + slack);
        let mut frames = SnapshotBuffer::with_retention(u32::MAX);
        for (t, f) in self.frames.range(keep_from, to) {
            frames.insert(t, f.clone());
        }
        let in_clip = |tick: u32| tick >= keep_from && tick <= to;
        let events = self
            .events
            .iter()
            .filter(|e| in_clip(e.tick))
            .cloned()
            .collect();
        let mut fx: Vec<FxEvent> = self
            .fx
            .iter()
            .filter(|e| in_clip(e.tick))
            .cloned()
            .collect();
        fx.sort_by_key(|e| (e.tick, e.seq));
        Some(Clip {
            highlight: h.clone(),
            from,
            to,
            tick_rate: rate,
            frames,
            events,
            fx,
        })
    }
}

/// A recorded stretch of the match around one highlight.
#[derive(Clone, Debug)]
pub struct Clip {
    pub highlight: Highlight,
    /// Ticks shown, lead-in and tail included (the viewpoint's time).
    pub from: u32,
    pub to: u32,
    pub tick_rate: u32,
    pub frames: SnapshotBuffer,
    /// By tick.
    pub events: Vec<RecordedEvent>,
    /// By tick, then sequence.
    pub fx: Vec<FxEvent>,
}

impl Clip {
    /// Is this the clip of `h` (same player and moment)?
    pub fn is_of(&self, h: &Highlight) -> bool {
        self.highlight.player == h.player
            && self.highlight.start_tick == h.start_tick
            && self.highlight.end_tick == h.end_tick
    }

    /// Clip seconds at normal speed.
    pub fn secs(&self) -> f32 {
        self.to.saturating_sub(self.from) as f32 / self.tick_rate.max(1) as f32
    }

    /// Every entity at `tick` (interpolated, frozen past the ends), skipping `exclude`.
    pub fn sample(&self, tick: f64, exclude: Option<NetId>, out: &mut Vec<EntityState>) {
        let dt = 1.0 / self.tick_rate.max(1) as f32;
        self.frames.sample(tick, 0.0, dt, exclude, out);
    }

    /// The viewpoint's replicated entity at `tick`.
    pub fn viewpoint(&self, tick: f64) -> Option<EntityState> {
        let mut all = Vec::new();
        self.sample(tick, None, &mut all);
        let id = NetId::player(self.highlight.player);
        all.into_iter().find(|e| e.id == id)
    }
}

/// Replay speed at clip tick `tick`: 1, easing into [`SLOWMO_SPEED`] just before the last kill
/// (`last_kill`) and back out after it.
pub fn replay_speed(tick: f64, last_kill: u32, tick_rate: u32) -> f64 {
    let d = (tick - last_kill as f64) / tick_rate.max(1) as f64;
    let (a, b) = (-SLOWMO_BEFORE, SLOWMO_AFTER);
    let w = if d < a - SLOWMO_RAMP || d > b + SLOWMO_RAMP {
        0.0
    } else if d < a {
        (d - (a - SLOWMO_RAMP)) / SLOWMO_RAMP
    } else if d > b {
        1.0 - (d - b) / SLOWMO_RAMP
    } else {
        1.0
    };
    1.0 - (1.0 - SLOWMO_SPEED) * w.clamp(0.0, 1.0)
}

/// Server text shown big: printable ASCII, at most [`MAX_TEXT`] characters.
pub fn display_text(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(MAX_TEXT)
        .collect::<String>()
        .trim()
        .to_string()
}

/// What a replicated player holds, for the viewmodel: the weapon, the selected ability (its
/// glow) and the charge / draw level 0..1 while charging or drawing
/// (`shared::protocol::player_weapon`).
pub(super) fn held_weapon(e: &EntityState) -> (WeaponKind, Ability, f32) {
    let (weapon, ability, charge) = player_weapon(e.state);
    let charging = e.flags & (player_flags::CHARGING | player_flags::DRAWING) != 0;
    (
        weapon,
        ability,
        if charging { charge as f32 / 255.0 } else { 0.0 },
    )
}

/// A movement state for the camera rig and the viewmodel from a replicated player.
pub(super) fn move_state(e: &EntityState) -> MoveState {
    let f = |bit: u32| e.flags & bit != 0;
    let on_ground = f(player_flags::ON_GROUND);
    MoveState {
        velocity: e.velocity,
        on_ground,
        crouched: f(player_flags::LOW_PROFILE),
        mode: if f(player_flags::SLIDING) {
            MoveMode::Slide
        } else if f(player_flags::DASHING) {
            MoveMode::Dash
        } else if f(player_flags::SLAMMING) {
            MoveMode::Slam
        } else if on_ground {
            MoveMode::Walk
        } else {
            MoveMode::Air
        },
        ..MoveState::spawn(e.position)
    }
}

/// Where a hitscan ray from `origin` stops: the level or the first drawn body.
fn trace(
    origin: Vec3,
    dir: Vec3,
    range: f32,
    world: Option<&CollisionWorld>,
    entities: &[EntityState],
) -> Vec3 {
    let mut end = world
        .and_then(|w| w.cast_ray(origin, dir, range, MASK_SHOTS))
        .map_or(range, |h| h.distance);
    for e in entities {
        if let Some((a, b, r)) = entity_capsule(e)
            && let Some(d) = HitShape::capsule(HitPart::Body, a, b, r).ray_hit(origin, dir, end)
        {
            end = d;
        }
    }
    origin + dir * end
}

// ---------------------------------------------------------------------------------------------
// The end-of-match sequence
// ---------------------------------------------------------------------------------------------

/// The replay of a clip: its clock and the drawers' state (fresh for every replay, so the live
/// game's animation state is untouched).
pub struct Playback {
    /// The viewpoint's present (fractional server tick).
    pub tick: f64,
    entities: EntityView,
    remote: RemotePlayers,
    viewmodel: Viewmodel,
    camera: CameraRig,
    sounds: RemoteSounds,
    feed: TextFeed,
    rng: Rng,
    /// Clip `fx` already played.
    played: Vec<bool>,
    next_event: usize,
    others: Vec<EntityState>,
    present: Vec<EntityState>,
    weapon: Option<(WeaponKind, Ability)>,
    on_ground: bool,
    fall_speed: f32,
    dead: bool,
    slowmo: bool,
    /// When the viewpoint last killed (kill marker) and when the clip ran out (fade).
    kill_at: Option<f64>,
    ended_at: Option<f64>,
    started: f64,
    last_camera: Camera,
}

impl Playback {
    fn new(clip: &Clip, now: f64) -> Self {
        let mut feed = TextFeed::new(5, 4.0);
        feed.newest_on_top = true;
        Self {
            tick: clip.from as f64,
            entities: EntityView::new(),
            remote: RemotePlayers::new(),
            viewmodel: Viewmodel::new(),
            camera: CameraRig::new(),
            sounds: RemoteSounds::new(),
            feed,
            rng: Rng::default(),
            played: vec![false; clip.fx.len()],
            next_event: 0,
            others: Vec::new(),
            present: Vec::new(),
            weapon: None,
            on_ground: true,
            fall_speed: 0.0,
            dead: false,
            slowmo: false,
            kill_at: None,
            ended_at: None,
            started: now,
            last_camera: Camera::default(),
        }
    }
}

/// The running play of the game.
pub struct Potg {
    pub highlight: Highlight,
    /// The player's name and the title as shown.
    pub name: String,
    pub title: String,
    pub color: Vec4,
    /// When the card opened (`InGame` clock).
    pub started: f64,
    pub clip: Option<Clip>,
    /// The clip is cut from the rolling recording when the card ends (its tail is recorded
    /// by then).
    needs_cut: bool,
    pub playback: Option<Playback>,
    /// The card is over and there is nothing to replay.
    card_only: bool,
    stinger: bool,
    done: bool,
}

/// Play-of-the-game state of the game screen.
pub struct PotgClient {
    pub recorder: Recorder,
    /// The clip of the best highlight marked so far.
    pub marked: Option<Clip>,
    pub potg: Option<Potg>,
    rng: Rng,
}

impl PotgClient {
    pub fn new() -> Self {
        Self {
            recorder: Recorder::new(shared::sim::DEFAULT_TICK_RATE),
            marked: None,
            potg: None,
            rng: Rng::default(),
        }
    }
}

impl Default for PotgClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Space, Esc, Enter, gamepad A / B / Start.
fn skip_pressed(io: &FrameIo) -> bool {
    io.input.key_pressed(Key::Space)
        || io.input.gamepad_pressed(GamepadButton::Start)
        || io.ui_input.key(UiKey::Enter)
        || io.ui_input.key(UiKey::Escape)
}

impl InGame {
    /// The play of the game is on screen (title card or replay).
    pub fn potg_active(&self) -> bool {
        self.potg.potg.is_some()
    }

    /// The running play of the game (tools / screenshots).
    pub fn potg(&self) -> Option<&Potg> {
        self.potg.potg.as_ref()
    }

    /// Recording, marked clip, play of the game (tools / tests).
    pub fn potg_client(&self) -> &PotgClient {
        &self.potg
    }

    /// Every network event passes here first: recording kills and parries, marks, the start
    /// of the play of the game, a new level forgetting everything.
    pub(super) fn potg_event(&mut self, ev: &NetEvent, common: &mut Common) {
        match ev {
            NetEvent::LevelLoaded { .. } => {
                self.potg.recorder.clear();
                self.potg.marked = None;
                self.potg.potg = None;
            }
            NetEvent::Game(e @ GameEvent::Killed(k)) => {
                self.potg.recorder.record_event(k.tick, e.clone());
            }
            NetEvent::Game(e @ GameEvent::Parried(p)) => {
                self.potg.recorder.record_event(p.tick, e.clone());
            }
            NetEvent::Game(GameEvent::HighlightMark(h)) => {
                let clip = self.potg.recorder.cut(h);
                log::info!(
                    "highlight marked: {} {:?} ({}..{}){}",
                    h.player,
                    h.title,
                    h.start_tick,
                    h.end_tick,
                    if clip.is_some() { "" } else { ", not recorded" }
                );
                self.potg.marked = clip;
            }
            NetEvent::Game(GameEvent::PlayOfTheGame(h)) => self.start_potg(h, common),
            _ => {}
        }
    }

    fn start_potg(&mut self, h: &Highlight, common: &mut Common) {
        let net = self.session.as_ref().map(Session::client);
        let name = net
            .and_then(|n| n.player_name(h.player))
            .map(|n| display_text(&n.to_uppercase()))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "A SHRIKE".to_string());
        let clip = self.potg.marked.take().filter(|c| c.is_of(h));
        let needs_cut = clip.is_none() && !self.potg.recorder.is_empty();
        if needs_cut {
            let rate = self.potg.recorder.tick_rate();
            let keep = ticks(LEAD_SECS + VIEW_LAG_SECS + 0.2, rate);
            self.potg
                .recorder
                .pin(Some(h.start_tick.saturating_sub(keep)));
        }
        log::info!(
            "play of the game: {name} {:?}{}",
            h.title,
            if clip.is_some() || needs_cut {
                ""
            } else {
                " (no recording: card only)"
            }
        );
        self.potg.potg = Some(Potg {
            highlight: h.clone(),
            name,
            title: display_text(&h.title),
            color: player_color(h.player),
            started: self.time,
            clip,
            needs_cut,
            playback: None,
            card_only: false,
            stinger: false,
            done: false,
        });
        // The sequence owns the screen: no chat field, no pause menu, no charge whine.
        self.chat.close();
        self.resume();
        common.audio.stop(Sfx::DischargeCharge);
        common.audio.stop(Sfx::Slide);
    }

    /// Records this frame's new snapshot frames (PvP matches only; the recording is dropped
    /// otherwise). Called after the network update.
    pub(super) fn potg_record(&mut self) {
        // PvP matches only (not co-op, not a world running in the lobby phase).
        let running = self
            .net()
            .and_then(NetClient::lobby_phase)
            .is_none_or(|p| p.level_running());
        let pvp = self.match_mode().is_pvp() && running;
        let InGame { session, potg, .. } = self;
        let Some(net) = session.as_ref().map(Session::client) else {
            return;
        };
        if !pvp {
            if !potg.recorder.is_empty() || potg.marked.is_some() {
                potg.recorder.clear();
                potg.marked = None;
            }
            return;
        }
        // While the play of the game runs, only a clip still to be cut needs more.
        let wanted = potg.potg.as_ref().is_none_or(|p| p.needs_cut);
        if net.is_in_game() && wanted {
            potg.recorder.set_tick_rate(net.tick_rate());
            potg.recorder.record_frames(net.frames());
        }
    }

    /// The local player's predicted shots of this frame's ticks, recorded as the `Fx` everyone
    /// else receives for them (the server never sends those to us). Called before
    /// `combat_after_ticks` consumes them.
    pub(super) fn potg_record_local_shots(&mut self) {
        let InGame {
            session,
            potg,
            combat,
            last_camera,
            ..
        } = self;
        let shots = combat.pending_shots();
        if shots.is_empty() || potg.recorder.is_empty() || potg.potg.is_some() {
            return;
        }
        let Some(net) = session.as_ref().map(Session::client) else {
            return;
        };
        let Some(me) = net.local_player_id() else {
            return;
        };
        let t = &net.tuning().combat;
        let world = net.world().map(|w| &w.collision.world);
        let origin = last_camera.position;
        let body = net
            .local_state()
            .map_or(origin - Vec3::Y * 0.7, |s| s.position + Vec3::Y * 0.9);
        // Input `seq` runs at the server tick the newest own state maps it to (one input per
        // tick).
        let own = net.own_combat().copied();
        let estimate = net.server_tick_estimate().unwrap_or(0.0).max(0.0) as u32;
        let tick_of =
            |seq: u32| own.map_or(estimate, |o| o.tick.wrapping_add(seq.wrapping_sub(o.seq)));
        let mut recorded = Vec::new();
        for shot in shots {
            let tick = tick_of(shot.input.seq);
            for intent in &shot.intents {
                let fx = match *intent {
                    FireIntent::Punch => Fx::Punch {
                        player: me,
                        hit: false,
                    },
                    FireIntent::Knack(Knack::Thorn) => Fx::ThornThrown { player: me },
                    FireIntent::Knack(knack) => Fx::Knack { player: me, knack },
                    // Melee swings (the Hurl flies: a shot like the other projectiles).
                    FireIntent::Primary { weapon, .. } | FireIntent::Ability { weapon, .. }
                        if weapon.class() == WeaponClass::Melee
                            && intent.attack() != Some(Attack::Hurl) =>
                    {
                        let Some(attack) = intent.attack() else {
                            continue;
                        };
                        Fx::Swing {
                            player: me,
                            attack,
                            hits: 0,
                        }
                    }
                    FireIntent::Ability { charge, .. }
                        if intent.attack() == Some(Attack::Discharge) =>
                    {
                        let a = &t.arclash;
                        let r = a.discharge_min_radius
                            + (a.discharge_max_radius - a.discharge_min_radius)
                                * charge.clamp(0.0, 1.0);
                        Fx::Discharge {
                            player: me,
                            position: body.into(),
                            radius_dm: (r * 10.0).round().clamp(0.0, u16::MAX as f32) as u16,
                        }
                    }
                    _ => {
                        let Some(attack) = intent.attack() else {
                            continue;
                        };
                        let range = hitscan_range(attack, t);
                        let ends = hitscan_dirs(intent, &shot.input, t, &mut potg.rng)
                            .into_iter()
                            .map(|d| trace(origin, d, range, world, net.remote_entities()).into())
                            .collect();
                        Fx::Shot {
                            shooter: me,
                            attack,
                            origin: origin.into(),
                            ends,
                        }
                    }
                };
                recorded.push(FxEvent { seq: 0, tick, fx });
            }
        }
        potg.recorder.record_fx(&recorded);
    }

    /// One frame of the play of the game, instead of the normal frame after the network
    /// update. `None` when there is none (or it just ended: the results take this frame).
    pub(super) fn potg_frame(
        &mut self,
        common: &mut Common,
        io: &FrameIo,
        out: &mut FrameBuffers,
        transition: &mut Option<Transition>,
    ) -> Option<ScreenOutput> {
        let potg = self.potg.potg.as_ref()?;
        if potg.done {
            self.end_potg(common);
            return None;
        }
        let dt = io.dt;
        let now = self.time;
        let skip = skip_pressed(io);
        // Inputs keep flowing (neutral), as on the results screen.
        if self.is_playing() && !self.sp_paused {
            self.run_ticks(common, io, false);
        }
        let vdt = self.feedback.update(dt);

        // Card → replay (or the card alone).
        self.potg_advance(common, now);
        let replaying = self
            .potg
            .potg
            .as_ref()
            .is_some_and(|p| p.playback.is_some());
        let scene = if replaying {
            // The live game's cosmetic events are of no use now.
            if let Some(s) = &mut self.session {
                drop(s.client_mut().drain_fx());
            }
            self.potg_replay_frame(common, out, dt, vdt)
        } else {
            // The card over the frozen end of the match.
            if self.is_playing() {
                self.combat_update(common, vdt);
            }
            let alpha = self.timestep.alpha();
            let scene = self.update_camera(common, alpha, dt, vdt);
            self.draw_world(common, out, dt, vdt);
            scene
        };
        self.potg_ui(common, out, now);
        common
            .audio
            .set_listener(self.last_camera.position, self.last_camera.rotation);
        common.audio.set_music_intensity(0.0);

        if let Some(p) = &mut self.potg.potg {
            let clip_over = p
                .playback
                .as_ref()
                .and_then(|pb| pb.ended_at)
                .is_some_and(|t| now - t >= FADE_OUT_SECS);
            let card_over = p.card_only && now - p.started >= CARD_ONLY_SECS;
            if skip || clip_over || card_over {
                p.done = true;
            }
        }
        if matches!(transition, Some(Transition::MainMenu { .. })) {
            self.leaving = true;
        }
        Some(ScreenOutput {
            scene,
            grab_cursor: false,
            transition: transition.take(),
        })
    }

    /// Stinger, and the cut from the card to the replay.
    fn potg_advance(&mut self, common: &mut Common, now: f64) {
        let InGame {
            potg,
            effects,
            combat,
            feedback,
            ..
        } = self;
        let Some(p) = potg.potg.as_mut() else {
            return;
        };
        let age = now - p.started;
        if !p.stinger && age >= STINGER_AT {
            p.stinger = true;
            common.audio.play_2d(Sfx::PotgStinger);
            feedback.shake(0.25);
        }
        if age < CARD_SECS || p.playback.is_some() || p.card_only {
            return;
        }
        if p.needs_cut {
            p.needs_cut = false;
            p.clip = potg.recorder.cut(&p.highlight);
            potg.recorder.pin(None);
        }
        match &p.clip {
            Some(clip) if clip.frames.len() >= 2 => {
                p.playback = Some(Playback::new(clip, now));
                // Fresh effects for the replay (the match is over; nothing live is lost).
                effects.clear();
                combat.beams.clear();
                feedback.flash(Vec3::ONE, 0.8);
                common.audio.play_2d(Sfx::PotgWhoosh);
            }
            _ => p.card_only = true,
        }
    }

    /// The play of the game is over (or skipped): the results reveal starts now.
    fn end_potg(&mut self, common: &mut Common) {
        self.potg.potg = None;
        self.potg.recorder.pin(None);
        self.effects.clear();
        self.combat.beams.clear();
        self.camera.reset();
        common.audio.stop(Sfx::DischargeCharge);
        let now = self.time;
        self.restart_results_reveal(now);
    }

    /// One replay frame: advances the clip clock, plays its events at their ticks and draws the
    /// world from the viewpoint's eyes.
    fn potg_replay_frame(
        &mut self,
        common: &mut Common,
        out: &mut FrameBuffers,
        dt: f32,
        vdt: f32,
    ) -> SceneParams {
        let InGame {
            session,
            potg,
            world: level,
            effects,
            combat,
            feedback,
            last_camera,
            time,
            ..
        } = self;
        let now = *time;
        let mut scene = level.scene();
        let Some(net) = session.as_ref().map(Session::client) else {
            return scene;
        };
        let Some(p) = potg.potg.as_mut() else {
            return scene;
        };
        let (Some(clip), Some(pb)) = (p.clip.as_ref(), p.playback.as_mut()) else {
            return scene;
        };
        let rate = clip.tick_rate.max(1);
        let vp = clip.highlight.player;
        let vp_id = NetId::player(vp);
        let vp_owner = vp.0 as u16;
        let lag = (VIEW_LAG_SECS * rate as f32) as f64;
        let audio = &mut common.audio;
        let lib = common.models.as_ref();
        let world = net.world().map(|w| &w.collision.world);
        let tuning = net.tuning();

        // Clock: slow motion around the last kill.
        let speed = replay_speed(pb.tick, clip.highlight.end_tick, rate);
        if speed < 0.95 && !pb.slowmo {
            pb.slowmo = true;
            audio.play_2d(Sfx::PotgSlowmo);
        }
        if pb.ended_at.is_none() {
            pb.tick = (pb.tick + dt as f64 * rate as f64 * speed).min(clip.to as f64);
            if pb.tick >= clip.to as f64 {
                pb.ended_at = Some(now);
            }
        }
        let vdt = vdt * speed as f32;
        let view_tick = pb.tick - lag;

        // Entities: the others behind, the viewpoint (and its projectiles) at its present.
        clip.sample(view_tick, Some(vp_id), &mut pb.others);
        clip.sample(pb.tick, None, &mut pb.present);
        let me = pb.present.iter().find(|e| e.id == vp_id).copied();
        let own: Vec<EntityState> = pb
            .present
            .iter()
            .filter(|e| kind::is_projectile(e.kind) && e.owner == vp_owner)
            .copied()
            .collect();

        // Camera from the viewpoint's eyes.
        if let Some(e) = me {
            let state = move_state(&e);
            if state.on_ground && !pb.on_ground {
                pb.camera.on_landed(pb.fall_speed, &tuning.movement);
            }
            pb.on_ground = state.on_ground;
            pb.fall_speed = (-e.velocity.y).max(0.0);
            pb.last_camera = pb.camera.update(&CameraFrame {
                feet: e.position,
                state: &state,
                tuning: &tuning.movement,
                yaw: e.yaw,
                pitch: e.pitch,
                settings: &common.settings,
                shake: feedback.shake_offset(),
                downed: pb.dead || e.flags & (player_flags::DEAD | player_flags::DOWNED) != 0,
                dt,
            });
            let (weapon, mode, charge) = held_weapon(&e);
            let drawing = e.flags & player_flags::DRAWING != 0;
            pb.viewmodel.set_draw(if drawing { charge } else { 0.0 });
            if pb.weapon.is_some_and(|w| w != (weapon, mode)) {
                pb.viewmodel.on_switch();
            }
            pb.weapon = Some((weapon, mode));
            pb.viewmodel.set_charge(if drawing { 0.0 } else { charge });
            pb.viewmodel.set_unarmed(!player_armed(e.state));
            pb.viewmodel
                .update(vdt, &state, e.yaw, e.pitch, common.settings.view_bob);
        }
        let cam = pb.last_camera;
        *last_camera = cam;
        let muzzle_of = |vm: &Viewmodel| {
            vm.last_draw()
                .map_or(cam.position + cam.rotation * Vec3::NEG_Z * 0.6, |d| {
                    cam.position + cam.rotation * d.muzzle
                })
        };

        // Reliable events at the time they are seen.
        let theme = &common.theme;
        while let Some(e) = clip.events.get(pb.next_event) {
            if e.tick as f64 > view_tick {
                break;
            }
            pb.next_event += 1;
            match &e.event {
                GameEvent::Killed(k) => {
                    replay_kill(k, vp, net, pb, audio, feedback, theme, now);
                }
                GameEvent::Parried(parry) if parry.player == vp => {
                    feedback.parry_flash();
                    audio.play_2d(Sfx::Parry);
                }
                GameEvent::Parried(parry) => audio.play_at(Sfx::Parry, parry.position),
                _ => {}
            }
        }

        // Cosmetic events: the viewpoint's own at its present, the rest when seen.
        let thorns = &pb.entities;
        let thorn_position = |id: NetId| thorns.thorn_position(id);
        let mut first_person = Vec::new();
        {
            let mut ctx = fx::FxCtx {
                audio,
                effects,
                feedback,
                beams: &mut combat.beams,
                remote: &mut pb.remote,
                world,
                entities: &pb.others,
                local: Some(vp),
                listener: cam.position,
                local_chest: me.map(|e| e.position + Vec3::Y * 1.1),
                thorn_position: &thorn_position,
                rng: &mut pb.rng,
            };
            for (i, ev) in clip.fx.iter().enumerate() {
                if pb.played[i] {
                    continue;
                }
                let own = own_fx(&ev.fx, vp);
                let due = ev.tick as f64 <= if own { pb.tick } else { view_tick };
                if !due {
                    continue;
                }
                pb.played[i] = true;
                if own {
                    first_person.push(ev.fx.clone());
                } else {
                    fx::play(&mut ctx, ev);
                }
            }
        }
        for f in &first_person {
            let muzzle = muzzle_of(&pb.viewmodel);
            play_first_person(
                f,
                cam,
                muzzle,
                world,
                &mut pb.viewmodel,
                audio,
                effects,
                &mut combat.beams,
                feedback,
                &mut pb.rng,
            );
        }

        // Remote players' life state, then every drawer.
        for e in pb.others.iter().filter(|e| e.kind == kind::PLAYER) {
            let v = pb.remote.visual_mut(e.id);
            v.dead = e.flags & player_flags::DEAD != 0;
            v.downed = e.flags & player_flags::DOWNED != 0;
        }
        let own_offset = pb
            .viewmodel
            .last_draw()
            .map_or(Vec3::ZERO, |d| cam.rotation * d.muzzle);
        let frame = EntityFrame {
            entities: &pb.others,
            render_tick: view_tick,
            tick_rate: rate,
            tuning: &tuning.combat,
            own: &own,
            own_time: pb.tick,
            local_owner: Some(vp_owner),
            own_offset,
            camera: cam.position,
            world,
        };
        pb.entities
            .update(&frame, vdt, effects, audio, &mut pb.rng, &mut combat.beams);
        combat.beams.update(vdt);
        pb.entities
            .brand_sigils(&pb.others, Some(vp), cam.position, &mut combat.beams);
        pb.sounds.update(
            audio,
            pb.others
                .iter()
                .filter(|e| e.kind == kind::PLAYER)
                .chain(me.as_ref()),
        );

        level.draw(net.world(), &mut out.draws, &mut out.lights);
        level.draw_props(lib, now, &mut out.draws);
        // Everyone in the replay wears their look (lobby: cosmetics).
        let look = |id| {
            net.players()
                .get(&id)
                .map(|p| p.cosmetics)
                .unwrap_or_default()
        };
        pb.remote.draw(
            lib,
            pb.others.iter().filter(|e| e.kind == kind::PLAYER),
            &look,
            vdt,
            &mut out.draws,
        );
        pb.entities
            .draw(lib, &frame, &mut out.draws, &mut out.lights);
        combat.beams.draw(&mut out.draws, &mut out.lights);
        effects.update(vdt, world);
        effects.draw(&mut out.sprites, &mut out.decals);
        let alive = me.is_some_and(|e| e.flags & (player_flags::DEAD | player_flags::DOWNED) == 0);
        if common.settings.show_viewmodel && alive && !pb.dead {
            let (weapon, mode) = pb.weapon.unwrap_or_default();
            pb.viewmodel.draw(
                lib,
                weapon,
                mode,
                player_color(vp),
                pb.camera.dip(),
                pb.camera.roll(),
                &mut out.viewmodel,
            );
        }

        scene.camera = cam;
        scene.viewmodel_fov = common.settings.viewmodel_fov;
        scene.screen_flash = feedback.screen_flash();
        scene
    }

    /// The card, or the replay's HUD.
    fn potg_ui(&mut self, common: &Common, out: &mut FrameBuffers, now: f64) {
        let Some(p) = &self.potg.potg else {
            return;
        };
        let list = &mut out.ui;
        let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
        let t = &common.theme;
        match &p.playback {
            None => draw_card(
                list,
                t,
                c,
                &CardInfo {
                    name: &p.name,
                    title: &p.title,
                    color: p.color,
                    replay: !p.card_only && (p.clip.is_some() || p.needs_cut),
                },
                now - p.started,
            ),
            Some(pb) => {
                let fade_out = pb
                    .ended_at
                    .map_or(0.0, |e| ((now - e) / FADE_OUT_SECS).clamp(0.0, 1.0) as f32);
                draw_replay_hud(
                    list,
                    t,
                    c,
                    &ReplayHud {
                        name: &p.name,
                        title: &p.title,
                        color: p.color,
                        feed: &pb.feed,
                        kill_age: pb.kill_at.map(|k| now - k),
                        age: now - pb.started,
                        fade_out,
                        dead: pb.dead,
                    },
                    now,
                );
            }
        }
    }
}

/// Is this `Fx` an action of the viewpoint shown in first person (its shots, punches, thorn
/// throws, Discharges, slam landings)?
fn own_fx(f: &Fx, vp: PlayerId) -> bool {
    match f {
        Fx::Shot { shooter, .. } => *shooter == vp,
        Fx::Punch { player, .. }
        | Fx::ThornThrown { player }
        | Fx::Discharge { player, .. }
        | Fx::SlamShockwave { player, .. } => *player == vp,
        _ => false,
    }
}

/// A kill in the replay: the feed line; the viewpoint's kills get the kill confirm and the
/// marker, its death the death flash.
#[allow(clippy::too_many_arguments)]
fn replay_kill(
    k: &KillInfo,
    vp: PlayerId,
    net: &NetClient,
    pb: &mut Playback,
    audio: &mut engine::audio::Audio,
    feedback: &mut super::feedback::Feedback,
    theme: &Theme,
    now: f64,
) {
    let mine = k.killer == Owner::Player(vp);
    let died = k.victim == shared::sim::HitTarget::Player(vp);
    let involves_player = matches!(k.killer, Owner::Player(_)) || k.victim.player().is_some();
    if involves_player {
        let color = if mine {
            player_color(vp)
        } else if died {
            theme.crimson_bright
        } else {
            theme.bone_dim
        };
        pb.feed.push(kill_line(net, k), color, now);
    }
    if mine && !died {
        pb.kill_at = Some(now);
        audio.play_2d(Sfx::KillConfirm);
        feedback.shake(0.12);
    }
    if died {
        pb.dead = true;
        audio.play_2d(Sfx::Death);
        feedback.death_flash();
    }
}

/// The viewpoint's own action, as its player saw it: recoil and sound in first person, the
/// tracers from its viewmodel's muzzle.
#[allow(clippy::too_many_arguments)]
fn play_first_person(
    f: &Fx,
    cam: Camera,
    muzzle: Vec3,
    world: Option<&CollisionWorld>,
    vm: &mut Viewmodel,
    audio: &mut engine::audio::Audio,
    effects: &mut super::effects::Effects,
    beams: &mut fx::Beams,
    feedback: &mut super::feedback::Feedback,
    rng: &mut Rng,
) {
    let aim = cam.rotation * Vec3::NEG_Z;
    match f {
        Fx::Shot { attack, ends, .. } => {
            let style = attack_style(*attack);
            audio.play_2d(style.sfx);
            vm.fire(style.kick);
            feedback.shake(style.shake);
            if *attack == Attack::Tumble {
                vm.turn_drum();
            }
            if *attack == Attack::Gust {
                fx::gust_puff(effects, beams, rng, muzzle, aim);
                return;
            }
            if *attack == Attack::Arc {
                effects.glow(muzzle, fx::ARC_COLOR, 0.25, 0.05);
            } else {
                effects.muzzle_flash(muzzle + aim * 0.06, aim);
                beams.light(muzzle + aim * 0.3, Vec3::new(1.8, 1.2, 0.7), 5.0, 0.05);
            }
            for end in ends {
                let end = end.to_vec3();
                if style.jagged {
                    beams.arc(
                        muzzle,
                        end,
                        style.beam_width,
                        style.beam_color,
                        style.beam_life,
                        rng,
                    );
                } else {
                    beams.beam(
                        muzzle,
                        end,
                        style.beam_width,
                        style.beam_color,
                        style.beam_life,
                    );
                }
                fx::world_impact(effects, world, cam.position, end, 5);
            }
        }
        Fx::Punch { hit, .. } => {
            vm.punch();
            audio.play_2d(if *hit { Sfx::PunchHit } else { Sfx::Punch });
        }
        Fx::ThornThrown { .. } => {
            vm.throw_thorn();
            audio.play_2d(Sfx::ThornThrow);
        }
        Fx::Discharge {
            position,
            radius_dm,
            ..
        } => {
            fx::discharge_nova(
                effects,
                beams,
                rng,
                position.to_vec3(),
                *radius_dm as f32 / 10.0,
            );
            audio.play_2d(Sfx::DischargeRelease);
            feedback.shake(0.4);
        }
        Fx::SlamShockwave {
            position,
            power,
            radius_dm,
            ..
        } => {
            let power = *power as f32 / 255.0;
            effects.shockwave(
                position.to_vec3(),
                *radius_dm as f32 / 10.0 * (0.4 + 0.6 * power),
            );
            audio.play(Sfx::SlamImpact, PlayParams::default());
            feedback.shake(0.25 + 0.4 * power);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

/// What the title card shows.
pub struct CardInfo<'a> {
    pub name: &'a str,
    pub title: &'a str,
    pub color: Vec4,
    /// A replay follows (else "REPLAY UNAVAILABLE").
    pub replay: bool,
}

fn ease_out(x: f64) -> f32 {
    let x = x.clamp(0.0, 1.0) as f32;
    1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x)
}

/// The "PLAY OF THE GAME" title card at `age` seconds: the world darkens, the title slams in
/// with an extrusion in the player's colour, the name on a band of that colour slides in from
/// the left, the title of the play from the right; a white wipe into the replay.
pub fn draw_card(list: &mut UiDrawList, t: &Theme, c: Rect, card: &CardInfo, age: f64) {
    let dark = (age / 0.25).clamp(0.0, 1.0) as f32;
    list.fill(c, fade(t.ink, 0.86 * dark));
    let color = card.color;
    let cx = c.center().x;
    let cy = c.center().y;
    // Slow diagonal slashes of the player's colour behind everything.
    for i in 0..5 {
        let w = 26.0 + 14.0 * (i % 3) as f32;
        let x = c.min.x - 120.0 + i as f32 * 150.0 + ((age * 40.0) % 150.0) as f32;
        draw::parallelogram(
            list,
            x.round(),
            c.min.y,
            w,
            c.height(),
            fade(color, 0.07 * dark),
        );
    }
    let hit = age - STINGER_AT;
    if hit < 0.0 {
        // The build-up: a thin line drawing itself across.
        let p = ease_out(age / STINGER_AT);
        draw::hline(
            list,
            cx - 200.0 * p,
            cx + 200.0 * p,
            cy - 40.0,
            fade(color, 0.8 * p),
        );
    } else {
        // The title: pops from scale 6 to 4 (whole scales stay crisp) with an extrusion.
        let title = "PLAY OF THE GAME";
        let scale = if hit < 0.08 {
            6.0
        } else if hit < 0.16 {
            5.0
        } else {
            4.0
        };
        let w = line_width(title, scale);
        let x = (cx - w * 0.5).round();
        let y = (cy - 62.0 - (scale - 4.0) * 4.5).round();
        for k in (1..=5).rev() {
            let o = k as f32;
            list.text(
                vec2(x + o, y + o),
                title,
                &TextStyle::new(mix(color, t.ink, 0.25 + k as f32 / 8.0)).scale(scale),
            );
        }
        list.text(
            vec2(x, y),
            title,
            &TextStyle::new(t.bone).scale(scale).outline(t.ink),
        );
        let rule = ease_out(hit / 0.3);
        draw::slanted(
            list,
            Rect::new(
                vec2(cx - 190.0 * rule, cy - 20.0),
                vec2(cx + 190.0 * rule, cy - 17.0),
            ),
            true,
            color,
        );
        // The name on a band of the player's colour, from the left.
        let n = ease_out((hit - 0.25) / 0.3);
        if n > 0.0 {
            let ns = 3.0;
            let nw = line_width(card.name, ns);
            let band_h = 32.0;
            let band_w = nw + 48.0;
            let bx = (cx - band_w * 0.5 - (1.0 - n) * 140.0).round();
            let by = (cy - 8.0).round();
            draw::parallelogram(list, bx, by, band_w, band_h, fade(color, 0.92 * n));
            draw::hline(list, bx, bx + band_w + band_h, by + band_h, fade(t.ink, n));
            list.text(
                vec2((bx + (band_w + band_h) * 0.5).round(), by + 6.0),
                card.name,
                &TextStyle::new(fade(t.ink, n)).scale(ns).centered(),
            );
            // Speed streaks trailing the band.
            for i in 0..3 {
                let len = 24.0 + i as f32 * 16.0;
                draw::hline(
                    list,
                    bx - 12.0 - len - (1.0 - n) * 60.0,
                    bx - 12.0,
                    by + 8.0 + i as f32 * 8.0,
                    fade(color, 0.6 * n),
                );
            }
        }
        // What the play was, from the right.
        let s = ease_out((hit - 0.55) / 0.3);
        if s > 0.0 && !card.title.is_empty() {
            list.text(
                vec2((cx + (1.0 - s) * 120.0).round(), cy + 34.0),
                card.title,
                &TextStyle::new(fade(t.bone, s))
                    .scale(2.0)
                    .centered()
                    .shadow_by(fade(mix(color, t.ink, 0.5), s), 2.0),
            );
        }
        if !card.replay && hit > 0.9 {
            list.text(
                vec2(cx, cy + 62.0),
                "REPLAY UNAVAILABLE",
                &TextStyle::new(t.ash).centered(),
            );
        }
        // The slam's flash.
        let flash = (1.0 - hit / 0.18).clamp(0.0, 1.0) as f32;
        if flash > 0.0 {
            list.fill(c, fade(Vec4::ONE, 0.35 * flash));
        }
    }
    // The wipe into the replay.
    if card.replay {
        let wipe = ((age - (CARD_SECS - 0.2)) / 0.2).clamp(0.0, 1.0) as f32;
        if wipe > 0.0 {
            list.fill(c, fade(Vec4::ONE, 0.85 * wipe));
        }
    }
    list.text(
        vec2(cx, c.max.y - 16.0),
        "SPACE  SKIP",
        &TextStyle::new(fade(t.ash, dark)).centered(),
    );
}

/// What the replay's HUD shows.
pub struct ReplayHud<'a> {
    pub name: &'a str,
    pub title: &'a str,
    pub color: Vec4,
    pub feed: &'a TextFeed,
    /// Seconds since the viewpoint's last kill (kill marker).
    pub kill_age: Option<f64>,
    /// Seconds since the replay started.
    pub age: f64,
    /// 0..1 fade to black at the end.
    pub fade_out: f32,
    pub dead: bool,
}

/// The replay's HUD: letterbox bars, the kill feed, a crosshair with the kill marker, the
/// "PLAY OF THE GAME // NAME" banner and the title bottom left, the skip hint.
pub fn draw_replay_hud(list: &mut UiDrawList, t: &Theme, c: Rect, h: &ReplayHud, now: f64) {
    let bar = 14.0;
    list.fill(
        Rect::new(c.min, vec2(c.max.x, c.min.y + bar)),
        fade(t.ink, 0.9),
    );
    list.fill(
        Rect::new(vec2(c.min.x, c.max.y - bar), c.max),
        fade(t.ink, 0.9),
    );
    let mut feed = h.feed.clone();
    feed.update(now);
    feed.draw(
        list,
        t,
        vec2(c.max.x - 10.0, c.min.y + bar + 8.0),
        Align::Right,
        1.0,
        now,
    );
    if !h.dead {
        hud_draw::crosshair(list, c.center(), &CrosshairStyle::new(t));
    }
    if let Some(k) = h.kill_age.filter(|k| *k < 0.45) {
        let grow = ease_out(k / 0.12);
        let alpha = (1.0 - (k - 0.25).max(0.0) / 0.2).clamp(0.0, 1.0) as f32;
        kill_marker(
            list,
            c.center(),
            5.0 + 3.0 * grow,
            6.0,
            fade(t.crimson_bright, alpha),
            fade(t.ink, 0.8 * alpha),
        );
    }
    // The banner, sliding in from the left.
    let s = ease_out(h.age / 0.35);
    let top = "PLAY OF THE GAME";
    let width = line_width(top, 1.0)
        .max(line_width(h.name, 1.0) + line_width(top, 1.0) + 24.0)
        .max(line_width(h.title, 2.0))
        + 30.0;
    let x = (c.min.x + 10.0 - (1.0 - s) * (width + 40.0)).round();
    let y = c.max.y - bar - 44.0;
    draw::parallelogram(list, x, y, width, 36.0, fade(t.ink, 0.72));
    draw::hline(list, x, x + width, y + 35.0, h.color);
    list.text(
        vec2(x + 12.0, y + 5.0),
        top,
        &TextStyle::new(h.color).outline(t.ink),
    );
    let after = x + 12.0 + line_width(top, 1.0);
    list.text(
        vec2(after + 6.0, y + 5.0),
        &format!("// {}", h.name),
        &TextStyle::new(t.bone).outline(t.ink),
    );
    list.text(
        vec2(x + 12.0, y + 16.0),
        h.title,
        &TextStyle::new(t.bone).scale(2.0).outline(t.ink),
    );
    list.text(
        vec2(c.max.x - 10.0, c.max.y - 11.0),
        "SPACE  SKIP",
        &TextStyle::new(t.ash).right(),
    );
    if h.fade_out > 0.0 {
        list.fill(c, fade(t.ink, h.fade_out));
    }
}

/// The kill marker: four diagonal strokes around the crosshair.
fn kill_marker(list: &mut UiDrawList, center: Vec2, r0: f32, len: f32, color: Vec4, shadow: Vec4) {
    for (sx, sy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
        for (col, size) in [(shadow, 4.0), (color, 2.0)] {
            for i in 0..len as i32 {
                let d = r0 + i as f32;
                let p = center + vec2(sx * d, sy * d);
                list.fill(
                    Rect::from_center_size(p.floor() + vec2(0.5, 0.5), vec2(size, size)),
                    col,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::loadout::{Ability, WeaponKind};
    use shared::protocol::{QPos, kind, pack_player_weapon};
    use shared::sim::{DamageFlags, HitTarget};

    use super::super::combat::{kill_distance, kill_line_text};
    use super::*;

    const RATE: u32 = 60;
    const P1: PlayerId = PlayerId(1);
    const P2: PlayerId = PlayerId(2);

    /// A 30 Hz frame: P1 runs along +x (dead when `p1_dead`), P2 stands, plus `extra`
    /// projectiles (a busy match).
    fn frame(tick: u32, p1_dead: bool, extra: usize) -> Arc<NetFrame> {
        let mut f = NetFrame::new();
        let x = tick as f32 / RATE as f32 * 10.0;
        let mut flags = player_flags::ON_GROUND;
        if p1_dead {
            flags |= player_flags::DEAD;
        }
        f.insert(&EntityState {
            id: NetId::player(P1),
            kind: kind::PLAYER,
            position: Vec3::new(x, 0.0, 0.0),
            velocity: Vec3::new(10.0, 0.0, 0.0),
            yaw: 1.0,
            flags,
            state: pack_player_weapon(WeaponKind::Arclash, Ability::Discharge, 200),
            ..Default::default()
        });
        f.insert(&EntityState {
            id: NetId::player(P2),
            kind: kind::PLAYER,
            position: Vec3::new(0.0, 0.0, -30.0),
            flags: player_flags::ON_GROUND,
            ..Default::default()
        });
        for i in 0..extra {
            f.insert(&EntityState {
                id: NetId(1024 + i as u32),
                kind: kind::BOMB,
                owner: 1,
                position: Vec3::new(i as f32, 2.0, 0.0),
                velocity: Vec3::new(0.0, 5.0, -20.0),
                aux: tick,
                ..Default::default()
            });
        }
        Arc::new(f)
    }

    fn kill(by: PlayerId, victim: PlayerId, tick: u32, distance: f32) -> KillInfo {
        KillInfo {
            victim: HitTarget::Player(victim),
            victim_kind: kind::PLAYER,
            killer: Owner::Player(by),
            attack: Attack::Quill,
            flags: DamageFlags::HITSCAN | DamageFlags::HEADSHOT,
            part: HitPart::Head,
            position: Vec3::ZERO,
            ricochets: 0,
            killer_airborne: false,
            victim_airborne: false,
            parried_projectile: false,
            killer_health: 100.0,
            killer_effective_max: 100.0,
            overkill: 0.0,
            distance,
            weapon: Some(shared::loadout::WeaponKind::Quill),
            tick,
        }
    }

    fn highlight(start: u32, end: u32) -> Highlight {
        Highlight {
            player: P1,
            start_tick: start,
            end_tick: end,
            score: 300.0,
            title: "DOUBLE KILL".into(),
        }
    }

    /// Records ticks `from..to` every 2 ticks (30 Hz), with a kill event and a shot fx at each
    /// second.
    fn record(rec: &mut Recorder, from: u32, to: u32, dead: impl Fn(u32) -> bool) {
        for tick in (from..to).step_by(2) {
            rec.record_frame(tick, frame(tick, dead(tick), 0));
            if tick % RATE == 0 {
                rec.record_event(tick, GameEvent::Killed(kill(P1, P2, tick, 12.0)));
                rec.record_fx(&[FxEvent {
                    seq: tick,
                    tick,
                    fx: Fx::Shot {
                        shooter: P1,
                        attack: Attack::Quill,
                        origin: QPos::from(Vec3::ZERO),
                        ends: vec![QPos::from(Vec3::NEG_Z * 10.0)],
                    },
                }]);
            }
        }
    }

    #[test]
    fn the_recording_rolls_and_shares_the_sessions_frames() {
        let mut source = SnapshotBuffer::new();
        let mut rec = Recorder::new(RATE);
        assert!(rec.is_empty());
        // Nothing is kept before frames come in (not a PvP match yet).
        rec.record_event(5, GameEvent::Killed(kill(P1, P2, 5, 3.0)));
        assert_eq!(rec.event_count(), 0);
        for tick in (1000..1000 + 30 * RATE).step_by(2) {
            source.insert(tick, frame(tick, false, 0));
            if tick % 20 == 0 {
                rec.record_frames(&source);
            }
        }
        rec.record_frames(&source);
        let (oldest, newest) = rec.span().unwrap();
        assert_eq!(newest, 1000 + 30 * RATE - 2);
        assert_eq!(newest - oldest, ticks(RECORD_SECS, RATE), "20 s kept");
        // Shared with the session's buffer, not copied.
        let (t, f) = source.latest().unwrap();
        assert!(Arc::ptr_eq(f, rec.frames.get(t).unwrap()));
        // A pin keeps older frames; unpinning lets them go.
        rec.pin(Some(oldest));
        for tick in (newest + 2..newest + 2 + 4 * RATE).step_by(2) {
            rec.record_frame(tick, frame(tick, false, 0));
        }
        assert_eq!(rec.span().unwrap().0, oldest);
        rec.pin(None);
        assert!(rec.span().unwrap().0 > oldest);
        rec.clear();
        assert!(rec.is_empty() && rec.frame_count() == 0);
    }

    #[test]
    fn a_mark_cuts_the_highlight_with_its_lead_in_and_tail() {
        let mut rec = Recorder::new(RATE);
        record(&mut rec, 0, 30 * RATE, |_| false);
        // A kill burst from 20 s to 23 s, marked 2.5 s after it.
        let h = highlight(20 * RATE, 23 * RATE);
        let clip = rec.cut(&h).expect("recorded");
        assert!(clip.is_of(&h));
        assert_eq!(clip.from, h.start_tick - ticks(LEAD_SECS, RATE));
        assert_eq!(clip.to, h.end_tick + ticks(TAIL_SECS, RATE));
        assert!((clip.secs() - (3.0 + LEAD_SECS + TAIL_SECS)).abs() < 0.01);
        // Frames from a view lag before the start; events and fx of the stretch only, in order.
        let (first, _) = clip.frames.oldest().unwrap();
        assert!(first < clip.from && clip.from - first <= ticks(VIEW_LAG_SECS + 0.2, RATE));
        // 18 s .. 24.5 s: the kills (one a second) from 18 s to 24 s.
        let kills: Vec<u32> = clip.events.iter().map(|e| e.tick / RATE).collect();
        assert_eq!(kills, vec![18, 19, 20, 21, 22, 23, 24]);
        assert!(clip.fx.windows(2).all(|w| w[0].tick <= w[1].tick));
        assert_eq!(clip.fx.len(), 7);
        // The viewpoint's pose is interpolated between the 30 Hz frames.
        let at = 21 * RATE + 1;
        let pose = clip.viewpoint(at as f64).expect("P1 in the clip");
        assert!((pose.position.x - at as f32 / RATE as f32 * 10.0).abs() < 0.01);
        // The Arclash with Discharge (its pool's second ability); no charge shown unless
        // CHARGING.
        assert_eq!(
            held_weapon(&pose),
            (WeaponKind::Arclash, Ability::Discharge, 0.0)
        );
        let charging = EntityState {
            flags: pose.flags | player_flags::CHARGING,
            ..pose
        };
        let (_, _, charge) = held_weapon(&charging);
        assert!((charge - 200.0 / 255.0).abs() < 0.01, "{charge}");
        // Others sampled without the viewpoint.
        let mut others = Vec::new();
        clip.sample(at as f64, Some(NetId::player(P1)), &mut others);
        assert_eq!(others.len(), 1);
    }

    #[test]
    fn the_lead_in_starts_after_a_respawn_and_the_tail_ends_at_a_death() {
        let mut rec = Recorder::new(RATE);
        // P1 dead until 19.5 s, dead again from 24 s.
        let dead = |t: u32| !(19 * RATE + RATE / 2..24 * RATE).contains(&t);
        record(&mut rec, 0, 30 * RATE, dead);
        let h = highlight(20 * RATE, 23 * RATE);
        let clip = rec.cut(&h).unwrap();
        assert_eq!(clip.from, 19 * RATE + RATE / 2, "from the respawn");
        assert!(clip.to > 24 * RATE && clip.to <= h.end_tick + ticks(TAIL_SECS, RATE));
    }

    #[test]
    fn nothing_is_cut_without_the_highlight_in_the_recording() {
        let mut rec = Recorder::new(RATE);
        assert!(rec.cut(&highlight(10, 20)).is_none(), "nothing recorded");
        // Joined at 15 s: a highlight from 12 s is not (fully) there.
        record(&mut rec, 15 * RATE, 30 * RATE, |_| false);
        assert!(rec.cut(&highlight(12 * RATE, 16 * RATE)).is_none());
        // One still to come (the tail is cut short, never invented).
        assert!(rec.cut(&highlight(29 * RATE, 40 * RATE)).is_none());
        let late = rec.cut(&highlight(28 * RATE, 29 * RATE)).expect("recorded");
        assert!(late.to <= 30 * RATE);
    }

    #[test]
    fn twenty_seconds_of_a_busy_match_stay_within_a_few_megabytes() {
        let mut rec = Recorder::new(RATE);
        // 8 players' worth of projectiles in flight all the time, a kill every second.
        for tick in (0..40 * RATE).step_by(2) {
            rec.record_frame(tick, frame(tick, false, 40));
            rec.record_fx(&[FxEvent {
                seq: tick,
                tick,
                fx: Fx::Shot {
                    shooter: P1,
                    attack: Attack::Blast,
                    origin: QPos::from(Vec3::ZERO),
                    ends: vec![QPos::from(Vec3::ONE); 12],
                },
            }]);
        }
        assert_eq!(rec.frame_count(), ticks(RECORD_SECS, RATE) as usize / 2 + 1);
        let bytes = rec.approx_bytes();
        println!("20 s recording: {:.2} MB", bytes as f64 / (1 << 20) as f64);
        assert!(bytes < 4 << 20, "{bytes} bytes");
        assert!(
            bytes > 1 << 20,
            "{bytes} bytes (the estimate counts the frames)"
        );
    }

    #[test]
    fn slow_motion_eases_in_before_the_last_kill_and_out_after() {
        let end = 1000;
        let at = |secs: f64| replay_speed(end as f64 + secs * RATE as f64, end, RATE);
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(at(-2.0), 1.0));
        assert!(near(at(0.0), SLOWMO_SPEED));
        assert!(near(at(0.5), SLOWMO_SPEED));
        assert!(near(at(2.0), 1.0));
        let ramp_in = at(-SLOWMO_BEFORE - SLOWMO_RAMP * 0.5);
        assert!(ramp_in > SLOWMO_SPEED && ramp_in < 1.0);
        let ramp_out = at(SLOWMO_AFTER + SLOWMO_RAMP * 0.5);
        assert!(ramp_out > SLOWMO_SPEED && ramp_out < 1.0);
    }

    #[test]
    fn server_text_is_shown_safely() {
        assert_eq!(display_text("LONGSHOT 87m"), "LONGSHOT 87m");
        assert_eq!(display_text("\u{202E}TRIPLE\u{7} KILL "), "TRIPLE KILL");
        assert_eq!(display_text(&"X".repeat(200)).len(), MAX_TEXT);
    }

    #[test]
    fn kill_feed_lines_show_the_distance_when_the_killer_has_a_body() {
        let k = kill(P1, P2, 0, 38.4);
        assert_eq!(kill_distance(&k), Some(38));
        assert_eq!(
            kill_line_text("YOU", "QUILL", "GNAWER", &k),
            "YOU [QUILL] GNAWER 38m HEADSHOT"
        );
        // Point blank still reads as a distance.
        let mut close = kill(P1, P2, 0, 0.3);
        close.flags = DamageFlags::MELEE;
        assert_eq!(
            kill_line_text("GNAWER", "BITE", "YOU", &close),
            "GNAWER [BITE] YOU 1m"
        );
        // No body (the world, a dev command), no distance; nor for a suicide.
        let mut world = kill(P1, P2, 0, 0.0);
        world.killer = Owner::World;
        world.flags = DamageFlags::ENVIRONMENT;
        assert_eq!(kill_distance(&world), None);
        assert_eq!(
            kill_line_text("THE PIT", "WORLD", "BOB", &world),
            "THE PIT [WORLD] BOB"
        );
        let mut suicide = kill(P1, P1, 0, 0.0);
        suicide.flags = DamageFlags::EXPLOSIVE | DamageFlags::SELF;
        assert_eq!(kill_distance(&suicide), None);
        let mut trick = kill(P2, P1, 0, 61.6);
        trick.ricochets = 2;
        trick.killer_airborne = true;
        assert_eq!(
            kill_line_text("RIVET_WITCH", "QUILL", "YOU", &trick),
            "RIVET_WITCH [QUILL] YOU 62m HEADSHOT RICOCHET x2 AIR"
        );
    }

    #[test]
    fn kill_feed_brackets_name_the_weapon_and_its_ability() {
        use super::super::combat::weapon_attack_label;
        assert_eq!(
            weapon_attack_label(WeaponKind::Longwick, Attack::Longwick),
            "LONGWICK"
        );
        assert_eq!(
            weapon_attack_label(WeaponKind::Riveter, Attack::Volley),
            "RIVETER VOLLEY"
        );
        assert_eq!(
            weapon_attack_label(WeaponKind::Longwick, Attack::Brand),
            "LONGWICK BRAND"
        );
        assert_eq!(
            weapon_attack_label(WeaponKind::Quill, Attack::Quill),
            "QUILL"
        );
    }

    #[test]
    fn the_viewpoints_own_actions_play_in_first_person() {
        let shot = Fx::Shot {
            shooter: P1,
            attack: Attack::Quill,
            origin: QPos::from(Vec3::ZERO),
            ends: Vec::new(),
        };
        assert!(own_fx(&shot, P1));
        assert!(!own_fx(&shot, P2));
        assert!(own_fx(&Fx::ThornThrown { player: P1 }, P1));
        let boom = Fx::Explosion {
            position: QPos::from(Vec3::ZERO),
            radius_dm: 40,
            attack: Attack::Tumble,
            owner: 1,
        };
        assert!(!own_fx(&boom, P1), "explosions play in the world");
    }

    #[test]
    fn replicated_movement_drives_the_camera_state() {
        let e = EntityState {
            id: NetId::player(P1),
            kind: kind::PLAYER,
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(20.0, 0.0, 0.0),
            flags: player_flags::ON_GROUND | player_flags::SLIDING | player_flags::LOW_PROFILE,
            ..Default::default()
        };
        let s = move_state(&e);
        assert_eq!(s.position, e.position);
        assert!(s.on_ground && s.crouched);
        assert_eq!(s.mode, MoveMode::Slide);
        let air = move_state(&EntityState { flags: 0, ..e });
        assert_eq!(air.mode, MoveMode::Air);
    }
}
