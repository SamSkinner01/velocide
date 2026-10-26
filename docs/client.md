# Game client (`client` crate, `velocide` binary)

The playable game on top of the engine, the shared simulation and the networking core
(`client::net`, see `docs/network.md`). Everything a player sees, hears and touches lives
here: menus, settings, controls, the first-person camera, HUD, chat, effects, audio hooks and
the debug overlay.

## Module map

| module | contents |
|---|---|
| `main.rs` | parses the command line (`cli`), loads settings, calls `app::run` |
| `app` | `VelocideApp` (`engine::app::Game`): the screen state machine, `Common` (app-wide state incl. the `ModelLibrary`), `FrameStats` |
| `frame` | `FrameBuffers` (reused draw lists → `FrameDesc`), `SceneParams`, `FrameIo` (engine inputs of a frame), `ScreenOutput`, `Transition` |
| `cli` | `LaunchOptions`, `StartMode`, `parse`, `USAGE` |
| `settings` | `Settings` (RON, persisted), `CROSSHAIRS`, `RESOLUTIONS` |
| `controls` | `Action`, `Bindings`, `WeaponSelect`, `WEAPONS`, `Controls` (look + per-tick `PlayerInput`) |
| `ui_input` | `UiInputMapper`: `InputState` → `ui::UiInput` (keyboard + gamepad navigation with repeat) |
| `bot` | `--bot` auto-pilot for multi-client testing |
| `menus::main_menu` | main menu: single player (level select), host (mode, level, port, max players), join, settings, notices |
| `menus::lobby` | `LobbyScreen`: the lobby of hosted / joined games, `Handoff` (docs/lobby.md) |
| `menus::results` | `ResultsScreen`: animated campaign / co-op / deathmatch results (docs/style_modes.md) |
| `menus::settings_menu` | settings screen (tabs Video/Retro, Audio, Controls with rebinding, Gameplay), reused by the pause menu |
| `menus::backdrop`, `menus::style` | the 3D scene behind the menu, title / shading helpers |
| `ingame` (`mod.rs`) | `InGame`: one `net::Session` + everything while playing |
| `ingame::camera` | `CameraRig`: eye height, FOV kick, view roll, landing dip, stair smoothing, shake |
| `ingame::feedback` | `Feedback`: screen shake (trauma), hitstop / visual time scale, screen flashes |
| `ingame::effects` | `Effects`: billboard particles with simple physics, decal ring buffer, presets |
| `ingame::viewmodel` | `Viewmodel`: first-person weapon + arms (`ModelLibrary::draw_viewmodel`) with bob, sway, switch, kick, punch / throw, Riveter spin, plus landing dip / slide tilt / dash pull-back |
| `ingame::world_render` | `WorldRender`: level via `LevelRender`, environment, model props (jump pads, checkpoints, exit gate), ghost-outline helpers |
| `ingame::combat` | combat presentation (`docs/combat.md` "client presentation contract"): `CombatClient`; `weapon_feel` (client-side `step_weapons` prediction → instant shots), `fx` (tracer beams, temporary lights, `Fx` playback at the render tick), `entities` (enemies, projectiles, blood bursts, prisms), `hud` (hit markers, damage numbers, direction, vignette, death screen, prisms / stickies / cooldowns) |
| `ingame::remote_players` | `RemotePlayers`: other players as animated Shrikes (`ModelLibrary::draw_player`), per-player `PlayerVisual`, `player_color` |
| `ingame::hud` | `Hud` + `HudInfo`: health, stamina, weapon, crosshair, speed, timer, ping/FPS, feed, banner, name tags |
| `ingame::chat`, `ingame::scoreboard`, `ingame::pause` | chat log + input, Tab scoreboard (pings, pluggable score columns), pause menu (host: return to lobby) |
| `ingame::match_flow` | lobby phase changes in game, results screen, scoreboard rows |
| `ingame::style_hud`, `ingame::mode_hud` | style meter + style feed + rank banner; game-mode state and HUD (checkpoints, downed / revive, extraction, deathmatch clock), score cells, results glue (docs/style_modes.md) |
| `ingame::audio_hooks` | local movement sounds from predicted `MoveEvents`, positional remote-player sounds |
| `ingame::debug`, `ingame::tuning_edit` | F3 overlay (egui with `debug-ui`, text otherwise), tuning editor |
| `level_mesh` | `LevelRender`: level geometry for the renderer |
| `models` | procedural models, rigs and poses (owned by the models area, `docs/models.md`) |
| `net` | networking core (owned by the network area) |

