//! Spectating a teammate while you are out of the fight in the PvE modes (campaign co-op,
//! grind; docs/style_modes.md "Spectating").
//!
//! While the local player is dead, downed or out and a teammate is up, the camera moves to a
//! teammate after a moment of your own death view ([`SPECTATE_DELAY`]; FIRE / ALT_FIRE skip
//! it): from their eyes by default — their camera feel (eye height, landing dips, slides) from
//! a [`CameraRig`] of our own, their held weapon drawn as a viewmodel from the replicated
//! player entity and kicked by their shots — or, JUMP toggles it, a third-person chase camera
//! that the mouse / right stick orbits round them and that walls pull in, so it never looks
//! through the level. FIRE / ALT_FIRE cycle the teammates (gamepad: the triggers). When the
//! one you watch goes down too it moves on to the next one up; with nobody left (a team wipe,
//! solo) it is the death view again. The audio listener follows the view (it is
//! `InGame::last_camera`). Being revived or respawning ends it on the frame the server says
//! you are back: the camera is your own again at once.
//!
//! Who to watch is pure ([`first_target`], [`cycle`], [`Spectate::step`]) and tested in
//! `client/tests/client_spectate.rs`; the rest hooks into the frame from `ingame/mod.rs`
//! (controls, camera, the world and HUD draws) and `combat_hud` (no death screen over the
//! view).

use engine::gfx::{Camera, DrawItem, UiDrawList};
use engine::input::InputState;
use engine::physics::{CollisionWorld, MASK_SHOTS};
use engine::ui::draw::{self, fade};
use engine::ui::{Rect, TextStyle, Theme, UiDrawExt};
use glam::{Vec2, Vec3, vec2};
use shared::loadout::{Ability, WeaponKind};
use shared::lobby::GameMode;
use shared::protocol::{EntityState, NetId, kind, player_armed, player_flags};
use shared::sim::PlayerId;
use shared::tuning::MovementTuning;

use super::InGame;
use super::camera::{CameraFrame, CameraRig, look_rotation};
use super::cues;
use super::feedback::ShakeOffset;
use super::remote_players::{PlayerVisual, player_color};
use super::replay::{held_weapon, move_state};
use super::viewmodel::Viewmodel;
use crate::app::Common;
use crate::controls::{Action, Bindings, is_gamepad_binding};
use crate::models::ModelLibrary;
use crate::net::Session;
use crate::settings::Settings;

/// Seconds of your own death view before the camera moves to a teammate (FIRE / ALT_FIRE
/// skip the wait).
pub const SPECTATE_DELAY: f64 = 1.5;
/// Chase camera: how far behind the teammate's head it sits ...
pub const CHASE_DISTANCE: f32 = 3.6;
/// ... the sphere it sweeps from their head to stay off walls ...
const CHASE_RADIUS: f32 = 0.25;
/// ... its pivot above their feet (standing / sliding or crouched) ...
const CHASE_PIVOT: f32 = 1.55;
const CHASE_PIVOT_LOW: f32 = 0.9;
/// ... the pitch it starts at (looking down a little) and its pitch range ...
const CHASE_PITCH: f32 = -0.3;
const CHASE_PITCH_RANGE: (f32, f32) = (-1.3, 1.0);
/// ... and how fast it eases back out after a wall pulled it in (1/s; walls pull it in at
/// once).
const CHASE_EASE_OUT: f32 = 4.0;
/// Closer than this the chase camera is inside them: their model is hidden.
const CHASE_HIDE_DISTANCE: f32 = 0.9;

/// Is this replicated player up (not dead, downed or out)?
pub fn is_up(e: &EntityState) -> bool {
    e.kind == kind::PLAYER && e.flags & (player_flags::DEAD | player_flags::DOWNED) == 0
}

