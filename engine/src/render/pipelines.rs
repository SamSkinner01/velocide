//! Shader sources, bind group layouts and every render pipeline.
//!
//! Shaders live in `assets/shaders/*.wgsl`. They are embedded at compile time as the startup
//! default and can be re-read from disk at runtime (`Renderer::reload_shaders`).

use std::path::Path;

use crate::gfx::Vertex;

use super::batch::{FxVertex, UiVertex};

pub(crate) const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const POST_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Textures hold raw sRGB bytes and are sampled as-is (gamma-space shading).
pub(crate) const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// WGSL sources of all shaders. `common` is prepended to `scene` and `fx`.
#[derive(Clone)]
pub(crate) struct ShaderSources {
    pub common: String,
    pub scene: String,
    pub fx: String,
    pub post: String,
    pub blit: String,
}

impl ShaderSources {
    pub fn embedded() -> Self {
        Self {
            common: include_str!("../../../assets/shaders/common.wgsl").to_owned(),
            scene: include_str!("../../../assets/shaders/scene.wgsl").to_owned(),
            fx: include_str!("../../../assets/shaders/fx.wgsl").to_owned(),
            post: include_str!("../../../assets/shaders/post.wgsl").to_owned(),
            blit: include_str!("../../../assets/shaders/blit.wgsl").to_owned(),
        }
    }

    pub fn from_dir(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
        };
        Ok(Self {
            common: read("common.wgsl")?,
            scene: read("scene.wgsl")?,
            fx: read("fx.wgsl")?,
            post: read("post.wgsl")?,
            blit: read("blit.wgsl")?,
        })
    }
}

pub(crate) struct Layouts {
    /// Group 0 of scene/fx: frame uniform, per-draw storage buffer, point sampler.
    pub frame: wgpu::BindGroupLayout,
    /// Group 1 of scene/fx/ui: one colour texture.
    pub texture: wgpu::BindGroupLayout,
    /// Post pass: uniform + low-res scene colour.
    pub post: wgpu::BindGroupLayout,
    /// Upscale/UI: uniform + post-processed image + sampler.
    pub output: wgpu::BindGroupLayout,
    scene: wgpu::PipelineLayout,
    post_pipeline: wgpu::PipelineLayout,
    upscale: wgpu::PipelineLayout,
    ui: wgpu::PipelineLayout,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl Layouts {
    pub fn new(device: &wgpu::Device) -> Self {
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let frame = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                uniform_entry(0, vf),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                sampler_entry(2),
            ],
        });
        let texture = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[texture_entry(0)],
        });
        let post = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
            ],
        });
        let output = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("output"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let layout = |label: &str, groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        Self {
            scene: layout("scene", &[Some(&frame), Some(&texture)]),
            post_pipeline: layout("post", &[Some(&post)]),
            upscale: layout("upscale", &[Some(&output)]),
            ui: layout("ui", &[Some(&output), Some(&texture)]),
            frame,
            texture,
            post,
            output,
        }
    }
}

pub(crate) struct Pipelines {
    pub scene_opaque: wgpu::RenderPipeline,
    pub scene_translucent: wgpu::RenderPipeline,
    pub fx_alpha: wgpu::RenderPipeline,
    pub fx_decal: wgpu::RenderPipeline,
    pub fx_additive: wgpu::RenderPipeline,
    pub lines: wgpu::RenderPipeline,
    pub post: wgpu::RenderPipeline,
    pub upscale: wgpu::RenderPipeline,
    pub ui: wgpu::RenderPipeline,
}

const MESH_ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4];
const FX_ATTRIBUTES: [wgpu::VertexAttribute; 4] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];
const UI_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

/// Premultiplied-style additive blend that leaves destination alpha alone.
const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Defaults shared by the scene-target pipelines: opaque, back-face culled, depth tested+written.
fn scene_desc<'a>(
    layout: &'a wgpu::PipelineLayout,
    label: &'a str,
    module: &'a wgpu::ShaderModule,
    vs: &'a str,
    fs: &'a str,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
) -> PipelineDesc<'a> {
    PipelineDesc {
        label,
        layout,
        module,
        vs,
        fs,
        buffers,
        format: SCENE_FORMAT,
        blend: None,
        topology: wgpu::PrimitiveTopology::TriangleList,
        cull: Some(wgpu::Face::Back),
        depth: Some(true),
        depth_bias: Default::default(),
    }
}

struct PipelineDesc<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vs: &'a str,
    fs: &'a str,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    topology: wgpu::PrimitiveTopology,
    cull: Option<wgpu::Face>,
    /// `None` = no depth attachment; `Some(write)` = reverse-Z depth test, optionally writing.
    depth: Option<bool>,
    depth_bias: wgpu::DepthBiasState,
}

