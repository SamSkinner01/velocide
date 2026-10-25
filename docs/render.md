# Rendering, assets, app & input

Owner modules: `engine::render`, `engine::assets`, `engine::app`, `engine::input`,
`engine::debug_ui` (feature `debug-ui`), shaders in `assets/shaders/`, models in `assets/models/`.

## Pipeline overview

The game fills a `gfx::FrameDesc` every frame and calls `Renderer::render(&frame, overlay)`.
Each frame runs these passes:

| # | pass | target | contents |
|---|------|--------|----------|
| 1 | **scene** | internal-res colour (`Rgba8Unorm`) + depth (`Depth32Float`, reverse-Z) | opaque meshes (alpha-tested, sorted by texture) → decals (depth-biased) → translucent meshes (`tint.a < 1`, back to front) → alpha sprites (back to front) → additive sprites → debug lines |
| 2 | **viewmodel** | same colour, depth **cleared** | `FrameDesc::viewmodel` items with their own FOV (`viewmodel_fov_y_degrees`, near 0.01 m), so the weapon never clips into walls |
| 3 | **post** | internal-res `Rgba8Unorm` | screen flash blend, colour depth reduction (`color_bits`), 4x4 Bayer dithering |
| 4 | **output** | swapchain / headless image | nearest-neighbour upscale (`textureLoad`, so no filtering at all), then the UI quads at window resolution |
| 5 | overlay | swapchain | `OverlayPainter::paint` (egui) if given |

Internal size: height = `RetroSettings::internal_height` (0 = native, never larger than the
window), width from the window aspect (`render::internal_size_for`). 360 at 1080p is an exact 3x.

### Retro shading (assets/shaders/scene.wgsl + common.wgsl)

- **Vertex snapping**: clip-space XY are snapped to the internal pixel grid divided by
  `vertex_snap_strength` (`snap_vertex`). Depth is untouched.
- **Affine texturing**: the vertex shader also outputs the UV with `@interpolate(linear)`;
  the fragment shader picks it when `affine_textures` is on (big polygons warp, like a PS1).
- **Point sampling**: one nearest/repeat sampler, no mipmaps (distant textures shimmer — on
  purpose).
- **Gouraud lighting** per vertex: `ambient + key_color * max(N·L, 0)` plus up to **16 point
  lights** (`vertex_lighting`; when more are given the 16 nearest the camera win). Point lights use
  `(1 - d/r)^2` falloff and a slightly wrapped Lambert. Light is clamped to 2.0 (2x overbright).
  `DrawItem::unlit` skips lighting (fullbright, still fogged).
- **Fog**: per pixel, linear from `fog.start` to `fog.end` by distance to the camera.
  Additive sprites fade to black instead of to the fog colour.
- Opaque pass **alpha-tests** at 0.5 (grates, cut-outs). Use `tint.a < 1` for real translucency.

Level geometry should be subdivided (meshgen `BoxDesc::brush` splits faces every 2 m), otherwise
point lights only touch the corners of big faces.

### Colour conventions

Everything is shaded in **gamma space** (raw sRGB values), like the hardware we imitate:
textures are uploaded as-is (`Rgba8Unorm`, not `*Srgb`), vertex colours / tints / fog / clear /
sprite / UI / flash colours are all sRGB-encoded `0..1` values, and lights are plain
multipliers. Colour quantisation therefore happens in gamma space (a 5-bit channel is the PS1's
15-bit framebuffer). The swapchain is chosen non-sRGB when available; if only an sRGB format
exists, the output pass linearises before writing so colours look identical. `surface_format()`
reports the format overlay painters render into.

### Depth

Reverse-Z (`near → 1, far → 0`, compare `GreaterEqual`, clear 0) for good precision at distance.
Decals sit 1.2 cm above their surface *and* use a positive depth bias.

## Using the API

```rust
let mut r = Renderer::new(window.clone(), &settings)?;      // or Renderer::new_headless(w, h, &settings)?
let tex = r.create_texture(&assets::procedural::material_texture("brick"));
let mesh = r.create_mesh(&meshgen_builder.build());
// every frame
r.render(&FrameDesc { camera, draws: &draws, sprites: &sprites, ui: &ui, /* ... */ }, None)?;
```