## Screens

```text
  MainMenu ──Start(single player)─────────────────────────▶ InGame
     │  ▲                                                     ▲  │  connecting → loading → playing
     │  └──── MainMenu{message} (leave, disconnect, error) ───┼──┤
     └──Start(host / join)──▶ Lobby ──EnterMatch (Handoff)───┘  │
                                ▲                               │
                                └──ReturnToLobby (Handoff)──────┘
```

Hosted and joined games go through the lobby screen, which owns the session between matches
and hands it to `InGame` when a level arrives (and back when the server returns to its lobby).
Details: `docs/lobby.md`.

`VelocideApp::frame` each frame:

1. Global hotkeys (`Fullscreen` F11, `DebugOverlay` F3) unless a text field / key capture owns
   the keyboard.
2. `UiInputMapper::build` → `UiInput`; `FrameBuffers::clear`.
3. The screen's `frame(&mut Common, &mut FrameIo, &mut FrameBuffers) -> ScreenOutput`.
4. UI focus-change sound; settings changes applied (`Renderer::apply_settings`, mixer
   volumes, fullscreen, `InGame::apply_settings`); settings saved when a screen asked.
5. Cursor grab follows `ScreenOutput::grab_cursor` (true only while actually playing: never
   in menus, chat, pause or for `--bot` clients).
6. `Audio::update`, test timers (`--capture`, `--quit-after`), egui overlay, `render`.
   Redraws follow the compositor's frame callbacks; when none come for 50 ms (minimised or
   hidden window on Wayland) the engine runs the frame anyway without drawing
   (`Renderer::skip_next_frame`), so networking and the game keep going (~20 frames/s).
7. The screen transition, after rendering. Leaving `InGame` calls `InGame::shutdown`
   (disconnect, stop the in-process server, free GPU resources).

Screens never touch the window: everything they need is in `FrameIo` (renderer, input, UI
input, dt, time, window size), so they run headless in tests and tools.

## In-game frame and tick flow (`InGame::frame`)

1. **Network**: `NetClient::update(real dt)` (receive, reconcile, interpolate), then
   `InGame::on_net_event` for every `NetEvent` (level loaded → `WorldRender::load`, spawned →
   adopt yaw, reset camera and timer, chat, joins/leaves → feed, tuning → debug editor,
   disconnect → main menu with the reason).
2. **Hotkeys**: Pause (Esc / Start), Chat (T / Enter), Restart (R, single-player), Scoreboard
   (hold Tab / Select).
3. **Look every frame**: `Controls::look` applies the raw mouse delta (× sensitivity,
   0.022°/count at 1.0) and the right stick (deg/s) to yaw / pitch immediately, so the camera
   is as responsive as the display. `Controls::latch` remembers presses between ticks (a tap
   shorter than a tick still jumps) and handles weapon selection.
4. **Fixed ticks** at the server's tick rate (`FixedTimestep`): `Controls::build` →
   `PlayerInput` with the latest angles, held + latched buttons, `weapon` / `weapon_mode` →
   `NetClient::tick` (predict + send) → predicted `MoveEvents` → `audio_hooks` and
   `InGame::on_local_move_events` (landing dip, stair smoothing, slam shockwave + shake +
   short hitstop, dust, jump-pad glow). Skipped while the single-player server is paused.
5. **Visual time**: `Feedback::update(real dt) -> visual dt` (0 during hitstop), then
   `combat_update` (drains `NetClient::drain_fx` into the Fx queue and plays what is due at
   the render tick, enemy / projectile bookkeeping, viewmodel charge + spin, music intensity).
   `CameraRig::update` places the camera at `local_render_position(alpha)` + smoothed eye
   height + landing dip + stair offset, with the FOV (settings, horizontal at 16:9, Hor+) +
   smoothed `fov_kick`, smoothed `view_roll`, and shake offsets. The viewmodel and effects
   advance on visual time.
6. **Draw** (`draw_world`, `draw_ui`): level (doors hidden while the replicated door state says
   open), remote players, combat entities + tracers (`combat_draw`), effects (sprites +
   decals), viewmodel, debug ghosts; name tags, HUD (+ `combat_hud`), chat log, scoreboard,
   then the interactive layer (connecting panel / pause menu / chat field) in one `Ui`. UI,
   feed, chat and combat HUD all use `InGame::time` as their clock.
