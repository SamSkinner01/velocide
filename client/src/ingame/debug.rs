//! Debug overlay (F3).
//!
//! - With the `debug-ui` feature (default): an egui window with performance, network
//!   (RTT, loss, bandwidth, snapshot rate, prediction corrections, interpolation delay, server
//!   vs render tick), render and audio stats, a ghost-outline toggle (server-authoritative vs
//!   predicted capsule) and a **tuning editor** that writes `assets/tuning.ron`; the server
//!   hot-reloads that file and broadcasts it, so prediction stays in sync.
//! - Without it: a compact text overlay with the key numbers ([`text_overlay`]).
//!
//! Press Esc (pause) to free the cursor and interact with the egui window.

use engine::gfx::UiDrawList;
use engine::render::RenderStats;
use engine::ui::{TextStyle, Theme, UiDrawExt};
use glam::vec2;
use shared::tuning::Tuning;

use super::tuning_edit::{self, Field};
use crate::app::FrameStats;
use crate::net::{NetClient, SessionKind};

/// Debug state kept across frames.
pub struct DebugOverlay {
    /// Draw the server / predicted capsules.
    pub show_ghosts: bool,
    pub editor: TuningEditor,
}

impl Default for DebugOverlay {
    fn default() -> Self {
        Self {
            show_ghosts: true,
            editor: TuningEditor::default(),
        }
    }
}

/// Working copy of the server's tuning being edited.
#[derive(Default)]
pub struct TuningEditor {
    pub movement: Vec<Field>,
    pub net: Vec<Field>,
    /// Values as last received from the server (to know what changed).
    base_movement: Vec<Field>,
    base_net: Vec<Field>,
    pub filter: String,
    pub auto_save: bool,
    pub status: String,
    dirty_since: Option<f64>,
}