- `resize(w, h)` on window resize (0 x 0 = minimised: rendering is skipped). `apply_settings`
  whenever the settings menu changes something (recreates targets / reconfigures vsync only when
  needed).
- `create_mesh` / `update_mesh` / `free_mesh`, `create_texture` / `update_texture` /
  `free_texture`. Invalid data (out-of-range indices, wrong pixel count) is logged and replaced
  by an empty mesh / magenta texel — never a panic. Unknown ids draw nothing (meshes) or white
  (textures).
- `white_texture()` (1x1 white, for vertex-coloured meshes) and `font_texture()` (from
  `ui::font::build_atlas()`; what `UiTexture::Font` samples).
- Viewmodel transforms are view space (camera at origin looking down -Z). They are lit by the same
  world lights as the scene.
- Sprites: `uv_rect = (u0, v0, u1, v1)` for atlases/animation frames; `rotation` around the view
  axis; `additive` for fire/glows. Decals: oriented by `normal`, `size` is the full width.
- UI: `UiDrawList` canvas units (height 360) are scaled to the window and snapped to whole pixels,
  drawn in order with alpha blending. If `canvas_size` is zero the window's canvas is assumed.
- `capture_png(path)`: the next rendered frame (window-size, including UI and overlay) is saved.
  Works headless and windowed (needs `COPY_SRC` on the swapchain, which Vulkan/NVIDIA provides;
  otherwise a warning is logged). `read_pixels()` returns the headless image (tests).
- `stats()` returns draw/cull/sprite/triangle counts of the last frame; `adapter_name()` for the
  debug overlay.

### Shader hot reload

Shaders: `common.wgsl` (shared structs, bindings, snapping, lighting, fog — prepended to
`scene.wgsl` and `fx.wgsl`), `scene.wgsl` (meshes), `fx.wgsl` (sprites, decals, lines),
`post.wgsl`, `blit.wgsl` (upscale + UI). They are `include_str!`-embedded as the startup default.

- `reload_shaders()` re-reads them from `assets_dir()/shaders`, builds all pipelines inside a
  wgpu validation error scope, and on failure keeps the old pipelines and returns the naga error
  (file + line; for scene/fx the message notes the prepended `common.wgsl` offset).
  `shader_error()` keeps the last error for display.
- `set_shader_hot_reload(true)` watches the directory (`hotreload::FileWatcher`) and reloads
  automatically at the start of `render`.

Uncaptured wgpu errors are logged (`log::error!`) instead of panicking.

## Assets

- `assets::procedural`: `material_texture(name)` for concrete, brick, metal, rust, grate (alpha
  holes), flesh, bone, lava, tile (32 px), wood, hazard (32 px), glass (translucent, 32 px), dirt,
  panel (128 px); unknown names give a magenta checker. `sprite_texture(SpriteTex::…)` for
  MuzzleFlash, Blood, BloodSplat (decal), Spark, Smoke, Glow, Ring, Shard, Gib, Scorch. All built
  from a seeded integer hash, periodic so they tile, and identical on every machine. The noise
  helpers (`hash`, `value_noise`, `fbm`, `cellular`, `ramp`, `Canvas`) are public.
- `assets::meshgen`: `MeshBuilder` (`vertex`, `quad`, `add_box(&BoxDesc)`, `append(mesh, Mat4)`,
  `add_cylinder_between`, `transform`, colour helpers `set_color`, `multiply_color`,
  `map_colors`, `vertical_gradient`, `shade_by_normal`, `ao_near_floor`, `ao_spheres`,
  `jitter_colors`, `bounds`, `flip`) and primitives `cuboid`, `quad`, `cylinder`, `cone`,
  `icosphere` (faceted), `uv_sphere`, `wedge`. `BoxDesc::brush(center, half, rotation)` = level
  brush with world-space UVs at `DEFAULT_TEXELS_PER_METRE` (32 texels/m, so a 64 px texture
  repeats every 2 m; use `BoxUv::texel_density(32.0, texture_size)` for other sizes) and faces
  subdivided every 2 m. World UVs line up across adjacent brushes. Front faces are CCW.
- `assets::gltf`: `load(path) -> Vec<ModelPart>` (default scene, node transforms baked, embedded
  base64 or external `.bin` buffers, PNG/JPEG images, missing normals → flat normals);
  `write(path, &MeshData)` and `write_parts(path, &[ModelPart])` produce a self-contained `.gltf`
  (base64 data URIs, embedded PNG textures, nearest samplers). `ModelPart::base_color` is meant
  as the draw tint.
