//! wgpu renderer: low-res retro scene pass, viewmodel pass, post-process (dither / colour
//! reduction), nearest-neighbour upscale and the 2D UI pass.
//!
//! Pass order every frame (see `docs/render.md`):
//! 1. **Scene** (internal resolution, colour + reverse-Z depth): opaque meshes (alpha-tested,
//!    sorted by texture), decals, translucent meshes (back to front), alpha sprites (back to
//!    front), additive sprites, debug lines.
//! 2. **Viewmodel** (same targets, depth cleared): held weapon with its own FOV.
//! 3. **Post** (internal resolution): screen flash, colour depth reduction, Bayer dithering.
//! 4. **Output** (window resolution): nearest-neighbour upscale, then UI quads.
//! 5. Optional overlay (egui) painted straight onto the swapchain image.
//!
//! Shading happens in gamma space, like the hardware it imitates: textures are uploaded as raw
//! sRGB bytes and the intermediate targets are plain `Rgba8Unorm`. Only the final output is
//! linearised if the swapchain happens to be an sRGB format.

mod arena;
mod batch;
mod pipelines;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use glam::{Mat4, Vec3};
use winit::window::Window;

use crate::gfx::{FrameDesc, MeshData, MeshId, PointLight, RetroSettings, TextureData, TextureId};
use crate::hotreload::FileWatcher;

use arena::{MeshAlloc, MeshArena};
use batch::{
    Frustum, FxBatch, GpuDraw, GpuFrame, MAX_LIGHTS, OutputUniform, PostUniform, Run, UiBatch,
    select_lights,
};
use pipelines::{
    DEPTH_FORMAT, Layouts, POST_FORMAT, Pipelines, SCENE_FORMAT, ShaderSources, TEXTURE_FORMAT,
};

/// Something painted on top of the finished frame (the egui debug overlay).
pub trait OverlayPainter {
    fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target_view: &wgpu::TextureView,
        size_px: [u32; 2],
    );
}

/// Counters from the last rendered frame (for the debug overlay).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderStats {
    /// Mesh draw calls issued (world + viewmodel).
    pub draws: u32,
    /// World draw items rejected by frustum culling.
    pub culled: u32,
    pub sprites: u32,
    pub decals: u32,
    pub debug_lines: u32,
    pub ui_quads: u32,
    pub triangles: u64,
    pub internal_size: (u32, u32),
}

/// Near/far planes of the viewmodel projection (view-space metres).
const VIEWMODEL_NEAR: f32 = 0.01;
const VIEWMODEL_FAR: f32 = 50.0;

enum Output {
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        window: Arc<Window>,
        present_modes: Vec<wgpu::PresentMode>,
    },
    Headless {
        texture: wgpu::Texture,
    },
}

enum Acquired {
    /// Swapchain image (+ whether the surface should be reconfigured after presenting).
    Surface(wgpu::SurfaceTexture, bool),
    Headless,
    /// Nothing to render into this frame (minimised, occluded, outdated...).
    Skip,
}

struct Targets {
    size: (u32, u32),
    scene_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    post_bind_group: wgpu::BindGroup,
    output_bind_group: wgpu::BindGroup,
    post_view: wgpu::TextureView,
}

struct GpuTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

struct GpuMesh {
    alloc: MeshAlloc,
    index_count: u32,
    center: Vec3,
    radius: f32,
}

/// A buffer rewritten every frame that grows (to the next power of two) when needed.
struct DynBuffer {
    buffer: wgpu::Buffer,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl DynBuffer {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        size: u64,
    ) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            label,
            usage,
        }
    }

    /// Uploads `data`, growing the buffer first if needed. Returns true if the buffer was
    /// recreated (bind groups referencing it must be rebuilt).
    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) -> bool {
        let needed = data.len() as u64;
        let mut grown = false;
        if needed > self.buffer.size() {
            let size = needed.next_power_of_two().max(4096);
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size,
                usage: self.usage,
                mapped_at_creation: false,
            });
            grown = true;
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
        grown
    }
}

/// One mesh draw, resolved to arena offsets.
#[derive(Clone, Copy, Debug)]
struct DrawCmd {
    draw: u32,
    texture: u32,
    first_index: u32,
    index_count: u32,
    base_vertex: i32,
    sort: f32,
}

#[derive(Default)]
struct FrameLists {
    draws: Vec<GpuDraw>,
    opaque: Vec<DrawCmd>,
    translucent: Vec<DrawCmd>,
    vm_opaque: Vec<DrawCmd>,
    vm_translucent: Vec<DrawCmd>,
    lights: Vec<PointLight>,
    fx: FxBatch,
    ui: UiBatch,
}

pub struct Renderer {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    output: Output,
    output_format: wgpu::TextureFormat,
    window_size: (u32, u32),
    settings: RetroSettings,
    layouts: Layouts,
    pipelines: Pipelines,
    point_sampler: wgpu::Sampler,
    targets: Targets,
    frame_buffer: wgpu::Buffer,
    draw_buffer: DynBuffer,
    frame_bind_group: wgpu::BindGroup,
    post_buffer: wgpu::Buffer,
    output_buffer: wgpu::Buffer,
    fx_buffer: DynBuffer,
    ui_buffer: DynBuffer,
    arena: MeshArena,
    meshes: Vec<Option<GpuMesh>>,
    free_mesh_ids: Vec<u32>,
    textures: Vec<Option<GpuTexture>>,
    white: TextureId,
    font: TextureId,
    lists: FrameLists,
    pending_capture: Option<PathBuf>,
    shader_watcher: Option<FileWatcher>,
    shader_error: Option<String>,
    stats: RenderStats,
    /// The next `render` only polls the shader watcher (window hidden, see `skip_next_frame`).
    skip_frame: bool,
}