/// The teammates who are up (the remote players: in PvE everyone is a teammate), by player id.
pub fn living_teammates(remote: &[EntityState]) -> Vec<PlayerId> {
    let mut ids: Vec<PlayerId> = remote
        .iter()
        .filter(|e| is_up(e))
        .filter_map(|e| e.id.as_player())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Who to watch first: the teammate nearest to where you went down (the likeliest to come and
/// revive you), ties by player id; without your position the first by id.
pub fn first_target(remote: &[EntityState], from: Option<Vec3>) -> Option<PlayerId> {
    remote
        .iter()
        .filter(|e| is_up(e))
        .filter_map(|e| {
            let id = e.id.as_player()?;
            let d = from.map_or(0.0, |p| p.distance_squared(e.position));
            Some((d, id))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, id)| id)
}

/// The teammate who is up after (`step` > 0) or before (`step` < 0) `current` in player id
/// order, wrapping round; `current` itself may be down or gone (its place in the order still
/// counts). `None` when nobody is up.
pub fn cycle(remote: &[EntityState], current: Option<PlayerId>, step: i32) -> Option<PlayerId> {
    let ids = living_teammates(remote);
    let first = *ids.first()?;
    let last = *ids.last()?;
    let Some(cur) = current else {
        return Some(if step < 0 { last } else { first });
    };
    if step < 0 {
        Some(
            ids.iter()
                .rev()
                .copied()
                .find(|id| *id < cur)
                .unwrap_or(last),
        )
    } else {
        Some(ids.iter().copied().find(|id| *id > cur).unwrap_or(first))
    }
}

/// The spectating presses of a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpectateInput {
    /// FIRE: the next teammate (or start watching at once during the death view).
    pub next: bool,
    /// ALT_FIRE: the previous one (likewise).
    pub prev: bool,
    /// JUMP: first person ⇄ chase camera.
    pub toggle: bool,
}

/// Spectating state of the game screen (`InGame::spectate`).
#[derive(Default)]
pub struct Spectate {
    /// Who we watch (`None`: not spectating, the local view).
    target: Option<PlayerId>,
    /// Third-person chase camera instead of their eyes (kept across targets and deaths).
    chase: bool,
    /// The local player may spectate this frame (PvE, out of the fight).
    eligible: bool,
    /// When the local player went out (the `InGame` clock).
    out_since: Option<f64>,
    /// Presses since the last update.
    pending: SpectateInput,
    /// Chase camera orbit (yaw, pitch) and its current distance; `fresh`: put it behind them
    /// (a new target, or the chase camera just turned on).
    orbit: Vec2,
    distance: f32,
    fresh: bool,
    /// Their eyes: camera feel and the weapon in their hands.
    camera: CameraRig,
    viewmodel: Viewmodel,
    weapon: Option<(WeaponKind, Ability)>,
    on_ground: bool,
    fall_speed: f32,
    /// Their `firing` / `punching` animation last frame (a rise is a new shot / punch).
    last_firing: f32,
    last_punching: f32,
    /// Their melee swing's progress last frame (`Fx::Swing` restarts it: a new swing).
    last_swing: f32,
    /// The device the controls hint names: the gamepad (`true`) or the keyboard and mouse,
    /// the last one used to spectate, else a gamepad if one is connected.
    pad_hints: Option<bool>,
    pad_connected: bool,
}

impl Spectate {
    pub fn new() -> Self {
        Self::default()
    }

    /// The teammate being watched.
    pub fn target(&self) -> Option<PlayerId> {
        self.target
    }

    pub fn is_active(&self) -> bool {
        self.target.is_some()
    }

    /// Watching from behind them (JUMP toggles) rather than through their eyes.
    pub fn chase(&self) -> bool {
        self.chase
    }

    /// The teammate whose model isn't drawn: watched from their eyes, or the chase camera
    /// pulled right into them.
    pub fn hidden_player(&self) -> Option<PlayerId> {
        self.target
            .filter(|_| !self.chase || self.distance < CHASE_HIDE_DISTANCE)
    }

    /// One frame of who to watch. `eligible`: PvE and the local player is dead, downed or out;
    /// `remote`: the remote players; `local`: where the local player is (lies); `now`: the
    /// game clock. Starts [`SPECTATE_DELAY`] after going out (or at once on a press) with the
    /// nearest teammate who is up, cycles on FIRE / ALT_FIRE, moves on when the watched one
    /// goes down or leaves, falls back to the death view when nobody is up and stops the
    /// moment the local player is back. Returns whether the target changed.
    pub fn step(
        &mut self,
        eligible: bool,
        remote: &[EntityState],
        local: Option<Vec3>,
        now: f64,
        input: SpectateInput,
    ) -> bool {
        let before = self.target;
        self.eligible = eligible;
        if !eligible {
            self.target = None;
            self.out_since = None;
            return before.is_some();
        }
        let since = *self.out_since.get_or_insert(now);
        // The one we watched went down or left: the next one up (or nobody).
        if let Some(t) = self.target
            && !remote.iter().any(|e| e.id == NetId::player(t) && is_up(e))
        {
            self.target = cycle(remote, Some(t), 1);
        }
        match self.target {
            None => {
                if now - since >= SPECTATE_DELAY || input.next || input.prev {
                    self.target = first_target(remote, local);
                }
            }
            Some(t) if input.next => self.target = cycle(remote, Some(t), 1),
            Some(t) if input.prev => self.target = cycle(remote, Some(t), -1),
            Some(_) => {}
        }
        if input.toggle && self.target.is_some() {
            self.chase = !self.chase;
            self.distance = 0.0;
            self.fresh = true;
        }
        self.target != before
    }

    /// A new teammate to watch: their camera and weapon start fresh, the chase camera behind
    /// them.
    fn retarget(&mut self) {
        self.camera = CameraRig::new();
        self.viewmodel = Viewmodel::new();
        self.viewmodel.on_switch();
        self.weapon = None;
        self.on_ground = true;
        self.fall_speed = 0.0;
        self.last_firing = 0.0;
        self.last_punching = 0.0;
        self.distance = 0.0;
        self.fresh = true;
    }

    /// Turns the chase camera by a look delta (yaw, pitch).
    fn orbit_by(&mut self, delta: Vec2) {
        if self.chase {
            self.orbit.x += delta.x;
            self.orbit.y = (self.orbit.y + delta.y).clamp(CHASE_PITCH_RANGE.0, CHASE_PITCH_RANGE.1);
        }
    }

    /// The view this frame of watching `e` (`vis`: its animation state, for the shots that
    /// kick the viewmodel). Advances the viewmodel too.
    #[allow(clippy::too_many_arguments)]
    fn view(
        &mut self,
        e: &EntityState,
        vis: Option<PlayerVisual>,
        world: Option<&CollisionWorld>,
        tuning: &MovementTuning,
        settings: &Settings,
        dt: f32,
        vdt: f32,
    ) -> Camera {
        // Their eyes (the replay's way: a movement state from the replicated flags).
        let state = move_state(e);
        if state.on_ground && !self.on_ground {
            self.camera.on_landed(self.fall_speed, tuning);
        }
        self.on_ground = state.on_ground;
        self.fall_speed = (-e.velocity.y).max(0.0);
        let eyes = self.camera.update(&CameraFrame {
            feet: e.position,
            state: &state,
            tuning,
            yaw: e.yaw,
            pitch: e.pitch,
            settings,
            shake: ShakeOffset::default(),
            downed: false,
            dt,
        });
        // The weapon in their hands, kicked by their shots and punches.
        let (weapon, ability, charge) = held_weapon(e);
        let drawing = e.flags & player_flags::DRAWING != 0;
        if self.weapon.is_some_and(|w| w != (weapon, ability)) {
            self.viewmodel.on_switch();
        }
        self.weapon = Some((weapon, ability));
        self.viewmodel.set_draw(if drawing { charge } else { 0.0 });
        self.viewmodel
            .set_charge(if drawing { 0.0 } else { charge });
        self.viewmodel.set_unarmed(!player_armed(e.state));
        if let Some(v) = vis {
            if v.firing > self.last_firing + 1e-3 {
                self.viewmodel.fire(1.0);
                if weapon == WeaponKind::Tumbler {
                    self.viewmodel.turn_drum();
                }
            }
            if v.punching > 0.0 && (self.last_punching == 0.0 || v.punching < self.last_punching) {
                self.viewmodel.punch();
            }
            // Their melee swings (`Fx::Swing` starts the visual's swing: combat/melee.rs).
            if v.swing.active() && (self.last_swing == 0.0 || v.swing.progress < self.last_swing) {
                self.viewmodel.swing(v.swing.kind, v.swing.left);
            }
            self.last_swing = if v.swing.active() {
                v.swing.progress
            } else {
                0.0
            };
            self.last_firing = v.firing;
            self.last_punching = v.punching;
        }
        self.viewmodel
            .update(vdt, &state, e.yaw, e.pitch, settings.view_bob);
        if !self.chase {
            return eyes;
        }
        if std::mem::take(&mut self.fresh) {
            self.orbit = vec2(e.yaw, CHASE_PITCH);
        }

        // Behind them: orbit round their head, pulled in by walls at once, easing back out.
        let low = e.flags & player_flags::LOW_PROFILE != 0;
        let pivot = e.position + Vec3::Y * if low { CHASE_PIVOT_LOW } else { CHASE_PIVOT };
        let rotation = look_rotation(self.orbit.x, self.orbit.y);
        let back = rotation * Vec3::Z;
        let room = world
            .and_then(|w| w.cast_sphere(pivot, CHASE_RADIUS, back * CHASE_DISTANCE, MASK_SHOTS))
            .map_or(CHASE_DISTANCE, |hit| hit.fraction * CHASE_DISTANCE)
            .max(0.0);
        self.distance = if room <= self.distance {
            room
        } else {
            let k = 1.0 - (-CHASE_EASE_OUT * dt.clamp(0.0, 0.1)).exp();
            self.distance + (room - self.distance) * k
        };
        Camera {
            position: pivot + back * self.distance,
            rotation,
            fov_y_degrees: settings.vertical_fov(0.0),
            near: 0.05,
            far: 600.0,
        }
    }

    /// Draws the watched teammate's weapon into the viewmodel pass (first person only).
    /// Returns whether it did (the local player's own stays away then).
    pub fn draw_viewmodel(&mut self, lib: Option<&ModelLibrary>, out: &mut Vec<DrawItem>) -> bool {
        let (Some(target), false) = (self.target, self.chase) else {
            return false;
        };
        let (weapon, ability) = self.weapon.unwrap_or_default();
        self.viewmodel.draw(
            lib,
            weapon,
            ability,
            player_color(target),
            self.camera.dip(),
            self.camera.roll(),
            out,
        );
        true
    }
}

/// The name of an action's first binding on the gamepad (`pad`) or the keyboard / mouse
/// (whatever it has if it has none there).
fn key_label(bindings: &Bindings, action: Action, pad: bool) -> String {
    let list = bindings.get(action);
    list.iter()
        .find(|b| is_gamepad_binding(b) == pad)
        .or(list.first())
        .map_or_else(|| "-".into(), |b| b.name().to_uppercase())
}

/// What the spectating HUD shows.
pub struct SpectateHud<'a> {
    pub name: &'a str,
    pub player: PlayerId,
    /// Their health and the maximum (the replicated entity rounds it up).
    pub health: f32,
    pub max_health: f32,
    pub chase: bool,
    /// Seconds to the local player's respawn while dead (not downed; nor out for a wave).
    pub respawn_in: Option<f32>,
    pub next: &'a str,
    pub prev: &'a str,
    pub toggle: &'a str,
}