fn create_pipeline(device: &wgpu::Device, d: PipelineDesc<'_>) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(d.label),
        layout: Some(d.layout),
        vertex: wgpu::VertexState {
            module: d.module,
            entry_point: Some(d.vs),
            compilation_options: Default::default(),
            buffers: d.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: d.topology,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: d.cull,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: d.depth.map(|write| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            // Reverse-Z: larger depth = closer.
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: Default::default(),
            bias: d.depth_bias,
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: d.module,
            entry_point: Some(d.fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: d.format,
                blend: d.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Pipelines {
    /// Builds every pipeline. Errors (bad WGSL) are reported through wgpu's error handling, so
    /// callers wrap this in an error scope.
    pub fn new(
        device: &wgpu::Device,
        layouts: &Layouts,
        sources: &ShaderSources,
        output_format: wgpu::TextureFormat,
    ) -> Self {
        let module = |label: &str, source: String| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let scene = module(
            "scene.wgsl",
            format!("{}\n{}", sources.common, sources.scene),
        );
        let fx = module("fx.wgsl", format!("{}\n{}", sources.common, sources.fx));
        let post = module("post.wgsl", sources.post.clone());
        let blit = module("blit.wgsl", sources.blit.clone());

        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRIBUTES,
        })];
        let fx_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FxVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &FX_ATTRIBUTES,
        })];
        let ui_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<UiVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &UI_ATTRIBUTES,
        })];

        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);

        Self {
            scene_opaque: create_pipeline(
                device,
                scene_desc(
                    &layouts.scene,
                    "scene opaque",
                    &scene,
                    "vs_main",
                    "fs_opaque",
                    &mesh_buffers,
                ),
            ),
            scene_translucent: create_pipeline(
                device,
                PipelineDesc {
                    blend: alpha,
                    depth: Some(false),
                    ..scene_desc(
                        &layouts.scene,
                        "scene translucent",
                        &scene,
                        "vs_main",
                        "fs_translucent",
                        &mesh_buffers,
                    )
                },
            ),
            fx_alpha: create_pipeline(
                device,
                PipelineDesc {
                    blend: alpha,
                    cull: None,
                    depth: Some(false),
                    ..scene_desc(
                        &layouts.scene,
                        "sprites alpha",
                        &fx,
                        "vs_main",
                        "fs_alpha",
                        &fx_buffers,
                    )
                },
            ),
            fx_decal: create_pipeline(
                device,
                PipelineDesc {
                    blend: alpha,
                    cull: None,
                    depth: Some(false),
                    // Reverse-Z: a positive bias pulls decals towards the camera.
                    depth_bias: wgpu::DepthBiasState {
                        constant: 8,
                        slope_scale: 1.5,
                        clamp: 0.0,
                    },
                    ..scene_desc(
                        &layouts.scene,
                        "decals",
                        &fx,
                        "vs_main",
                        "fs_alpha",
                        &fx_buffers,
                    )
                },
            ),
            fx_additive: create_pipeline(
                device,
                PipelineDesc {
                    blend: Some(ADDITIVE),
                    cull: None,
                    depth: Some(false),
                    ..scene_desc(
                        &layouts.scene,
                        "sprites additive",
                        &fx,
                        "vs_main",
                        "fs_additive",
                        &fx_buffers,
                    )
                },
            ),
            lines: create_pipeline(
                device,
                PipelineDesc {
                    blend: alpha,
                    cull: None,
                    depth: Some(false),
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..scene_desc(
                        &layouts.scene,
                        "debug lines",
                        &fx,
                        "vs_main",
                        "fs_alpha",
                        &fx_buffers,
                    )
                },
            ),
            post: create_pipeline(
                device,
                PipelineDesc {
                    layout: &layouts.post_pipeline,
                    format: POST_FORMAT,
                    cull: None,
                    depth: None,
                    ..scene_desc(
                        &layouts.scene,
                        "post",
                        &post,
                        "vs_fullscreen",
                        "fs_main",
                        &[],
                    )
                },
            ),
            upscale: create_pipeline(
                device,
                PipelineDesc {
                    layout: &layouts.upscale,
                    format: output_format,
                    cull: None,
                    depth: None,
                    ..scene_desc(
                        &layouts.scene,
                        "upscale",
                        &blit,
                        "vs_fullscreen",
                        "fs_upscale",
                        &[],
                    )
                },
            ),
            ui: create_pipeline(
                device,
                PipelineDesc {
                    layout: &layouts.ui,
                    format: output_format,
                    blend: alpha,
                    cull: None,
                    depth: None,
                    ..scene_desc(&layouts.scene, "ui", &blit, "vs_ui", "fs_ui", &ui_buffers)
                },
            ),
        }
    }
}
