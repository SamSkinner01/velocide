// Sprites, decals and debug lines. `common.wgsl` is prepended to this file.
//
// The CPU expands every sprite/decal into world-space vertices (one dynamic vertex buffer per
// frame), so this shader only projects, optionally lights, fogs and blends.

struct FxIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    // xyz = surface normal (decals), w = 1 when the vertex should be lit.
    @location(3) normal_lit: vec4<f32>,
}

struct FxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_pos: vec3<f32>,
}

@vertex
fn vs_main(in: FxIn) -> FxOut {
    var color = in.color;
    if in.normal_lit.w > 0.5 {
        color = vec4<f32>(color.rgb * vertex_light(in.position, safe_normalize(in.normal_lit.xyz)), color.a);
    }
    var out: FxOut;
    out.clip = snap_vertex(frame.view_proj * vec4<f32>(in.position, 1.0));
    out.uv = in.uv;
    out.color = color;
    out.world_pos = in.position;
    return out;
}

// Alpha-blended sprites, decals and lines: fade towards the fog colour.
@fragment
fn fs_alpha(in: FxOut) -> @location(0) vec4<f32> {
    let color = textureSample(color_tex, point_sampler, in.uv) * in.color;
    if color.a < 0.004 {
        discard;
    }
    return vec4<f32>(mix(color.rgb, frame.fog_color.rgb, fog_factor(in.world_pos)), color.a);
}

// Additive sprites (fire, flashes, glows): output is premultiplied and fades out in fog.
@fragment
fn fs_additive(in: FxOut) -> @location(0) vec4<f32> {
    let color = textureSample(color_tex, point_sampler, in.uv) * in.color;
    let fade = 1.0 - fog_factor(in.world_pos);
    return vec4<f32>(color.rgb * color.a * fade, 0.0);
}
