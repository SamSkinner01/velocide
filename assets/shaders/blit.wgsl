// Nearest-neighbour upscale of the post-processed low-res image to the window, plus the 2D UI.

struct Output {
    // xy = source (internal) size, zw = destination (window) size.
    sizes: vec4<f32>,
    // x = 1 when the output view is an sRGB format (values must be linearised before writing).
    flags: vec4<f32>,
}

@group(0) @binding(0) var<uniform> output: Output;
@group(0) @binding(1) var source_tex: texture_2d<f32>;
@group(0) @binding(2) var ui_sampler: sampler;
@group(1) @binding(0) var ui_tex: texture_2d<f32>;

fn to_output(color: vec3<f32>) -> vec3<f32> {
    if output.flags.x > 0.5 {
        // Gamma -> linear; the sRGB view encodes it back on write.
        let lo = color / 12.92;
        let hi = pow((color + 0.055) / 1.055, vec3<f32>(2.4));
        return select(hi, lo, color <= vec3<f32>(0.04045));
    }
    return color;
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_upscale(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let src_size = output.sizes.xy;
    let scaled = floor(pos.xy * src_size / output.sizes.zw);
    let p = vec2<i32>(clamp(scaled, vec2<f32>(0.0), src_size - 1.0));
    let color = textureLoad(source_tex, p, 0).rgb;
    return vec4<f32>(to_output(color), 1.0);
}

struct UiIn {
    // Already in normalised device coordinates.
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct UiOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_ui(in: UiIn) -> UiOut {
    var out: UiOut;
    out.clip = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_ui(in: UiOut) -> @location(0) vec4<f32> {
    let color = textureSample(ui_tex, ui_sampler, in.uv) * in.color;
    return vec4<f32>(to_output(color.rgb), color.a);
}