/// Under the crosshair, above the downed panel (`mode_hud`; the bottom corners are the HUD's
/// health and weapons): SPECTATING and their name, their wick, the controls; while dead the
/// respawn countdown where the downed panel would be (the death screen's countdown: the view
/// replaces that screen).
pub fn draw_hud(list: &mut UiDrawList, t: &Theme, h: &SpectateHud) {
    let c = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
    let outline = fade(t.ink, 0.9);
    let cx = c.center().x;
    let y = c.max.y - 158.0;
    let color = player_color(h.player);
    let panel = Rect::new(vec2(cx - 135.0, y - 4.0), vec2(cx + 135.0, y + 31.0));
    draw::slanted(list, panel, true, fade(t.ink, 0.5));
    draw::hline(list, panel.min.x + 10.0, panel.max.x, panel.min.y, color);
    // "SPECTATING" small, then the name big in their colour, centred together.
    let label = "SPECTATING ";
    let name = engine::ui::text::truncate(h.name, 180.0, 2.0);
    let (lw, nw) = (
        engine::ui::text::line_width(label, 1.0),
        engine::ui::text::line_width(&name, 2.0),
    );
    let x = (cx - (lw + 8.0 + nw) * 0.5).round();
    list.text(
        vec2(x, y + 6.0),
        label,
        &TextStyle::new(t.bone_dim).outline(outline),
    );
    cues::player_swatch(
        list,
        vec2(x + lw, y),
        vec2(3.0, 14.0),
        h.player.0 as usize,
        color,
        fade(t.ink, 0.85),
    );
    list.text(
        vec2(x + lw + 8.0, y),
        &name,
        &TextStyle::new(color).scale(2.0).outline(outline),
    );
    // Their wick.
    let bar = Rect::from_center_size(vec2(cx, y + 17.0), vec2(90.0, 2.0));
    let frac = (h.health / h.max_health.max(1.0)).clamp(0.0, 1.0);
    draw::fill(list, bar.expand(1.0), fade(t.ink, 0.85));
    draw::fill(list, bar, fade(t.bone, 0.2));
    let filled = vec2(bar.min.x + (bar.width() * frac).round(), bar.max.y);
    let wick = if frac < 0.35 {
        t.crimson_bright
    } else {
        t.bone
    };
    draw::fill(list, Rect::new(bar.min, filled), wick);
    let view = if h.chase {
        "FIRST PERSON"
    } else {
        "THIRD PERSON"
    };
    list.text(
        vec2(cx, y + 21.0),
        &format!(
            "[{}] NEXT  [{}] PREVIOUS  [{}] {view}",
            h.next, h.prev, h.toggle
        ),
        &TextStyle::new(t.bone_dim).centered().outline(outline),
    );
    if let Some(secs) = h.respawn_in {
        list.text(
            vec2(cx, c.max.y - 120.0),
            &format!("RESPAWNING IN {secs:.1}"),
            &TextStyle::new(t.bone)
                .scale(2.0)
                .centered()
                .outline(outline),
        );
    }
}

