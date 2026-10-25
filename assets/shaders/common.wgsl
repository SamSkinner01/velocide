// Shared declarations for the 3D shaders (scene.wgsl and fx.wgsl).
// The renderer prepends this file to both before compiling, so keep it free of entry points.
//
// All colour maths happens in *gamma space* (like the PS1): textures and vertex colours are used
// as stored (sRGB bytes), lights simply multiply them.

struct Frame {
    view_proj: mat4x4<f32>,
    // View-space viewmodel projection times the world->view matrix, so viewmodel draws can use
    // world-space transforms (and get lit by world lights) but their own FOV / depth range.
    viewmodel_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // rgb = fog colour, a = 1 when fog is enabled.
    fog_color: vec4<f32>,
    // x = fog start, y = 1 / (end - start).
    fog_params: vec4<f32>,
    ambient: vec4<f32>,
    // Unit vector pointing *towards* the key light.
    key_dir: vec4<f32>,
    key_color: vec4<f32>,
    // xy = snap grid (half the internal resolution divided by strength),
    // z = vertex snapping on, w = affine texturing on.
    snap: vec4<f32>,
    // x = number of point lights, y = time in seconds.
    lighting: vec4<f32>,
    // xyz = position, w = radius.
    light_pos: array<vec4<f32>, 16>,
    light_color: array<vec4<f32>, 16>,
}

struct Draw {
    model: mat4x4<f32>,
    normal_matrix: mat3x3<f32>,
    tint: vec4<f32>,
    // x = unlit (fullbright), y = viewmodel.
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> draws: array<Draw>;
@group(0) @binding(2) var point_sampler: sampler;
@group(1) @binding(0) var color_tex: texture_2d<f32>;

// PS1-style wobble: snap the projected vertex to the (coarsened) internal pixel grid.
fn snap_vertex(clip: vec4<f32>) -> vec4<f32> {
    if frame.snap.z < 0.5 || clip.w <= 0.0001 {
        return clip;
    }
    let grid = frame.snap.xy;
    let ndc = clip.xy / clip.w;
    let snapped = floor(ndc * grid + 0.5) / grid;
    return vec4<f32>(snapped * clip.w, clip.z, clip.w);
}

// Gouraud lighting evaluated per vertex: ambient + key light (+ point lights).
fn vertex_light(world_pos: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    var light = frame.ambient.rgb + frame.key_color.rgb * max(dot(normal, frame.key_dir.xyz), 0.0);
    let count = min(u32(frame.lighting.x), 16u);
    for (var i = 0u; i < count; i++) {
        let to_light = frame.light_pos[i].xyz - world_pos;
        let dist = length(to_light);
        let radius = max(frame.light_pos[i].w, 0.001);
        var atten = clamp(1.0 - dist / radius, 0.0, 1.0);
        atten = atten * atten;
        // Slightly wrapped lambert so lights still read on surfaces at grazing angles.
        let ndl = clamp(dot(normal, to_light / max(dist, 0.0001)) * 0.8 + 0.2, 0.0, 1.0);
        light += frame.light_color[i].rgb * (atten * ndl);
    }
    return min(light, vec3<f32>(2.0));
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let len = length(v);
    if len < 0.00001 {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return v / len;
}

fn fog_factor(world_pos: vec3<f32>) -> f32 {
    let dist = distance(world_pos, frame.camera_pos.xyz);
    return clamp((dist - frame.fog_params.x) * frame.fog_params.y, 0.0, 1.0) * frame.fog_color.a;
}
