# VELOCIDE — design notes

A fast, style-driven retro multiplayer FPS on a small custom Rust engine. Everything (names,
characters, weapons, enemies, art, audio, UI) is original; art and audio are procedural.

This file holds architecture decisions, contracts between modules, network protocol notes and
tuning values. Area-specific notes live in `docs/*.md` and are linked from here.

## Workspace layout

| crate     | role | key deps |
|-----------|------|----------|
| `engine/` | platform (winit), input (+gilrs), rendering (wgpu/WGSL), audio (kira), assets (gltf, image, procedural), physics queries (parry3d), fixed timestep, hot reload (notify), debug UI (egui, `debug-ui` feature) | |
| `shared/` | the game simulation: input commands, movement, combat, enemies, style, level format, tuning, network protocol. Run by both client and server. | `engine` (no default features), glam, serde, ron, bincode |
| `server/` | authoritative server. Library (in-process for single-player / listen servers) + headless `velocide-server` binary | `shared`, renet, renet_netcode |
| `client/` | the game (`velocide` binary): rendering the world, input, prediction, interpolation, UI, audio | `engine` (client), `shared`, `server` (for in-process hosting), renet |

`engine` features: `client` (default: wgpu, winit, gilrs, kira, gltf, image) and `debug-ui`
(egui). The dedicated server builds `engine` with no default features, so it only gets
`physics`, `time` and `hotreload`.

Single-player always runs the server in-process and connects to it over loopback UDP, so there
is exactly one code path for SP, co-op and PvP.

## Conventions

- **Colour:** everything is shaded in gamma space. Texture, tint, fog, clear, sprite, flash and
  UI colours are sRGB-encoded 0..1 values; light colours are plain multipliers. Depth is
  reverse-Z.

- Right-handed, **Y up**, metres. Yaw 0 looks down **-Z**; positive yaw turns left
  (counter-clockwise from above); positive pitch looks up. `shared::input::look_dir`.
- Player position = feet (bottom of capsule). Capsule radius 0.4, height 1.8 (0.9 sliding).
- Simulation runs at a fixed **60 Hz** (`engine::time::FixedTimestep`); rendering interpolates
  with `alpha`. Nothing gameplay-related depends on frame rate.
- No `unsafe`, no `unwrap()` on anything that can fail at runtime because of user data/network
  input. `cargo build` and `cargo clippy --workspace --all-targets` must have zero warnings.
- `engine::assets_dir()` locates `assets/` (env `VELOCIDE_ASSETS` overrides).

## Original names

- **Game:** VELOCIDE. Players are *Shrikes*: chrome-plated, blood-fuelled machines.
- **Weapons** (each has primary fire + 2 alt-fire modes, cycled by pressing the slot key again):
  1. **Lancet** (hitscan sidearm) — alt modes: *Overcharge* (hold to charge a piercing beam),
     *Fan* (3-round horizontal fan).
  2. **Thresher** (hitscan scattergun) — alt modes: *Slug* (single heavy pellet with knockback),
     *Flak* (lobbed grenade; explodes; parryable).
  3. **Riveter** (projectile nailer) — alt modes: *Sawblade* (bouncing piercing disc),
     *Volley* (ring of rivets).
  4. **Cinderbox** (explosive launcher) — alt modes: *Sticky* (adhesive charge, detonate with alt
     again), *Mortar* (high arc, big blast).
  - **Fist**: punch / parry. **Prism**: a throwable mirrored shard; hitscan shots that hit it
    ricochet to the nearest enemy, chaining through other prisms for bonus damage.
- **Enemies:** *Husk* (fodder melee), *Spitter* (ranged projectile), *Gnat* (flying),
  *Bastion* (heavy armoured, telegraphed parryable slam).
- **Style ranks** (low → high): DULL, CRUDE, FERAL, VICIOUS, MERCILESS, GRISLY, SANGUINE,
  VELOCIDE.

## Module contracts

These are the seams between areas. Change them only deliberately and update this section.

### `engine::gfx` (plain data, no wgpu)
`Vertex`, `MeshData`, `TextureData`, `MeshId`, `TextureId`, `RetroSettings`, `Camera`, `Fog`,
`PointLight`, `DrawItem`, `Sprite`, `Decal`, `DebugLine`, `UiQuad`, `UiTexture`, `UiDrawList`,
`FrameDesc`. The game fills a `FrameDesc` each frame.