// ---- glue with the in-game screen (called from `ingame/mod.rs`) ------------------------------

impl InGame {
    /// Spectating controls, after this frame's mouse / stick look (`view`: the look angles
    /// before it). FIRE / ALT_FIRE / JUMP pick and toggle; the look turns the chase camera and
    /// leaves your own view where it was, so you get up looking where you lay looking.
    pub(super) fn spectate_controls(&mut self, common: &Common, input: &InputState, view: Vec2) {
        let s = &mut self.spectate;
        if !s.eligible {
            return;
        }
        let bindings = &common.settings.bindings;
        s.pad_connected = input.gamepad_connected();
        let mut pressed = |action: Action| {
            let hit = bindings
                .get(action)
                .iter()
                .find(|b| input.pressed(**b))
                .copied();
            if let Some(b) = hit {
                s.pad_hints = Some(is_gamepad_binding(&b));
            }
            hit.is_some()
        };
        let next = pressed(Action::Fire);
        let prev = pressed(Action::AltFire);
        let toggle = pressed(Action::Jump);
        s.pending.next |= next;
        s.pending.prev |= prev;
        s.pending.toggle |= toggle;
        if s.is_active() {
            let now = vec2(self.controls.yaw, self.controls.pitch);
            s.orbit_by(now - view);
            self.controls.set_view(view.x, view.y);
        }
    }