7. **Audio**: listener = camera, positional remote-player movement sounds, music
   (`Menu` in menus, `Campaign` / `Arena` by `LevelKind` in game; intensity 0.2 → 1.0 from
   nearby alert enemies and recent combat, rising fast and decaying slowly, set in
   `combat_update`).

Mouse look is per frame, inputs are per tick: the input sent each tick carries the newest
angles, so the server always simulates the view the player saw last.

### Single-player pause and restart

Single-player is an in-process server, so the pause menu really pauses it:
`ServerCommand::SetPaused(true)` (server keeps flushing the transport so the connection stays
up, runs no ticks and sends no snapshots; on resume the input queue's jitter estimate is
reset). The client stops ticking and the level timer freezes. In multiplayer the pause menu
is just an overlay: the game keeps running and the client sends neutral inputs.

Restart (R or the pause menu) sends `ServerCommand::RestartLevel` → `SimWorld::restart_level`
puts every player back at the start and broadcasts `GameEvent::Respawned`; the client resets
the timer, clears effects and shows a banner when its `Spawned` arrives. Systems with level
state (encounters, enemies, pickups) reset themselves in `SimWorld::restart_level`.

## Extending the client

### Draw a new replicated entity kind (enemies, projectiles, pickups, prisms)

1. The simulation adds a `kind` constant and replicates it (see `docs/network.md`).
2. Add a module next to `ingame/remote_players.rs` (e.g. `ingame/enemies.rs`) with a
   `draw(&mut self, lib: Option<&ModelLibrary>, entities: impl Iterator<Item = &EntityState>,
   dt, out: &mut Vec<DrawItem>)` that builds the model's animation input from the entity
   (`EnemyAnim { kind, state, speed, cycle, .. }`, `lib.draw_enemy(&anim, EnemyAnim::root(pos,
   yaw), &Tint::default(), out)`; projectiles: `lib.draw_model(ModelId::Rocket, ..)`). Keep
   per-entity animation state (walk `cycle`, hit flash) in a `HashMap<NetId, _>` and drop
   unseen ids (see `RemotePlayers::draw`). The library lives in `common.models`.
3. Create it in `InGame::start` and call it in `InGame::draw_world` (where `lib` is already
   in scope) with `net.remote_entities().iter().filter(|e| e.kind == kind::GNAWER)`.
   Fast projectiles may extrapolate from `position + velocity * dt` using
   `NetClient::server_tick_estimate()`.
4. Positional sounds for state changes go in a tracker like `audio_hooks::RemoteSounds`;
   anything the server reports as an event goes in `on_net_event` (below).

Players: `RemotePlayers::draw` builds a `PlayerAnim` per remote player from the replicated
entity (`player_flags::{ON_GROUND, SLIDING, DASHING, SLAMMING, WALL_CLING}`, held weapon in
`state`, wall normal in `aux`) plus the client-kept walk `cycle`, and draws the Shrike with
`Tint::team(player_color(id))`; name tags hang above the drawn head. Combat drives the
third-person shot recoil / punch / downed / dead through `self.remote.visual_mut(id)`
(`firing = 1.0`, `punching = 0.01`, ...); `visual(id).muzzle` gives the world-space muzzle for
tracers. The local player's weapon: `self.viewmodel.fire(kick)`, `punch()`, `throw_prism()`,
`set_charge(c)`, `spinning = true` (Riveter); `viewmodel.last_draw()` has the view-space muzzle
/ fist (transform by the camera for world tracers).

### Combat presentation (`ingame::combat`)

Hooks in `InGame` (all in `ingame/combat/mod.rs`, an `impl InGame` block):
`combat_after_ticks` (after the tick loop: effects for the shots `WeaponPredictor::after_tick`
recorded right after each `NetClient::tick`), `combat_game_event` (`Damaged`, `HitConfirm`,
`Killed`, `Parried`, `Respawned`), `combat_update` (per frame, visual time), `combat_draw`,
`combat_hud`, `combat_health` (feeds `HudInfo`). `NetClient` additions: `own_combat()`
(`OwnCombat { tick, seq, combat }`, `dead_at(seq)`), `drain_fx()` (de-duplicated by `seq`);
`tick` predicts with `neutral_input` while dead. Own projectiles are drawn from the newest
snapshot at the present (see `docs/combat.md`), everything else at the render tick.
`InGame::send_chat` / `server_addr` exist for tools.