### `engine::render::Renderer`
```rust
Renderer::new(window: Arc<winit::window::Window>, settings: &RetroSettings) -> anyhow::Result<Renderer>
Renderer::new_headless(width: u32, height: u32, settings: &RetroSettings) -> anyhow::Result<Renderer>
fn resize(&mut self, width: u32, height: u32)
fn apply_settings(&mut self, settings: &RetroSettings)
fn create_mesh(&mut self, data: &MeshData) -> MeshId
fn update_mesh(&mut self, id: MeshId, data: &MeshData)
fn create_texture(&mut self, data: &TextureData) -> TextureId
fn white_texture(&self) -> TextureId
fn render(&mut self, frame: &FrameDesc, overlay: Option<&mut dyn OverlayPainter>) -> anyhow::Result<()>
fn capture_png(&mut self, path: &Path)           // saves the next rendered frame
fn reload_shaders(&mut self) -> Result<(), String> // from assets/shaders/*.wgsl, keeps old on error
fn device(&self) -> &wgpu::Device; fn queue(&self) -> &wgpu::Queue; fn surface_format(&self) -> wgpu::TextureFormat
fn window_size(&self) -> (u32, u32); fn internal_size(&self) -> (u32, u32)
// also: free_mesh, update_texture, free_texture, font_texture, read_pixels (headless),
// reload_shaders_from, set_shader_hot_reload, shader_error, stats, settings, adapter_name
trait OverlayPainter { fn paint(&mut self, device, queue, encoder, target_view, size_px: [u32; 2]); }
```

### `engine::app`
`run::<G: Game>(AppConfig) -> anyhow::Result<()>`; `trait Game { fn init(ctx: &mut EngineContext)
-> anyhow::Result<Self>; fn frame(&mut self, ctx: &mut EngineContext); fn window_event(&mut self,
ctx, event: &WindowEvent) -> bool { false }; fn exit(&mut self, ctx) {} }`. `EngineContext` holds
`window`, `renderer`, `input`, `frame_dt`, `time`, plus `set_cursor_grab`, `set_fullscreen`,
`quit`. The app does **not** own audio; the game creates `engine::audio::Audio` itself.

### `engine::input`
`InputState` (keys/mouse/gamepad: held, pressed-this-frame, released, raw mouse delta, scroll,
typed text), `Binding` (serde-able key / mouse button / gamepad button with display names).

### `engine::ui`
Immediate-mode widgets drawing into a `UiDrawList` (canvas height 360 units), bitmap font in
`ui::font` (`build_atlas()`, `glyph_uv()`, `GLYPH_W/H/ADVANCE`), independent of winit (the game
converts `InputState` into `ui::UiInput`).

### `engine::audio::Audio`
Never fails to construct (silently disabled if there is no audio device). `play(Sfx, PlayParams)`,
`play_2d`, `play_at`, `set_listener(pos, rot)`, `set_volumes(master, sfx, music)`,
`set_music(MusicTrack)`, `set_music_intensity(f32)`, `update(dt)`. All sounds are synthesised
at startup. Additions: `Audio::disabled()`, `stop(Sfx)`, `music_ready`, `active_voices`,
`PlayParams::at/with_volume/with_pitch`. Music: 6 sample-locked stems per track; stem thresholds
in `docs/audio.md`.

### `engine::physics::CollisionWorld`
Static boxes / convex hulls / trimeshes in a uniform XZ grid. `cast_capsule`, `cast_sphere`,
`cast_shape`, `cast_ray`, `line_blocked`, `intersects_capsule`, `capsule_contacts`. Colliders
have group bits (`GROUP_WORLD`, `GROUP_PLAYER_CLIP`, `GROUP_DOOR`) and can be disabled (doors).

### `shared::movement`
`MoveState` (all state that affects future movement — fully serialisable, no hidden state),
`MoveEnv { world: &CollisionWorld, jump_pads: &[JumpPad] }`, `MoveEvents`,
`step(&mut MoveState, &PlayerInput, &MoveEnv, &MovementTuning, dt) -> MoveEvents`.
Deterministic: same inputs + same start state ⇒ bit-identical result. External impulses
(explosions) are applied by the server to `MoveState::velocity` between steps.

