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

- **Game:** VELOCIDE. Premise (full text: `docs/premise.md`): the Foundry never stopped pouring;
  its wax-and-slag *Moulds* hold every scrap of heat left in the world. Players are *Shrikes*:
  masked hunters whose life is a flame (the *wick*) in a chest lantern. Healing is *kindling*
  (drawing in the embers that hits knock out of Moulds); hard damage is *smother*.
- **Weapons** (loadouts of up to four builds; a build = a base weapon with fixed stats + two
  abilities from its class's pool, cycled by pressing the slot key again; full reference:
  `docs/arsenal.md`, the second-generation design notes: `docs/roster.md`):
  - *Precision*: **Quill** (spike-caster, hitscan), **Lancet** (rapid needle pistol) — Brand,
    Splinter, Skewer, Flurry.
  - *Scatter*: **Bellows** (twin-barrel blast cannon), **Thresher** (drum-fed
    auto-scattergun) — Gust, Both Barrels, Slug, Flak.
  - *Rapid*: **Arclash** (chaining arc caster), **Riveter** (hot rivets), **Kiln** (ember
    sprayer, Burn) — Wisp, Discharge, Volley, Flare.
  - *Explosive*: **Tumbler** (bouncing bombs), **Cinderbox** (three-rocket salvo) — Breach,
    Gravewell, Limpet, Mortar.
  - *Draw*: **Longwick** (recurve bow) — Brand, Triple Nock, Pinning Arrow, Ember Arrow.
  - *Melee*: **Trimmer** (hooked wick-shears; fast slashes that deflect projectiles), **Tamper**
    (foundry ramming hammer; slow heavy arcs) — Lunge, Whirl, Hurl, Upswing. Arc hits are
    lag-compensated like hitscan; no weak points.
  - Lancet, Thresher, Riveter and Cinderbox are the first-generation names with new
    silhouettes, new primaries and no fixed alt set (their old signature combos are gone).
  - **Fist**: punch / parry.
  - **Knacks** (the Q slot, one per loadout, `shared::loadout::Knack`): **Thorn** (PvE only: a
    thrown relay spike that plants itself in surfaces; Quill shots that pass it ricochet to the
    next thorn, a branded target or the nearest enemy, ×1.5 damage per bounce), **Kindle Flask**
    (healing pool), **Snare** (floor trap that pins), **Soot Pot** (a cloud Moulds can't see
    through), **Tallow Skin** (damage reduction, no knockback), **Surge** (speed impulse). In a
    PvP world a knack that isn't `pvp_allowed` (the Thorn) becomes Surge.
  - **Balance per mode:** `Tuning::for_mode` applies the `balance.pve` / `balance.pvp` profile
    (per-weapon and per-ability multipliers + overrides); the server runs and sends that
    effective tuning, so prediction matches. Method, measurements and every change's reason:
    `docs/balance.md` (`cargo run -p shared --release --example balance_report`).
- **Enemies** (the Moulds, `docs/enemies.md`): *Gnawer* (fodder melee, pounces), *Censer*
  (ranged lobber, parryable ember bombs), *Harrier* (flying dive-bomber, swattable mid-dive),
  *Gaoler* (heavy shielded, parryable bash, jumpable sweep, lantern weak point), *Heron*
  (sniper with a replicated aim laser that locks before it fires), *Bellwether* (support: its
  bell frenzies nearby Moulds), *Snuffer* (latches on and smothers the wick until punched off),
  *Smelter* (heavy that pours slag lines on the floor), *Effigy* (warm-up training dummy).
  Any Mould can spawn as an *elite*.
- **Hit assist:** player-owned attacks test against hit shapes inflated by `combat.hit_assist`
  (enemies +0.35 m body / +0.15 m weak points; PvP players +0.12 / +0.06), and player
  projectiles test targets with `combat.projectile_hit_pad` (0.2 m) on top of their radius;
  enemy attacks don't. Mould hit shapes are built separately from the movement capsule and
  cover more than the drawn model (`EnemyDef::hitbox`, `docs/enemies.md`).
- **Style ranks** (low → high): DULL, CRUDE, FERAL, VICIOUS, MERCILESS, GRISLY, SEARING,
  VELOCIDE.
- **Levels:** campaigns *Crucible of Veins*, *Spire of Wicks* and *The Drowned Chandlery*;
  arenas *Cistern Nine*, *Knell Yard*, *Ingot Run*, *Tallow Works*, *Wickmarket* and *Furnace
  Ring*; the warm-up hub *Vestry* (`docs/levels.md`).