    /// Who to watch and from where, after the local camera is placed: while spectating it
    /// replaces `last_camera` (so the audio listener follows it too).
    pub(super) fn update_spectate(&mut self, common: &Common, dt: f32, vdt: f32) {
        let pve = matches!(
            self.modes.mode(),
            Some(GameMode::Campaign | GameMode::Grind)
        );
        let open = !self.in_warmup() && !self.results_open();
        let InGame {
            session,
            spectate,
            remote,
            last_camera,
            time,
            ..
        } = self;
        let Some(net) = session.as_ref().map(Session::client) else {
            spectate.step(false, &[], None, *time, SpectateInput::default());
            return;
        };
        let out = net.own_combat().is_some_and(|c| c.combat.dead);
        let eligible = pve && open && out && net.is_in_game();
        let players: Vec<EntityState> = net.remote_players().copied().collect();
        let local = net.local_state().map(|s| s.position);
        let input = std::mem::take(&mut spectate.pending);
        let changed = spectate.step(eligible, &players, local, *time, input);
        let Some(target) = spectate.target else {
            return;
        };
        let Some(e) = players.iter().find(|e| e.id == NetId::player(target)) else {
            return;
        };
        if changed {
            spectate.retarget();
        }
        let world = net.world().map(|w| &w.collision.world);
        let vis = remote.visual(e.id).copied();
        *last_camera = spectate.view(
            e,
            vis,
            world,
            &net.tuning().movement,
            &common.settings,
            dt,
            vdt,
        );
    }