fn request_device(adapter: &wgpu::Adapter) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let adapter_limits = adapter.limits();
    let limits = wgpu::Limits {
        max_texture_dimension_2d: adapter_limits.max_texture_dimension_2d,
        ..wgpu::Limits::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("velocide"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        ..Default::default()
    }))
    .context("requesting a wgpu device")?;
    device.on_uncaptured_error(Arc::new(|error| log::error!("wgpu: {error}")));
    Ok((device, queue))
}

/// Internal render size for a window: height from the settings, width from the aspect ratio.
pub fn internal_size_for(window: (u32, u32), internal_height: u32) -> (u32, u32) {
    let (w, h) = (window.0.max(1), window.1.max(1));
    if internal_height == 0 || internal_height >= h {
        return (w, h);
    }
    let ih = internal_height.max(8);
    let iw = ((ih as f64) * w as f64 / h as f64).round().max(1.0) as u32;
    (iw, ih)
}

fn choose_present_mode(modes: &[wgpu::PresentMode], vsync: bool) -> wgpu::PresentMode {
    if vsync {
        return wgpu::PresentMode::Fifo;
    }
    [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|m| modes.contains(m))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

impl Renderer {
    /// Creates a renderer presenting to `window`.
    pub fn new(window: Arc<Window>, settings: &RetroSettings) -> anyhow::Result<Renderer> {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window.clone())
            .context("creating the window surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .context("no GPU adapter can present to this window")?;
        let (device, queue) = request_device(&adapter)?;

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB format: we output gamma-space colour directly.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .context("the surface reports no supported formats")?;
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: choose_present_mode(&caps.present_modes, settings.vsync),
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!(
            "renderer: {} ({:?}), surface {:?}, {:?}",
            adapter.get_info().name,
            adapter.get_info().backend,
            format,
            config.present_mode
        );
        let output = Output::Surface {
            surface,
            config,
            window,
            present_modes: caps.present_modes,
        };
        Ok(Self::build(
            instance,
            adapter,
            device,
            queue,
            output,
            format,
            (size.width, size.height),
            settings,
        ))
    }

    /// Creates a renderer drawing into an offscreen `width` x `height` image (no window).
    pub fn new_headless(
        width: u32,
        height: u32,
        settings: &RetroSettings,
    ) -> anyhow::Result<Renderer> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .context("no GPU adapter available")?;
        let (device, queue) = request_device(&adapter)?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let size = (width.max(1), height.max(1));
        let texture = create_headless_texture(&device, size, format);
        log::info!(
            "headless renderer: {} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        Ok(Self::build(
            instance,
            adapter,
            device,
            queue,
            Output::Headless { texture },
            format,
            size,
            settings,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        output: Output,
        output_format: wgpu::TextureFormat,
        window_size: (u32, u32),
        settings: &RetroSettings,
    ) -> Renderer {
        let layouts = Layouts::new(&device);
        let pipelines =
            Pipelines::new(&device, &layouts, &ShaderSources::embedded(), output_format);
        let point_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let frame_buffer = uniform("frame", std::mem::size_of::<GpuFrame>());
        let post_buffer = uniform("post", std::mem::size_of::<PostUniform>());
        let output_buffer = uniform("output", std::mem::size_of::<OutputUniform>());
        let draw_buffer = DynBuffer::new(
            &device,
            "draws",
            wgpu::BufferUsages::STORAGE,
            256 * std::mem::size_of::<GpuDraw>() as u64,
        );
        let fx_buffer = DynBuffer::new(&device, "fx vertices", wgpu::BufferUsages::VERTEX, 1 << 16);
        let ui_buffer = DynBuffer::new(&device, "ui vertices", wgpu::BufferUsages::VERTEX, 1 << 16);
        let frame_bind_group = create_frame_bind_group(
            &device,
            &layouts,
            &frame_buffer,
            &draw_buffer.buffer,
            &point_sampler,
        );
        let internal = internal_size_for(window_size, settings.internal_height);
        let targets = create_targets(
            &device,
            &layouts,
            internal,
            &post_buffer,
            &output_buffer,
            &point_sampler,
        );
        let arena = MeshArena::new(&device);

        let mut renderer = Renderer {
            instance,
            adapter,
            device,
            queue,
            output,
            output_format,
            window_size,
            settings: settings.clone(),
            layouts,
            pipelines,
            point_sampler,
            targets,
            frame_buffer,
            draw_buffer,
            frame_bind_group,
            post_buffer,
            output_buffer,
            fx_buffer,
            ui_buffer,
            arena,
            meshes: Vec::new(),
            free_mesh_ids: Vec::new(),
            textures: Vec::new(),
            white: TextureId(0),
            font: TextureId(0),
            lists: FrameLists::default(),
            pending_capture: None,
            shader_watcher: None,
            shader_error: None,
            skip_frame: false,
            stats: RenderStats::default(),
        };
        renderer.white = renderer.create_texture(&TextureData {
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        });
        renderer.font = renderer.create_texture(&crate::ui::font::build_atlas());
        renderer
    }

    // ----------------------------------------------------------------------------------------
    // Accessors

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Format of the view passed to overlay painters (the swapchain / headless image).
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    pub fn window_size(&self) -> (u32, u32) {
        self.window_size
    }

    pub fn internal_size(&self) -> (u32, u32) {
        self.targets.size
    }

    pub fn settings(&self) -> &RetroSettings {
        &self.settings
    }

    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    /// "GPU name (backend)".
    pub fn adapter_name(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }

    /// 1x1 white texture (use for untextured, vertex-coloured meshes).
    pub fn white_texture(&self) -> TextureId {
        self.white
    }

    /// The bitmap font atlas built from `ui::font::build_atlas()`.
    pub fn font_texture(&self) -> TextureId {
        self.font
    }

    /// The last shader hot-reload error, if the current shaders failed to compile.
    pub fn shader_error(&self) -> Option<&str> {
        self.shader_error.as_deref()
    }

    // ----------------------------------------------------------------------------------------
    // Configuration

    /// The next [`Renderer::render`] draws and presents nothing. The app runs frames like that
    /// while the window gets no redraws (minimised / hidden on Wayland), so the game keeps
    /// running without touching a surface the compositor isn't showing.
    pub fn skip_next_frame(&mut self) {
        self.skip_frame = true;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) == self.window_size {
            return;
        }
        self.window_size = (width, height);
        if width == 0 || height == 0 {
            // Minimised: keep the old targets, skip rendering until we get a real size.
            return;
        }
        match &mut self.output {
            Output::Surface {
                surface, config, ..
            } => {
                config.width = width;
                config.height = height;
                surface.configure(&self.device, config);
            }
            Output::Headless { texture } => {
                *texture =
                    create_headless_texture(&self.device, (width, height), self.output_format);
            }
        }
        self.recreate_targets();
    }

    pub fn apply_settings(&mut self, settings: &RetroSettings) {
        let old = std::mem::replace(&mut self.settings, settings.clone());
        if old.internal_height != settings.internal_height {
            self.recreate_targets();
        }
        if old.vsync != settings.vsync
            && let Output::Surface {
                surface,
                config,
                present_modes,
                ..
            } = &mut self.output
        {
            config.present_mode = choose_present_mode(present_modes, settings.vsync);
            if config.width > 0 && config.height > 0 {
                surface.configure(&self.device, config);
            }
        }
    }

    fn recreate_targets(&mut self) {
        let max = self.device.limits().max_texture_dimension_2d;
        let (w, h) = internal_size_for(self.window_size, self.settings.internal_height);
        let size = (w.min(max), h.min(max));
        if size != self.targets.size {
            self.targets = create_targets(
                &self.device,
                &self.layouts,
                size,
                &self.post_buffer,
                &self.output_buffer,
                &self.point_sampler,
            );
        }
    }

    // ----------------------------------------------------------------------------------------
    // Resources

    pub fn create_mesh(&mut self, data: &MeshData) -> MeshId {
        let mesh = self.upload_mesh(data, None);
        let id = match self.free_mesh_ids.pop() {
            Some(id) => {
                self.meshes[id as usize] = Some(mesh);
                id
            }
            None => {
                self.meshes.push(Some(mesh));
                self.meshes.len() as u32 - 1
            }
        };
        MeshId(id)
    }

    pub fn update_mesh(&mut self, id: MeshId, data: &MeshData) {
        let Some(slot) = self.meshes.get_mut(id.0 as usize) else {
            log::warn!("update_mesh: unknown mesh {id:?}");
            return;
        };
        let previous = slot.take().map(|m| m.alloc);
        let mesh = self.upload_mesh(data, previous);
        self.meshes[id.0 as usize] = Some(mesh);
    }

    /// Releases a mesh's GPU memory; the id may be handed out again by `create_mesh`.
    pub fn free_mesh(&mut self, id: MeshId) {
        if let Some(mesh) = self.meshes.get_mut(id.0 as usize).and_then(Option::take) {
            self.arena.free(mesh.alloc);
            self.free_mesh_ids.push(id.0);
        }
    }

    fn upload_mesh(&mut self, data: &MeshData, previous: Option<MeshAlloc>) -> GpuMesh {
        let valid = data.indices.len().is_multiple_of(3)
            && data
                .indices
                .iter()
                .all(|&i| (i as usize) < data.vertices.len());
        let empty = MeshData::default();
        let data = if valid {
            data
        } else {
            log::error!("mesh has out-of-range indices or a partial triangle; uploading it empty");
            &empty
        };
        let (center, radius) = bounding_sphere(data);
        let alloc = self.arena.upload(&self.device, &self.queue, data, previous);
        GpuMesh {
            alloc,
            index_count: data.indices.len() as u32,
            center,
            radius,
        }
    }

    pub fn create_texture(&mut self, data: &TextureData) -> TextureId {
        let texture = self.upload_texture(data);
        self.textures.push(Some(texture));
        TextureId(self.textures.len() as u32 - 1)
    }

    /// Replaces a texture's pixels (and size).
    pub fn update_texture(&mut self, id: TextureId, data: &TextureData) {
        match self.textures.get(id.0 as usize) {
            Some(Some(existing))
                if existing.size == (data.width, data.height)
                    && texture_data_valid(data, u32::MAX) =>
            {
                write_texture_pixels(&self.queue, &existing.texture, data);
            }
            Some(_) => {
                let texture = self.upload_texture(data);
                self.textures[id.0 as usize] = Some(texture);
            }
            None => log::warn!("update_texture: unknown texture {id:?}"),
        }
    }

    /// Releases a texture. Draws still referencing it fall back to white.
    pub fn free_texture(&mut self, id: TextureId) {
        if id == self.white || id == self.font {
            return;
        }
        if let Some(slot) = self.textures.get_mut(id.0 as usize) {
            *slot = None;
        }
    }

    fn upload_texture(&self, data: &TextureData) -> GpuTexture {
        let max = self.device.limits().max_texture_dimension_2d;
        let fallback;
        let data = if texture_data_valid(data, max) {
            data
        } else {
            log::error!(
                "invalid texture data ({}x{}, {} bytes); using a placeholder",
                data.width,
                data.height,
                data.rgba.len()
            );
            fallback = TextureData {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 255, 255],
            };
            &fallback
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size: wgpu::Extent3d {
                width: data.width,
                height: data.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_texture_pixels(&self.queue, &texture, data);
        let view = texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture"),
            layout: &self.layouts.texture,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        GpuTexture {
            texture,
            bind_group,
            size: (data.width, data.height),
        }
    }

    fn texture_bind_group(&self, id: u32) -> &wgpu::BindGroup {
        match self.textures.get(id as usize) {
            Some(Some(t)) => &t.bind_group,
            _ => match &self.textures[self.white.0 as usize] {
                Some(t) => &t.bind_group,
                None => unreachable!("the white texture is never freed"),
            },
        }
    }

    // ----------------------------------------------------------------------------------------
    // Shaders

    /// Re-reads `assets/shaders/*.wgsl` and rebuilds every pipeline. On a compile error the old
    /// pipelines stay active and the error message is returned.
    pub fn reload_shaders(&mut self) -> Result<(), String> {
        self.reload_shaders_from(&crate::assets_dir().join("shaders"))
    }

    /// Like [`Renderer::reload_shaders`] but from an explicit directory.
    pub fn reload_shaders_from(&mut self, dir: &Path) -> Result<(), String> {
        let result = ShaderSources::from_dir(dir).and_then(|sources| {
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let pipelines = Pipelines::new(&self.device, &self.layouts, &sources, self.output_format);
            match pollster::block_on(scope.pop()) {
                Some(error) => {
                    let mut message = error.to_string();
                    if message.contains("scene.wgsl") || message.contains("fx.wgsl") {
                        message.push_str(&format!(
                            "\nnote: line numbers include common.wgsl ({} lines) prepended to the file",
                            sources.common.lines().count() + 1
                        ));
                    }
                    Err(message)
                }
                None => Ok(pipelines),
            }
        });
        match result {
            Ok(pipelines) => {
                self.pipelines = pipelines;
                self.shader_error = None;
                log::info!("shaders reloaded from {}", dir.display());
                Ok(())
            }
            Err(error) => {
                self.shader_error = Some(error.clone());
                Err(error)
            }
        }
    }

    /// Watch `assets/shaders/` and reload automatically (checked once per `render`).
    pub fn set_shader_hot_reload(&mut self, enabled: bool) {
        if !enabled {
            self.shader_watcher = None;
            return;
        }
        if self.shader_watcher.is_none() {
            let dir = crate::assets_dir().join("shaders");
            match FileWatcher::new_dir(&dir) {
                Ok(watcher) => self.shader_watcher = Some(watcher),
                Err(e) => log::warn!("cannot watch {}: {e}", dir.display()),
            }
        }
    }

    fn poll_shader_watcher(&mut self) {
        let changed = self.shader_watcher.as_ref().is_some_and(|w| {
            w.poll_changed()
                .iter()
                .any(|p| p.extension().is_some_and(|e| e == "wgsl"))
        });
        if changed && let Err(e) = self.reload_shaders() {
            log::error!("shader reload failed, keeping the old shaders:\n{e}");
        }
    }

    // ----------------------------------------------------------------------------------------
    // Capture

    /// Saves the next rendered frame (final window-size image, including UI and overlay) as PNG.
    pub fn capture_png(&mut self, path: &Path) {
        self.pending_capture = Some(path.to_path_buf());
    }

    /// Reads back the current headless image (call after `render`). Errors for window output.
    pub fn read_pixels(&mut self) -> anyhow::Result<TextureData> {
        let Output::Headless { texture } = &self.output else {
            anyhow::bail!("read_pixels is only available on headless renderers");
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let readback = Readback::encode(&self.device, &mut encoder, texture);
        self.queue.submit([encoder.finish()]);
        readback.finish(&self.device, self.output_format)
    }

    // ----------------------------------------------------------------------------------------
    // Frame

    /// Renders one frame. Returns `Ok` without drawing while minimised or when the surface is
    /// temporarily unavailable.
    pub fn render(
        &mut self,
        frame: &FrameDesc,
        overlay: Option<&mut dyn OverlayPainter>,
    ) -> anyhow::Result<()> {
        self.poll_shader_watcher();
        if std::mem::take(&mut self.skip_frame) {
            return Ok(());
        }
        if self.window_size.0 == 0 || self.window_size.1 == 0 {
            return Ok(());
        }
        let (surface_texture, suboptimal) = match self.acquire()? {
            Acquired::Surface(texture, suboptimal) => (Some(texture), suboptimal),
            Acquired::Headless => (None, false),
            Acquired::Skip => return Ok(()),
        };
        let output_texture = match (&surface_texture, &self.output) {
            (Some(st), _) => st.texture.clone(),
            (None, Output::Headless { texture }) => texture.clone(),
            (None, Output::Surface { .. }) => return Ok(()),
        };
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.output_format),
            ..Default::default()
        });
        let size = [output_texture.width(), output_texture.height()];

        self.prepare(frame, (size[0], size[1]));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.encode_scene(&mut encoder, frame);
        self.encode_post_and_output(&mut encoder, &output_view);
        if let Some(overlay) = overlay {
            overlay.paint(&self.device, &self.queue, &mut encoder, &output_view, size);
        }

        let capture = self.pending_capture.take().and_then(|path| {
            if output_texture
                .usage()
                .contains(wgpu::TextureUsages::COPY_SRC)
            {
                Some((
                    path,
                    Readback::encode(&self.device, &mut encoder, &output_texture),
                ))
            } else {
                log::warn!("this surface does not support capturing frames");
                None
            }
        });

        self.queue.submit([encoder.finish()]);

        if let Some((path, readback)) = capture {
            match readback
                .finish(&self.device, self.output_format)
                .and_then(|image| save_png(&path, &image))
            {
                Ok(()) => log::info!("saved capture {}", path.display()),
                Err(e) => log::error!("capture {} failed: {e:#}", path.display()),
            }
        }

        if let Some(st) = surface_texture {
            if let Output::Surface { window, .. } = &self.output {
                window.pre_present_notify();
            }
            self.queue.present(st);
        }
        if suboptimal
            && let Output::Surface {
                surface, config, ..
            } = &self.output
        {
            surface.configure(&self.device, config);
        }
        Ok(())
    }

    /// Gets the image to render into for this frame.
    fn acquire(&mut self) -> anyhow::Result<Acquired> {
        let Output::Surface {
            surface,
            config,
            window,
            ..
        } = &mut self.output
        else {
            return Ok(Acquired::Headless);
        };
        match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => Ok(Acquired::Surface(t, false)),
            // Still usable: render into it and reconfigure after presenting.
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => Ok(Acquired::Surface(t, true)),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Ok(Acquired::Skip)
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    config.width = size.width;
                    config.height = size.height;
                    surface.configure(&self.device, config);
                }
                Ok(Acquired::Skip)
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                log::warn!("surface lost; recreating it");
                *surface = self
                    .instance
                    .create_surface(window.clone())
                    .context("recreating the lost window surface")?;
                surface.configure(&self.device, config);
                Ok(Acquired::Skip)
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error while acquiring a frame");
                Ok(Acquired::Skip)
            }
        }
    }

    /// Builds all per-frame GPU data (uniforms, draw list, sprite/UI vertices) and uploads it.
    fn prepare(&mut self, frame: &FrameDesc, output_size: (u32, u32)) {
        let (iw, ih) = self.targets.size;
        let aspect = iw as f32 / ih.max(1) as f32;
        let camera = &frame.camera;
        let near = camera.near.max(0.001);
        let far = camera.far.max(near + 0.01);
        let fov = camera.fov_y_degrees.clamp(1.0, 170.0).to_radians();
        // Reverse-Z (near -> 1, far -> 0) for much better depth precision far away.
        let view = camera.view_matrix();
        let view_proj = reverse_z_perspective(fov, aspect, near, far) * view;
        let vm_fov = if frame.viewmodel_fov_y_degrees > 0.0 {
            frame.viewmodel_fov_y_degrees.clamp(1.0, 170.0).to_radians()
        } else {
            fov
        };
        let vm_view_proj =
            reverse_z_perspective(vm_fov, aspect, VIEWMODEL_NEAR, VIEWMODEL_FAR) * view;
        let camera_world = Mat4::from_rotation_translation(camera.rotation, camera.position);
        let frustum = Frustum::from_view_proj(view_proj);
        let s = &self.settings;
        let lists = &mut self.lists;

        // Frame uniform.
        select_lights(frame.lights, camera.position, &mut lists.lights);
        let mut light_pos = [[0.0; 4]; MAX_LIGHTS];
        let mut light_color = [[0.0; 4]; MAX_LIGHTS];
        for (i, l) in lists.lights.iter().enumerate() {
            light_pos[i] = l.position.extend(l.radius).to_array();
            light_color[i] = l.color.extend(1.0).to_array();
        }
        let key_dir = (-frame.light_dir).try_normalize().unwrap_or(Vec3::Y);
        let strength = s.vertex_snap_strength.max(0.25);
        let gpu_frame = GpuFrame {
            view_proj: view_proj.to_cols_array_2d(),
            viewmodel_view_proj: vm_view_proj.to_cols_array_2d(),
            camera_pos: camera.position.extend(1.0).to_array(),
            fog_color: frame.fog.color.extend(s.fog as u32 as f32).to_array(),
            fog_params: [
                frame.fog.start,
                1.0 / (frame.fog.end - frame.fog.start).max(0.001),
                0.0,
                0.0,
            ],
            ambient: frame.ambient.extend(0.0).to_array(),
            key_dir: key_dir.extend(0.0).to_array(),
            key_color: frame.light_color.extend(0.0).to_array(),
            snap: [
                iw as f32 * 0.5 / strength,
                ih as f32 * 0.5 / strength,
                s.vertex_snap as u32 as f32,
                s.affine_textures as u32 as f32,
            ],
            lighting: [
                if s.vertex_lighting {
                    lists.lights.len() as f32
                } else {
                    0.0
                },
                0.0,
                0.0,
                0.0,
            ],
            light_pos,
            light_color,
        };
        self.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&gpu_frame));

        // Mesh draws.
        lists.draws.clear();
        lists.opaque.clear();
        lists.translucent.clear();
        lists.vm_opaque.clear();
        lists.vm_translucent.clear();
        let mut culled = 0;
        let mut triangles = 0u64;
        for (items, viewmodel) in [(frame.draws, false), (frame.viewmodel, true)] {
            for item in items {
                let Some(Some(mesh)) = self.meshes.get(item.mesh.0 as usize) else {
                    continue;
                };
                if mesh.index_count == 0 {
                    continue;
                }
                let model = if viewmodel {
                    camera_world * item.transform
                } else {
                    item.transform
                };
                let center = model.transform_point3(mesh.center);
                if !viewmodel {
                    let scale = model
                        .x_axis
                        .truncate()
                        .length()
                        .max(model.y_axis.truncate().length())
                        .max(model.z_axis.truncate().length());
                    if !frustum.sphere_visible(center, mesh.radius * scale) {
                        culled += 1;
                        continue;
                    }
                }
                let translucent = item.tint.w < 0.999;
                let cmd = DrawCmd {
                    draw: lists.draws.len() as u32,
                    texture: item.texture.0,
                    first_index: mesh.alloc.indices.start,
                    index_count: mesh.index_count,
                    base_vertex: mesh.alloc.vertices.start as i32,
                    sort: center.distance_squared(camera.position),
                };
                triangles += mesh.index_count as u64 / 3;
                lists
                    .draws
                    .push(GpuDraw::new(model, item.tint, item.unlit, viewmodel));
                match (viewmodel, translucent) {
                    (false, false) => lists.opaque.push(cmd),
                    (false, true) => lists.translucent.push(cmd),
                    (true, false) => lists.vm_opaque.push(cmd),
                    (true, true) => lists.vm_translucent.push(cmd),
                }
            }
        }
        // Opaque: group by texture, then by position in the arena. Translucent: back to front.
        lists
            .opaque
            .sort_unstable_by_key(|c| ((c.texture as u64) << 32) | c.first_index as u64);
        lists
            .translucent
            .sort_unstable_by(|a, b| b.sort.total_cmp(&a.sort));
        lists
            .vm_translucent
            .sort_by(|a, b| b.sort.total_cmp(&a.sort));
        if self.draw_buffer.write(
            &self.device,
            &self.queue,
            bytemuck::cast_slice(&lists.draws),
        ) {
            self.frame_bind_group = create_frame_bind_group(
                &self.device,
                &self.layouts,
                &self.frame_buffer,
                &self.draw_buffer.buffer,
                &self.point_sampler,
            );
        }

        // Sprites, decals, debug lines.
        lists.fx.build(
            camera,
            &frustum,
            frame.sprites,
            frame.decals,
            frame.debug_lines,
        );
        self.fx_buffer.write(
            &self.device,
            &self.queue,
            bytemuck::cast_slice(&lists.fx.vertices),
        );

        // UI.
        lists.ui.build(frame.ui, output_size, self.font, self.white);
        self.ui_buffer.write(
            &self.device,
            &self.queue,
            bytemuck::cast_slice(&lists.ui.vertices),
        );

        // Post + output uniforms.
        let bits = s.color_bits.clamp(1, 8);
        let post = PostUniform {
            flash: frame.screen_flash.to_array(),
            params: [
                if bits >= 8 {
                    0.0
                } else {
                    ((1u32 << bits) - 1) as f32
                },
                s.dithering as u32 as f32,
                0.0,
                0.0,
            ],
        };
        self.queue
            .write_buffer(&self.post_buffer, 0, bytemuck::bytes_of(&post));
        let out = OutputUniform {
            sizes: [
                iw as f32,
                ih as f32,
                output_size.0 as f32,
                output_size.1 as f32,
            ],
            flags: [self.output_format.is_srgb() as u32 as f32, 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.output_buffer, 0, bytemuck::bytes_of(&out));

        let fx = &lists.fx;
        self.stats = RenderStats {
            draws: lists.draws.len() as u32,
            culled,
            sprites: (fx
                .sprites_alpha
                .iter()
                .chain(&fx.sprites_additive)
                .map(|r| r.count)
                .sum::<u32>())
                / 6,
            decals: fx.decals.iter().map(|r| r.count).sum::<u32>() / 6,
            debug_lines: fx.lines.1 / 2,
            ui_quads: lists.ui.vertices.len() as u32 / 6,
            triangles,
            internal_size: self.targets.size,
        };
    }

    fn draw_meshes(&self, pass: &mut wgpu::RenderPass<'_>, cmds: &[DrawCmd]) {
        let mut bound = u32::MAX;
        for c in cmds {
            if c.texture != bound {
                pass.set_bind_group(1, self.texture_bind_group(c.texture), &[]);
                bound = c.texture;
            }
            pass.draw_indexed(
                c.first_index..c.first_index + c.index_count,
                c.base_vertex,
                c.draw..c.draw + 1,
            );
        }
    }

    fn draw_runs(&self, pass: &mut wgpu::RenderPass<'_>, runs: &[Run]) {
        for run in runs {
            pass.set_bind_group(1, self.texture_bind_group(run.texture.0), &[]);
            pass.draw(run.start..run.start + run.count, 0..1);
        }
    }

    fn bind_arena(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.arena.vertex_buffer.slice(..));
        pass.set_index_buffer(self.arena.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
    }

    fn encode_scene(&self, encoder: &mut wgpu::CommandEncoder, frame: &FrameDesc) {
        let lists = &self.lists;
        let p = &self.pipelines;
        let c = frame.clear_color;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: c.x as f64,
                            g: c.y as f64,
                            b: c.z as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);

            if !lists.opaque.is_empty() {
                self.bind_arena(&mut pass);
                pass.set_pipeline(&p.scene_opaque);
                self.draw_meshes(&mut pass, &lists.opaque);
            }
            let fx = &lists.fx;
            if !fx.decals.is_empty() {
                pass.set_vertex_buffer(0, self.fx_buffer.buffer.slice(..));
                pass.set_pipeline(&p.fx_decal);
                self.draw_runs(&mut pass, &fx.decals);
            }
            if !lists.translucent.is_empty() {
                self.bind_arena(&mut pass);
                pass.set_pipeline(&p.scene_translucent);
                self.draw_meshes(&mut pass, &lists.translucent);
            }
            if !fx.vertices.is_empty() {
                pass.set_vertex_buffer(0, self.fx_buffer.buffer.slice(..));
                if !fx.sprites_alpha.is_empty() {
                    pass.set_pipeline(&p.fx_alpha);
                    self.draw_runs(&mut pass, &fx.sprites_alpha);
                }
                if !fx.sprites_additive.is_empty() {
                    pass.set_pipeline(&p.fx_additive);
                    self.draw_runs(&mut pass, &fx.sprites_additive);
                }
                if fx.lines.1 > 0 {
                    pass.set_pipeline(&p.lines);
                    pass.set_bind_group(1, self.texture_bind_group(self.white.0), &[]);
                    pass.draw(fx.lines.0..fx.lines.0 + fx.lines.1, 0..1);
                }
            }
        }

        if lists.vm_opaque.is_empty() && lists.vm_translucent.is_empty() {
            return;
        }
        // Viewmodel: same colour target, fresh depth so the weapon never clips into walls.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewmodel"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.targets.scene_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.targets.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.frame_bind_group, &[]);
        self.bind_arena(&mut pass);
        if !lists.vm_opaque.is_empty() {
            pass.set_pipeline(&p.scene_opaque);
            self.draw_meshes(&mut pass, &lists.vm_opaque);
        }
        if !lists.vm_translucent.is_empty() {
            pass.set_pipeline(&p.scene_translucent);
            self.draw_meshes(&mut pass, &lists.vm_translucent);
        }
    }

    fn encode_post_and_output(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output_view: &wgpu::TextureView,
    ) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.post_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines.post);
            pass.set_bind_group(0, &self.targets.post_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("output"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipelines.upscale);
        pass.set_bind_group(0, &self.targets.output_bind_group, &[]);
        pass.draw(0..3, 0..1);

        let ui = &self.lists.ui;
        if !ui.runs.is_empty() {
            pass.set_pipeline(&self.pipelines.ui);
            pass.set_vertex_buffer(0, self.ui_buffer.buffer.slice(..));
            self.draw_runs(&mut pass, &ui.runs);
        }
    }
}