- **Modes:** Campaign, Grind (endless co-op waves), Deathmatch, Gun Game; PvE difficulty Easy →
  Inferno with player-count scaling; play of the game after PvP matches (`docs/style_modes.md`,
  `docs/potg.md`).
- **Cosmetics:** mask, crest, cloak, emblem, two colours, wick flame (`shared::cosmetics`;
  decorative only, separate from the identity colour).
- **Accessibility:** colour-vision modes (protan / deutan / tritan + strength) correct the 3D
  view in the post pass and switch gameplay-critical colours to a semantic palette
  (`client::palette`), plus shape cues (`docs/render.md`, `docs/ui.md`).

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
  Player entity packing: `state` = held weapon (bits 0–3) | ability (4–8) | charge / draw
  (9–15), through `protocol::pack_player_weapon`; `aux` = hard | knack charges<<8 |
  wall-yaw<<16; flags in `protocol::player_flags` (bit 15 `BRANDED`, 16 `TALLOW`). Details:
  `docs/combat.md`, `docs/arsenal.md`.
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
in `SimWorld::match_settings` for the game-mode rules. While in its lobby phase the server runs
the warm-up world (the *Vestry* hub, `SimWorld::warmup` / `warmup_rules`) and players play in it
with the lobby panel as an overlay; `--no-warmup` keeps the menu-style lobby. Single-player has
no lobby. Client: `menus::lobby::LobbyScreen` ⇄ `InGame` hand the session over (`Handoff`).
Per-player `ClientMessage::SetLoadout` / `SetCosmetics` are sanitised by the server. Full notes:
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
- `docs/style_modes.md` — style meter / ranks / freshness, game modes (checkpoints, downed &
  revive, exit, deathmatch), the death rule, scoreboard, results grading and screens.
- `docs/enemies.md` — the enemy types, AI states and telegraphs, nav grid, arena
  encounters, co-op scaling, elites, difficulty scaling, hooks for game modes, measurements.
- `docs/arsenal.md` — loadouts, builds, the ten weapons and nineteen abilities, Burn / Pinned,
  balance per mode, replication.
- `docs/potg.md` — highlight scoring, recording and the play-of-the-game replay.
- `docs/expansion.md` — the contract the expansion areas were built against.
- `docs/refit.md` — the contract of the refit (melee, knacks, feel, the loadout screen, readable
  Moulds, spectating, balance), with each area's as-built notes.
- `docs/feel.md` — projectile visibility, press-frame firing and stand-in projectiles, parry
  feedback, the Longwick's handling, the latency audit.
- `docs/balance.md` — the balance harness, the outliers it found, every number changed and why,
  the bands the tests hold.

## Running it
- Single-player: `cargo run -p client --release -- --sp crucible_of_veins`
- Host: `cargo run -p client --release -- --host 47420` (opens the lobby; start the match
  there); friends: `--connect <ip>:47420`
- Dedicated server: `cargo run -p server --release --bin velocide-server -- --port 47420`
  (starts in the lobby; the first player to join hosts; `--autostart` starts when all are
  ready)
- Server + two clients locally in one command: `scripts/local_test.sh --netsim 100,20,2`
  (server `--autostart`, clients `--ready`: they reach the match after ~5 s)

## Milestone status
All 14 milestones of `PROMPT.txt` are implemented. Each milestone was built by one area owner,
then merged and checked against the whole test suite with zero clippy warnings.

| # | Milestone | Where |
|---|---|---|
| 1 | Workspace, window, render loop, fixed timestep, input, debug overlay | `engine/` (app, input, render, time, debug_ui) |
| 2 | 3D scene, camera, glTF loading, low-res target, nearest upscale, fog | `engine::render`, `engine::assets::gltf`, `client::models` |
| 3 | Networked character controller, prediction/reconciliation, interpolation, net simulator | `shared::movement`, `server`, `client::net` |
| 4 | Advanced movement, tuning hot-reload, 2 clients at 100 ms | `shared::movement`, `assets/tuning.ron`, `scripts/local_test.sh` |
| 5 | Retro post-processing, viewmodel pass | `engine::render` (shaders in `assets/shaders/`) |
| 6 | First weapon, lag-compensated hits, server enemy, hitstop/shake | `shared::sim::{weapons,hitscan,enemy}`, `server::lagcomp`, `client::ingame::{feedback,combat}` |
| 7 | Parry with network tolerance, proximity healing, hard damage, explosions | `shared::sim::{parry,embers,combat,projectiles}` |
| 8 | Full weapon set with alt-fires, ricochet thorns | `shared::sim::{weapons,thorn,brand}` |
| 9 | Style meter, ranks, results with scoreboard | `shared::sim::style`, `client::ingame::{style_hud,match_flow}` |
| 10 | All enemy types, navigation, wave encounters, co-op scaling | `shared::sim::{enemy,nav,encounter}`, `assets/enemies/` |
| 11 | PvP deathmatch, respawns | `shared::sim::modes` |
| 12 | Lobby, host/join, chat, scoreboard, name tags | `shared::lobby`, `server::lobby`, `client::menus::lobby` |
| 13 | Audio and dynamic music | `engine::audio`, `client::ingame::audio_hooks` |
| 14 | Menus, settings, rebinding, campaign level, PvP arena | `client::menus`, `client::settings`, `assets/levels/` |