Look at it: `cargo run -p client --release --example combat_shots -- [out_dir]` plays a
scripted single-player session (dev commands spawn gnawers / needles, every weapon fires, a
parry, damage, death) plus a hosted game with a scripted rival client shooting at you, and
writes PNGs (`combat_lancet`, `combat_thresher`, `combat_explosion`, `combat_overcharge_beam`,
`combat_ricochet`, `combat_gnawer_windup`, `combat_parry`, `combat_hit_by_gnawer`,
`combat_low_health`, `combat_death`, `combat_remote_shot`, ...) and frame-time numbers.

### React to gameplay events (damage, kills, parries...)

Add a match arm in `InGame::on_net_event` for your `NetEvent::Game(GameEvent::...)` and call
the feedback / effects / audio / HUD APIs from there:

```rust
NetEvent::Game(GameEvent::Damaged { amount, .. }) => {
    self.feedback.damage_flash(amount / max_health);
    self.feedback.shake(0.2 + amount / 100.0);
    common.audio.play_2d(Sfx::Hurt);
}
NetEvent::Game(GameEvent::Parried { .. }) => {
    self.feedback.hitstop(0.12);
    self.feedback.parry_flash();
    common.audio.play_2d(Sfx::Parry);
}
NetEvent::Game(GameEvent::Killed { killer, victim, .. }) => {
    self.hud.event(format!("{killer} > {victim}"), t.blood_bright, now);
}
```

Owner-only state arrives in `OwnState`: combat's is exposed as `NetClient::own_combat()` and
already feeds health / hard damage into `HudInfo`; add new owner-only fields (style) the same
way.

### Feedback and effects API (`self.feedback`, `self.effects` on `InGame`)

| call | effect |
|---|---|
| `feedback.shake(trauma)` | adds 0..1 trauma; shake = trauma², decays at 1.4/s |
| `feedback.hitstop(seconds)` | freezes what is *drawn* (shake, viewmodel, particles) up to 0.35 s; never the simulation or networking |
| `feedback.set_time_scale(s)` | visual slow motion |
| `feedback.flash(color, strength)`, `damage_flash(fraction)`, `parry_flash()` | full-screen flash (red / white), fades in ~0.3 s |
| `effects.muzzle_flash(pos, dir)`, `sparks(pos, normal, n)`, `blood(pos, dir, amount)`, `gibs(pos, n)`, `smoke(pos, n)`, `dust(pos, n, spread)`, `shockwave(pos, radius)`, `glow(pos, color, size, life)` | presets |
| `effects.spawn(Particle { .. })` | anything custom (gravity, drag, size/alpha over life, spin, additive, collision: bounce / splat decal / die) |
| `effects.decal(pos, normal, size, DecalKind)` | ring buffer of 256 (oldest replaced) |
| `viewmodel.fire(kick)`, `punch()`, `throw_prism()`, `set_charge(c)` | first-person weapon animation |

All of it respects the player's `screen_shake` / `screen_flashes` / `hitstop` toggles.
Particles are capped at 2048 (oldest replaced); colliding particles cost one ray per frame.

### Add a HUD element

Add a field to `HudInfo` (`ingame/hud.rs`), fill it in `InGame::draw_hud`, draw it in
`Hud::draw` with the `engine::ui::hud` helpers (`style_meter`, `meter`, `banner`, `TextFeed`,
`readout`...). Stateful elements (feeds, banners, animations) live in `Hud`. `Hud::event` is the
top-right feed; `Hud::banner` a big centred announcement. Respect the canvas: 360 units tall,
width follows the aspect (640 at 16:9).

### Sounds for events

Local predicted movement: `audio_hooks::play_local_move_events`. Remote players:
`RemoteSounds`. Everything else: `common.audio.play_2d(Sfx::..)` for the local player,
`common.audio.play_at(Sfx::.., position)` for anything in the world, from `on_net_event`.
Music intensity: set it every frame from game state in `InGame::frame` (step 7).

### Menu screens and the lobby