impl TuningEditor {
    /// Takes the server's tuning (on connect and whenever the server broadcasts a change).
    pub fn sync_from(&mut self, tuning: &Tuning) {
        self.base_movement = tuning_edit::fields(&tuning.movement);
        self.base_net = tuning_edit::fields(&tuning.net);
        if self.dirty_since.is_none() {
            self.movement = self.base_movement.clone();
            self.net = self.base_net.clone();
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.movement != self.base_movement || self.net != self.base_net
    }

    pub fn mark_dirty(&mut self, now: f64) {
        self.dirty_since.get_or_insert(now);
    }

    pub fn revert(&mut self) {
        self.movement = self.base_movement.clone();
        self.net = self.base_net.clone();
        self.dirty_since = None;
    }

    pub fn defaults(&mut self) {
        let d = Tuning::default();
        self.movement = tuning_edit::fields(&d.movement);
        self.net = tuning_edit::fields(&d.net);
    }

    fn changed(current: &[Field], base: &[Field]) -> Vec<Field> {
        current
            .iter()
            .filter(|f| base.iter().find(|b| b.name == f.name) != Some(*f))
            .cloned()
            .collect()
    }

    /// Writes the changed values into `assets/tuning.ron` (keeping its comments). The
    /// server's file watcher reloads and broadcasts it.
    pub fn save(&mut self, now: f64) {
        self.save_to(&Tuning::default_path(), now);
    }

    /// Like [`TuningEditor::save`] for any tuning file.
    pub fn save_to(&mut self, path: &std::path::Path, now: f64) {
        let text = std::fs::read_to_string(path).unwrap_or_else(|_| "(\n)\n".to_string());
        let moved = Self::changed(&self.movement, &self.base_movement);
        let net = Self::changed(&self.net, &self.base_net);
        let mut out = tuning_edit::rewrite(&text, "movement", &moved);
        out = tuning_edit::rewrite(&out, "net", &net);
        if let Err(e) = ron::from_str::<Tuning>(&out) {
            self.status = format!("not saved, result doesn't parse: {e}");
            return;
        }
        match std::fs::write(path, out) {
            Ok(()) => {
                self.status = format!(
                    "saved {} value(s) to {} at {:.1}s; waiting for the server",
                    moved.len() + net.len(),
                    path.display(),
                    now
                );
                self.dirty_since = None;
            }
            Err(e) => self.status = format!("can't write {}: {e}", path.display()),
        }
    }

    /// Auto-save after edits settle (0.4 s).
    pub fn update(&mut self, now: f64) {
        if self.auto_save
            && let Some(t) = self.dirty_since
            && now - t > 0.4
            && self.is_dirty()
        {
            self.save(now);
        }
    }

    pub fn on_server_tuning(&mut self, tuning: &Tuning, now: f64) {
        self.sync_from(tuning);
        if self.status.contains("waiting for the server") {
            self.status =
                format!("server applied the new tuning at {now:.1}s (prediction in sync)");
        }
    }
}

/// Numbers shown by both overlays.
pub struct DebugNumbers {
    pub lines: Vec<String>,
}

/// Formats bytes/s compactly.
pub fn rate(bytes_per_sec: f32) -> String {
    if bytes_per_sec >= 1000.0 {
        format!("{:.1}K", bytes_per_sec / 1000.0)
    } else {
        format!("{bytes_per_sec:.0}")
    }
}

/// The key lines of the overlay (also used by the text fallback).
pub fn summary(
    net: Option<&NetClient>,
    kind: Option<SessionKind>,
    frame: &FrameStats,
    render: &RenderStats,
    voices: usize,
) -> DebugNumbers {
    let mut lines = vec![format!(
        "FPS {:.0}  {:.2} MS  WORST {:.1} MS",
        frame.fps, frame.frame_ms, frame.worst_ms
    )];
    lines.push(format!(
        "DRAWS {}  CULLED {}  SPRITES {}  TRIS {}  RES {}X{}  VOICES {}",
        render.draws,
        render.culled,
        render.sprites,
        render.triangles,
        render.internal_size.0,
        render.internal_size.1,
        voices
    ));
    if let Some(net) = net {
        let s = net.stats();
        lines.push(format!(
            "{:?} {:?}  RTT {:.0} MS  LOSS {:.1}%  IN {}/S  OUT {}/S",
            kind.unwrap_or(SessionKind::Join),
            net.state(),
            s.rtt_ms,
            s.packet_loss * 100.0,
            rate(s.bytes_in_per_sec),
            rate(s.bytes_out_per_sec)
        ));
        lines.push(format!(
            "SNAPSHOTS {:.0}/S  FULL {}  DROPPED {}  INTERP {:.0} MS  JITTER {:.1} MS{}",
            s.snapshot_rate,
            s.full_snapshots_received,
            s.snapshots_dropped,
            s.interp_delay_ms,
            s.jitter_ms,
            if s.extrapolating {
                "  EXTRAPOLATING"
            } else {
                ""
            }
        ));
        let tick_ms = 1000.0 / net.tick_rate().max(1) as f64;
        lines.push(format!(
            "SERVER TICK {:.1}  RENDER TICK {:.1}  (-{:.0} MS)  UNACKED {}",
            s.server_tick,
            s.render_tick,
            (s.server_tick - s.render_tick) * tick_ms,
            s.unacked_inputs
        ));
        lines.push(format!(
            "CORRECTIONS {}  LAST {:.3} M  AVG {:.3}  MAX {:.3}  SHIFT {:.2}  SNAPS {}  OFFSET {:.3}",
            s.corrections,
            s.last_correction_m,
            s.avg_correction_m,
            s.max_correction_m,
            s.max_correction_shift_m,
            s.correction_snaps,
            s.render_offset_m
        ));
    }
    DebugNumbers { lines }
}

/// Compact bitmap-font overlay (used without the `debug-ui` feature).
pub fn text_overlay(list: &mut UiDrawList, t: &Theme, numbers: &DebugNumbers) {
    let style = TextStyle::new(t.bone)
        .outline(engine::ui::draw::fade(t.ink, 0.9))
        .scale(1.0);
    let mut y = 40.0;
    for line in &numbers.lines {
        list.text(vec2(8.0, y), line, &style);
        y += 11.0;
    }
}

#[cfg(feature = "debug-ui")]
pub use egui_impl::*;

#[cfg(feature = "debug-ui")]
mod egui_impl {
    use engine::debug_ui::egui;
    use engine::render::Renderer;