/// Right-handed perspective with reverse-Z depth (near plane -> 1, far plane -> 0).
pub(crate) fn reverse_z_perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    // Swapping near and far in the regular [0, 1] projection flips the depth range.
    glam::camera::rh::proj::directx::perspective(fov_y, aspect, far, near)
}

fn texture_data_valid(data: &TextureData, max_dim: u32) -> bool {
    data.width > 0
        && data.height > 0
        && data.width <= max_dim
        && data.height <= max_dim
        && data.rgba.len() == data.width as usize * data.height as usize * 4
}

fn write_texture_pixels(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &TextureData) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(data.width * 4),
            rows_per_image: Some(data.height),
        },
        wgpu::Extent3d {
            width: data.width,
            height: data.height,
            depth_or_array_layers: 1,
        },
    );
}

fn bounding_sphere(data: &MeshData) -> (Vec3, f32) {
    if data.vertices.is_empty() {
        return (Vec3::ZERO, 0.0);
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in &data.vertices {
        let p = Vec3::from(v.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let center = (lo + hi) * 0.5;
    let radius = data
        .vertices
        .iter()
        .map(|v| Vec3::from(v.position).distance_squared(center))
        .fold(0.0f32, f32::max)
        .sqrt();
    (center, radius)
}

fn create_headless_texture(
    device: &wgpu::Device,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless output"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn create_frame_bind_group(
    device: &wgpu::Device,
    layouts: &Layouts,
    frame: &wgpu::Buffer,
    draws: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame"),
        layout: &layouts.frame,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: draws.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn create_targets(
    device: &wgpu::Device,
    layouts: &Layouts,
    size: (u32, u32),
    post_uniform: &wgpu::Buffer,
    output_uniform: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
) -> Targets {
    let target = |label, format, usage| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let scene_view = target("scene colour", SCENE_FORMAT, rt);
    let depth_view = target(
        "scene depth",
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let post_view = target("post", POST_FORMAT, rt);
    let post_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("post"),
        layout: &layouts.post,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: post_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene_view),
            },
        ],
    });
    let output_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("output"),
        layout: &layouts.output,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: output_uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&post_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    Targets {
        size,
        scene_view,
        depth_view,
        post_bind_group,
        output_bind_group,
        post_view,
    }
}

