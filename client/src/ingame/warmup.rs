//! The warm-up on the game screen (docs/lobby.md "Warm-up"): while the server is in its lobby
//! phase with the warm-up hub running, the game screen *is* the lobby.
//!
//! - **Lobby overlay**: the same [`LobbyPanel`] as the lobby screen, over the world. It opens by
//!   itself the first time you arrive (not for `--bot` clients); TAB / Select (the scoreboard
//!   binding) or the WARM UP button closes it, TAB opens it again. While it is open you stand
//!   still and the cursor is free. CUSTOMISE opens the customise sub-screen with the preview in
//!   front of the world.
//! - **Ready key** (`Action::Ready`, F1 / d-pad down) toggles ready without opening anything.
//! - **HUD card** (top left): WARM-UP, the next match, a dot per player (identity colour, filled
//!   when ready), your state and the keys; a **countdown banner** on top of everything.
//! - **The hub itself** ([`HubView`]): the ready board in the nave lists who is ready (text
//!   meshes in the world, rebuilt when it changes), the practice console's sign names the next
//!   pack, standing at the console shows a `USE` prompt, and a summon announces the pack.
//!
//! Nothing here runs outside the warm-up: matches, single-player and lobbies without a world
//! behave as before.

use engine::audio::Sfx;
use engine::gfx::{DrawItem, MeshData, MeshId, UiDrawList, Vertex};
use engine::render::Renderer;
use engine::ui::draw::{self, fade};
use engine::ui::font;
use engine::ui::{Rect, TextStyle, Ui, UiDrawExt};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3, Vec4, vec2};
use shared::level::{Encounter, LevelDef, LevelKind};
use shared::lobby::{LobbyPhase, LobbyState};
use shared::protocol::GameEvent;
use shared::sim::warmup::is_console;

use super::InGame;
use super::remote_players::player_color;
use crate::app::Common;
use crate::controls::Action;
use crate::frame::{FrameBuffers, FrameIo, Transition};
use crate::menus::lobby::{self, LobbyAction, LobbyPanel, PanelMode};
use crate::models::hub::{board, console};
use crate::net::{NetClient, Session};

/// The board's glyph height (m) and the console sign's.
const BOARD_GLYPH_H: f32 = 0.18;
const HEADER_GLYPH_H: f32 = 0.2;
const SIGN_GLYPH_H: f32 = 0.072;
/// Model files the hub places as props (the client writes on them).
const BOARD_MODEL: &str = "prop_ready_board.gltf";
const CONSOLE_MODEL: &str = "prop_console.gltf";

/// The warm-up's state on the game screen.
pub struct WarmupUi {
    pub panel: LobbyPanel,
    /// The lobby overlay is open.
    pub open: bool,
    /// We have been in the warm-up during this session (the overlay opens by itself once).
    seen: bool,
    /// Open by itself on arrival (off for `--bot` clients).
    auto_open: bool,
    hub: HubView,
}

impl WarmupUi {
    pub fn new(panel: LobbyPanel, auto_open: bool) -> Self {
        Self {
            panel,
            open: false,
            seen: false,
            auto_open,
            hub: HubView::default(),
        }
    }

    pub fn free(&mut self, renderer: &mut Renderer) {
        self.hub.free(renderer);
    }
}

/// A line of world text: where it starts on the surface (u = right, v = up from the bottom of
/// the glyphs, metres), its colour and glyph height; `centered` around `u`.
#[derive(Clone, Debug, PartialEq)]
struct TextLine {
    text: String,
    u: f32,
    v: f32,
    glyph_h: f32,
    color: Vec4,
    centered: bool,
}