    use super::*;
    use crate::app::Common;

    fn stats_grid(ui: &mut egui::Ui, numbers: &DebugNumbers) {
        for line in &numbers.lines {
            ui.monospace(line.to_lowercase());
        }
    }

    /// Main-menu variant: performance only.
    pub fn menu_debug_window(ctx: &egui::Context, common: &mut Common, renderer: &Renderer) {
        let numbers = summary(
            None,
            None,
            &common.stats,
            &renderer.stats(),
            common.audio.active_voices(),
        );
        egui::Window::new("VELOCIDE debug (F3)")
            .default_pos([10.0, 60.0])
            .show(ctx, |ui| {
                ui.label(renderer.adapter_name());
                stats_grid(ui, &numbers);
            });
    }

    fn field_editor(ui: &mut egui::Ui, id: &str, fields: &mut [Field], filter: &str) -> bool {
        let mut changed = false;
        egui::Grid::new(id)
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                for f in fields.iter_mut() {
                    if !filter.is_empty() && !f.name.contains(filter) {
                        continue;
                    }
                    ui.label(&f.name);
                    let speed = (f.value.abs() * 0.01).max(if f.integer { 0.05 } else { 0.001 });
                    let mut drag = egui::DragValue::new(&mut f.value).speed(speed);
                    if f.integer {
                        drag = drag.max_decimals(0).range(0.0..=1000.0);
                    } else {
                        drag = drag.max_decimals(4);
                    }
                    changed |= ui.add(drag).changed();
                    ui.end_row();
                }
            });
        changed
    }

    /// The in-game window.
    pub fn game_window(
        ctx: &egui::Context,
        numbers: &DebugNumbers,
        overlay: &mut DebugOverlay,
        net: Option<&NetClient>,
        renderer: &Renderer,
        extra: &[String],
        now: f64,
    ) {
        egui::Window::new("VELOCIDE debug (F3)")
            .default_pos([10.0, 60.0])
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.label(renderer.adapter_name());
                stats_grid(ui, numbers);
                for line in extra {
                    ui.monospace(line);
                }
                ui.checkbox(
                    &mut overlay.show_ghosts,
                    "ghosts: red = server capsule, green ring = predicted feet, blue = remote",
                );
                if let Some(net) = net {
                    let s = net.stats();
                    if let (Some(server), Some(pred)) = (s.server_position, s.predicted_position) {
                        ui.monospace(format!(
                            "server pos {:.2} {:.2} {:.2} | predicted {:.2} {:.2} {:.2}",
                            server.x, server.y, server.z, pred.x, pred.y, pred.z
                        ));
                    }
                }
                ui.separator();
                egui::CollapsingHeader::new("Tuning editor (writes assets/tuning.ron)")
                    .default_open(false)
                    .show(ui, |ui| {
                        let ed = &mut overlay.editor;
                        ui.horizontal(|ui| {
                            ui.label("filter");
                            ui.text_edit_singleline(&mut ed.filter);
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Save + apply").clicked() {
                                ed.save(now);
                            }
                            if ui.button("Revert").clicked() {
                                ed.revert();
                            }
                            if ui.button("Defaults").clicked() {
                                ed.defaults();
                                ed.mark_dirty(now);
                            }
                            ui.checkbox(&mut ed.auto_save, "auto-save");
                        });
                        if ed.is_dirty() {
                            ui.colored_label(egui::Color32::YELLOW, "unsaved changes");
                        }
                        if !ed.status.is_empty() {
                            ui.label(&ed.status);
                        }
                        let filter = ed.filter.clone();
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                ui.strong("movement");
                                if field_editor(ui, "movement", &mut ed.movement, &filter) {
                                    ed.mark_dirty(now);
                                }
                                ui.strong("net");
                                if field_editor(ui, "net", &mut ed.net, &filter) {
                                    ed.mark_dirty(now);
                                }
                            });
                    });
            });
    }
}