/// A texture -> buffer copy waiting to be mapped.
struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row: u32,
}

impl Readback {
    fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Self {
        let (width, height) = (texture.width(), texture.height());
        let padded_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Self {
            buffer,
            width,
            height,
            padded_row,
        }
    }

    /// Waits for the GPU and returns tightly packed RGBA8 pixels.
    fn finish(
        self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> anyhow::Result<TextureData> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow::anyhow!("waiting for the GPU: {e}"))?;
        rx.recv()
            .context("readback callback dropped")?
            .map_err(|e| anyhow::anyhow!("mapping the readback buffer: {e}"))?;
        let mapped = self
            .buffer
            .get_mapped_range(..)
            .map_err(|e| anyhow::anyhow!("reading the readback buffer: {e}"))?;
        let row = self.width as usize * 4;
        let mut rgba = Vec::with_capacity(row * self.height as usize);
        for y in 0..self.height as usize {
            let start = y * self.padded_row as usize;
            rgba.extend_from_slice(&mapped[start..start + row]);
        }
        drop(mapped);
        self.buffer.unmap();
        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        if bgra {
            for px in rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        // The presentation alpha is meaningless; save opaque images.
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(TextureData {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

fn save_png(path: &Path, image: &TextureData) -> anyhow::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    image::save_buffer_with_format(
        path,
        &image.rgba,
        image.width,
        image.height,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::{Camera, DrawItem, Fog, UiDrawList, Vertex};
    use glam::{Vec2, Vec4};

    /// Headless renderer, or `None` (test skipped) on machines without a GPU adapter.
    fn headless(settings: &RetroSettings) -> Option<Renderer> {
        match Renderer::new_headless(64, 48, settings) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("skipping GPU test: {e:#}");
                None
            }
        }
    }

    fn clean_settings() -> RetroSettings {
        RetroSettings {
            internal_height: 0,
            vertex_snap: false,
            dithering: false,
            color_bits: 8,
            fog: false,
            ..Default::default()
        }
    }

    /// Unlit quad covering the middle of the view, 2 m in front of the camera.
    fn render_quad(r: &mut Renderer, tint: Vec4, flash: Vec4) -> TextureData {
        let v = |x: f32, y: f32| Vertex {
            position: [x, y, -2.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.5, 0.5],
            color: [1.0; 4],
        };
        let mesh = r.create_mesh(&MeshData {
            vertices: vec![v(-0.5, -0.5), v(0.5, -0.5), v(0.5, 0.5), v(-0.5, 0.5)],
            indices: vec![0, 1, 2, 0, 2, 3],
        });
        let draws = [DrawItem {
            mesh,
            texture: r.white_texture(),
            transform: Mat4::IDENTITY,
            tint,
            unlit: true,
        }];
        let ui = UiDrawList::new(Vec2::ZERO);
        let frame = FrameDesc {
            camera: Camera::default(),
            clear_color: Vec3::new(0.0, 0.0, 1.0),
            fog: Fog::default(),
            ambient: Vec3::ONE,
            light_dir: -Vec3::Y,
            light_color: Vec3::ZERO,
            lights: &[],
            draws: &draws,
            sprites: &[],
            decals: &[],
            viewmodel: &[],
            viewmodel_fov_y_degrees: 60.0,
            debug_lines: &[],
            screen_flash: flash,
            ui: &ui,
        };
        r.render(&frame, None).unwrap();
        r.read_pixels().unwrap()
    }

    fn pixel(image: &TextureData, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width + x) * 4) as usize;
        [
            image.rgba[i],
            image.rgba[i + 1],
            image.rgba[i + 2],
            image.rgba[i + 3],
        ]
    }