Movement helpers for the client camera: `view_roll`, `fov_kick`, `landing_dip`. Full rules and
tuning rationale in `docs/movement.md`.

Physics note: `CollisionWorld` clips large box colliders to the query region (+2 m margin) before
parry queries, because parry's GJK tolerances scale with shape size (a 400 m floor produced cm
errors and bad normals otherwise).

### `shared::sim` / `shared::protocol` / `server` / `client::net`
- `SimWorld::step(inputs, tuning, history: Option<&dyn HitboxHistory>) -> Vec<SimEvent>`; tick
  pipeline: inputs → movement (`sim::player_move_step`, the single movement entry point used by
  server and prediction) → gameplay systems → world rules → events. `replicate()` fills a
  `NetFrame` of quantised `NetEntity`s (delta-compressed per client against its last ack).
- Combat: one damage pipeline `SimWorld::apply_damage`; weapons are pure tick-based
  (`step_weapons`) so clients can predict firing; enemies are data-driven (`assets/enemies/*.ron`).
  Player entity packing: `state` = weapon | mode<<4 | charge<<8; `aux` = hard | prisms<<8 |
  stickies<<12 | wall-yaw<<16; flags in `protocol::player_flags`. Details: `docs/combat.md`.
- Server: `Server::new(ServerConfig)`, `ServerHandle::spawn(config)` (own thread, used for SP and
  listen servers), `velocide-server` binary. Lag compensation in `server::lagcomp`.
- Client: `client::net::Session::{single_player, host, join}` → `NetClient` (`update(dt)` per
  frame, `tick(input)` per fixed tick — predicts and sends), `remote_entities()` (interpolated),
  `stats()` (`NetStats` for the debug overlay), `drain_events()`.
- Custom transport glue (`server::transport`, `client::net::transport`) wraps `renetcode` so the
  network simulator (`NetSimConfig { latency_ms (one-way), jitter_ms, loss_percent }`) can sit
  between the socket and netcode. Full protocol notes: `docs/network.md`.

### `shared::lobby` / `server::lobby` (match flow)
`GameMode`, `MatchSettings` (`clamp`, `validate` against the server's `LevelInfo` catalog),
`LobbyPhase { Lobby, Countdown, InMatch, Results }`, `LobbyState` (players with ready / ping /
host), messages `ClientMessage::Lobby(LobbyRequest)` / `ServerMessage::Lobby(LobbyUpdate)`.
The server owns the phase (`Server::start_match`, `end_match`, `return_to_lobby`; host-only
requests validated and rate-limited); a fresh `SimWorld` is built per match with the settings
in `SimWorld::match_settings` for the game-mode rules. Single-player has no lobby. Client:
`menus::lobby::LobbyScreen` ⇄ `InGame` hand the session over (`Handoff`). Full notes:
`docs/lobby.md`.

### `shared::level::LevelDef`
RON brush levels in `assets/levels/`: brushes (boxes with rotation, material, tint), lights,
spawns, checkpoints, doors, encounters (trigger, lock/unlock doors, waves of enemy spawns), exit,
jump pads, kill volumes, glTF props. `build_collision()` → `LevelCollision`.

## Docs per area
- `docs/gameplay.md` — combat, enemies, style and game-mode rules with default numbers.
- `docs/render.md`, `docs/ui.md`, `docs/audio.md`, `docs/movement.md`, `docs/network.md`,
  `docs/models.md`, `docs/client.md` (client architecture + extension points),
  `docs/levels.md` (shipping levels, generator, tests), `docs/combat.md`
  (written by each area as it is built).
- `docs/lobby.md` — lobby phases, messages, host rules, screens, how game modes plug in.

## Running it
- Single-player: `cargo run -p client --release -- --sp crucible_of_veins`
- Host: `cargo run -p client --release -- --host 47420` (opens the lobby; start the match
  there); friends: `--connect <ip>:47420`
- Dedicated server: `cargo run -p server --release --bin velocide-server -- --port 47420`
  (starts in the lobby; the first player to join hosts; `--autostart` starts when all are
  ready)
- Server + two clients locally in one command: `scripts/local_test.sh --netsim 100,20,2`
  (server `--autostart`, clients `--ready`: they reach the match after ~5 s)
