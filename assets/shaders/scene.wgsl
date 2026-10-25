// World meshes and the viewmodel. `common.wgsl` is prepended to this file.
//
// One storage buffer holds the per-draw data; draws are issued with
// `first_instance = draw index`, so `instance_index` selects the entry.

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @builtin(instance_index) draw_index: u32,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Screen-space (non perspective-correct) copy of the UV for the affine look.
    @location(1) @interpolate(linear) uv_affine: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    let draw = draws[in.draw_index];
    let world = draw.model * vec4<f32>(in.position, 1.0);
    var clip: vec4<f32>;
    if draw.params.y > 0.5 {
        clip = frame.viewmodel_view_proj * world;
    } else {
        clip = frame.view_proj * world;
    }

    var light = vec3<f32>(1.0);
    if draw.params.x < 0.5 {
        let normal = safe_normalize(draw.normal_matrix * in.normal);
        light = vertex_light(world.xyz, normal);
    }

    var out: VertexOut;
    out.clip = snap_vertex(clip);
    out.uv = in.uv;
    out.uv_affine = in.uv;
    out.color = vec4<f32>(in.color.rgb * draw.tint.rgb * light, in.color.a * draw.tint.a);
    out.world_pos = world.xyz;
    return out;
}

fn shade(in: VertexOut) -> vec4<f32> {
    let uv = select(in.uv, in.uv_affine, frame.snap.w > 0.5);
    let color = textureSample(color_tex, point_sampler, uv) * in.color;
    let fog = fog_factor(in.world_pos);
    return vec4<f32>(mix(color.rgb, frame.fog_color.rgb, fog), color.a);
}

// Opaque pass: alpha-tested (grates, cut-outs), writes depth.
@fragment
fn fs_opaque(in: VertexOut) -> @location(0) vec4<f32> {
    let color = shade(in);
    if color.a < 0.5 {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}

// Translucent pass: alpha blended, sorted back to front, no depth write.
@fragment
fn fs_translucent(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in);
}