/// Glyph quads of `lines` on a surface facing -Z at depth `z` (model space of a prop: the
/// reader's right is -X), textured with the UI font atlas.
fn text_mesh(lines: &[TextLine], z: f32) -> MeshData {
    let mut mesh = MeshData::default();
    for line in lines {
        let s = line.glyph_h / font::GLYPH_H;
        let advance = font::GLYPH_ADVANCE * s;
        let width = line.text.chars().count() as f32 * advance;
        let mut u = if line.centered {
            line.u - width * 0.5
        } else {
            line.u
        };
        for c in line.text.chars() {
            if c != ' ' {
                let (uv0, uv1) = font::glyph_uv(c);
                let (u0, u1) = (u, u + font::GLYPH_W * s);
                let (v0, v1) = (line.v, line.v + line.glyph_h);
                let base = mesh.vertices.len() as u32;
                // Counter-clockwise seen from the front (-Z).
                for (x, y, uv) in [
                    (u0, v0, Vec2::new(uv0.x, uv1.y)),
                    (u1, v0, Vec2::new(uv1.x, uv1.y)),
                    (u1, v1, Vec2::new(uv1.x, uv0.y)),
                    (u0, v1, Vec2::new(uv0.x, uv0.y)),
                ] {
                    mesh.vertices.push(Vertex {
                        position: [-x, y, z],
                        normal: [0.0, 0.0, -1.0],
                        uv: uv.to_array(),
                        color: line.color.to_array(),
                    });
                }
                mesh.indices.extend_from_slice(&[
                    base,
                    base + 1,
                    base + 2,
                    base,
                    base + 2,
                    base + 3,
                ]);
            }
            u += advance;
        }
    }
    mesh
}

/// A prop's world transform (as the level renderer places it).
fn prop_transform(p: &shared::level::Prop) -> Mat4 {
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        p.rotation.y.to_radians(),
        p.rotation.x.to_radians(),
        p.rotation.z.to_radians(),
    );
    Mat4::from_scale_rotation_translation(Vec3::splat(p.scale), rotation, p.position)
}

/// "DEATHMATCH // CISTERN NINE": the next match's mode and level.
fn next_match(lobby: &LobbyState, net: &NetClient) -> String {
    let level = net
        .levels()
        .iter()
        .find(|l| l.name == lobby.settings.level)
        .map_or_else(
            || shared::lobby::level_display_name(&lobby.settings.level),
            shared::lobby::LevelInfo::display_title,
        );
    format!("{} // {level}", lobby.settings.mode.title())
}