    #[test]
    fn gpu_renders_quad_over_clear_colour() {
        let Some(mut r) = headless(&clean_settings()) else {
            return;
        };
        let image = render_quad(&mut r, Vec4::new(1.0, 0.0, 0.0, 1.0), Vec4::ZERO);
        assert_eq!((image.width, image.height), (64, 48));
        assert_eq!(
            pixel(&image, 32, 24),
            [255, 0, 0, 255],
            "quad in the centre"
        );
        assert_eq!(
            pixel(&image, 1, 1),
            [0, 0, 255, 255],
            "clear colour in the corner"
        );
        // Full-strength white flash covers everything.
        let flashed = render_quad(&mut r, Vec4::new(1.0, 0.0, 0.0, 1.0), Vec4::ONE);
        assert_eq!(pixel(&flashed, 32, 24), [255, 255, 255, 255]);
    }

    #[test]
    fn gpu_colour_reduction_quantises() {
        let settings = RetroSettings {
            color_bits: 2,
            ..clean_settings()
        };
        let Some(mut r) = headless(&settings) else {
            return;
        };
        let image = render_quad(&mut r, Vec4::new(0.45, 0.45, 0.45, 1.0), Vec4::ZERO);
        // 2 bits = levels 0, 85, 170, 255; 0.45 rounds to 1/3.
        assert_eq!(pixel(&image, 32, 24), [85, 85, 85, 255]);
    }