    /// Your own body, drawn while you watch a teammate (it lies where the teammate coming to
    /// revive you sees it): the local player's replicated entity, else nothing.
    pub(super) fn spectate_own_body(&mut self) -> Option<EntityState> {
        if !self.spectate.is_active() {
            return None;
        }
        let net = self.session.as_ref().map(Session::client)?;
        let id = NetId::player(net.local_player_id()?);
        let e = net.frames().latest().and_then(|(_, f)| f.get(id))?;
        let vis = self.remote.visual_mut(id);
        vis.downed = e.flags & player_flags::DOWNED != 0;
        vis.dead = e.flags & player_flags::DEAD != 0;
        Some(e)
    }

    /// The spectating HUD (after the mode HUD).
    pub(super) fn draw_spectate_hud(&self, common: &Common, list: &mut UiDrawList) {
        let Some(target) = self.spectate.target else {
            return;
        };
        let Some(net) = self.net() else {
            return;
        };
        let e = net.remote_entity(NetId::player(target));
        let name = net
            .player_name(target)
            .map_or_else(|| target.to_string(), str::to_uppercase);
        let rate = net.tick_rate().max(1) as f32;
        let respawn_in = net
            .own_combat()
            .filter(|_| !self.local_downed())
            .map(|c| c.combat.respawn_in as f32 / rate)
            .filter(|s| *s > 0.0 && *s < 600.0);
        let bindings = &common.settings.bindings;
        let pad = self
            .spectate
            .pad_hints
            .unwrap_or(self.spectate.pad_connected);
        let (next, prev, toggle) = (
            key_label(bindings, Action::Fire, pad),
            key_label(bindings, Action::AltFire, pad),
            key_label(bindings, Action::Jump, pad),
        );
        draw_hud(
            list,
            &common.theme,
            &SpectateHud {
                name: &name,
                player: target,
                health: e.map_or(0.0, |e| e.health as f32),
                max_health: net.tuning().combat.max_health,
                chase: self.spectate.chase,
                respawn_in,
                next: &next,
                prev: &prev,
                toggle: &toggle,
            },
        );
    }
}