/// "4 GNAWERS", "2 CENSERS + 2 GNAWERS", "GAOLER": what a practice pack holds.
pub fn pack_name(enc: &Encounter, wave: usize) -> String {
    let Some(w) = enc.waves.get(wave) else {
        return String::new();
    };
    let mut counts: Vec<(String, usize)> = Vec::new();
    for s in &w.spawns {
        let name = s.enemy.to_uppercase();
        match counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, c)) => *c += 1,
            None => counts.push((name, 1)),
        }
    }
    counts
        .iter()
        .map(|(n, c)| {
            if *c > 1 {
                format!("{c} {n}S")
            } else {
                n.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// A text surface in the hub (the ready board's slate or header, a console sign) and its mesh.
#[derive(Default)]
struct Surface {
    transform: Mat4,
    z: f32,
    lines: Vec<TextLine>,
    mesh: Option<MeshId>,
}

impl Surface {
    fn set(&mut self, renderer: &mut Renderer, lines: Vec<TextLine>) {
        if lines == self.lines && self.mesh.is_some() {
            return;
        }
        let data = text_mesh(&lines, self.z);
        match self.mesh {
            Some(id) => renderer.update_mesh(id, &data),
            None => self.mesh = Some(renderer.create_mesh(&data)),
        }
        self.lines = lines;
    }

    fn draw(&self, font: engine::gfx::TextureId, out: &mut Vec<DrawItem>) {
        if let Some(mesh) = self.mesh
            && !self.lines.is_empty()
        {
            out.push(DrawItem {
                mesh,
                texture: font,
                transform: self.transform,
                tint: Vec4::ONE,
                unlit: true,
            });
        }
    }

    fn free(&mut self, renderer: &mut Renderer) {
        if let Some(id) = self.mesh.take() {
            renderer.free_mesh(id);
        }
        self.lines.clear();
    }
}

/// A practice console of the hub: its encounter (USE zone, packs) and sign.
struct ConsoleView {
    encounter: usize,
    sign: Surface,
    /// The pack the next press summons (from the summons we saw).
    next: usize,
}

/// The hub's writable props, found in the level (`HubView::load`).
#[derive(Default)]
pub struct HubView {
    /// Level name the view was built for (rebuilt when another level loads).
    level: String,
    board: Option<(Surface, Surface)>,
    consoles: Vec<ConsoleView>,
}

impl HubView {
    /// Finds the hub's board and consoles (nothing for other levels).
    fn load(&mut self, renderer: &mut Renderer, name: &str, level: &LevelDef) {
        self.free(renderer);
        self.level = name.to_string();
        if level.kind != LevelKind::Hub {
            return;
        }
        for p in &level.props {
            let transform = prop_transform(p);
            if p.model.ends_with(BOARD_MODEL) && self.board.is_none() {
                self.board = Some((
                    Surface {
                        transform,
                        z: board::FRONT_Z,
                        ..Default::default()
                    },
                    Surface {
                        transform,
                        z: board::HEADER_Z,
                        ..Default::default()
                    },
                ));
            } else if p.model.ends_with(CONSOLE_MODEL) {
                // The console encounter whose USE zone is nearest.
                let nearest = level
                    .encounters
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| is_console(e))
                    .min_by(|a, b| {
                        let d = |e: &Encounter| e.trigger.center().distance_squared(p.position);
                        d(a.1).total_cmp(&d(b.1))
                    });
                if let Some((encounter, _)) = nearest {
                    self.consoles.push(ConsoleView {
                        encounter,
                        sign: Surface {
                            transform,
                            z: console::SIGN_Z,
                            ..Default::default()
                        },
                        next: 0,
                    });
                }
            }
        }
    }

    fn free(&mut self, renderer: &mut Renderer) {
        if let Some((mut a, mut b)) = self.board.take() {
            a.free(renderer);
            b.free(renderer);
        }
        for mut c in self.consoles.drain(..) {
            c.sign.free(renderer);
        }
        self.level.clear();
    }

    /// Rewrites the board and signs from the lobby state (meshes change only when the text
    /// does).
    fn update(
        &mut self,
        renderer: &mut Renderer,
        level: &LevelDef,
        lobby: Option<&LobbyState>,
        net: &NetClient,
    ) {
        let bone = Vec4::new(0.92, 0.88, 0.8, 1.0);
        let dim = Vec4::new(0.55, 0.52, 0.5, 1.0);
        let green = Vec4::new(0.45, 1.0, 0.5, 1.0);
        let ember = Vec4::new(1.0, 0.62, 0.25, 1.0);
        if let Some((slate, header)) = &mut self.board {
            let mut lines = Vec::new();
            let h = BOARD_GLYPH_H;
            let step = h * 1.2;
            let left = -board::HALF_W + 0.1;
            let right_col = board::HALF_W - 0.1 - 6.0 * h / font::GLYPH_H * font::GLYPH_ADVANCE;
            let mut v = board::TOP - h;
            let mut line = |text: String, u: f32, v: f32, color: Vec4, centered: bool| {
                lines.push(TextLine {
                    text,
                    u,
                    v,
                    glyph_h: h,
                    color,
                    centered,
                });
            };
            match lobby {
                Some(l) => {
                    line(format!("NEXT: {}", next_match(l, net)), 0.0, v, ember, true);
                    v -= step * 1.5;
                    for p in &l.players {
                        let mut name = p.name.to_uppercase();
                        if p.is_host {
                            name.push_str(" (HOST)");
                        }
                        name.truncate(34);
                        line(
                            format!("{} {name}", if p.ready { ">" } else { " " }),
                            left,
                            v,
                            player_color(p.id),
                            false,
                        );
                        let (status, color) = if p.bot {
                            ("BOT", dim)
                        } else if p.ready {
                            ("READY", green)
                        } else {
                            ("...", dim)
                        };
                        line(status.to_string(), right_col, v, color, false);
                        v -= step;
                    }
                    let footer = match l.phase {
                        LobbyPhase::Countdown { secs } => format!("MATCH STARTS IN {secs}"),
                        _ if l.all_ready() => "EVERYONE IS READY".to_string(),
                        _ => format!("{}/{} READY", l.ready_count(), l.players.len()),
                    };
                    line(footer, 0.0, board::BOTTOM, bone, true);
                }
                None => line("WAITING FOR THE LOBBY".into(), 0.0, v, dim, true),
            }
            slate.set(renderer, lines);
            header.set(
                renderer,
                vec![TextLine {
                    text: "READY BOARD".into(),
                    u: 0.0,
                    v: board::HEADER_Y - HEADER_GLYPH_H * 0.5,
                    glyph_h: HEADER_GLYPH_H,
                    color: Vec4::new(0.24, 0.14, 0.07, 1.0),
                    centered: true,
                }],
            );
        }
        for c in &mut self.consoles {
            let Some(enc) = level.encounters.get(c.encounter) else {
                continue;
            };
            let pack = pack_name(enc, c.next % enc.waves.len().max(1));
            let h = SIGN_GLYPH_H;
            c.sign.set(
                renderer,
                vec![
                    TextLine {
                        text: "PRACTICE PIT".into(),
                        u: 0.0,
                        v: console::SIGN_Y + 0.01,
                        glyph_h: h,
                        color: ember,
                        centered: true,
                    },
                    TextLine {
                        text: format!("NEXT: {pack}"),
                        u: 0.0,
                        v: console::SIGN_Y - h - 0.02,
                        glyph_h: h * 0.8,
                        color: bone,
                        centered: true,
                    },
                ],
            );
        }
    }

    fn draw(&self, font: engine::gfx::TextureId, out: &mut Vec<DrawItem>) {
        if let Some((slate, header)) = &self.board {
            slate.draw(font, out);
            header.draw(font, out);
        }
        for c in &self.consoles {
            c.sign.draw(font, out);
        }
    }

    /// The console (encounter index) whose USE zone `feet` stands in.
    fn console_at<'a>(
        &self,
        level: &'a LevelDef,
        feet: Vec3,
    ) -> Option<(&ConsoleView, &'a Encounter)> {
        self.consoles.iter().find_map(|c| {
            let e = level.encounters.get(c.encounter)?;
            (e.trigger.contains(feet) || e.trigger.contains(feet + Vec3::Y)).then_some((c, e))
        })
    }
}

impl InGame {
    /// The server runs the warm-up and we have a world: the game screen is the lobby.
    pub fn in_warmup(&self) -> bool {
        self.net().is_some_and(NetClient::in_warmup) && self.world.is_loaded()
    }

    /// The lobby overlay is open (tools, tests).
    pub fn lobby_overlay_open(&self) -> bool {
        self.warmup.open
    }

    /// Opens / closes the lobby overlay (tools, tests; players press TAB).
    pub fn set_lobby_overlay(&mut self, open: bool) {
        if !open {
            self.warmup.panel.close_dialogs();
        } else {
            self.warmup.panel.focus_ready();
        }
        self.warmup.open = open;
    }

    /// The overlay's CUSTOMISE sub-screen (tools, tests).
    pub fn open_customize(&mut self) {
        self.set_lobby_overlay(true);
        self.warmup.panel.open_customize();
    }

    /// Per frame, before input: the panel's clock, auto-ready, the host's edits, our look; the
    /// overlay opens by itself on the first arrival and closes when a match starts; the hub's
    /// board follows the lobby.
    pub(super) fn warmup_update(&mut self, common: &mut Common, renderer: &mut Renderer, dt: f32) {
        self.warmup.panel.tick(dt);
        let look = common.settings.cosmetics;
        if let Some(s) = &mut self.session {
            self.warmup.panel.update(s.client_mut(), look);
        }
        let warm = self.in_warmup();
        if warm && !self.warmup.seen {
            self.warmup.seen = true;
            if self.warmup.auto_open {
                self.set_lobby_overlay(true);
            }
        }
        if !warm && self.warmup.open {
            self.set_lobby_overlay(false);
        }
        let Some(net) = self.session.as_ref().map(Session::client) else {
            return;
        };
        let Some(world) = net.world() else {
            return;
        };
        if self.warmup.hub.level != world.level_name {
            self.warmup
                .hub
                .load(renderer, &world.level_name, &world.level);
        }
        if world.level.kind == LevelKind::Hub {
            self.warmup
                .hub
                .update(renderer, &world.level, net.lobby(), net);
        }
    }

    /// Warm-up hotkeys: TAB / Select (the scoreboard binding: no scoreboard in the warm-up)
    /// toggles the overlay, the Ready key toggles ready.
    pub(super) fn warmup_hotkeys(
        &mut self,
        common: &mut Common,
        input: &engine::input::InputState,
    ) {
        let typing = common.ui_state.wants_text_input() || common.ui_state.is_capturing();
        if typing || self.chat.is_open() || self.pause.is_open() {
            return;
        }
        let b = &common.settings.bindings;
        let (toggle, ready) = (
            b.pressed(input, Action::Scoreboard),
            b.pressed(input, Action::Ready),
        );
        let customizing = self.warmup.panel.is_customizing();
        if toggle && !customizing {
            let open = !self.warmup.open;
            self.set_lobby_overlay(open);
            common.ui_sound(if open { Sfx::UiSelect } else { Sfx::UiBack });
        }
        if ready
            && !customizing
            && let Some(s) = &mut self.session
        {
            LobbyPanel::toggle_ready(s.client_mut());
            common.ui_sound(Sfx::UiSelect);
        }
    }

    /// `NetEvent::Game` in the hub: a practice pack summoned at a console. True if handled.
    pub(super) fn warmup_game_event(&mut self, ev: &GameEvent, common: &mut Common) -> bool {
        let GameEvent::WaveStarted {
            index, wave, waves, ..
        } = ev
        else {
            return false;
        };
        if self.world.kind() != LevelKind::Hub {
            return false;
        }
        let now = self.time;
        let name = self
            .net()
            .and_then(NetClient::world)
            .and_then(|w| w.level.encounters.get(*index as usize))
            .map(|e| pack_name(e, (*wave as usize).saturating_sub(1)))
            .unwrap_or_default();
        for c in &mut self.warmup.hub.consoles {
            if c.encounter == *index as usize {
                c.next = (*wave as usize) % (*waves as usize).max(1);
            }
        }
        self.hud.banner(
            format!("PRACTICE: {name}"),
            common.theme.crimson_bright,
            now,
        );
        common.audio.play_2d(Sfx::WaveStart);
        true
    }

    /// World text of the hub (board, signs).
    pub(super) fn warmup_draw_world(
        &self,
        renderer_font: engine::gfx::TextureId,
        out: &mut Vec<DrawItem>,
    ) {
        self.warmup.hub.draw(renderer_font, out);
    }

    /// The HUD card (top left) and the console prompt; nothing while the overlay is open.
    pub(super) fn warmup_hud(&self, common: &Common, list: &mut UiDrawList) {
        if !self.in_warmup() || self.warmup.open {
            return;
        }
        let Some(net) = self.net() else {
            return;
        };
        let Some(lobby) = net.lobby() else {
            return;
        };
        let t = &common.theme;
        let b = &common.settings.bindings;
        let outline = fade(t.ink, 0.9);
        let me_ready = net
            .local_player_id()
            .and_then(|id| lobby.player(id))
            .is_some_and(|p| p.ready);
        let summary = engine::ui::text::truncate(&next_match(lobby, net), 190.0, 1.0);
        let slab = Rect::new(vec2(8.0, 6.0), vec2(216.0, 62.0));
        draw::slanted(list, slab, true, fade(t.ink, 0.55));
        draw::fill(
            list,
            Rect::new(slab.min, vec2(slab.min.x + 2.0, slab.max.y)),
            t.crimson,
        );
        list.text(
            vec2(16.0, 10.0),
            "WARM-UP",
            &TextStyle::new(t.crimson_bright).outline(outline),
        );
        list.text(
            vec2(16.0, 21.0),
            &format!("NEXT: {summary}"),
            &TextStyle::new(t.bone_dim).outline(outline),
        );
        // A dot per player: identity colour, filled when ready.
        let mut x = 16.0;
        for p in &lobby.players {
            let r = Rect::from_min_size(vec2(x, 34.0), vec2(6.0, 6.0));
            let c = player_color(p.id);
            draw::fill(list, r.expand(1.0), outline);
            draw::fill(list, r, if p.ready { c } else { fade(c, 0.25) });
            x += 9.0;
        }
        list.text(
            vec2(x + 3.0, 33.0),
            &format!("{}/{} READY", lobby.ready_count(), lobby.players.len()),
            &TextStyle::new(t.bone).outline(outline),
        );
        let key = |a: Action| b.hint(a).unwrap_or_else(|| "-".into());
        let (state, color) = if me_ready {
            (format!("READY  ({} CANCEL)", key(Action::Ready)), t.stamina)
        } else {
            (format!("{} READY UP", key(Action::Ready)), t.bone)
        };
        list.text(
            vec2(16.0, 46.0),
            &format!("{state}   {} LOBBY", key(Action::Scoreboard)),
            &TextStyle::new(color).outline(outline),
        );

        // Standing at a practice console: what USE summons.
        if let (Some(world), Some(feet)) = (net.world(), net.local_state().map(|s| s.position))
            && let Some((c, enc)) = self.warmup.hub.console_at(&world.level, feet)
        {
            let pack = pack_name(enc, c.next % enc.waves.len().max(1));
            let canvas = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
            list.text(
                vec2(canvas.center().x, canvas.center().y + 34.0),
                &format!("{} SUMMON {pack}", key(Action::Use)),
                &TextStyle::new(t.bone).centered().outline(outline),
            );
        }
    }

    /// The countdown banner on top of the world (and of the overlay).
    pub(super) fn warmup_banner(&self, common: &Common, list: &mut UiDrawList) {
        if !self.in_warmup() {
            return;
        }
        let Some(net) = self.net() else {
            return;
        };
        let (Some(lobby), Some((_, since))) = (net.lobby(), self.warmup.panel.countdown()) else {
            return;
        };
        if let LobbyPhase::Countdown { secs } = lobby.phase {
            let canvas = Rect::from_min_size(Vec2::ZERO, list.canvas_size);
            let band = Rect::new(
                vec2(canvas.min.x, 40.0),
                vec2(canvas.max.x, canvas.max.y - 60.0),
            );
            lobby::countdown_band(
                list,
                &common.theme,
                Rect::new(band.min, vec2(band.max.x, 210.0)),
                secs,
                self.warmup.panel.time() - since,
                &lobby.settings.summary(net.levels()),
            );
        }
    }

    /// The interactive lobby overlay (or its CUSTOMISE sub-screen). Returns a transition
    /// (LEAVE) and a UI sound.
    pub(super) fn warmup_overlay_ui(
        &mut self,
        common: &mut Common,
        io: &FrameIo,
        out: &mut FrameBuffers,
        viewmodel_fov: f32,
    ) -> (Option<Transition>, Option<Sfx>) {
        let mut sound = None;
        let mut transition = None;
        if self.warmup.panel.is_customizing() {
            let team = self
                .net()
                .and_then(NetClient::local_player_id)
                .map_or(crate::models::DEFAULT_TEAM_COLOR, player_color);
            // A lamp in front of the preview (lit by the world's lights otherwise).
            let cam = self.last_camera;
            out.lights.push(engine::gfx::PointLight {
                position: cam.position + cam.rotation * Vec3::new(1.2, 1.0, -1.0),
                color: Vec3::new(1.3, 1.15, 1.0),
                radius: 9.0,
            });
            let FrameBuffers {
                ui: list,
                viewmodel,
                ..
            } = out;
            // The preview stands where the weapon would.
            viewmodel.clear();
            let mut ui = Ui::new(list, &mut common.ui_state, io.ui_input, &common.theme);
            let resp = self.warmup.panel.customize_ui(
                &mut ui,
                &mut common.settings.cosmetics,
                team,
                common.models.as_ref(),
                viewmodel,
                viewmodel_fov,
                io.dt,
            );
            drop(ui);
            if resp.changed {
                sound = Some(Sfx::UiMove);
            }
            if resp.back {
                common.settings_save = true;
                sound = Some(Sfx::UiBack);
            }
            return (None, sound);
        }
        let typing = common.ui_state.wants_text_input();
        let chat_key = !typing && io.input.key_pressed(engine::input::Key::KeyT);
        let player_name = common.player_name();
        let output = {
            let Some(net) = self.session.as_ref().map(Session::client) else {
                return (None, None);
            };
            let mut ui = Ui::new(
                &mut out.ui,
                &mut common.ui_state,
                io.ui_input,
                &common.theme,
            );
            self.warmup.panel.ui(
                &mut ui,
                net,
                &self.chat,
                &player_name,
                PanelMode::Overlay,
                chat_key,
            )
        };
        for action in output.actions {
            if action == LobbyAction::Close {
                self.set_lobby_overlay(false);
                continue;
            }
            if let Some(s) = &mut self.session
                && let Some(t) = self
                    .warmup
                    .panel
                    .run(action, s.client_mut(), &mut common.audio)
            {
                transition = Some(t);
            }
        }
        (transition, output.sound.or(sound))
    }
}