### After the milestones: roster, premise, levels, accessibility
The first weapons, enemies and premise mapped too closely onto another game's signature
designs, so all of them were replaced (`docs/roster.md`, `docs/premise.md`; the mechanics the
spec requires — parry, ricochet object, close-range healing, hard damage — stayed, with new
fiction and new designs). The same pass made enemies bigger and easier to read, made
player-owned hits more forgiving (hit assist), added the campaign *Spire of Wicks* and the
arenas *Knell Yard* and *Ingot Run*, and added colour-vision accessibility.
`PROTOCOL_VERSION` 7.

### Expansion: loadouts, modes, play of the game, warm-up, cosmetics, content
Built by seven parallel areas against `docs/expansion.md`: loadouts and customisable builds
(ten weapons, the first-generation four redesigned, the Longwick bow and the Kiln; nineteen
abilities; Burn and Pinned), separate PvE / PvP balance with no Thorns in PvP (`docs/arsenal.md`);
difficulty with stronger co-op scaling, Grind and Gun Game (`docs/style_modes.md`); play of the
game and kill distance in the feed (`docs/potg.md`); the warm-up lobby in the Vestry and
character cosmetics (`docs/lobby.md`); the Bellwether, Snuffer, Smelter, Effigy and elites
(`docs/enemies.md`); the levels The Drowned Chandlery, Tallow Works, Wickmarket and Furnace Ring
plus grind spawns on every arena (`docs/levels.md`). `PROTOCOL_VERSION` 8.

### Refit: melee, knacks, feel, the loadout screen, readable Moulds, spectating, balance
Built by six parallel areas against `docs/refit.md`: the melee class (Trimmer, Tamper, four
abilities) and gun game's melee finale replacing the Fist tier, the Q slot as a chosen knack
(six), all drawn and heard (`docs/arsenal.md`); bigger, glowing projectiles, press-frame firing
with local stand-in projectiles, instant parry feedback and a reworked Longwick
(`docs/feel.md`); Moulds drawn up to ×1.3 with hit shapes past the model and more hit assist,
and spectating a teammate while down in PvE (`docs/enemies.md`, `docs/style_modes.md`); a
redesigned LOADOUT screen with a turning 3D weapon and explained abilities (`docs/ui.md`); a
balance harness that measures the real sim and a rebalance of every weapon, ability and knack,
with tests that hold the bands (`docs/balance.md`). `PROTOCOL_VERSION` 9.

### Moulds that get there
The nav grid links leaps (across gaps, lava and deep water, up and down ledges, per-Mould
limits) and pad rides; ground Moulds crouch and leap or ride pads along their paths, prowl
the nearest edge or find a sight line when their target is out of reach, and when stranded
for 6 s melt into a wax puddle and re-form near their target (never in a kill volume or a room
cut off from it), so arenas and grind waves never stall; Harriers fly round or over what hides
their target. No wading (the shipped water is sheer-sided pits over the Moulds' heads). New
replicated states `AiState::{Leap, Melt, Reform}`, dev `/melt`, the measurements in
`docs/enemies.md` "Leaps, pads, melting". `PROTOCOL_VERSION` 10.

### Deliberate deviations from the spec
- **Netcode transport.** Connection authentication uses `renet_netcode`'s types
  (`ServerAuthentication`, `ClientAuthentication`, `ServerConfig`). The netcode.io state machine
  (`renetcode`, which `renet_netcode` wraps) is driven by about 100 lines of our own socket glue
  instead of `renet_netcode`'s transports. Those transports own the UDP socket and offer no hook
  where the network simulator could inject latency, jitter and loss for every packet, including
  handshakes and acks. The protocol on the wire is identical.
- **Lag compensation** rewinds up to 350 ms instead of ~250 ms. The rewind a shot needs is about
  RTT plus the interpolation delay, so 250 ms would only cover players up to ~140 ms RTT. The
  history buffer holds 1 s.
