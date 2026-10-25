// Post-process at internal resolution: colour-vision correction, screen flash, colour depth
// reduction with 4x4 ordered (Bayer) dithering. Runs in gamma space, like the PS1's 15-bit
// framebuffer; only the colour-vision matrix works on linear values.

struct Post {
    // rgb + strength.
    flash: vec4<f32>,
    // x = quantisation levels per channel (2^bits - 1; 0 = off), y = dithering on,
    // z = colour-vision correction on.
    params: vec4<f32>,
    // Rows of the colour-vision correction matrix (linear RGB, `engine::color_vision`).
    cvd_r: vec4<f32>,
    cvd_g: vec4<f32>,
    cvd_b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var scene_tex: texture_2d<f32>;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

var<private> BAYER: array<f32, 16> = array<f32, 16>(
    0.0, 8.0, 2.0, 10.0,
    12.0, 4.0, 14.0, 6.0,
    3.0, 11.0, 1.0, 9.0,
    15.0, 7.0, 13.0, 5.0,
);

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = x * 12.92;
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, x <= vec3<f32>(0.0031308));
}

// Daltonisation: the matrix moves what the viewer's deficiency hides into channels they see.
// It comes before the flash (flash colours are already picked from the colour-blind-safe
// palette) and before quantisation, so the dither pattern stays crisp.
fn correct_color_vision(c: vec3<f32>) -> vec3<f32> {
    let l = srgb_to_linear(c);
    let m = vec3<f32>(dot(post.cvd_r.xyz, l), dot(post.cvd_g.xyz, l), dot(post.cvd_b.xyz, l));
    return linear_to_srgb(m);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    var color = textureLoad(scene_tex, p, 0).rgb;
    if post.params.z > 0.5 {
        color = correct_color_vision(color);
    }
    color = mix(color, post.flash.rgb, clamp(post.flash.a, 0.0, 1.0));

    let levels = post.params.x;
    if levels > 0.0 {
        // Plain rounding without dithering; otherwise a per-pixel threshold from the Bayer matrix.
        var threshold = 0.5;
        if post.params.y > 0.5 {
            threshold = (BAYER[(p.y & 3) * 4 + (p.x & 3)] + 0.5) / 16.0;
        }
        color = floor(color * levels + threshold) / levels;
    }
    return vec4<f32>(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