- `assets::load_png`, `decode_png`, `save_png`.
- `cargo run -p engine --example render_gen_models` regenerates `assets/models/warden.gltf`
  (demo model) and `assets/models/lancet.gltf` (demo gun).

## App, input, debug UI

- `app::run::<G: Game>(AppConfig { title, width, height, retro })`. `EngineContext { window,
  renderer, input, frame_dt, time }` + `set_cursor_grab` (Locked, falls back to Confined, hides the
  cursor, re-applied on focus), `set_fullscreen` (borderless), `quit`. The game renders itself
  from `Game::frame`. `Game::window_event` sees every window event first; returning `true` keeps
  it from the input state.
- `input::InputState`: `key_down/pressed/released`, `key_pressed_or_repeated`, mouse and gamepad
  equivalents, `mouse_delta` (raw `DeviceEvent::MouseMotion`), `scroll_delta` (lines),
  `typed_text`, `cursor_position`, `left_stick`/`right_stick` (radial deadzone),
  `left_trigger`/`right_trigger`, `is_down/pressed/released(Binding)`, `last_pressed_binding`.
  `Binding::{Key, Mouse, Wheel, GamepadButton, GamepadAxis(axis, dir)}` is serde-able with
  `name()` for menus. "Pressed" is true for exactly one frame; focus loss releases everything.
- `debug_ui::DebugUi` (feature `debug-ui`): `new(ctx)`, `on_window_event(window, event)`,
  `run(window, |egui_ctx| …)`, and it is an `OverlayPainter`. `pub use egui`.

## Performance notes

- All meshes live in one vertex + one index buffer (`render/arena.rs`, first-fit allocator,
  buffers double when full), so mesh draws never rebind buffers.
- Per-draw data (model matrix, normal matrix, tint, flags) goes into one storage buffer written
  once per frame; each draw uses `first_instance = draw index` and the shader reads
  `draws[instance_index]`. Opaque draws are sorted by texture so the only state change is the
  texture bind group.
- World draw items are frustum-culled with bounding spheres computed at mesh creation.
- Sprites, decals and lines are expanded on the CPU into one dynamic vertex buffer; additive
  sprites are grouped by texture (order-independent), alpha sprites sorted back to front.
- Measured on the RTX 4070 Ti SUPER, 1920x1080 output, 360p internal, stress scene (~1400
  visible draws after culling 600, ~2100 sprites, `--bench 500 --stress`, GPU waited every
  frame): **~0.75 ms/frame in release** (0.47 ms CPU in `render`), ~2.5 ms in the dev profile.

## Demo / verification

```text
cargo run -p engine --example render_demo                          # windowed, fly camera
cargo run -p engine --features debug-ui --example render_demo      # + egui panel (F1)
cargo run -p engine --example render_demo -- --capture out.png     # headless capture
    [--frames N] [--width W --height H] [--res H] [--no-snap] [--snap-strength S] [--affine]
    [--no-dither] [--bits N] [--no-fog] [--no-lighting] [--stress] [--cam x,y,z,yaw,pitch] [--wall]
cargo run -p engine --example render_demo -- --window-capture out.png --quit-after 90
cargo run -p engine --example render_demo --release -- --bench 500 --stress
```

Windowed keys: click grabs the mouse, WASD/Space/Ctrl/Shift fly, Esc releases; 1 snap, 2 affine,
3 dither, 4 colour bits, 5 fog, 6 point lights, 7 internal res, 8 vsync, 9 snap strength,
0 stress, F2 capture to `captures/`, F5 reload shaders, F11 fullscreen.

GPU tests in `render::tests` (pixel-exact quad/clear colour/flash, colour quantisation, broken
shader reload) skip themselves when no adapter is available.

## Known limitations

- Alpha and additive sprites are drawn in two groups, not interleaved in one depth order;
  translucent meshes are drawn before all sprites.
- Decals are plain quads: they hang over edges and are not clipped to the surface.
- No mipmaps, no shadows, no MSAA (by design for the look).
- Lighting uses at most 16 point lights per frame (nearest to the camera), not per object.
- The scene renders with a single directional key light.