- A main-menu page: add a `Page` variant, an entry in `ROOT_ITEMS` and a `fn xxx_page(&mut
  self, ui, panel, settings) -> Option<PageResult>` in `menus/main_menu.rs`.
- A full screen of its own (results, lobby): add a `Screen` variant in `app.rs` with a
  `frame(&mut Common, &mut FrameIo, &mut FrameBuffers) -> ScreenOutput` and a `Transition`.
- **Lobby**: `menus::lobby::LobbyScreen` (docs/lobby.md) creates host / join sessions,
  shows players / pings / ready-up, mode, level and host settings and chat, and hands the
  running session to `InGame::from_handoff` when the match starts; `InGame::into_handoff`
  gives it back. Results screen / scoreboard rows for game modes: `ingame::match_flow`.

## Controls

| action | keyboard / mouse | gamepad |
|---|---|---|
| move | W A S D | left stick |
| look | mouse | right stick (deg/s setting, quadratic response) |
| jump | Space | A |
| slide / slam | L Ctrl, C | B |
| dash | L Shift | LB |
| fire / alt fire | Mouse 1 / Mouse 2 | RT / LT |
| punch / parry | F | RB |
| throw prism | Q | Y |
| use / revive | E | X |
| weapons 1–4 | 1 2 3 4 (same key again = cycle alt mode) | — |
| next / prev weapon | wheel down / up | d-pad right / left |
| cycle fire mode | Mouse 3 | d-pad up |
| scoreboard | Tab (hold) | Select |
| chat | T, Enter | — |
| pause | Esc | Start |
| restart (solo) | R | — |
| debug overlay / fullscreen | F3 / F11 | — |

Every action can hold several bindings; the rebinding UI keeps one keyboard/mouse and one
gamepad binding per action (binding a key removes it from other actions). `PlayerInput::weapon`
is the 0-based slot (`WEAPONS[0]` = Lancet), `weapon_mode` the alt mode 0..2 of that weapon.

Menu navigation: arrows / d-pad / left stick move focus (with hold-repeat), Enter / A
activates, Esc / B goes back, PageUp/PageDown / shoulders scroll.

## Settings

`Settings` (RON) at `$VELOCIDE_CONFIG`, else `$XDG_CONFIG_HOME/velocide/settings.ron`, else
`~/.config/velocide/settings.ron` (`%APPDATA%` on Windows), else `./settings.ron`. Missing or
invalid files give defaults (never a crash), unknown fields are ignored, values are clamped.
Contains every `RetroSettings` toggle (resolution, snapping, affine, dithering, colour depth,
fog, vertex lights, vsync, shake, flashes, hitstop), FOV (horizontal at 16:9), weapon FOV,
mouse sensitivity / invert, gamepad look speed / invert, master / effects / music volume
(squared before the mixer), bindings, player name, last join address, host port and max
players, last level, HUD toggles (timer, speed, FPS, ping), crosshair, view bob, show weapon,
fullscreen. Changes apply live and are saved when leaving the settings screen (and when a game
starts from the menu).

## Command line

```text
velocide                                   main menu
velocide --sp [LEVEL]                      single-player (in-process server)
velocide --host [PORT] [--level L] [--max-players N] [--mode campaign|deathmatch] [--autostart]
velocide --connect IP:PORT [--name NAME] [--ready]
velocide --level L                         = --sp L
         --netsim LAT,JIT,LOSS             network simulator on this client (one-way ms)
         --windowed WxH | --fullscreen
         --bot                             auto-pilot (doesn't grab the mouse)
         --capture PATH.png [--capture-after S]   screenshot, then keep running
         --quit-after S                    clean exit
         --no-audio  --debug (F3 overlay open)  --add-bots N (host: N standing server bots)
         --ready                           ready up in the lobby automatically (--bot implies it)
         --autostart                       host: start the match once everyone is ready
```

## Debug overlay (F3)

With `debug-ui` (default feature) an egui window: FPS / frame time / worst frame, render stats
(draws, culled, sprites, triangles, internal resolution), audio voices, session and connection
state, RTT, packet loss, bandwidth in/out, snapshot rate (full / dropped), interpolation delay,
jitter, extrapolation, server tick vs render tick (and the gap in ms), unacked inputs,
prediction corrections (count / last / avg / max / max shift / snaps / current smoothing
offset), server vs predicted position, particles / decals, trauma / hitstop.