    #[test]
    fn gpu_broken_shader_keeps_old_pipelines() {
        let Some(mut r) = headless(&clean_settings()) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("velocide-shaders-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sources = ShaderSources::embedded();
        for (name, text) in [
            ("common.wgsl", &sources.common),
            ("scene.wgsl", &sources.scene),
            ("fx.wgsl", &sources.fx),
            ("post.wgsl", &sources.post),
            ("blit.wgsl", &sources.blit),
        ] {
            std::fs::write(dir.join(name), text).unwrap();
        }
        assert_eq!(r.reload_shaders_from(&dir), Ok(()));
        std::fs::write(dir.join("post.wgsl"), "fn broken( {").unwrap();
        let error = r.reload_shaders_from(&dir).unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(r.shader_error(), Some(error.as_str()));
        // Still renders with the previous pipelines.
        let image = render_quad(&mut r, Vec4::new(0.0, 1.0, 0.0, 1.0), Vec4::ZERO);
        assert_eq!(pixel(&image, 32, 24), [0, 255, 0, 255]);
        assert!(r.reload_shaders_from(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).ok();
        eprintln!("shader error message:\n{error}");
    }

    #[test]
    fn projection_is_reverse_z() {
        let proj = reverse_z_perspective(1.2, 1.5, 0.1, 100.0);
        let depth = |z: f32| {
            let clip = proj * glam::Vec4::new(0.0, 0.0, z, 1.0);
            clip.z / clip.w
        };
        assert!((depth(-0.1) - 1.0).abs() < 1e-5);
        assert!(depth(-100.0).abs() < 1e-5);
        assert!(depth(-1.0) > depth(-10.0));
    }

    #[test]
    fn internal_size_follows_aspect() {
        assert_eq!(internal_size_for((1920, 1080), 360), (640, 360));
        assert_eq!(internal_size_for((1280, 1024), 240), (300, 240));
        // 0 = native, and never upscale above the window.
        assert_eq!(internal_size_for((800, 600), 0), (800, 600));
        assert_eq!(internal_size_for((800, 600), 1080), (800, 600));
        assert_eq!(internal_size_for((0, 0), 360), (1, 1));
    }

    #[test]
    fn present_mode_fallbacks() {
        use wgpu::PresentMode::*;
        assert_eq!(choose_present_mode(&[Fifo, Mailbox], true), Fifo);
        assert_eq!(
            choose_present_mode(&[Fifo, Mailbox, Immediate], false),
            Mailbox
        );
        assert_eq!(choose_present_mode(&[Fifo, Immediate], false), Immediate);
        assert_eq!(choose_present_mode(&[Fifo], false), Fifo);
    }
}
