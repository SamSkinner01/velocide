//! egui debug overlay (only with the `debug-ui` feature).
//!
//! ```ignore
//! // init:
//! let mut debug = DebugUi::new(ctx);
//! // Game::window_event:
//! return debug.on_window_event(&ctx.window, event);
//! // Game::frame:
//! debug.run(&ctx.window, |ui| { egui::Window::new("Tuning").show(ui, |ui| { ... }); });
//! ctx.renderer.render(&frame, Some(&mut debug))?;
//! ```

pub use egui;

use winit::event::WindowEvent;
use winit::window::Window;

use crate::app::EngineContext;
use crate::render::OverlayPainter;

pub struct DebugUi {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    primitives: Vec<egui::ClippedPrimitive>,
    textures: egui::TexturesDelta,
    pixels_per_point: f32,
}

impl DebugUi {
    pub fn new(ctx: &EngineContext) -> Self {
        let egui_ctx = egui::Context::default();
        let max_texture_side = ctx.renderer.device().limits().max_texture_dimension_2d as usize;
        let state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &ctx.window,
            Some(ctx.window.scale_factor() as f32),
            None,
            Some(max_texture_side),
        );
        let renderer = egui_wgpu::Renderer::new(
            ctx.renderer.device(),
            ctx.renderer.surface_format(),
            egui_wgpu::RendererOptions::default(),
        );
        Self {
            ctx: egui_ctx,
            state,
            renderer,
            primitives: Vec::new(),
            textures: egui::TexturesDelta::default(),
            pixels_per_point: ctx.window.scale_factor() as f32,
        }
    }

    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }

    /// Feeds a window event to egui. Returns `true` if egui consumed it (e.g. typing into a
    /// text field), in which case the game should ignore it.
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// True while egui wants mouse input (pointer over a panel or dragging a widget).
    pub fn wants_pointer(&self) -> bool {
        self.ctx.egui_wants_pointer_input()
    }

    /// True while egui has keyboard focus (a text field is active).
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// Builds this frame's UI. Call once per frame before `Renderer::render(.., Some(self))`;
    /// frames without a `run` draw nothing.
    pub fn run(&mut self, window: &Window, mut build: impl FnMut(&egui::Context)) {
        let input = self.state.take_egui_input(window);
        let output = self.ctx.run_pass(input, |ctx| build(ctx));
        self.state
            .handle_platform_output(window, output.platform_output);
        self.pixels_per_point = output.pixels_per_point;
        self.primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.textures.append(output.textures_delta);
    }
}

impl OverlayPainter for DebugUi {
    fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        size_px: [u32; 2],
    ) {
        for (id, deltas) in &self.textures.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size_px,
            pixels_per_point: self.pixels_per_point,
        };
        // Only paint callbacks produce extra command buffers, and we register none.
        let _ = self
            .renderer
            .update_buffers(device, queue, encoder, &self.primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &self.primitives, &screen);
        }
        for id in &self.textures.free {
            self.renderer.free_texture(id);
        }
        self.textures.clear();
        self.primitives.clear();
    }
}

impl Drop for DebugUi {
    fn drop(&mut self) {
        // egui asserts (in debug builds) that texture deltas are never dropped unapplied.
        self.textures.clear();
    }
}