- **Ghosts** (toggle): red = server-authoritative capsule of the local player, green ring =
  predicted feet (the camera is inside the full capsule), blue = remote players' interpolated
  capsules.
- **Tuning editor**: every `MovementTuning` and `NetTuning` value as a drag field (filter,
  revert, defaults, auto-save). *Save + apply* rewrites only the changed numbers in
  `assets/tuning.ron` (comments survive). The server (in-process, or a dedicated server on the
  same machine) hot-reloads the file and broadcasts the tuning; the status line confirms when
  the server applied it, and prediction uses it from then on (tested end to end in
  `tests/client_tuning_reload.rs`). Remember: `shared` tests expect the committed file to
  match the defaults.

Press Esc to free the cursor and interact with the window. Without `debug-ui`
(`--no-default-features`) a compact text overlay shows the same key numbers.

## Testing and tools

```text
cargo test -p client                                   # unit + integration tests (GPU tests skip without an adapter)
cargo run -p client --release --example client_shots -- out/   # headless PNGs of every screen
cargo run -p client --release --example lobby_shots -- out/    # lobby (host / guest), countdown, match, results
cargo run -p client --release --example playthrough -- out/    # every player-facing flow end to end, PASS/FAIL
PLAYTHROUGH=sp,coop cargo run -p client --release --example playthrough   # only some: sp coop dm robust settings perf window
scripts/local_test.sh --release --level movement_test --netsim 100,20,2 --bot --debug \
    --quit-after 20 --capture-dir captures/mp --capture-after 15   # dedicated server + 2 clients
velocide --sp movement_test --bot --capture shot.png --capture-after 4 --quit-after 6
```

- `client_settings`: settings load/save/repair, CLI parsing.
- `client_controls`: weapon selection rule, taps between ticks, per-frame look, input
  building, bindings, the rebinding flow through the real settings screen.
- `client_ingame` (GPU): a bot-driven single-player session renders, the pause menu freezes
  the server and the timer, restart returns to the start; the main menu renders.
- `client_tuning_reload`: editor → file → server watcher → client prediction in sync.
- `lobby` (headless network) / `lobby_screen` (GPU): lobby and match flow, see
  `docs/lobby.md`.
- `client_combat`: client weapon prediction == server weapon state (0 mismatches at 100 ms
  RTT), own combat state follows damage, Fx arrive exactly once.
- unit tests: camera dip / slide eye height, feedback (hitstop, toggles, trauma decay),
  effects (decal ring, particle cap, blood splats), tuning file rewriting, UI repeat.

Measured: 2 bot clients on a dedicated server with `--netsim 100,20,2` (≈210 ms RTT, ±20 ms
jitter, 2 % loss each way): 0 prediction corrections over 20 s of running, jumping, dashing,
sliding, slamming and wall jumps; 28–30 snapshots/s, interpolation delay ≈100–107 ms, remote
players drawn ≈200 ms behind the server tick; ≈7.8 KB/s down, 12 KB/s up per client; 165 FPS
(vsync) at 960x540 on an RTX 4070 Ti SUPER.

## Server / shared additions made for the client

- `server::ServerCommand::SetPaused(bool)` / `Server::set_paused` (single-player pause: no
  ticks, no snapshots, transport keeps flushing; `InputQueue::reset_timing` on resume) and
  `ServerCommand::RestartLevel` / `Server::restart_level` → `SimWorld::restart_level`.
- Player entities replicate animation state: `player_flags::{ON_GROUND, SLIDING, DASHING,
  SLAMMING, WALL_CLING}`, `state` = held weapon | alt mode << 8, `aux` = wall-cling normal yaw.

## Known limitations

- Health / hard damage / style are placeholders until combat replicates them (`HudInfo`).
- Style is a placeholder until the style system replicates it (`HudInfo`).
- Scoreboard ping is only known for the local player (the protocol has no per-player ping).
- After a server-side teleport (restart, respawn) the `Spawned` event can arrive a snapshot
  before the corrected position, so the new view angle is shown at the old spot for a frame
  or two.
- The tuning editor writes the local `assets/tuning.ron`; it can't change a remote server's
  tuning.
- Key rebinding UI shows one keyboard/mouse and one gamepad binding per action (more can be
  added by editing the settings file).
- Text fields are ASCII only (the bitmap font).
